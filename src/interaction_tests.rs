use std::borrow::Cow;

use egui::{Event, Modifiers, PointerButton, Pos2};

use crate::error::TableError;
use crate::interaction::*;
use crate::operations::{BorrowedRow, HeaderIter, RowCallback, TableCell, TableProvider};
use crate::state::TableState;
use crate::table::TableKit;

struct Data {
    names: Vec<&'static str>,
    tree: bool,
    fail_cell: bool,
    fail_sort: bool,
    cycle: bool,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            names: vec!["Alpha", "Bravo", "Charlie", "Delta"],
            tree: false,
            fail_cell: false,
            fail_sort: false,
            cycle: false,
        }
    }
}

impl TableProvider for Data {
    fn column_count(&self) -> usize {
        2
    }

    fn header(&self, column: usize) -> Option<Cow<'_, str>> {
        ["Name", "Value"].get(column).map(|s| Cow::Borrowed(*s))
    }

    fn headers(&self) -> HeaderIter<'_> {
        HeaderIter::new(self)
    }

    fn row_count(&self) -> usize {
        self.names.len()
    }

    fn row_key(&self, row: usize) -> egui::Id {
        egui::Id::new(self.names[row])
    }

    fn row_selectable(&self, row: usize) -> bool {
        !self.tree || row != 0
    }

    fn cell_at(&self, row: usize, _: usize) -> Result<Option<TableCell<'_>>, TableError> {
        if self.fail_cell {
            return Err(TableError::Operation("injected cell failure".into()));
        }
        Ok(self.names.get(row).map(|name| (Cow::Borrowed(*name), None)))
    }

    fn for_all_rows(&self, f: &mut RowCallback<'_>) -> Result<(), TableError> {
        for row_index in 0..self.row_count() {
            f(&BorrowedRow {
                provider: self,
                row_index,
            })?;
        }
        Ok(())
    }

    fn for_selected_rows(
        &self,
        state: &TableState,
        f: &mut RowCallback<'_>,
    ) -> Result<(), TableError> {
        for row_index in &state.selected_rows {
            f(&BorrowedRow {
                provider: self,
                row_index: row_index as usize,
            })?;
        }
        Ok(())
    }

    fn sort_active_rows(
        &self,
        rows: &mut Vec<usize>,
        _: usize,
        up: bool,
    ) -> Result<(), TableError> {
        if self.fail_sort {
            return Err(TableError::Operation("injected sort failure".into()));
        }
        rows.sort_by_key(|&r| self.names[r]);
        if !up {
            rows.reverse();
        }
        Ok(())
    }

    fn is_tree(&self) -> bool {
        self.tree
    }

    fn row_children(&self, row: usize) -> Vec<usize> {
        if row == 0 {
            if self.cycle { vec![0] } else { vec![1, 2, 3] }
        } else {
            vec![]
        }
    }

    fn row_parent(&self, row: usize) -> Option<usize> {
        (row != 0).then_some(0)
    }

    fn row_hierarchy(
        &self,
        state: &TableState,
        row: usize,
    ) -> Option<crate::operations::RowHierarchy> {
        Some(crate::operations::RowHierarchy {
            indent_level: usize::from(row != 0),
            has_children: row == 0,
            is_expanded: state.expanded_rows.contains(row as u32),
        })
    }
}

fn explorer() -> TableState {
    let mut state = TableState::new("test", 4);
    state.selection_mode = SelectionMode::Explorer;
    state
}

