//! Lazy tables: identity rows, provider hooks, loading and failed cells, bounded work for huge
//! row counts, and rows past the bitmaps.

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::BTreeSet,
    ops::Range,
};

use egui::{Event, Modifiers, PointerButton, Pos2};

use crate::error::TableError;
use crate::filter::Filter;
use crate::header::ColResponse;
use crate::interaction::{SelectionInput, SelectionMode, TableOutput};
use crate::operations::{
    BorrowedRow, CellStatus, HeaderIter, OperationContext, RowCallback, TableCell, TableOperation,
    TableProvider, Viewport, VisibleRows,
};
use crate::state::{ActiveRows, BITMAP_ROWS, TableState, row_bit, row_bits};
use crate::table::{LAZY_SIZING_ROWS, TableKit};

/// What a provider was asked, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Prepare {
        visible: Range<u64>,
        rows: Vec<usize>,
        columns: Vec<usize>,
    },
    Status(usize, usize),
    Cell(usize, usize),
}

/// A provider of `rows` × `columns` cells that records what it is asked. Rows can be lazy (it
/// panics if asked to walk them), cells loading or failed (it panics if asked for their values),
/// and columns unsortable or unfilterable (it panics if asked to sort or filter them).
struct Recording {
    rows: usize,
    columns: usize,
    lazy: bool,
    status: fn(usize, usize) -> CellStatus<'static>,
    sortable: fn(usize) -> bool,
    filterable: fn(usize) -> bool,
    calls: RefCell<Vec<Call>>,
    keys: Cell<usize>,
    /// The columns whose header tooltip was asked for, in order.
    tooltips: RefCell<Vec<usize>>,
}

impl Recording {
    fn new(rows: usize, columns: usize) -> Self {
        Self {
            rows,
            columns,
            lazy: false,
            status: |_, _| CellStatus::Ready,
            sortable: |_| true,
            filterable: |_| true,
            calls: RefCell::new(Vec::new()),
            keys: Cell::new(0),
            tooltips: RefCell::new(Vec::new()),
        }
    }

    fn lazy(rows: usize, columns: usize) -> Self {
        Self {
            lazy: true,
            sortable: |_| false,
            filterable: |_| false,
            ..Self::new(rows, columns)
        }
    }

    /// The rows whose cells were asked about (status or value).
    fn rows_touched(&self) -> BTreeSet<usize> {
        self.calls
            .borrow()
            .iter()
            .filter_map(|call| match call {
                Call::Status(row, _) | Call::Cell(row, _) => Some(*row),
                Call::Prepare { .. } => None,
            })
            .collect()
    }

    fn prepares(&self) -> Vec<Call> {
        self.calls
            .borrow()
            .iter()
            .filter(|call| matches!(call, Call::Prepare { .. }))
            .cloned()
            .collect()
    }
}

impl TableProvider for Recording {
    fn column_count(&self) -> usize {
        self.columns
    }

    fn header(&self, index: usize) -> Option<Cow<'_, str>> {
        (index < self.columns).then(|| Cow::Owned(format!("Column {index}")))
    }

    fn headers(&self) -> HeaderIter<'_> {
        HeaderIter::new(self)
    }

    fn header_tooltip(&self, index: usize) -> Option<Cow<'_, str>> {
        self.tooltips.borrow_mut().push(index);
        (index != 2).then(|| Cow::Owned(format!("About column {index}")))
    }

    fn header_subtitle(&self, index: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(format!("type {index}")))
    }

    fn row_count(&self) -> usize {
        self.rows
    }

    fn row_key(&self, row_index: usize) -> egui::Id {
        self.keys.set(self.keys.get() + 1);
        egui::Id::new(row_index)
    }

    fn prepare(&self, view: &Viewport<'_>) {
        self.calls.borrow_mut().push(Call::Prepare {
            visible: view.visible.clone(),
            rows: view.rows.iter().collect(),
            columns: view.columns.to_vec(),
        });
    }

    fn cell_status(&self, row: usize, col: usize) -> CellStatus<'_> {
        self.calls.borrow_mut().push(Call::Status(row, col));
        (self.status)(row, col)
    }

    fn cell_at(&self, row: usize, col: usize) -> Result<Option<TableCell<'_>>, TableError> {
        assert!(
            matches!((self.status)(row, col), CellStatus::Ready),
            "asked for the value of cell ({row}, {col}), which isn't ready"
        );
        self.calls.borrow_mut().push(Call::Cell(row, col));
        Ok(Some((Cow::Owned(format!("r{row}c{col}")), None)))
    }

    fn is_lazy(&self) -> bool {
        self.lazy
    }

    fn can_sort(&self, col: usize) -> bool {
        (self.sortable)(col)
    }

    fn can_filter(&self, col: usize) -> bool {
        (self.filterable)(col)
    }

    fn for_all_rows(&self, f: &mut RowCallback<'_>) -> Result<(), TableError> {
        assert!(!self.lazy, "walked every row of a lazy table");
        for row_index in 0..self.rows {
            f(&BorrowedRow {
                provider: self,
                row_index,
            })?;
        }
        Ok(())
    }

    fn for_selected_rows(
        &self,
        _state: &TableState,
        _f: &mut RowCallback<'_>,
    ) -> Result<(), TableError> {
        Ok(())
    }

    fn sort_active_rows(
        &self,
        active_rows: &mut Vec<usize>,
        col: usize,
        ascending: bool,
    ) -> Result<(), TableError> {
        assert!(
            (self.sortable)(col),
            "sorted by column {col}, which can't be"
        );
        assert!(!self.lazy, "sorted a lazy table");
        // Rows sort by their text, "r{row}c{col}": reversed is enough for these tests.
        active_rows.sort_unstable();
        if !ascending {
            active_rows.reverse();
        }
        Ok(())
    }

    fn filter_rows(
        &self,
        _state: &TableState,
        filters: &[(usize, Filter)],
    ) -> Result<Vec<usize>, TableError> {
        assert!(!filters.is_empty(), "asked to filter with no filter");
        for (col, _) in filters {
            assert!(
                (self.filterable)(*col),
                "filtered column {col}, which can't be"
            );
        }
        assert!(!self.lazy, "filtered a lazy table");
        Ok((0..self.rows)
            .filter(|row| {
                filters
                    .iter()
                    .all(|(col, filter)| filter.matches(&format!("r{row}c{col}"), None))
            })
            .collect())
    }
}

