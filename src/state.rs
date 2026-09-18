//! Custom interactive view states mapped seamlessly to virtualization targets.

use std::{collections::HashMap, fmt::Write as _, sync::Arc};

use compact_str::CompactString;
use fluent_zero::t;
use roaring::RoaringBitmap;

use crate::operations::Row as _;

use super::{
    error::TableError,
    filter::Filter,
    header::{ColResponse, ColumnState},
    highlights::Highlights,
    operations::{RowHierarchy, TableProvider},
};

#[derive(Debug)]
pub enum FilterUpdate {
    Added(usize, Filter),
    Removed(usize),
    Modified(usize, Filter),
}

#[derive(Debug)]
pub struct TableChanges {
    pub filter_update: Option<FilterUpdate>,
    pub sort_update: Option<usize>,
    pub filter_state: Vec<(usize, Filter)>,
    pub sort_state: Option<(usize, bool)>,
}

/// Holds interactive visual properties for columns and selection structures.
#[derive(Debug, Default)]
pub struct TableState {
    pub id: CompactString,
    pub columns: Vec<ColumnState>,
    pub highlights: Highlights,
    pub highlights_changed: bool,
    pub active_rows: Vec<usize>,
    pub selected_rows: RoaringBitmap,
    pub expanded_rows: RoaringBitmap,
    pub last_clicked_visible_index: Option<usize>,
    pub filter_matches: RoaringBitmap,
    pub filter_cache_dirty: bool,
    pub sorted_children_cache: HashMap<usize, Arc<Vec<usize>>, ahash::RandomState>,

    // Row interaction, selection model and keyboard focus state
    pub selection_mode: crate::interaction::SelectionMode,
    pub focused_key: Option<egui::Id>,
    pub anchor_key: Option<egui::Id>,

    #[doc(hidden)]
    pub event_frame: Option<u64>,
    #[doc(hidden)]
    pub frame_events:
        std::collections::HashSet<(egui::Id, crate::interaction::RowEventKind), ahash::RandomState>,
}

impl TableState {
    /// Constructs and initializes view settings.
    #[must_use]
    pub fn new(id: impl Into<CompactString>, row_count: usize) -> Self {
        Self {
            id: id.into(),
            active_rows: (0..row_count).collect(),
            filter_cache_dirty: true, // Mark dirty initially to force first-frame population
            ..Default::default()
        }
    }

    /// Accesses the active filter options defined for each column.
    #[must_use]
    pub fn get_filter_state(&self) -> Vec<(usize, Filter)> {
        self.columns
            .iter()
            .enumerate()
            .filter_map(|(col_index, col)| {
                if col.response.filtering.is_empty() {
                    None
                } else {
                    Some((col_index, col.response.filtering.clone()))
                }
            })
            .collect()
    }

    /// Accesses active sorting criteria (column offset and sorting order).
    #[must_use]
    pub fn get_sort_state(&self) -> Option<(usize, bool)> {
        self.columns
            .iter()
            .enumerate()
            .find_map(|(col_index, col)| col.sort_up.map(|sort_up| (col_index, sort_up)))
    }

    /// Renders dynamic status headers detailing matching selection counts.
    #[must_use]
    pub fn counts_header(&self, row_len: usize) -> String {
        let mut counts = String::with_capacity(64);
        counts.push_str(&t!("total-rows", { "count" => row_len }));

        let active_rows_count = self.active_rows.len();
        if active_rows_count != row_len {
            counts.push_str(", ");
            let _ = write!(counts, "{active_rows_count} {}", t!("passing-filter"));
        }

        let selected_rows_count = self.selected_rows.len();
        if selected_rows_count != 0 {
            counts.push_str(", ");
            let _ = write!(counts, "{selected_rows_count} {}", t!("selected"));
        }

        counts
    }