#[test]
fn explorer_selection_anchors_follow_identity_through_sort_and_snapshot_replacement() {
    let data = Data::default();
    let mut state = explorer();

    state.select_row(&data, 1, Modifiers::NONE, SelectionInput::Click);
    state.select_row(&data, 1, Modifiers::NONE, SelectionInput::Click);
    assert_eq!(state.selected_rows.iter().collect::<Vec<_>>(), [1]);

    state.select_row(&data, 3, Modifiers::COMMAND, SelectionInput::Click);
    state.active_rows.reverse();
    state.select_row(&data, 2, Modifiers::SHIFT, SelectionInput::Click);
    assert_eq!(state.selected_rows.iter().collect::<Vec<_>>(), [2, 3]);

    state.select_row(&data, 2, Modifiers::NONE, SelectionInput::Drag);
    assert_eq!(
        state.selected_rows.iter().collect::<Vec<_>>(),
        [2, 3],
        "Dragging selected rows preserves the group"
    );

    let snapshot = state.capture_identity(&data);
    let reordered = Data {
        names: vec!["Delta", "Alpha", "Charlie", "Echo"],
        ..Default::default()
    };
    state.restore_identity(&reordered, &snapshot).unwrap();
    assert_eq!(state.selected_rows.iter().collect::<Vec<_>>(), [0, 2]);
    assert_eq!(state.anchor_key, Some(egui::Id::new("Delta")));

    state.select_row(&reordered, 0, Modifiers::NONE, SelectionInput::Context);
    assert_eq!(state.selected_rows.len(), 2);

    state.select_row(&reordered, 1, Modifiers::NONE, SelectionInput::Context);
    assert_eq!(state.selected_rows.iter().collect::<Vec<_>>(), [1]);

    let mut legacy = TableState::new("legacy", 4);
    legacy.select_row(&data, 0, Modifiers::NONE, SelectionInput::Click);
    legacy.select_row(&data, 0, Modifiers::NONE, SelectionInput::Click);
    assert!(legacy.selected_rows.is_empty());
}

#[test]
fn tree_sort_failures_cycles_and_duplicate_keys_are_reported_and_can_retry() {
    let mut data = Data {
        tree: true,
        fail_sort: true,
        ..Default::default()
    };
    let mut state = explorer();
    state.columns.resize_with(2, Default::default);
    state.columns[0].sort_up = Some(true);
    state.expanded_rows.insert(0);

    assert!(state.refresh_view(&data).is_err());
    assert!(state.filter_cache_dirty);

    data.fail_sort = false;
    state.refresh_view(&data).unwrap();
    assert_eq!(state.active_rows.len(), 4);

    data.cycle = true;
    state.filter_cache_dirty = true;
    assert!(state.refresh_view(&data).is_err());
    assert_eq!(state.active_rows.len(), 4);

    let duplicate = Data {
        names: vec!["Alpha", "Alpha"],
        ..Default::default()
    };
    assert!(
        state
            .restore_identity(&duplicate, &IdentityState::default())
            .is_err()
    );
}

fn frame(
    ctx: &egui::Context,
    data: &Data,
    state: &mut TableState,
    time: f64,
    events: Vec<Event>,
) -> Result<TableOutput, TableError> {
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
            result = Some(
                TableKit::new("test", data, state)
                    .with_drag_enabled(true)
                    .with_row_height(24.0)
                    .with_columns(vec![
                        crate::layout::Column::new(180.0),
                        crate::layout::Column::new(140.0),
                    ])
                    .show_with_output(ui, |_, _, _, _| None),
            );
        },
    );
    output.textures_delta.clear();
    result.unwrap()
}

fn pointer(pos: Pos2, pressed: bool) -> Vec<Event> {
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

#[test]
fn rendered_rows_emit_one_drag_with_the_selected_keys_and_no_release_click() {
    let ctx = egui::Context::default();
    let data = Data::default();
    let mut state = explorer();

    frame(&ctx, &data, &mut state, 0.0, vec![]).unwrap();
    let output = frame(&ctx, &data, &mut state, 0.05, vec![]).unwrap();
    let point = output.rows[1].hit_regions[0].center();

    state.selected_rows.extend([1, 2]);
    frame(&ctx, &data, &mut state, 1.0, pointer(point, true)).unwrap();

    let drag = frame(
        &ctx,
        &data,
        &mut state,
        1.05,
        vec![Event::PointerMoved(point + egui::vec2(30.0, 0.0))],
    )
    .unwrap();

    let starts = drag
        .events
        .iter()
        .filter(|e| e.kind == RowEventKind::DragStarted)
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].dragged_keys.len(), 2);

    let release = frame(
        &ctx,
        &data,
        &mut state,
        1.1,
        pointer(point + egui::vec2(30.0, 0.0), false),
    )
    .unwrap();

    assert!(
        !release
            .events
            .iter()
            .any(|e| e.kind == RowEventKind::Clicked)
    );
    assert_eq!(state.selected_rows.len(), 2);
}