/// A frame of a 500×350 window showing `provider` with rows 20 tall, as `configure` says.
fn show(
    ctx: &egui::Context,
    provider: &Recording,
    state: &mut TableState,
    time: f64,
    events: Vec<Event>,
    configure: impl Fn(TableKit<'_>) -> TableKit<'_>,
) -> (TableOutput, egui::FullOutput) {
    let mut result = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(500.0, 350.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| {
            let table = TableKit::new("lazy", provider, &mut *state)
                .with_row_height(20.0)
                .with_max_height(Some(300.0));
            result = Some(configure(table).show_with_output(ui, |_, _, _, _| None));
        },
    );
    output.textures_delta.clear();
    (result.expect("shown").expect("no error"), output)
}

fn click(pos: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            pressed,
            button: PointerButton::Primary,
            modifiers: Modifiers::NONE,
        },
    ]
}

/// Every text drawn among `shapes`, with its colour.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Color32)> {
    fn collect(shape: &egui::Shape, out: &mut Vec<(String, egui::Color32)>) {
        match shape {
            egui::Shape::Text(text) => {
                let color = text
                    .galley
                    .job
                    .sections
                    .first()
                    .map_or(text.fallback_color, |section| section.format.color);
                out.push((text.galley.text().to_owned(), color));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes
        .iter()
        .for_each(|clipped| collect(&clipped.shape, &mut out));
    out
}

#[test]
fn active_rows_answer_alike_with_or_without_a_list() {
    let all = ActiveRows::All(5);
    let list = ActiveRows::from(vec![0, 1, 2, 3, 4]);
    assert!(all.is_identity() && !list.is_identity());
    assert_eq!(all, list, "the same rows, list or not");
    assert_eq!(all, [0, 1, 2, 3, 4]);
    for rows in [&all, &list] {
        assert_eq!(rows.len(), 5);
        assert_eq!(rows.get(3), Some(3));
        assert_eq!(rows.get(5), None);
        assert_eq!(rows.position(4), Some(4));
        assert_eq!(rows.position(9), None);
        assert_eq!(rows.iter().collect::<Vec<_>>(), [0, 1, 2, 3, 4]);
        assert_eq!(rows.iter_range(1..3).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(rows.iter_range(3..99).collect::<Vec<_>>(), [3, 4], "cut");
        assert_eq!(rows.iter().rposition(|row| row < 2), Some(1));
        assert_eq!(rows.iter().len(), 5);
    }
    assert_eq!(all.slice(1..3), VisibleRows::Identity(1..3));
    assert_eq!(list.slice(1..3), VisibleRows::Mapped(&[1, 2]));

    let mut sorted = ActiveRows::All(3);
    sorted.to_mut().reverse();
    assert_eq!(sorted, ActiveRows::Rows(vec![2, 1, 0]));
    assert_eq!(sorted.position(0), Some(2));
    assert_eq!(sorted.slice(0..2).span(), Some(1..=2));
    assert_ne!(sorted, ActiveRows::All(3));
    assert_eq!(sorted.clone().into_vec(), [2, 1, 0]);
    sorted.clear();
    assert!(sorted.is_empty());

    // A billion rows without a list.
    let huge = ActiveRows::All(1_000_000_000);
    assert_eq!(huge.position(999_999_999), Some(999_999_999));
    assert_eq!(huge.iter().next_back(), Some(999_999_999));
    assert_eq!(huge.slice(10..12).iter().collect::<Vec<_>>(), [10, 11]);
}

#[test]
fn identity_rows_stay_without_a_list_across_show_passes() {
    let ctx = egui::Context::default();
    for provider in [Recording::new(40, 3), Recording::lazy(1_000_000_000, 3)] {
        let mut state = TableState::new("identity", provider.rows);
        assert!(state.active_rows.is_identity());
        for frame in 0..4 {
            show(&ctx, &provider, &mut state, f64::from(frame), vec![], |t| t);
            assert_eq!(state.active_rows, ActiveRows::All(provider.rows));
            assert!(state.active_rows.is_identity(), "no list made");
            assert!(!state.filter_cache_dirty);
        }
    }
}

#[test]
fn clearing_the_filters_returns_to_every_row_in_order() {
    let provider = Recording::new(12, 2);
    let mut state = TableState::new("filters", 12);
    state.refresh_view(&provider).unwrap();
    assert!(state.active_rows.is_identity());

    let mut filtered = ColResponse::default();
    filtered.filtering.search.set_text("r1");
    filtered.filtering.search.open();
    state
        .process_responses(&provider, vec![filtered.clone()])
        .unwrap();
    assert_eq!(state.active_rows, [1, 10, 11]);
    state.refresh_view(&provider).unwrap();
    assert_eq!(state.active_rows, [1, 10, 11]);

    // Cleared: every row, without a list, and the provider isn't asked to filter nothing.
    state
        .process_responses(&provider, vec![ColResponse::default()])
        .unwrap();
    assert_eq!(state.active_rows, ActiveRows::All(12));
    state.refresh_view(&provider).unwrap();
    assert_eq!(state.active_rows, ActiveRows::All(12));

    // A sort keeps a list (sorted), and clearing it returns to every row in order.
    let sort = ColResponse {
        to_sort: true,
        ..ColResponse::default()
    };
    state.process_responses(&provider, vec![sort]).unwrap();
    assert_eq!(state.get_sort_state(), Some((0, true)));
    assert!(!state.active_rows.is_identity());
    state.columns[0].sort_up = None;
    state.filter_cache_dirty = true;
    state.refresh_view(&provider).unwrap();
    assert_eq!(state.active_rows, ActiveRows::All(12));
}

#[test]
fn prepare_comes_first_each_pass_with_provider_rows_and_columns() {
    let ctx = egui::Context::default();
    let provider = Recording::new(100, 3);
    let mut state = TableState::new("prepare", 100);
    // Shown in reverse: visible index i shows row 99 - i.
    state.columns.resize_with(3, Default::default);
    state.columns[0].sort_up = Some(false);
    state.filter_cache_dirty = true;
    for frame in 0..3 {
        provider.calls.borrow_mut().clear();
        let passes = ctx.cumulative_pass_nr();
        show(&ctx, &provider, &mut state, f64::from(frame), vec![], |t| {
            t.with_column_map(vec![2, 0]).with_columns(vec![
                crate::layout::Column::new(120.0),
                crate::layout::Column::new(120.0),
            ])
        });
        let passes = ctx.cumulative_pass_nr() - passes;
        let calls = provider.calls.borrow();
        assert!(
            matches!(calls.first(), Some(Call::Prepare { .. })),
            "prepare comes before any cell"
        );
        assert_eq!(provider.prepares().len() as u64, passes, "once per pass");
        let Some(Call::Prepare {
            visible,
            rows,
            columns,
        }) = calls.first().cloned()
        else {
            unreachable!()
        };
        assert_eq!(columns, [2, 0], "provider columns, through the map");
        assert_eq!(visible.start, 0);
        assert!(visible.end > 5 && visible.end < 30, "{visible:?}");
        let expected: Vec<usize> = (visible.start..visible.end)
            .map(|v| 99 - v as usize)
            .collect();
        assert_eq!(rows, expected, "the provider rows shown");
        // Every cell asked about is a drawn one.
        for call in calls.iter() {
            if let Call::Status(row, col) | Call::Cell(row, col) = call {
                assert!(rows.contains(row) && columns.contains(col), "{call:?}");
            }
        }
    }
}

#[test]
fn loading_and_failed_cells_are_drawn_without_their_values() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut provider = Recording::new(20, 3);
    provider.status = |row, col| match (row, col) {
        (_, 0) => CellStatus::Loading,
        (1, 1) => CellStatus::Failed(Cow::Borrowed("disk on fire")),
        _ => CellStatus::Ready,
    };
    let mut state = TableState::new("status", 20);
    state.selection_mode = SelectionMode::Explorer;
    let mut custom = Vec::new();
    let mut output = None;
    for frame in 0..2 {
        let mut result = None;
        let mut full = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(500.0, 350.0),
                )),
                time: Some(f64::from(frame)),
                ..Default::default()
            },
            |ui| {
                result = Some(
                    TableKit::new("lazy", &provider, &mut state)
                        .with_row_height(20.0)
                        .with_columns(vec![
                            crate::layout::Column::new(120.0),
                            crate::layout::Column::new(120.0),
                            crate::layout::Column::new(120.0),
                        ])
                        .show_with_output(ui, |_, cell, row, _| {
                            custom.push((row.row_index(), cell.col_nr));
                            None
                        }),
                );
            },
        );
        full.textures_delta.clear();
        output = Some((result.unwrap().unwrap(), full));
    }
    let (table, full) = output.unwrap();

    // The custom renderer, like `cell_at`, only saw ready cells.
    assert!(!custom.is_empty());
    for (row, col) in &custom {
        assert!(*col != 0 && (*row, *col) != (Some(1), 1), "{row:?}, {col}");
    }
    let texts = texts(&full.shapes);
    let error = egui::Visuals::dark().error_fg_color;
    assert!(texts.contains(&("Failed".to_owned(), error)), "{texts:?}");
    assert!(
        texts.iter().any(|(text, _)| text == "r2c1"),
        "ready cells drawn"
    );
    assert!(
        !texts
            .iter()
            .any(|(text, _)| text.starts_with('r') && text.ends_with("c0")),
        "no loading cell's value"
    );

    // A faint bar in each loading cell, about 60% of it wide.
    let faint = egui::Visuals::dark()
        .widgets
        .inactive
        .text_color()
        .gamma_multiply(0.18);
    let bars: Vec<egui::Rect> = full
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill == faint => Some(rect.rect),
            _ => None,
        })
        .collect();
    assert!(bars.len() >= table.rows.len(), "{} bars", bars.len());
    let width = 120.0 - 16.0; // the cell less its padding
    for bar in &bars {
        assert!((bar.width() - 0.6 * width).abs() < 2.0, "{bar:?}");
        assert!(bar.height() < 20.0);
    }

    // Labelled for screen readers.
    let update = full.platform_output.accesskit_update.expect("accesskit");
    let labels: Vec<String> = update
        .nodes
        .iter()
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect();
    assert!(labels.iter().any(|label| label == "Loading"), "{labels:?}");
    assert!(
        labels.iter().any(|label| label.contains("disk on fire")),
        "{labels:?}"
    );

    // A loading cell's row is clicked and selected like any other.
    let loading_cell = table.rows[3].hit_regions[0].center();
    show(
        &ctx,
        &provider,
        &mut state,
        2.0,
        click(loading_cell, true),
        |t| {
            t.with_columns(vec![
                crate::layout::Column::new(120.0),
                crate::layout::Column::new(120.0),
                crate::layout::Column::new(120.0),
            ])
        },
    );
    show(
        &ctx,
        &provider,
        &mut state,
        2.05,
        click(loading_cell, false),
        |t| {
            t.with_columns(vec![
                crate::layout::Column::new(120.0),
                crate::layout::Column::new(120.0),
                crate::layout::Column::new(120.0),
            ])
        },
    );
    assert_eq!(state.selected_rows.iter().collect::<Vec<_>>(), [3]);
}