    /// Applies header actions (sorting updates and filtering changes) to the active indices.
    pub fn process_responses(
        &mut self,
        provider: &dyn TableProvider,
        responses: Vec<ColResponse>,
    ) -> Result<(), TableError> {
        let anchor_row = self
            .last_clicked_visible_index
            .and_then(|visible| self.active_rows.get(visible))
            .copied();
        let changes = self.collect_responses(responses);
        let filter_update = changes.filter_update;
        let sort_update = changes.sort_update;
        let filter_state = changes.filter_state;
        let sort_state = changes.sort_state;

        if provider.is_tree() {
            if let Some(sort_col) = sort_update {
                let ascending = sort_state
                    .is_none_or(|(previous, ascending)| previous != sort_col || !ascending);
                for (index, column) in self.columns.iter_mut().enumerate() {
                    column.sort_up = (index == sort_col).then_some(ascending);
                }
                self.filter_cache_dirty = true;
            }
            if filter_update.is_some() || self.highlights_changed {
                self.filter_cache_dirty = true;
            }
            self.highlights_changed = false;
            // Sort siblings while traversing the tree. Sorting active_rows directly
            // detaches descendants from their parents and can move the root itself.
            if self.filter_cache_dirty {
                self.sorted_children_cache.clear();
                self.flatten_tree(provider)?;
                self.last_clicked_visible_index = anchor_row
                    .and_then(|row| self.active_rows.iter().position(|index| *index == row));
            }
            return Ok(());
        }

        if self.highlights_changed {
            self.highlights_changed = false;
            self.filter_cache_dirty = true;
            self.sorted_children_cache.clear();

            // Only apply flat filters if this is a flat table
            if !provider.is_tree() {
                self.apply_all_filters(provider, &filter_state)?;
            }

            if let Some((sort_col, sort_up)) = sort_state {
                provider.sort_active_rows(&mut self.active_rows, sort_col, sort_up)?;
            }
            return Ok(());
        }

        if let Some(update) = filter_update {
            self.sorted_children_cache.clear();

            // Only apply flat filters if this is a flat table
            if !provider.is_tree() {
                match update {
                    FilterUpdate::Added(col, filter) => {
                        self.apply_incremental_filter(provider, col, &filter)?;
                    }
                    FilterUpdate::Removed(_) | FilterUpdate::Modified(_, _) => {
                        self.apply_all_filters(provider, &filter_state)?;
                    }
                }
            }

            if let Some((sort_col, sort_up)) = sort_state {
                provider.sort_active_rows(&mut self.active_rows, sort_col, sort_up)?;
            }
        } else if let Some(sort_col) = sort_update {
            self.sorted_children_cache.clear();
            if let Some((already_sorted_col, sort_up)) = sort_state {
                if already_sorted_col == sort_col {
                    let column = self
                        .columns
                        .get_mut(sort_col)
                        .ok_or(TableError::CorruptedState)?;

                    let new_sort_up = !sort_up;
                    column.sort_up = Some(new_sort_up);

                    // Apply the sorted indices to active_rows in the new direction
                    provider.sort_active_rows(&mut self.active_rows, sort_col, new_sort_up)?;
                } else {
                    self.apply_new_sort(provider, sort_col)?;
                }
            } else {
                self.apply_new_sort(provider, sort_col)?;
            }
        }

        Ok(())
    }

    /// Sets up a new sorting column constraint and sorts active elements.
    pub fn apply_new_sort(
        &mut self,
        provider: &dyn TableProvider,
        sort_col: usize,
    ) -> Result<(), TableError> {
        for (i, column) in self.columns.iter_mut().enumerate() {
            column.sort_up = if i == sort_col { Some(true) } else { None };
        }
        if provider.is_tree() {
            self.filter_cache_dirty = true;
            self.sorted_children_cache.clear();
            self.flatten_tree(provider)?;
            return Ok(());
        }
        provider.sort_active_rows(&mut self.active_rows, sort_col, true)
    }

    /// Evaluates filtering constraints, resetting the active index map.
    pub fn apply_all_filters(
        &mut self,
        provider: &dyn TableProvider,
        filters: &[(usize, Filter)],
    ) -> Result<(), TableError> {
        self.active_rows = provider.filter_rows(self, filters)?;
        Ok(())
    }