#[test]
fn expanders_and_column_separators_cannot_start_row_drags_and_cell_errors_propagate() {
    let ctx = egui::Context::default();
    let data = Data {
        tree: true,
        ..Default::default()
    };
    let mut state = explorer();

    frame(&ctx, &data, &mut state, 0.0, vec![]).unwrap();
    let output = frame(&ctx, &data, &mut state, 0.05, vec![]).unwrap();
    let arrow = output.rows[0].rect.left_center() + egui::vec2(8.0, 0.0);

    frame(&ctx, &data, &mut state, 1.0, pointer(arrow, true)).unwrap();
    let expanded = frame(&ctx, &data, &mut state, 1.02, pointer(arrow, false)).unwrap();
    assert!(
        expanded
            .events
            .iter()
            .any(|e| e.kind == RowEventKind::ExpansionChanged(true))
    );
    assert!(state.selected_rows.is_empty());

    let output = frame(&ctx, &data, &mut state, 1.1, vec![]).unwrap();
    let separator = egui::pos2(
        output.rows[0].hit_regions[0].right() + 2.0,
        output.rows[0].rect.center().y,
    );
    frame(&ctx, &data, &mut state, 2.0, pointer(separator, true)).unwrap();

    let moved = frame(
        &ctx,
        &data,
        &mut state,
        2.05,
        vec![Event::PointerMoved(separator + egui::vec2(30.0, 0.0))],
    )
    .unwrap();
    assert!(
        !moved
            .events
            .iter()
            .any(|e| e.kind == RowEventKind::DragStarted)
    );

    frame(&ctx, &data, &mut state, 2.1, pointer(separator, false)).unwrap();
    let resized = frame(&ctx, &data, &mut state, 2.2, vec![]).unwrap();
    assert!(resized.rows[0].hit_regions[0].right() > output.rows[0].hit_regions[0].right() + 20.0);

    state.selected_rows.extend([1, 2]);
    let root = resized.rows[0].hit_regions[0].center();
    frame(&ctx, &data, &mut state, 2.3, pointer(root, true)).unwrap();

    let moved = frame(
        &ctx,
        &data,
        &mut state,
        2.35,
        vec![Event::PointerMoved(root + egui::vec2(30.0, 0.0))],
    )
    .unwrap();
    assert!(
        !moved
            .events
            .iter()
            .any(|e| e.kind == RowEventKind::DragStarted),
        "A nonselectable root cannot drag an unrelated selection"
    );

    frame(
        &ctx,
        &data,
        &mut state,
        2.4,
        pointer(root + egui::vec2(30.0, 0.0), false),
    )
    .unwrap();
    assert_eq!(state.selected_rows.iter().collect::<Vec<_>>(), [1, 2]);

    let faulty = Data {
        fail_cell: true,
        ..Default::default()
    };
    assert!(frame(&ctx, &faulty, &mut state, 3.0, vec![]).is_err());
}

#[test]
fn invalid_column_maps_report_errors_without_calling_the_provider_out_of_bounds() {
    for map in [vec![], vec![0, 0], vec![2]] {
        let ctx = egui::Context::default();
        let data = Data::default();
        let mut state = explorer();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(
                TableKit::new("bad-map", &data, &mut state)
                    .with_column_map(map.clone())
                    .show_with_output(ui, |_, _, _, _| None)
                    .is_err()
            );
        });
        output.textures_delta.clear();
    }
}