#[test]
fn a_billion_row_lazy_table_touches_only_the_rows_it_draws() {
    let ctx = egui::Context::default();
    let mut provider = Recording::lazy(1_000_000_000, 8);
    provider.status = |row, col| {
        if (row + col) % 3 == 0 {
            CellStatus::Loading
        } else {
            CellStatus::Ready
        }
    };
    let mut state = TableState::new("billion", provider.rows);
    // Columns fitted to their contents: the first frame is a full sizing pass.
    let fitted = || {
        (0..8)
            .map(|_| crate::layout::Column::new(80.0).auto_fit(true))
            .collect::<Vec<_>>()
    };
    let passes = ctx.cumulative_pass_nr();
    show(&ctx, &provider, &mut state, 0.0, vec![], |t| {
        t.with_columns(fitted())
    });
    assert!(ctx.cumulative_pass_nr() - passes >= 2, "a sizing pass ran");
    let touched = provider.rows_touched();
    assert!(!touched.is_empty());
    assert!(
        touched.len() <= LAZY_SIZING_ROWS as usize,
        "{} rows touched",
        touched.len()
    );
    assert!(
        touched.len() < 40,
        "only the visible ones: {}",
        touched.len()
    );
    let cells = provider.calls.borrow().len();
    assert!(cells < 40 * 8 * 2 * 3, "{cells} calls");
    assert!(state.active_rows.is_identity());

    // Scrolled far down: still only what is drawn.
    provider.calls.borrow_mut().clear();
    for frame in 1..4 {
        show(&ctx, &provider, &mut state, f64::from(frame), vec![], |t| {
            t.with_columns(fitted())
                .with_scroll_to_row(987_654_321, egui::Align::Center)
        });
    }
    // An `f32` scroll offset lands within about 120 rows of a row this far down (see
    // `layout::table::VerticalMap`).
    let near = |row: &usize| row.abs_diff(987_654_321) < 200;
    let touched = provider.rows_touched();
    assert!(touched.iter().any(near), "{touched:?}");
    assert!(touched.len() < 3 * 40, "{} rows touched", touched.len());
    assert!(
        touched.iter().all(|row| near(row) || *row < 40),
        "{touched:?}"
    );
}