    /// Brings the active row set up to date only when the view is dirty, returning
    /// `true` if it was recomputed this call. This is the intended per-frame entry
    /// point: it is a cheap no-op when nothing has changed, avoiding a full O(N)
    /// filter pass and sort every frame.
    ///
    /// For tree providers this delegates to [`Self::flatten_tree`]; for flat tables
    /// it reapplies the active filters and (if any) the current sort.
    pub fn refresh_view(&mut self, provider: &dyn TableProvider) -> Result<bool, TableError> {
        if !self.filter_cache_dirty {
            return Ok(false);
        }

        if provider.is_tree() {
            self.flatten_tree(provider)?;
        } else {
            let filter_state = self.get_filter_state();
            self.apply_all_filters(provider, &filter_state)?;
            if let Some((sort_col, sort_up)) = self.get_sort_state() {
                provider.sort_active_rows(&mut self.active_rows, sort_col, sort_up)?;
            }
        }

        self.filter_cache_dirty = false;
        Ok(true)
    }

    /// Runs filter constraints incrementally across the already-filtered row set.
    pub fn apply_incremental_filter(
        &mut self,
        provider: &dyn TableProvider,
        filter_col: usize,
        filter: &Filter,
    ) -> Result<(), TableError> {
        let mut new_active = Vec::with_capacity(self.active_rows.len());

        // Heuristic: If active rows are fewer than a threshold, look them up directly.
        if self.active_rows.len() < 1000 {
            for &row_idx in &self.active_rows {
                if let Some(row) = provider.row_at(row_idx)? {
                    let highlight = self.highlights.get_usize(row_idx);
                    if let Some(cell) = row.cell(filter_col)
                        && filter.matches(&cell.0, highlight)
                    {
                        new_active.push(row_idx);
                    }
                }
            }
        } else {
            // Fallback to sequential scan if the active set is large
            let active_set: RoaringBitmap = self.active_rows.iter().map(|&i| i as u32).collect();
            let mut row_idx = 0;
            provider.for_all_rows(&mut |row| {
                if active_set.contains(row_idx as u32) {
                    let highlight = self.highlights.get_usize(row_idx);
                    if let Some(cell) = row.cell(filter_col)
                        && filter.matches(&cell.0, highlight)
                    {
                        new_active.push(row_idx);
                    }
                }
                row_idx += 1;
                Ok(())
            })?;
        }

        self.active_rows = new_active;
        Ok(())
    }

    /// Renders the tree indentation guidelines and expand/collapse arrow inside a tree cell.
    /// Returns `true` if the expansion state changed (allowing immediate-mode viewport updates).
    pub fn show_tree_cell(
        &mut self,
        ui: &mut egui::Ui,
        row_index: usize,
        hierarchy: RowHierarchy,
    ) -> bool {
        let mut changed = false;

        #[allow(clippy::cast_precision_loss)]
        let spacing = hierarchy.indent_level as f32 * 22.0; // Comfortably spaced 22px indent
        if spacing > 0.0 {
            ui.add_space(spacing);

            let rect = ui.max_rect();
            let painter = ui.painter();
            let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(65, 65, 65));

            let dash_length = 2.0;
            let gap_length = 2.0;
            let step = dash_length + gap_length;
            let total_height = rect.max.y - rect.min.y;

            if total_height > 0.0 {
                let num_steps = (total_height / step).ceil() as usize;

                let segments = (0..hierarchy.indent_level).flat_map(|i| {
                    #[allow(clippy::cast_precision_loss)]
                    let x = (i as f32).mul_add(22.0, rect.min.x) + 6.8;
                    (0..num_steps).map(move |step_idx| {
                        #[allow(clippy::cast_precision_loss)]
                        let segment_y = (step_idx as f32).mul_add(step, rect.min.y);
                        let next_y = (segment_y + dash_length).min(rect.max.y);
                        egui::Shape::line_segment(
                            [egui::pos2(x, segment_y), egui::pos2(x, next_y)],
                            stroke,
                        )
                    })
                });

                painter.extend(segments);
            }
        }

