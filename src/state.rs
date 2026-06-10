use std::{collections::HashMap, fmt::Write as _};

use compact_str::CompactString;
use fluent_zero::t;
use roaring::RoaringBitmap;

use super::{
    error::TableError,
    filter::Filter,
    header::{ColResponse, ColumnState},
    highlights::Highlights,
    operations::TableProvider,
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
    pub sorted_children_cache: HashMap<usize, Vec<usize>, ahash::RandomState>,
}

impl TableState {
    #[must_use]
    pub fn new(id: impl Into<CompactString>, row_count: usize) -> Self {
        Self {
            id: id.into(),
            active_rows: (0..row_count).collect(),
            filter_cache_dirty: true, // Mark dirty initially to force first-frame population
            ..Default::default()
        }
    }

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

    #[must_use]
    pub fn get_sort_state(&self) -> Option<(usize, bool)> {
        self.columns
            .iter()
            .enumerate()
            .find_map(|(col_index, col)| col.sort_up.map(|sort_up| (col_index, sort_up)))
    }

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

    pub fn process_responses(
        &mut self,
        provider: &dyn TableProvider,
        responses: Vec<ColResponse>,
    ) -> Result<(), TableError> {
        let changes = self.collect_responses(responses);
        let filter_update = changes.filter_update;
        let sort_update = changes.sort_update;
        let filter_state = changes.filter_state;
        let sort_state = changes.sort_state;

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

    pub fn apply_new_sort(
        &mut self,
        provider: &dyn TableProvider,
        sort_col: usize,
    ) -> Result<(), TableError> {
        for (i, column) in self.columns.iter_mut().enumerate() {
            column.sort_up = if i == sort_col { Some(true) } else { None };
        }
        provider.sort_active_rows(&mut self.active_rows, sort_col, true)
    }

    pub fn apply_all_filters(
        &mut self,
        provider: &dyn TableProvider,
        filters: &[(usize, Filter)],
    ) -> Result<(), TableError> {
        self.active_rows = provider.filter_rows(self, filters)?;
        Ok(())
    }

    pub fn apply_incremental_filter(
        &mut self,
        provider: &dyn TableProvider,
        filter_col: usize,
        filter: &Filter,
    ) -> Result<(), TableError> {
        let mut active_mask = vec![false; provider.row_count()];
        for &idx in &self.active_rows {
            if idx < active_mask.len() {
                active_mask[idx] = true;
            }
        }

        let mut new_active = Vec::with_capacity(self.active_rows.len());
        let mut row_idx = 0;

        provider.for_all_rows(&mut |row| {
            if row_idx < active_mask.len() && active_mask[row_idx] {
                let highlight = self.highlights.get_usize(row_idx);
                if let Some(cell) = row.get(filter_col)
                    && filter.matches(&cell.0, highlight)
                {
                    new_active.push(row_idx);
                }
            }
            row_idx += 1;
            Ok(())
        })?;

        self.active_rows = new_active;
        Ok(())
    }

    /// Renders the tree indentation guidelines and expand/collapse arrow inside a tree cell.
    /// Returns `true` if the expansion state changed (allowing immediate-mode viewport updates).
    pub fn show_tree_cell(
        &mut self,
        ui: &mut egui::Ui,
        row_index: usize,
        hierarchy: crate::operations::RowHierarchy,
    ) -> bool {
        let mut changed = false;

        #[allow(clippy::cast_precision_loss)]
        let spacing = hierarchy.indent_level as f32 * 16.0;
        if spacing > 0.0 {
            ui.add_space(spacing);

            let rect = ui.max_rect();
            let painter = ui.painter();
            let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(65, 65, 65)); // Guidelines gray

            for i in 0..hierarchy.indent_level {
                #[allow(clippy::cast_precision_loss)]
                let x = (i as f32).mul_add(16.0, rect.min.x) + 8.0;

                // Draw dashed guideline segments
                let dash_length = 2.0;
                let gap_length = 2.0;
                let step = dash_length + gap_length;
                let total_height = rect.max.y - rect.min.y;
                if total_height > 0.0 {
                    let num_steps = (total_height / step).ceil() as usize;
                    for step_idx in 0..num_steps {
                        #[allow(clippy::cast_precision_loss)]
                        let segment_y = (step_idx as f32).mul_add(step, rect.min.y);
                        let next_y = (segment_y + dash_length).min(rect.max.y);
                        painter.line_segment(
                            [egui::pos2(x, segment_y), egui::pos2(x, next_y)],
                            stroke,
                        );
                    }
                }
            }
        }

        if hierarchy.has_children {
            let arrow = if hierarchy.is_expanded { "⏷" } else { "⏵" };

            let arrow_color = if hierarchy.is_expanded {
                ui.visuals().widgets.active.text_color()
            } else {
                ui.visuals().widgets.inactive.text_color()
            };

            let rich_arrow = egui::RichText::new(arrow).color(arrow_color);
            if ui
                .selectable_label(hierarchy.is_expanded, rich_arrow)
                .clicked()
            {
                if hierarchy.is_expanded {
                    self.expanded_rows.remove(row_index as u32);
                } else {
                    self.expanded_rows.insert(row_index as u32);
                }

                self.sorted_children_cache.remove(&row_index);
                changed = true;
            }
        } else {
            // Render an invisible, disabled placeholder label to reserve the exact layout footprint
            let dummy_arrow = egui::RichText::new("⏵").color(egui::Color32::TRANSPARENT);
            ui.add_enabled_ui(false, |ui| {
                let _ = ui.selectable_label(false, dummy_arrow);
            });
        }

        changed
    }