#[test]
fn columns_that_cannot_sort_or_filter_offer_no_controls_and_are_never_asked() {
    let ctx = egui::Context::default();
    let mut provider = Recording::new(10, 2);
    provider.sortable = |col| col != 0;
    provider.filterable = |col| col != 0;
    let mut state = TableState::new("controls", 10);
    let columns = || {
        vec![
            crate::layout::Column::new(150.0),
            crate::layout::Column::new(150.0),
        ]
    };
    // The header's row is 20 tall at the table's top; columns are 150 wide.
    let (first, _) = show(&ctx, &provider, &mut state, 0.0, vec![], |t| {
        t.with_columns(columns())
    });
    let origin = first.response.rect.min;
    let header = |col: f32| origin + egui::vec2(75.0 + 150.0 * col, 10.0);

    let mut sort_glyphs = |col: f32, time: f64| {
        show(
            &ctx,
            &provider,
            &mut state,
            time,
            vec![Event::PointerMoved(header(col))],
            |t| t.with_columns(columns()),
        );
        let (_, full) = show(
            &ctx,
            &provider,
            &mut state,
            time + 0.05,
            vec![Event::PointerMoved(header(col))],
            |t| t.with_columns(columns()),
        );
        texts(&full.shapes)
            .into_iter()
            .filter(|(text, _)| text == "🔻" || text == "🔺")
            .count()
    };
    assert_eq!(sort_glyphs(0.0, 0.5), 0, "no sort control on column 0");
    assert_eq!(sort_glyphs(1.0, 1.0), 1, "column 1 can sort");

    // Clicking column 0's title doesn't sort; column 1's does.
    for (col, time) in [(0.0, 2.0), (1.0, 3.0)] {
        show(
            &ctx,
            &provider,
            &mut state,
            time,
            click(header(col), true),
            |t| t.with_columns(columns()),
        );
        show(
            &ctx,
            &provider,
            &mut state,
            time + 0.05,
            click(header(col), false),
            |t| t.with_columns(columns()),
        );
        show(&ctx, &provider, &mut state, time + 0.1, vec![], |t| {
            t.with_columns(columns())
        });
    }
    assert_eq!(state.get_sort_state(), Some((1, true)));

    // Right-clicking column 0 opens no menu (it has neither control); column 1's opens.
    let mut menu_opens = |col: f32, time: f64| {
        let secondary = |pressed| {
            vec![
                Event::PointerMoved(header(col)),
                Event::PointerButton {
                    pos: header(col),
                    pressed,
                    button: PointerButton::Secondary,
                    modifiers: Modifiers::NONE,
                },
            ]
        };
        show(&ctx, &provider, &mut state, time, secondary(true), |t| {
            t.with_columns(columns())
        });
        show(
            &ctx,
            &provider,
            &mut state,
            time + 0.05,
            secondary(false),
            |t| t.with_columns(columns()),
        );
        let (_, full) = show(&ctx, &provider, &mut state, time + 0.1, vec![], |t| {
            t.with_columns(columns())
        });
        texts(&full.shapes)
            .iter()
            .any(|(text, _)| text == "Column Options")
    };
    assert!(!menu_opens(0.0, 4.0), "no menu on column 0");
    assert!(menu_opens(1.0, 5.0), "column 1's menu");

    // A sort and a filter set on column 0 some other way are left out: the provider is never
    // asked to sort or filter by it (it would panic).
    state.columns[1].sort_up = None;
    state.columns[0].sort_up = Some(true);
    state.columns[0].response.filtering.search.set_text("r1");
    state.columns[0].response.filtering.search.open();
    state.filter_cache_dirty = true;
    state.refresh_view(&provider).unwrap();
    assert_eq!(state.active_rows, ActiveRows::All(10));
    state.apply_new_sort(&provider, 0).unwrap();
    assert_eq!(state.active_rows, ActiveRows::All(10));
}