        ui.scope(|ui| {
            // Bypass minimum interactive limits to let the region shrink to its natural 14px size
            ui.spacing_mut().interact_size.x = 0.0;
            ui.spacing_mut().button_padding = egui::vec2(2.0, 2.0);
            ui.spacing_mut().item_spacing.x = 4.0;

            if hierarchy.has_children {
                let arrow = if hierarchy.is_expanded { "⏷" } else { "⏵" };

                // Allocate an exact interactive rectangle
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());

                // Resolve responsive colors based on interaction states
                let arrow_color = if response.hovered() {
                    if hierarchy.is_expanded {
                        ui.visuals().warn_fg_color.linear_multiply(0.9)
                    } else {
                        ui.visuals().widgets.hovered.text_color()
                    }
                } else if hierarchy.is_expanded {
                    ui.visuals().widgets.active.text_color()
                } else {
                    ui.visuals()
                        .widgets
                        .inactive
                        .text_color()
                        .linear_multiply(0.5)
                };

                // Draw the glyph centered inside the allocated rectangle
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    arrow,
                    egui::FontId::proportional(11.0),
                    arrow_color,
                );

                let frame = ui.ctx().cumulative_frame_nr();
                let activation_id = response.id.with("expansion_frame");
                if response.clicked()
                    && !ui.is_sizing_pass()
                    && ui.ctx().data(|d| d.get_temp::<u64>(activation_id)) != Some(frame)
                {
                    ui.ctx().data_mut(|d| d.insert_temp(activation_id, frame));
                    if hierarchy.is_expanded {
                        self.expanded_rows.remove(row_index as u32);
                    } else {
                        self.expanded_rows.insert(row_index as u32);
                    }

                    self.sorted_children_cache.remove(&row_index);
                    self.filter_cache_dirty = true;
                    changed = true;
                }
            } else {
                let dummy_arrow = egui::RichText::new("⏵").color(egui::Color32::TRANSPARENT);
                ui.add_enabled_ui(false, |ui| {
                    let _ = ui.selectable_label(false, dummy_arrow);
                });
            }
        });

        changed
    }

    /// Evaluates clicks and modifier keys to update row selections.
    pub fn handle_row_selection(&mut self, modifiers: egui::Modifiers, row_index: usize) {
        self.handle_row_selection_at_visible(modifiers, row_index, None);
    }

    /// Evaluates clicks and modifier keys to update row selections using an optional precomputed visible index.
    pub fn handle_row_selection_at_visible(
        &mut self,
        modifiers: egui::Modifiers,
        row_index: usize,
        visible_index: Option<usize>,
    ) {
        let active_rows = &self.active_rows;
        let row_idx_u32 = row_index as u32;

        let current_visible_pos = visible_index
            .filter(|&idx| idx < active_rows.len() && active_rows[idx] == row_index)
            .or_else(|| {
                if row_index < active_rows.len() && active_rows[row_index] == row_index {
                    Some(row_index)
                } else {
                    active_rows.iter().position(|&r| r == row_index)
                }
            });

        if modifiers.command || modifiers.ctrl {
            if self.selected_rows.contains(row_idx_u32) {
                self.selected_rows.remove(row_idx_u32);
                self.last_clicked_visible_index = None;
            } else {
                self.selected_rows.insert(row_idx_u32);
                self.last_clicked_visible_index = current_visible_pos;
            }
        } else if modifiers.shift && self.last_clicked_visible_index.is_some() {
            if let Some(anchor_visible_pos) = self.last_clicked_visible_index
                && let Some(cur_pos) = current_visible_pos
            {
                let start = anchor_visible_pos.min(cur_pos);
                let end = anchor_visible_pos.max(cur_pos);
                for visible_idx in start..=end {
                    if let Some(&actual_row_idx) = active_rows.get(visible_idx) {
                        self.selected_rows.insert(actual_row_idx as u32);
                    }
                }
            }
        } else if self.selected_rows.len() == 1 && self.selected_rows.contains(row_idx_u32) {
            self.selected_rows.clear();
            self.last_clicked_visible_index = None;
        } else {
            self.selected_rows.clear();
            self.selected_rows.insert(row_idx_u32);
            self.last_clicked_visible_index = current_visible_pos;
        }
    }

    /// Rebuilds the O(N) reverse-propagation subtree filter cache from scratch if dirty.
    pub fn rebuild_tree_filter_cache(&mut self, provider: &dyn TableProvider) {
        let row_count = provider.row_count();

        // Invalidate and clear sibling sort caches if the snapshot shifted (row count changed)
        let is_empty = self.filter_matches.is_empty();
        if self.filter_cache_dirty || is_empty {
            self.sorted_children_cache.clear();
        }

        if !self.filter_cache_dirty && !is_empty {
            return; // Cache is warm: do nothing!
        }
        self.filter_cache_dirty = false;
        self.filter_matches.clear();

        let active_filters = self.get_filter_state();
        if active_filters.is_empty() {
            // Memory Optimization: populate the entire range in O(1) time
            self.filter_matches.insert_range(0..row_count as u32);
            return;
        }

        // Single-pass O(N) reverse propagation of matching subtrees
        for row_idx in (0..row_count).rev() {
            let highlight = self.highlights.get_usize(row_idx);
            let matches = provider.row_matches(self, row_idx, &active_filters, highlight);

            if matches {
                self.filter_matches.insert(row_idx as u32);
            }

            // Propagate match state upwards to parents
            if self.filter_matches.contains(row_idx as u32)
                && let Some(parent_idx) = provider.row_parent(row_idx)
            {
                self.filter_matches.insert(parent_idx as u32);
            }
        }
    }

    /// Iteratively flattens the visible tree nodes matching the active filters into `active_rows`.
    ///
    /// Returns `Err(TableError::CorruptedState)` if a cycle or out-of-bounds row index is detected.
    pub fn flatten_tree(&mut self, provider: &dyn TableProvider) -> Result<(), TableError> {
        self.rebuild_tree_filter_cache(provider);
        let mut active = Vec::with_capacity(provider.row_count());
        let mut stack = Vec::with_capacity(provider.row_count().min(64));
        if provider.row_count() > 0 {
            stack.push(0);
        }
        let mut visited = RoaringBitmap::new();
        let result = (|| {
            while let Some(row) = stack.pop() {
                if row >= provider.row_count() {
                    return Err(TableError::CorruptedState);
                }
                if !self.filter_matches.contains(row as u32) {
                    continue;
                }
                if !visited.insert(row as u32) {
                    return Err(TableError::CorruptedState);
                }
                active.push(row);
                if !self.expanded_rows.contains(row as u32) {
                    continue;
                }
                let children = if let Some(cached) = self.sorted_children_cache.get(&row) {
                    Arc::clone(cached)
                } else {
                    let mut children = provider.row_children(row);
                    if children.iter().any(|&child| child >= provider.row_count()) {
                        return Err(TableError::CorruptedState);
                    }
                    if let Some((column, ascending)) = self.get_sort_state() {
                        provider.sort_active_rows(&mut children, column, ascending)?;
                    }
                    let children = Arc::new(children);
                    self.sorted_children_cache
                        .insert(row, Arc::clone(&children));
                    children
                };
                stack.extend(children.iter().rev().copied());
            }
            Ok(())
        })();
        if result.is_err() {
            self.filter_cache_dirty = true;
            self.sorted_children_cache.clear();
        } else {
            self.active_rows = active;
        }
        result
    }
}