#[test]
fn test_move_focus_navigation_and_boundaries() {
    let data = Data {
        names: vec!["Row0", "Row1", "Row2", "Row3"],
        tree: true,
        ..Default::default()
    };
    let mut state = explorer();
    state.active_rows = vec![0, 1, 2, 3];

    // Case 1: Unfocused initial navigation
    let first = state.move_focus(&data, 1, Modifiers::NONE);
    // Row 0 is unselectable in tree mode (row != 0 check in Data), so first selectable is row 1
    assert_eq!(first, Some(1));
    assert_eq!(state.focused_key, Some(data.row_key(1)));

    // Case 2: Step forward to next selectable row
    let next = state.move_focus(&data, 1, Modifiers::NONE);
    assert_eq!(next, Some(2));
    assert_eq!(state.focused_key, Some(data.row_key(2)));

    // Case 3: Step to end and saturate
    let end = state.move_focus(&data, 1, Modifiers::NONE);
    assert_eq!(end, Some(3));
    let past_end = state.move_focus(&data, 1, Modifiers::NONE);
    assert_eq!(past_end, Some(3));

    // Case 4: Jump to endpoints
    let max = state.move_focus(&data, isize::MAX, Modifiers::NONE);
    assert_eq!(max, Some(3));
    let min = state.move_focus(&data, isize::MIN, Modifiers::NONE);
    assert_eq!(min, Some(1)); // Row 0 is unselectable, so min is 1

    // Case 5: Empty table
    let mut empty_state = explorer();
    empty_state.active_rows.clear();
    assert_eq!(empty_state.move_focus(&data, 1, Modifiers::NONE), None);
}

#[test]
fn test_show_bypasses_geometry_and_event_allocations() {
    let ctx = egui::Context::default();
    let data = Data::default();
    let mut state = explorer();

    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let res = TableKit::new("no-output", &data, &mut state)
            .show(ui, |_, _, _, _| None)
            .expect("show succeeds");
        assert!(res.rect.is_positive());
    });
    output.textures_delta.clear();
}

#[test]
fn test_handle_row_selection_at_visible_legacy_mode() {
    let mut state = TableState::new("test", 5);
    state.selection_mode = SelectionMode::Legacy;
    state.active_rows = vec![0, 1, 2, 3, 4];

    // Single click selects row
    state.handle_row_selection_at_visible(Modifiers::NONE, 1, Some(1));
    assert!(state.selected_rows.contains(1));
    assert_eq!(state.last_clicked_visible_index, Some(1));

    // Ctrl-click adds row
    state.handle_row_selection_at_visible(Modifiers::COMMAND, 3, Some(3));
    assert!(state.selected_rows.contains(1));
    assert!(state.selected_rows.contains(3));
    assert_eq!(state.last_clicked_visible_index, Some(3));

    // Shift-click selects range from anchor (3) to 1
    state.handle_row_selection_at_visible(Modifiers::SHIFT, 1, Some(1));
    assert!(state.selected_rows.contains(1));
    assert!(state.selected_rows.contains(2));
    assert!(state.selected_rows.contains(3));

    // Single click toggles off if it was the only selected row
    state.selected_rows.clear();
    state.selected_rows.insert(2);
    state.handle_row_selection_at_visible(Modifiers::NONE, 2, Some(2));
    assert!(state.selected_rows.is_empty());
    assert_eq!(state.last_clicked_visible_index, None);
}