#[test]
fn filters_of_columns_out_of_view_are_kept() {
    let ctx = egui::Context::default();
    let provider = Recording::new(10, 20);
    let mut state = TableState::new("wide", 10);
    state.columns.resize_with(20, Default::default);
    state.columns[0].response.filtering.search.set_text("r1");
    state.columns[0].response.filtering.search.open();
    state.filter_cache_dirty = true;
    for (frame, scroll) in [false, false, true, true, true].into_iter().enumerate() {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                time: Some(frame as f64),
                ..Default::default()
            },
            |ui| {
                let mut collected = Vec::new();
                let mut halt = None;
                let (mut clicked, mut secondary) = (None, None);
                state.refresh_view(&provider).unwrap();
                let mut table = crate::layout::Table::new()
                    .id_salt("wide")
                    .num_rows(state.active_rows.len() as u64)
                    .columns(
                        (0..20)
                            .map(|_| crate::layout::Column::new(120.0))
                            .collect::<Vec<_>>(),
                    )
                    .headers([crate::layout::HeaderRow::new(20.0)]);
                if scroll {
                    table = table.scroll_to_column(15, Some(egui::Align::Min));
                }
                {
                    let mut delegate = crate::delegate::TableKitDelegate::new(
                        &provider,
                        &mut state,
                        &[],
                        &[],
                        &mut collected,
                        &mut halt,
                        None,
                        &mut clicked,
                        &mut secondary,
                    );
                    table.show(ui, &mut delegate);
                }
                assert!(
                    !scroll || collected.len() > 1,
                    "a later column's header was drawn"
                );
                state.process_responses(&provider, collected).unwrap();
            },
        );
        output.textures_delta.clear();
        assert!(
            !state.columns[0].response.filtering.is_empty(),
            "frame {frame}: the filter is kept"
        );
        assert_eq!(state.active_rows, [1], "frame {frame}");
    }
}

#[test]
fn keyboard_selection_in_a_lazy_table_never_searches_its_rows() {
    const AT: usize = 500_000_000;
    let provider = Recording::lazy(1_000_000_000, 2);
    let mut state = TableState::new("keys", provider.rows);
    state.selection_mode = SelectionMode::Explorer;
    state.select_row_at_visible(
        &provider,
        AT,
        Some(AT),
        Modifiers::NONE,
        SelectionInput::Click,
    );
    state.select_row_at_visible(
        &provider,
        AT + 3,
        Some(AT + 3),
        Modifiers::SHIFT,
        SelectionInput::Click,
    );
    // Shift+Down twice extends the range from the focused row, not from the top.
    for expected in [AT + 4, AT + 5] {
        assert_eq!(
            state.move_focus(&provider, 1, Modifiers::SHIFT),
            Some(expected)
        );
    }
    assert_eq!(
        state.selected_rows.iter().collect::<Vec<_>>(),
        (AT..=AT + 5).map(|row| row as u32).collect::<Vec<_>>()
    );
    assert_eq!(
        state.move_focus(&provider, -1, Modifiers::NONE),
        Some(AT + 4)
    );
    assert_eq!(
        state.selected_rows.iter().collect::<Vec<_>>(),
        [(AT + 4) as u32]
    );
    assert!(
        provider.keys.get() < 100,
        "{} keys asked",
        provider.keys.get()
    );
}