pub trait TableStateExt {
    fn collect_responses(&mut self, responses: Vec<ColResponse>) -> TableChanges;
}

impl TableStateExt for TableState {
    fn collect_responses(&mut self, responses: Vec<ColResponse>) -> TableChanges {
        let mut filter_update = None;
        let mut sort_update = None;
        let mut filter_state = Vec::with_capacity(responses.len());

        // Ensure the vector capacity covers the received header layout size
        if self.columns.len() < responses.len() {
            self.columns
                .resize_with(responses.len(), ColumnState::default);
        }

        for (col_index, response) in responses.into_iter().enumerate() {
            let col = &mut self.columns[col_index];
            let old_active = !col.response.filtering.is_empty();
            let new_active = !response.filtering.is_empty();

            if !old_active && new_active {
                filter_update = Some(FilterUpdate::Added(col_index, response.filtering.clone()));
                self.filter_cache_dirty = true;
            } else if old_active && !new_active {
                filter_update = Some(FilterUpdate::Removed(col_index));
                self.filter_cache_dirty = true;
            } else if old_active && new_active {
                let old = &col.response.filtering;
                let new = &response.filtering;

                if old.search.text() != new.search.text()
                    || old.search.options() != new.search.options()
                    || old.highlight != new.highlight
                {
                    filter_update = Some(FilterUpdate::Modified(
                        col_index,
                        response.filtering.clone(),
                    ));
                    self.filter_cache_dirty = true;
                }
            }

            if new_active {
                filter_state.push((col_index, response.filtering.clone()));
            }

            col.response = response;
            if col.response.to_sort {
                col.response.to_sort = false;
                sort_update = Some(col_index);
            }
        }

        let sort_state = self.get_sort_state();

        TableChanges {
            filter_update,
            sort_update,
            filter_state,
            sort_state,
        }
    }
}

