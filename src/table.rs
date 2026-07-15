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
        // Refresh filter/sorting view when dirty
        let _ = self.state.refresh_view(self.provider);

        // Prioritize custom pre-configured layout columns over fallback defaults
        let columns = self.columns.unwrap_or_else(|| {
            (0..self.provider.column_count())
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
