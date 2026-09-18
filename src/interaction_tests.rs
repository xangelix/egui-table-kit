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