#[cfg(test)]
mod tree_response_tests {
    use super::*;
    use crate::operations::{BorrowedRow, HeaderIter, RowCallback, TableCell};
    use std::borrow::Cow;

    struct Tree;
    impl TableProvider for Tree {
        fn column_count(&self) -> usize {
            1
        }
        fn header(&self, column: usize) -> Option<Cow<'_, str>> {
            (column == 0).then_some(Cow::Borrowed("Name"))
        }
        fn headers(&self) -> HeaderIter<'_> {
            HeaderIter::new(self)
        }
        fn row_count(&self) -> usize {
            5
        }
        fn cell_at(&self, row: usize, col: usize) -> Result<Option<TableCell<'_>>, TableError> {
            Ok(if col == 0 {
                ["root", "Zulu", "A file", "Alpha", "Z file"]
                    .get(row)
                    .map(|s| (Cow::Borrowed(*s), None))
            } else {
                None
            })
        }
        fn for_all_rows(&self, f: &mut RowCallback<'_>) -> Result<(), TableError> {
            for row_index in 0..5 {
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
        fn is_tree(&self) -> bool {
            true
        }
        fn row_matches(
            &self,
            _state: &TableState,
            row: usize,
            filters: &[(usize, Filter)],
            highlight: Option<u8>,
        ) -> bool {
            filters.iter().all(|(column, filter)| {
                self.cell_at(row, *column)
                    .ok()
                    .flatten()
                    .is_some_and(|(text, _)| filter.matches(&text, highlight))
            })
        }
        fn row_parent(&self, row: usize) -> Option<usize> {
            [None, Some(0), Some(1), Some(0), Some(3)][row]
        }
        fn row_children(&self, row: usize) -> Vec<usize> {
            match row {
                0 => vec![1, 3],
                1 => vec![2],
                3 => vec![4],
                _ => vec![],
            }
        }
    }

    #[test]
    fn header_sort_filter_and_highlight_changes_preserve_tree_order() {
        let mut state = TableState::new("tree", 5);
        state.expanded_rows.extend([0, 1, 3]);
        state.selected_rows.insert(2);
        state.refresh_view(&Tree).unwrap();
        state.last_clicked_visible_index = Some(2);
        let sort = || {
            vec![ColResponse {
                to_sort: true,
                ..Default::default()
            }]
        };
        state.process_responses(&Tree, sort()).unwrap();
        assert_eq!(state.active_rows, [0, 3, 4, 1, 2]);
        assert_eq!(state.last_clicked_visible_index, Some(4));
        state.process_responses(&Tree, sort()).unwrap();
        assert_eq!(state.active_rows, [0, 1, 2, 3, 4]);
        assert_eq!(state.last_clicked_visible_index, Some(2));
        let mut filter = ColResponse::default();
        filter.filtering.search.set_text("Z file");
        filter.filtering.search.open();
        state
            .process_responses(&Tree, vec![filter.clone()])
            .unwrap();
        assert_eq!(state.active_rows, [0, 3, 4]);
        state.highlights_changed = true;
        state.process_responses(&Tree, vec![filter]).unwrap();
        assert_eq!(state.active_rows, [0, 3, 4]);
        state
            .process_responses(&Tree, vec![ColResponse::default()])
            .unwrap();
        assert_eq!(state.active_rows, [0, 1, 2, 3, 4]);
        assert!(state.selected_rows.contains(2));
        assert!(!state.filter_cache_dirty);
    }

    #[test]
    fn applying_a_tree_sort_directly_preserves_hierarchy() {
        let mut state = TableState::new("tree", 5);
        state.columns.push(ColumnState::default());
        state.expanded_rows.extend([0, 1, 3]);
        state.apply_new_sort(&Tree, 0).unwrap();
        assert_eq!(state.active_rows, [0, 3, 4, 1, 2]);
    }

    struct CyclicTree;
    impl TableProvider for CyclicTree {
        fn column_count(&self) -> usize {
            1
        }
        fn header(&self, _col: usize) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed("Col"))
        }
        fn headers(&self) -> HeaderIter<'_> {
            HeaderIter::new(self)
        }
        fn row_count(&self) -> usize {
            3
        }
        fn cell_at(&self, _row: usize, _col: usize) -> Result<Option<TableCell<'_>>, TableError> {
            Ok(Some((Cow::Borrowed("item"), None)))
        }
        fn for_all_rows(&self, _f: &mut RowCallback<'_>) -> Result<(), TableError> {
            Ok(())
        }
        fn for_selected_rows(
            &self,
            _state: &TableState,
            _f: &mut RowCallback<'_>,
        ) -> Result<(), TableError> {
            Ok(())
        }
        fn is_tree(&self) -> bool {
            true
        }
        fn row_matches(
            &self,
            _state: &TableState,
            _row: usize,
            _filters: &[(usize, Filter)],
            _highlight: Option<u8>,
        ) -> bool {
            true
        }
        fn row_parent(&self, row: usize) -> Option<usize> {
            match row {
                1 => Some(0),
                0 => Some(1),
                _ => None,
            }
        }
        fn row_children(&self, row: usize) -> Vec<usize> {
            match row {
                0 => vec![1],
                1 => vec![0], // Cycle back to 0!
                _ => vec![],
            }
        }
    }

    #[test]
    fn test_tree_cycle_detection_returns_corrupted_state() {
        let mut state = TableState::new("cyclic", 3);
        state.expanded_rows.extend([0, 1]);
        let res = state.flatten_tree(&CyclicTree);
        assert!(matches!(res, Err(TableError::CorruptedState)));
        // Corrupted state should leave cache marked dirty and cleared
        assert!(state.filter_cache_dirty);
        assert!(state.sorted_children_cache.is_empty());
    }

    struct OutOfBoundsTree;
    impl TableProvider for OutOfBoundsTree {
        fn column_count(&self) -> usize {
            1
        }
        fn header(&self, _col: usize) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed("Col"))
        }
        fn headers(&self) -> HeaderIter<'_> {
            HeaderIter::new(self)
        }
        fn row_count(&self) -> usize {
            2
        }
        fn cell_at(&self, _row: usize, _col: usize) -> Result<Option<TableCell<'_>>, TableError> {
            Ok(Some((Cow::Borrowed("item"), None)))
        }
        fn for_all_rows(&self, _f: &mut RowCallback<'_>) -> Result<(), TableError> {
            Ok(())
        }
        fn for_selected_rows(
            &self,
            _state: &TableState,
            _f: &mut RowCallback<'_>,
        ) -> Result<(), TableError> {
            Ok(())
        }
        fn is_tree(&self) -> bool {
            true
        }
        fn row_matches(
            &self,
            _state: &TableState,
            _row: usize,
            _filters: &[(usize, Filter)],
            _highlight: Option<u8>,
        ) -> bool {
            true
        }
        fn row_parent(&self, _row: usize) -> Option<usize> {
            None
        }
        fn row_children(&self, row: usize) -> Vec<usize> {
            if row == 0 {
                vec![99] // 99 >= row_count (2)
            } else {
                vec![]
            }
        }
    }

    #[test]
    fn test_tree_out_of_bounds_child_returns_corrupted_state() {
        let mut state = TableState::new("oob", 2);
        state.expanded_rows.insert(0);
        let res = state.flatten_tree(&OutOfBoundsTree);
        assert!(matches!(res, Err(TableError::CorruptedState)));
    }

    #[test]
    fn test_refresh_view_clears_dirty_cache_for_trees() {
        let mut state = TableState::new("tree", 5);
        state.expanded_rows.extend([0, 1, 3]);
        assert!(state.filter_cache_dirty);

        let did_refresh = state.refresh_view(&Tree).unwrap();
        assert!(did_refresh);
        assert!(!state.filter_cache_dirty);
        assert_eq!(state.active_rows, [0, 1, 2, 3, 4]);

        // Subsequent frame: dirty flag is false, so refresh_view is a zero-work O(1) no-op
        let did_refresh_again = state.refresh_view(&Tree).unwrap();
        assert!(!did_refresh_again);
    }

    #[test]
    fn test_show_tree_expander_arrow_sizing_pass_ignored() {
        let mut state = TableState::new("tree", 5);
        state.filter_cache_dirty = false;
        let hierarchy = RowHierarchy {
            indent_level: 1,
            has_children: true,
            is_expanded: false,
        };

        let make_input = |pos: egui::Pos2, pressed: bool, time: f64| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500.0, 350.0),
            )),
            time: Some(time),
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        let arrow_pos = egui::pos2(7.0, 29.0);

        // Pass 1: Sizing pass - click must be ignored even when clicked
        let ctx1 = egui::Context::default();
        let mut out = ctx1.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 350.0),
                )),
                time: Some(0.0),
                events: vec![egui::Event::PointerMoved(arrow_pos)],
                ..Default::default()
            },
            |ui| {
                let mut sizing_ui = ui.new_child(egui::UiBuilder::new().sizing_pass());
                let _ = state.show_tree_cell(&mut sizing_ui, 0, hierarchy);
            },
        );
        out.textures_delta.clear();

        let mut out = ctx1.run_ui(make_input(arrow_pos, true, 1.0), |ui| {
            let mut sizing_ui = ui.new_child(egui::UiBuilder::new().sizing_pass());
            let _ = state.show_tree_cell(&mut sizing_ui, 0, hierarchy);
        });
        out.textures_delta.clear();

        let mut out = ctx1.run_ui(make_input(arrow_pos, false, 1.05), |ui| {
            let mut sizing_ui = ui.new_child(egui::UiBuilder::new().sizing_pass());
            let changed = state.show_tree_cell(&mut sizing_ui, 0, hierarchy);
            assert!(!changed, "Sizing pass must never toggle expansion");
            assert!(!state.expanded_rows.contains(0));
            assert!(!state.filter_cache_dirty);
        });
        out.textures_delta.clear();

        // Pass 2: Normal pass with click
        let ctx2 = egui::Context::default();
        // Frame 0: layout pass so widget bounds are registered
        let mut out = ctx2.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 350.0),
                )),
                time: Some(0.0),
                events: vec![egui::Event::PointerMoved(arrow_pos)],
                ..Default::default()
            },
            |ui| {
                let _ = state.show_tree_cell(ui, 0, hierarchy);
            },
        );
        out.textures_delta.clear();

        // Frame 1: Pointer press
        let mut out = ctx2.run_ui(make_input(arrow_pos, true, 1.0), |ui| {
            let _ = state.show_tree_cell(ui, 0, hierarchy);
        });
        out.textures_delta.clear();

        // Frame 2: Pointer release -> click
        let mut out = ctx2.run_ui(make_input(arrow_pos, false, 1.05), |ui| {
            let changed = state.show_tree_cell(ui, 0, hierarchy);
            assert!(changed, "Normal pass must process click");
            assert!(state.expanded_rows.contains(0));
            assert!(state.filter_cache_dirty);

            // Attempt duplicate activation in same frame
            let hierarchy_expanded = RowHierarchy {
                indent_level: 1,
                has_children: true,
                is_expanded: true,
            };
            let changed_dup = state.show_tree_cell(ui, 0, hierarchy_expanded);
            assert!(
                !changed_dup,
                "Second activation in same frame must be de-duplicated"
            );
        });
        out.textures_delta.clear();
    }
}