#[cfg(target_pointer_width = "64")]
#[test]
fn rows_past_the_bitmaps_are_never_selected_or_taken_for_others() {
    const PAST: usize = (1 << 32) + 5;
    let provider = Recording::lazy((1 << 32) + 10, 2);
    assert_eq!(row_bit(PAST), None);
    assert_eq!(row_bit(5), Some(5));
    assert_eq!(row_bits(provider.rows), Some(0..=u32::MAX));
    assert_eq!(row_bits(0), None);

    for mode in [SelectionMode::Legacy, SelectionMode::Explorer] {
        let mut state = TableState::new("past", provider.rows);
        state.selection_mode = mode;
        state.select_row(&provider, PAST, Modifiers::NONE, SelectionInput::Click);
        assert!(state.selected_rows.is_empty(), "{mode:?}: row 5 isn't it");
        state.select_row(&provider, 5, Modifiers::NONE, SelectionInput::Click);
        state.select_row(&provider, PAST, Modifiers::COMMAND, SelectionInput::Click);
        state.handle_row_selection(Modifiers::COMMAND, PAST);
        assert_eq!(
            state.selected_rows.iter().collect::<Vec<_>>(),
            [5],
            "{mode:?}"
        );
        assert!(state.is_selected(5) && !state.is_selected(PAST));
        assert_ne!(state.focused_key, Some(provider.row_key(PAST)));

        // The last row there is a bit for is the last that can be focused.
        let last = state.move_focus(&provider, isize::MAX, Modifiers::NONE);
        assert_eq!(last, Some(u32::MAX as usize), "{mode:?}");
    }

    // Highlights and expansion leave such rows out too.
    let mut state = TableState::new("past", provider.rows);
    state.highlights.insert(1, 5);
    state.expanded_rows.insert(5);
    assert_eq!(state.highlights.get_usize(5), Some(1));
    assert_eq!(state.highlights.get_usize(PAST), None);
    assert!(state.is_expanded(5) && !state.is_expanded(PAST));

    // Selecting every row selects every row there is a bit for, and no other.
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut context = OperationContext {
            ui,
            data: &mut state,
            provider: &provider,
        };
        crate::operations::SelectAll.exec(&mut context).unwrap();
        assert_eq!(context.data.selected_rows.len(), BITMAP_ROWS);
        crate::operations::FilterDeSelectAll
            .exec(&mut context)
            .unwrap();
        assert!(context.data.selected_rows.is_empty());
        crate::operations::FilterSelectAll
            .exec(&mut context)
            .unwrap();
        assert_eq!(context.data.selected_rows.len(), BITMAP_ROWS);
    });
    output.textures_delta.clear();

    // Drawn, row 2^32 + 5 isn't shown as selected when row 5 is, and clicking it selects nothing.
    let ctx = egui::Context::default();
    ctx.global_style_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
    let mut state = TableState::new("past", provider.rows);
    state.selection_mode = SelectionMode::Explorer;
    state.selected_rows.insert(5);
    let fill = egui::Visuals::dark().selection.bg_fill;
    let mut target = None;
    for frame in 0..4 {
        let (table, full) = show(&ctx, &provider, &mut state, f64::from(frame), vec![], |t| {
            t.with_scroll_to_row(PAST as u64, egui::Align::Center)
        });
        // The first frames are drawn before the scroll lands, with row 5 (selected) in view.
        if frame < 2 {
            continue;
        }
        assert!(table.rows.iter().all(|row| row.row_index > 1 << 31));
        let painted = full
            .shapes
            .iter()
            .any(|clipped| matches!(&clipped.shape, egui::Shape::Rect(rect) if rect.fill == fill));
        assert!(!painted, "no row drawn is selected");
        target = Some(table.rows);
    }
    let rows = target.expect("drawn");
    let at = |row: usize| {
        rows.iter()
            .find(|drawn| drawn.row_index == row)
            .unwrap_or_else(|| panic!("row {row} is drawn"))
            .hit_regions[0]
            .center()
    };
    fn scrolled(table: TableKit<'_>) -> TableKit<'_> {
        table.with_scroll_to_row(PAST as u64, egui::Align::Center)
    }
    show(
        &ctx,
        &provider,
        &mut state,
        4.0,
        click(at(PAST), true),
        scrolled,
    );
    show(
        &ctx,
        &provider,
        &mut state,
        4.05,
        click(at(PAST), false),
        scrolled,
    );
    assert_eq!(state.selected_rows.iter().collect::<Vec<_>>(), [5]);
    // The last row with a bit, beside it, is clicked and selected as usual.
    let last = u32::MAX as usize;
    show(
        &ctx,
        &provider,
        &mut state,
        5.0,
        click(at(last), true),
        scrolled,
    );
    show(
        &ctx,
        &provider,
        &mut state,
        5.05,
        click(at(last), false),
        scrolled,
    );
    assert_eq!(state.selected_rows.iter().collect::<Vec<_>>(), [u32::MAX]);
}

#[test]
fn every_column_sticky_still_prepares_before_its_cells() {
    use crate::layout::{CellInfo, Column, HeaderCellInfo, HeaderRow, PrefetchInfo, Table};

    #[derive(Default)]
    struct Order {
        calls: Vec<&'static str>,
        cells: Cell<usize>,
    }
    impl crate::layout::TableDelegate for Order {
        fn prepare(&mut self, _info: &PrefetchInfo) {
            self.calls.push("prepare");
        }
        fn header_cell_ui(&mut self, _ui: &mut egui::Ui, _cell: &HeaderCellInfo) {}
        fn cell_ui(&mut self, _ui: &mut egui::Ui, _cell: &CellInfo) {
            if self.calls.last() != Some(&"cell") {
                self.calls.push("cell");
            }
            self.cells.set(self.cells.get() + 1);
        }
    }

    let ctx = egui::Context::default();
    let mut order = Order::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(500.0, 350.0),
            )),
            ..Default::default()
        },
        |ui| {
            Table::new()
                .num_sticky_cols(2)
                .num_rows(10)
                .columns(vec![Column::new(100.0), Column::new(100.0)])
                .headers([HeaderRow::new(20.0)])
                .show(ui, &mut order);
        },
    );
    output.textures_delta.clear();
    assert_eq!(order.calls, ["prepare", "cell"]);
    assert!(order.cells.get() > 0);
}