    pub fn handle_row_selection(&mut self, modifiers: egui::Modifiers, row_index: usize) {
        let selected_rows = &mut self.selected_rows;
        let active_rows = &self.active_rows;

        let row_idx_u32 = row_index as u32;

        if modifiers.command || modifiers.ctrl {
            if selected_rows.contains(row_idx_u32) {
                selected_rows.remove(row_idx_u32);
                self.last_clicked_visible_index = None;
            } else {
                selected_rows.insert(row_idx_u32);
                self.last_clicked_visible_index = active_rows.iter().position(|&r| r == row_index);
            }
        } else if modifiers.shift && self.last_clicked_visible_index.is_some() {
            if let Some(anchor_visible_pos) = self.last_clicked_visible_index
                && let Some(current_visible_pos) = active_rows.iter().position(|&r| r == row_index)
            {
                let start = anchor_visible_pos.min(current_visible_pos);
                let end = anchor_visible_pos.max(current_visible_pos);
                for visible_idx in start..=end {
                    if let Some(&actual_row_idx) = active_rows.get(visible_idx) {
                        selected_rows.insert(actual_row_idx as u32);
                    }
                }
            }
        } else if selected_rows.len() == 1 && selected_rows.contains(row_idx_u32) {
            selected_rows.clear();
            self.last_clicked_visible_index = None;
        } else {
            selected_rows.clear();
            selected_rows.insert(row_idx_u32);
            self.last_clicked_visible_index = active_rows.iter().position(|&r| r == row_index);
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

    /// Recursively flattens the visible tree nodes matching the active filters into `active_rows`.
    pub fn flatten_tree(&mut self, provider: &dyn TableProvider) {
        self.rebuild_tree_filter_cache(provider);

        let mut active = Vec::with_capacity(provider.row_count());
        if provider.row_count() > 0 {
            // Flatten from the root node (0) downwards
            self.flatten_tree_impl(provider, 0, &mut active);
        }
        self.active_rows = active;
    }

    fn flatten_tree_impl(
        &mut self,
        provider: &dyn TableProvider,
        row_idx: usize,
        out: &mut Vec<usize>,
    ) {
        // Fast, O(log C) random access check over compressed roaring bitmap containers
        if !self.filter_matches.contains(row_idx as u32) {
            return; // Subtree does not match filters: discard early
        }

        out.push(row_idx);

        let is_expanded = self.expanded_rows.contains(row_idx as u32);
        if is_expanded {
            // Retrieve stable sibling order from cache or sort once on cache-miss
            let sorted_children = if let Some(cached) = self.sorted_children_cache.get(&row_idx) {
                cached.clone()
            } else {
                let mut children = provider.row_children(row_idx);
                let sort_state = self.get_sort_state();
                if let Some((sort_col, sort_up)) = sort_state {
                    let _ = provider.sort_active_rows(&mut children, sort_col, sort_up);
                }
                self.sorted_children_cache.insert(row_idx, children.clone());
                children
            };

            for child_idx in sorted_children {
                self.flatten_tree_impl(provider, child_idx, out);
            }
        }
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