#[test]
fn test_header_cell_ui_call_count_with_sticky_cols() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::delegate::TableKitDelegate;
    use crate::layout::{Column, HeaderRow, Table};

    let ctx = egui::Context::default();
    let data = Data::default();
    let mut state = TableState::new("test", 4);
    let col0_calls = AtomicUsize::new(0);
    let col1_calls = AtomicUsize::new(0);

    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(500.0, 350.0),
            )),
            ..Default::default()
        },
        |ui| {
            let mut collected = Vec::new();
            let mut halt_error = None;
            let mut item_clicked = None;
            let mut secondary_clicked = None;
            let org_colors = [];
            let user_colors = [];

            let table = Table::new()
                .num_sticky_cols(1)
                .num_rows(4)
                .columns(vec![
                    Column::new(85.0),
                    Column::new(140.0),
                    Column::new(140.0),
                ])
                .headers([HeaderRow::new(24.0)]);

            struct CountingDelegate<'a> {
                inner: TableKitDelegate<'a>,
                c0: &'a AtomicUsize,
                c1: &'a AtomicUsize,
            }

            impl crate::layout::TableDelegate for CountingDelegate<'_> {
                fn header_cell_ui(
                    &mut self,
                    ui: &mut egui::Ui,
                    cell: &crate::layout::HeaderCellInfo,
                ) {
                    if cell.col_range.start == 0 {
                        self.c0.fetch_add(1, Ordering::SeqCst);
                    }
                    if cell.col_range.start == 1 {
                        self.c1.fetch_add(1, Ordering::SeqCst);
                    }
                    self.inner.header_cell_ui(ui, cell);
                }

                fn cell_ui(&mut self, ui: &mut egui::Ui, cell: &crate::layout::CellInfo) {
                    self.inner.cell_ui(ui, cell);
                }
            }

            let delegate = TableKitDelegate::new(
                &data,
                &mut state,
                &org_colors,
                &user_colors,
                &mut collected,
                &mut halt_error,
                None,
                &mut item_clicked,
                &mut secondary_clicked,
            );
            let mut counting = CountingDelegate {
                inner: delegate,
                c0: &col0_calls,
                c1: &col1_calls,
            };

            table.show(ui, &mut counting);
            drop(counting);
            let _ = state.process_responses(&data, collected);
        },
    );
    output.textures_delta.clear();

    assert_eq!(col0_calls.load(Ordering::SeqCst), 1);
    assert_eq!(col1_calls.load(Ordering::SeqCst), 1);
}

/// Where `wanted` was drawn as text among `shapes`.
fn text_rect(shapes: &[egui::epaint::ClippedShape], wanted: &str) -> Option<egui::Rect> {
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

/// A frame of rows `height` points tall, in a style whose widgets are 26 tall, with the cells'
/// text aligned as `align` says (unset: as `ui.horizontal` places it): the first row's
/// rectangle, and where its text was drawn.
fn aligned(height: f32, align: Option<egui::Align>) -> (egui::Rect, egui::Rect) {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| style.spacing.interact_size.y = 26.0);
    let data = Data::default();
    let mut state = explorer();
    let mut drawn = (egui::Rect::NOTHING, egui::Rect::NOTHING);
    for time in [0.0, 0.05] {
        let mut result = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(500.0, 350.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                let mut table = TableKit::new("aligned", &data, &mut state)
                    .with_row_height(height)
                    .with_columns(vec![crate::layout::Column::new(180.0)]);
                if let Some(align) = align {
                    table = table.with_cell_align(align);
                }
                result = Some(table.show_with_output(ui, |_, _, _, _| None));
            },
        );
        output.textures_delta.clear();
        let table = result.unwrap().unwrap();
        if let (Some(row), Some(text)) = (table.rows.first(), text_rect(&output.shapes, "Alpha")) {
            drawn = (row.rect, text);
        }
    }
    drawn
}

#[test]
fn a_cell_s_text_sits_across_the_row_as_the_caller_asks() {
    // Unset, as `ui.horizontal` places it: centered in a row taller than the style's widgets,
    // but from the top (low) in one shorter.
    let (row, text) = aligned(48.0, None);
    assert!(row.is_positive() && text.is_positive());
    assert!(
        (text.center().y - row.center().y).abs() <= 1.5,
        "unset, tall: {text:?} in {row:?}"
    );
    let (row, text) = aligned(16.0, None);
    assert!(
        text.center().y - row.center().y > 3.0,
        "unset, short: {text:?} in {row:?}"
    );
    // Centered, whatever height the style gives widgets.
    for height in [16.0, 48.0] {
        let (row, text) = aligned(height, Some(egui::Align::Center));
        assert!(
            (text.center().y - row.center().y).abs() <= 1.5,
            "centered: {text:?} in {row:?}"
        );
    }
    // At the top, for rows beside taller custom cells; or at the bottom.
    let (row, text) = aligned(48.0, Some(egui::Align::Min));
    assert!(
        text.top() - row.top() < 4.0,
        "at the top: {text:?} in {row:?}"
    );
    let (row, text) = aligned(48.0, Some(egui::Align::Max));
    assert!(
        row.bottom() - text.bottom() < 4.0,
        "at the bottom: {text:?} in {row:?}"
    );
}