/// Where `wanted` is drawn among `shapes`, if it is.
fn drawn_at(shapes: &[egui::epaint::ClippedShape], wanted: &str) -> Option<egui::Rect> {
    fn find(shape: &egui::Shape, wanted: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => {
                Some(text.visual_bounding_rect())
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, wanted)),
            _ => None,
        }
    }
    shapes
        .iter()
        .find_map(|clipped| find(&clipped.shape, wanted))
}

#[test]
fn sticky_columns_stay_in_place_and_are_prepared_first() {
    let ctx = egui::Context::default();
    let provider = Recording::lazy(100, 8);
    let mut state = TableState::new("sticky", 100);
    // Provider column 7 is shown first, and stays while the others scroll.
    fn configure(table: TableKit<'_>) -> TableKit<'_> {
        table
            .with_columns((0..8).map(|_| crate::layout::Column::new(120.0)).collect())
            .with_column_map(vec![7, 0, 1, 2, 3, 4, 5, 6])
            .with_sticky_columns(1)
    }
    let (_, full) = show(&ctx, &provider, &mut state, 0.0, vec![], configure);
    let sticky_x = drawn_at(&full.shapes, "r1c7")
        .expect("sticky cell drawn")
        .left();
    let first_x = drawn_at(&full.shapes, "r1c0")
        .expect("first column drawn")
        .left();
    assert!(sticky_x < first_x);
    let Some(Call::Prepare { columns, .. }) = provider.prepares().last().cloned() else {
        panic!("prepared");
    };
    assert_eq!(columns.first(), Some(&7), "{columns:?}");

    // Scroll sideways past the first scrolling columns.
    let over = egui::pos2(300.0, 150.0);
    let wheel = vec![
        Event::PointerMoved(over),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(-400.0, 0.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        },
    ];
    let mut full = full;
    for frame in 1..12 {
        let events = if frame == 1 { wheel.clone() } else { vec![] };
        full = show(
            &ctx,
            &provider,
            &mut state,
            f64::from(frame) * 0.1,
            events,
            configure,
        )
        .1;
    }
    assert!(drawn_at(&full.shapes, "r1c0").is_none(), "scrolled out");
    let still = drawn_at(&full.shapes, "r1c7").expect("sticky cell still drawn");
    assert!((still.left() - sticky_x).abs() < 0.5, "{still:?}");
    let Some(Call::Prepare { columns, .. }) = provider.prepares().last().cloned() else {
        panic!("prepared");
    };
    assert_eq!(columns.first(), Some(&7), "{columns:?}");
    assert!(!columns.contains(&0), "{columns:?}");
}

#[test]
fn header_tooltips_are_asked_for_only_while_hovered() {
    let ctx = egui::Context::default();
    let provider = Recording::lazy(100, 3);
    let mut state = TableState::new("tooltips", 100);
    fn configure(table: TableKit<'_>) -> TableKit<'_> {
        table.with_columns((0..3).map(|_| crate::layout::Column::new(120.0)).collect())
    }
    let (_, full) = show(&ctx, &provider, &mut state, 0.0, vec![], configure);
    assert!(provider.tooltips.borrow().is_empty(), "not hovered");
    let header = drawn_at(&full.shapes, "Column 1").expect("header drawn");

    // Rest on the second column's header past the tooltip delay.
    let mut full = full;
    for (frame, time) in [1.0, 1.1, 1.8, 1.9].into_iter().enumerate() {
        let events = if frame == 0 {
            vec![Event::PointerMoved(header.center())]
        } else {
            vec![]
        };
        full = show(&ctx, &provider, &mut state, time, events, configure).1;
    }
    let shown = texts(&full.shapes);
    assert!(
        shown.iter().any(|(text, _)| text == "About column 1"),
        "{shown:?}"
    );
    let asked = provider.tooltips.borrow().clone();
    assert!(
        !asked.is_empty() && asked.iter().all(|&column| column == 1),
        "{asked:?}"
    );

    // A column without a description shows none.
    let header = drawn_at(&full.shapes, "Column 2").expect("header drawn");
    for (frame, time) in [3.0, 3.1, 3.8, 3.9].into_iter().enumerate() {
        let events = if frame == 0 {
            vec![Event::PointerMoved(header.center())]
        } else {
            vec![]
        };
        full = show(&ctx, &provider, &mut state, time, events, configure).1;
    }
    assert!(provider.tooltips.borrow().contains(&2));
    assert!(
        !texts(&full.shapes)
            .iter()
            .any(|(text, _)| text.starts_with("About column")),
        "no tooltip"
    );
}

/// A header tall enough for two lines shows each column's subtitle under its title; a header one
/// row tall shows none.
#[test]
fn tall_headers_show_subtitles_under_their_titles() {
    let ctx = egui::Context::default();
    let provider = Recording::lazy(100, 3);
    let mut state = TableState::new("subtitles", 100);
    fn tall(table: TableKit<'_>) -> TableKit<'_> {
        table
            .with_columns((0..3).map(|_| crate::layout::Column::new(120.0)).collect())
            .with_header_height(40.0)
            .with_header_color(egui::Color32::from_rgb(1, 2, 3))
    }
    let (_, full) = show(&ctx, &provider, &mut state, 0.0, vec![], tall);
    let title = drawn_at(&full.shapes, "Column 1").expect("title drawn");
    let subtitle = drawn_at(&full.shapes, "type 1").expect("subtitle drawn");
    assert!(
        subtitle.top() >= title.bottom(),
        "{title:?} above {subtitle:?}"
    );
    assert!((subtitle.left() - title.left()).abs() < 2.0, "aligned");

    fn short(table: TableKit<'_>) -> TableKit<'_> {
        table.with_columns((0..3).map(|_| crate::layout::Column::new(120.0)).collect())
    }
    let mut state = TableState::new("no-subtitles", 100);
    let (_, full) = show(&ctx, &provider, &mut state, 0.0, vec![], short);
    assert!(drawn_at(&full.shapes, "Column 1").is_some());
    assert!(
        !texts(&full.shapes)
            .iter()
            .any(|(text, _)| text.starts_with("type ")),
        "no room for a subtitle"
    );
}

/// What of each filled rectangle of `color` shows: the rectangle within its clip.
fn filled(shapes: &[egui::epaint::ClippedShape], color: egui::Color32) -> Vec<egui::Rect> {
    fn collect(
        shape: &egui::Shape,
        clip: egui::Rect,
        color: egui::Color32,
        out: &mut Vec<egui::Rect>,
    ) {
        match shape {
            egui::Shape::Rect(rect) if rect.fill == color => {
                let shown = rect.rect.intersect(clip);
                if shown.is_positive() {
                    out.push(shown);
                }
            }
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .for_each(|shape| collect(shape, clip, color, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        collect(&clipped.shape, clipped.clip_rect, color, &mut out);
    }
    out
}

/// The x of each vertical line of `color`, with its clip.
fn vertical_lines(
    shapes: &[egui::epaint::ClippedShape],
    color: egui::Color32,
) -> Vec<(f32, egui::Rect)> {
    fn collect(
        shape: &egui::Shape,
        clip: egui::Rect,
        color: egui::Color32,
        out: &mut Vec<(f32, egui::Rect)>,
    ) {
        match shape {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == color && (points[0].x - points[1].x).abs() < 0.01 =>
            {
                out.push((points[0].x, clip));
            }
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .for_each(|shape| collect(shape, clip, color, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        collect(&clipped.shape, clipped.clip_rect, color, &mut out);
    }
    out
}

/// A sticky fill covers each sticky cell edge to edge (not just inside its padding), in place of
/// the stripes there, and the sticky edge runs down the last sticky column's right side, inside
/// what is shown of it, header included.
#[test]
fn sticky_columns_are_filled_edge_to_edge_with_an_edge_line() {
    const FILL: egui::Color32 = egui::Color32::from_rgb(10, 20, 30);
    const STRIPE: egui::Color32 = egui::Color32::from_rgb(40, 50, 60);
    const EDGE: egui::Color32 = egui::Color32::from_rgb(70, 80, 90);
    let ctx = egui::Context::default();
    let provider = Recording::lazy(100, 4);
    let mut state = TableState::new("sticky-fill", 100);
    fn configure(table: TableKit<'_>) -> TableKit<'_> {
        table
            .with_columns((0..4).map(|_| crate::layout::Column::new(120.0)).collect())
            .with_sticky_columns(1)
            .with_striped(true)
            .with_striping_color(STRIPE)
            .with_sticky_fill(FILL)
            .with_sticky_edge(egui::Stroke::new(1.0, EDGE))
    }
    let (_, full) = show(&ctx, &provider, &mut state, 0.0, vec![], configure);
    let first = drawn_at(&full.shapes, "r1c0").expect("sticky cell drawn");
    let second = drawn_at(&full.shapes, "r1c1").expect("next cell drawn");

    let fills = filled(&full.shapes, FILL);
    assert!(fills.len() > 5, "{fills:?}");
    let (left, right) = (fills[0].left(), fills[0].right());
    for fill in &fills {
        assert!(
            (fill.left() - left).abs() < 0.01 && (fill.right() - right).abs() < 0.01,
            "{fill:?}"
        );
    }
    assert!(
        (right - left - 120.0).abs() < 0.5,
        "a whole column: {left}..{right}"
    );
    assert!(left < first.left() - 6.0, "fills the padding too");
    assert!(right <= second.left(), "stops at the column's edge");
    // Each row's fill meets the next one's.
    let mut tops: Vec<f32> = fills.iter().map(|fill| fill.top()).collect();
    tops.sort_by(f32::total_cmp);
    let height = fills[0].height();
    for pair in tops.windows(2) {
        assert!((pair[1] - pair[0] - height).abs() < 0.5, "{pair:?}");
    }
    // Stripes stay out of the sticky column.
    for stripe in filled(&full.shapes, STRIPE) {
        assert!(stripe.left() >= right - 0.01, "{stripe:?}");
    }

    let edges = vertical_lines(&full.shapes, EDGE);
    assert!(edges.len() > 5, "{edges:?}");
    for (x, clip) in &edges {
        assert!((x - (right - 0.5)).abs() < 0.01, "{x} against {right}");
        assert!(clip.x_range().contains(*x), "shown: {x} in {clip:?}");
    }
    let header = drawn_at(&full.shapes, "Column 0").expect("header drawn");
    assert!(
        edges
            .iter()
            .any(|(_, clip)| clip.y_range().contains(header.center().y)),
        "through the header"
    );
}
