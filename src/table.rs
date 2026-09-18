//! High-level table runner to completely encapsulate `egui_table` and Delegate boilerplate.

use crate::{
    error::TableError,
    operations::{Row, TableProvider},
    state::TableState,
};

pub struct TableKit<'a> {
    id: String,
    provider: &'a dyn TableProvider,
    state: &'a mut TableState,
    row_height: f32,
    org_colors: &'a [[u8; 3]],
    user_colors: &'a [[u8; 3]],
    scroll_to_row: Option<(u64, egui::Align)>,
    striped: bool,
    striping_color: Option<egui::Color32>,
    hover_color: Option<egui::Color32>,
    max_height: Option<f32>,
    max_rows: Option<u64>,
    columns: Option<Vec<crate::layout::Column>>,
    auto_size_mode: crate::layout::AutoSizeMode,
    column_map: Option<Vec<usize>>,
}

impl<'a> TableKit<'a> {
    pub fn new(
        id: impl Into<String>,
        provider: &'a dyn TableProvider,
        state: &'a mut TableState,
    ) -> Self {
        Self {
            id: id.into(),
            provider,
            state,
            row_height: 16.0,
            org_colors: &[],
            user_colors: &[],
            scroll_to_row: None,
            striped: false,
            striping_color: None,
            hover_color: None,
            max_height: Some(400.0), // Default limit fallback
            max_rows: None,
            columns: None,
            auto_size_mode: crate::layout::AutoSizeMode::OnParentResize,
            column_map: None,
        }
    }

    #[must_use]
    pub fn with_columns(mut self, columns: Vec<crate::layout::Column>) -> Self {
        self.columns = Some(columns);
        self
    }

    #[must_use]
    pub const fn with_row_height(mut self, height: f32) -> Self {
        self.row_height = height;
        self
    }

    #[must_use]
    pub const fn with_max_height(mut self, max_height: Option<f32>) -> Self {
        self.max_height = max_height;
        self
    }

    #[must_use]
    pub const fn with_colors(mut self, org: &'a [[u8; 3]], user: &'a [[u8; 3]]) -> Self {
        self.org_colors = org;
        self.user_colors = user;
        self
    }

    #[must_use]
    pub const fn with_auto_size_mode(mut self, mode: crate::layout::AutoSizeMode) -> Self {
        self.auto_size_mode = mode;
        self
    }

    #[must_use]
    pub const fn with_max_rows(mut self, max_rows: u64) -> Self {
        self.max_rows = Some(max_rows);
        self.max_height = None; // Prioritize explicit row limit over pixel defaults
        self
    }

    /// Set an optional row to scroll to during the next rendering pass.
    #[must_use]
    pub const fn with_scroll_to_row(mut self, row_nr: u64, align: egui::Align) -> Self {
        self.scroll_to_row = Some((row_nr, align));
        self
    }

    /// Enable or disable alternating row background colors.
    #[must_use]
    pub const fn with_striped(mut self, striped: bool) -> Self {
        self.striped = striped;
        self
    }

    /// Provide a custom background color for alternating striped rows.
    /// If none is provided, it falls back to `ui.visuals().faint_bg_color`.
    #[must_use]
    pub const fn with_striping_color(mut self, color: egui::Color32) -> Self {
        self.striping_color = Some(color);
        self
    }

    /// Set an optional custom background overlay color for hovered rows.
    #[must_use]
    pub const fn with_hover_color(mut self, color: egui::Color32) -> Self {
        self.hover_color = Some(color);
        self
    }

    /// Select/reorder provider columns. Custom cell callbacks receive provider column indices.
    #[must_use]
    pub fn with_column_map(mut self, columns: Vec<usize>) -> Self {
        self.column_map = Some(columns);
        self
    }