/// Whether a frame with the first row selected and the pointer on it paints `marker`, the
/// overlay asked for (or none).
fn selected_row_paints(overlay: Option<egui::Color32>, marker: egui::Color32) -> bool {
    let ctx = egui::Context::default();
    let data = Data::default();
    let mut state = explorer();
    state.selected_rows.insert(0);
    let mut painted = false;
    let mut row_center = None;
    for time in [0.0, 0.05, 0.1] {
        let mut result = None;
        let events = row_center
            .map(|center| vec![Event::PointerMoved(center)])
            .unwrap_or_default();
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
                let mut table = TableKit::new("selected-hover", &data, &mut state)
                    .with_row_height(24.0)
                    .with_columns(vec![crate::layout::Column::new(180.0)]);
                if let Some(overlay) = overlay {
                    table = table.with_selected_hover_color(overlay);
                }
                result = Some(table.show_with_output(ui, |_, _, _, _| None));
            },
        );
        output.textures_delta.clear();
        let table = result.unwrap().unwrap();
        row_center = table.rows.first().map(|row| row.rect.center());
        painted = output.shapes.iter().any(
            |clipped| matches!(&clipped.shape, egui::Shape::Rect(rect) if rect.fill == marker),
        );
    }
    painted
}

#[test]
fn a_selected_row_answers_the_pointer_only_when_asked() {
    let marker = egui::Color32::from_rgb(1, 2, 3);
    assert!(selected_row_paints(Some(marker), marker));
    // Unset, as before: the selection alone.
    assert!(!selected_row_paints(None, marker));
}

/// The colour a selected row's text was drawn in, with the text colour asked for (or none).
fn selected_text_drawn(color: Option<egui::Color32>) -> Option<egui::Color32> {
    let ctx = egui::Context::default();
    let data = Data::default();
    let mut state = explorer();
    state.selected_rows.insert(0);
    let mut drawn = None;
    for time in [0.0, 0.05] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(500.0, 350.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                let mut table = TableKit::new("selected-text", &data, &mut state)
                    .with_row_height(24.0)
                    .with_columns(vec![crate::layout::Column::new(180.0)]);
                if let Some(color) = color {
                    table = table.with_selected_text_color(color);
                }
                table.show_with_output(ui, |_, _, _, _| None).unwrap();
            },
        );
        output.textures_delta.clear();
        drawn = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == "Alpha" => text
                    .galley
                    .job
                    .sections
                    .first()
                    .map(|section| section.format.color),
                _ => None,
            });
    }
    drawn
}

#[test]
fn a_selected_row_s_text_takes_the_colour_asked_for() {
    let asked = egui::Color32::from_rgb(9, 8, 7);
    assert_eq!(selected_text_drawn(Some(asked)), Some(asked));
    // Unset, the selection stroke's colour, as before.
    let stroke = egui::Visuals::dark().selection.stroke.color;
    assert_eq!(selected_text_drawn(None), Some(stroke));
}

/// How many of a frame's rectangles are outlined in the selection stroke's colour, with the
/// first row focused and the focus outline as asked.
fn focus_outlines(outline: bool) -> usize {
    let ctx = egui::Context::default();
    let data = Data::default();
    let mut state = explorer();
    state.focused_key = Some(data.row_key(0));
    let stroke = egui::Visuals::dark().selection.stroke;
    let mut outlines = 0;
    for time in [0.0, 0.05] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(500.0, 350.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                TableKit::new("focus-outline", &data, &mut state)
                    .with_row_height(24.0)
                    .with_columns(vec![
                        crate::layout::Column::new(180.0),
                        crate::layout::Column::new(180.0),
                    ])
                    .with_focus_outline(outline)
                    .show_with_output(ui, |_, _, _, _| None)
                    .unwrap();
            },
        );
        output.textures_delta.clear();
        outlines = output
            .shapes
            .iter()
            .filter(|clipped| {
                matches!(&clipped.shape, egui::Shape::Rect(rect) if rect.stroke == stroke)
            })
            .count();
    }
    outlines
}

#[test]
fn the_focused_row_is_outlined_unless_asked_not_to_be() {
    assert!(focus_outlines(true) >= 2, "each cell, as before");
    assert_eq!(focus_outlines(false), 0);
}

/// Whether a frame drew the header's title "Name", and the first row's rectangle, with the
/// header as asked.
fn header_drawn(header: bool) -> (bool, egui::Rect) {
    let ctx = egui::Context::default();
    let data = Data::default();
    let mut state = explorer();
    let mut drawn = (false, egui::Rect::NOTHING);
    for time in [0.0, 0.05] {
        let mut result = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(500.0, 350.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                result = Some(
                    TableKit::new("header", &data, &mut state)
                        .with_row_height(24.0)
                        .with_columns(vec![crate::layout::Column::new(180.0)])
                        .with_header(header)
                        .show_with_output(ui, |_, _, _, _| None),
                );
            },
        );
        output.textures_delta.clear();
        let table = result.unwrap().unwrap();
        let first = table
            .rows
            .first()
            .map_or(egui::Rect::NOTHING, |row| row.rect);
        drawn = (text_rect(&output.shapes, "Name").is_some(), first);
    }
    drawn
}

#[test]
fn the_header_can_be_left_out() {
    let (titled, below) = header_drawn(true);
    assert!(titled, "shown by default");
    let (untitled, first) = header_drawn(false);
    assert!(!untitled);
    assert!(
        first.is_positive() && first.top() < below.top(),
        "rows begin at the top"
    );
}

/// The corners of the selected first row's fills, each cell's, left to right, with its rows'
/// outer corners rounded as asked.
fn selected_corners(rounding: Option<u8>) -> Vec<egui::CornerRadius> {
    let ctx = egui::Context::default();
    let data = Data::default();
    let mut state = explorer();
    state.selected_rows.insert(0);
    let fill = egui::Visuals::dark().selection.bg_fill;
    let mut corners = Vec::new();
    for time in [0.0, 0.05] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(500.0, 350.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                let mut table = TableKit::new("row-rounding", &data, &mut state)
                    .with_row_height(24.0)
                    .with_columns(vec![
                        crate::layout::Column::new(120.0),
                        crate::layout::Column::new(120.0),
                    ]);
                if let Some(radius) = rounding {
                    table = table.with_row_rounding(radius);
                }
                table.show_with_output(ui, |_, _, _, _| None).unwrap();
            },
        );
        output.textures_delta.clear();
        let mut fills: Vec<(f32, egui::CornerRadius)> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill == fill => {
                    Some((rect.rect.left(), rect.corner_radius))
                }
                _ => None,
            })
            .collect();
        fills.sort_by(|a, b| a.0.total_cmp(&b.0));
        corners = fills.into_iter().map(|(_, corner)| corner).collect();
    }
    corners
}

#[test]
fn a_row_s_outer_corners_can_be_rounded() {
    let rounded = selected_corners(Some(6));
    assert_eq!(rounded.len(), 2, "a fill per cell");
    let left = egui::CornerRadius {
        nw: 6,
        sw: 6,
        ne: 0,
        se: 0,
    };
    let right = egui::CornerRadius {
        nw: 0,
        sw: 0,
        ne: 6,
        se: 6,
    };
    assert_eq!(rounded, vec![left, right]);
    // Unset, square, as before.
    assert!(
        selected_corners(None)
            .iter()
            .all(|corner| *corner == egui::CornerRadius::ZERO)
    );
}