    pub fn show<F>(self, ui: &mut egui::Ui, custom_cell_ui: F) -> Result<egui::Response, TableError>
    where
        F: FnMut(
                &mut egui::Ui,
                &crate::layout::CellInfo,
                &dyn Row,
                egui::Color32,
            ) -> Option<egui::Response>
            + 'a,
    {
        if let Some(map) = &self.column_map {
            let col_count = self.provider.column_count();
            if map.is_empty()
                || self
                    .columns
                    .as_ref()
                    .is_some_and(|columns| columns.len() != map.len())
            {
                return Err(TableError::CorruptedState);
            }
            if col_count <= 64 {
                let mut seen = 0u64;
                for &c in map {
                    if c >= col_count || (seen & (1u64 << c)) != 0 {
                        return Err(TableError::CorruptedState);
                    }
                    seen |= 1u64 << c;
                }
            } else {
                let mut seen = std::collections::HashSet::with_capacity(map.len());
                for &c in map {
                    if c >= col_count || !seen.insert(c) {
                        return Err(TableError::CorruptedState);
                    }
                }
            }
        }

        // Refresh filter/sorting view when dirty
        let _ = self.state.refresh_view(self.provider);

        // Prioritize custom pre-configured layout columns over fallback defaults
        let columns = self.columns.unwrap_or_else(|| {
            (0..self
                .column_map
                .as_ref()
                .map_or(self.provider.column_count(), Vec::len))
                .map(|_| {
                    crate::layout::Column::new(120.0)
                        .range(15.0..=f32::INFINITY)
                        .resizable(true)
                })
                .collect::<Vec<_>>()
        });

        let mut table = crate::layout::Table::new()
            .id_salt(&self.id)
            .num_rows(self.state.active_rows.len() as u64)
            .columns(columns)
            .auto_size_mode(self.auto_size_mode) // <--- Forward auto-size mode
            .headers([crate::layout::HeaderRow::new(self.row_height)]);

        // Apply scroll-to-row instruction to the table builder
        if let Some((row_nr, align)) = self.scroll_to_row {
            table = table.scroll_to_row(row_nr, Some(align));
        }

        if let Some(max_r) = self.max_rows {
            table = table.max_rows(max_r);
        } else if let Some(max_h) = self.max_height {
            table = table.max_height(max_h);
        }

        let mut collected_responses = Vec::new();
        let mut halt_error = None;

        let response = {
            let mut item_clicked = None;
            let mut secondary_clicked = None;

            let mut delegate = crate::delegate::TableKitDelegate::new(
                self.provider,
                self.state,
                self.org_colors,
                self.user_colors,
                &mut collected_responses,
                &mut halt_error,
                Some(Box::new(custom_cell_ui)),
                &mut item_clicked,
                &mut secondary_clicked,
            );
            delegate.row_height = self.row_height;
            delegate.striped = self.striped;
            delegate.striping_color = self.striping_color;
            delegate.hover_color = self.hover_color;
            delegate.column_map = self.column_map;

            table.show(ui, &mut delegate)
        };

        if let Some(err) = halt_error {
            return Err(err);
        }

        self.state
            .process_responses(self.provider, collected_responses)?;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::{BorrowedRow, HeaderIter, RowCallback, TableCell};
    use std::borrow::Cow;

    struct TestProvider {
        names: Vec<&'static str>,
    }
    impl TableProvider for TestProvider {
        fn column_count(&self) -> usize {
            2
        }
        fn header(&self, index: usize) -> Option<Cow<'_, str>> {
            ["First", "Second"].get(index).map(|s| Cow::Borrowed(*s))
        }
        fn headers(&self) -> HeaderIter<'_> {
            HeaderIter::new(self)
        }
        fn row_count(&self) -> usize {
            self.names.len()
        }
        fn cell_at(&self, row: usize, col: usize) -> Result<Option<TableCell<'_>>, TableError> {
            let val = format!("{}-{}", self.names[row], col);
            Ok(Some((Cow::Owned(val), None)))
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
            _state: &TableState,
            _f: &mut RowCallback<'_>,
        ) -> Result<(), TableError> {
            Ok(())
        }
    }

    #[test]
    fn test_invalid_column_maps_return_error() {
        let provider = TestProvider { names: vec!["A"] };
        let mut state = TableState::new("test", 1);
        let ctx = egui::Context::default();

        for map in [vec![], vec![0, 0], vec![2]] {
            let mut full_output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let res = TableKit::new("bad_map", &provider, &mut state)
                    .with_column_map(map.clone())
                    .show(ui, |_, _, _, _| None);
                assert!(res.is_err());
            });
            full_output.textures_delta.clear();
        }
    }

    #[test]
    fn test_column_map_reordering_dispatches_provider_indices() {
        let provider = TestProvider {
            names: vec!["Item"],
        };
        let mut state = TableState::new("test", 1);
        let ctx = egui::Context::default();

        let mut visited_cols = Vec::new();
        let mut full_output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let res = TableKit::new("reorder", &provider, &mut state)
                .with_column_map(vec![1, 0])
                .with_columns(vec![
                    crate::layout::Column::new(100.0),
                    crate::layout::Column::new(100.0),
                ])
                .show(ui, |_ui, cell, _row, _color| {
                    visited_cols.push(cell.col_nr);
                    None
                });
            assert!(res.is_ok());
        });
        full_output.textures_delta.clear();

        // Custom cell callback must receive provider column indices in layout order:
        // Visual col 0 -> provider col 1
        // Visual col 1 -> provider col 0
        assert_eq!(visited_cols, vec![1, 0]);
    }
}
