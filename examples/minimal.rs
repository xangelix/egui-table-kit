use std::borrow::Cow;

use eframe::egui;
use egui_table_kit::{
    error::TableError,
    filter::highlight::select_color,
    header::HeaderMenuAnchor,
    operations::{
        CopyRows, DeSelectAll, HeaderIter, OperationContext, OwnedRow, Row, RowCallback, SelectAll,
        TableCell, TableOperation, TableOperationEnablement, TableOperations, TableProvider,
    },
    state::TableState,
};

#[derive(Debug, Default)]
pub struct TagSelected {
    pub color_index: u8,
}

impl TableOperation for TagSelected {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("Tag Selected")
    }
    fn icon(&self) -> &'static str {
        "🏷"
    }
    fn enabled(&self) -> TableOperationEnablement {
        TableOperationEnablement::AtLeastOneSelected
    }
    fn exec(&mut self, ctx: &mut OperationContext<'_, '_>) -> Result<(), TableError> {
        for row_idx in &ctx.data.selected_rows {
            ctx.data.highlights.insert(self.color_index, row_idx);
        }
        ctx.data.highlights_changed = true;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct UntagSelected;

impl TableOperation for UntagSelected {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("Untag Selected")
    }
    fn icon(&self) -> &'static str {
        "🚫"
    }
    fn enabled(&self) -> TableOperationEnablement {
        TableOperationEnablement::AtLeastOneSelected
    }
    fn exec(&mut self, ctx: &mut OperationContext<'_, '_>) -> Result<(), TableError> {
        ctx.data.highlights.remove_map(&ctx.data.selected_rows);
        ctx.data.highlights_changed = true;
        Ok(())
    }
}

struct ContactDataset {
    headers: Vec<&'static str>,
    records: Vec<Vec<String>>,
}

struct ContactRow<'a> {
    record: &'a [String],
}

impl Row for ContactRow<'_> {
    fn cell(&self, col_index: usize) -> Option<TableCell<'_>> {
        self.record
            .get(col_index)
            .map(|s| (Cow::Borrowed(s.as_str()), None))
    }

    fn column_count(&self) -> usize {
        self.record.len()
    }
}

impl TableProvider for ContactDataset {
    fn column_count(&self) -> usize {
        self.headers.len()
    }

    fn header(&self, index: usize) -> Option<Cow<'_, str>> {
        self.headers.get(index).map(|&s| Cow::Borrowed(s))
    }

    fn headers(&self) -> HeaderIter<'_> {
        HeaderIter::new(self)
    }

    fn row_count(&self) -> usize {
        self.records.len()
    }

    fn cell_at(
        &self,
        row_index: usize,
        col_index: usize,
    ) -> Result<Option<TableCell<'_>>, TableError> {
        Ok(self
            .records
            .get(row_index)
            .and_then(|row| row.get(col_index))
            .map(|s| (Cow::Borrowed(s.as_str()), None)))
    }

    /// Keeps backward-compatibility wrapper intact for export operations (e.g. clipboard copy)
    fn row_at(&self, index: usize) -> Result<Option<OwnedRow>, TableError> {
        Ok(self.records.get(index).map(|record| OwnedRow {
            cells: record
                .iter()
                .map(|s| (compact_str::CompactString::from(s.as_str()), None))
                .collect(),
        }))
    }

    fn for_all_rows(&self, f: &mut RowCallback<'_>) -> Result<(), TableError> {
        for record in &self.records {
            f(&ContactRow { record })?;
        }
        Ok(())
    }

    fn for_selected_rows(
        &self,
        state: &TableState,
        f: &mut RowCallback<'_>,
    ) -> Result<(), TableError> {
        for idx in &state.selected_rows {
            if let Some(record) = self.records.get(idx as usize) {
                f(&ContactRow { record })?;
            }
        }
        Ok(())
    }
}

struct TableApp {
    provider: ContactDataset,
    state: TableState,
    operations: TableOperations,
}

impl Default for TableApp {
    fn default() -> Self {
        let provider = ContactDataset {
            headers: vec!["Name", "Role", "Status"],
            records: vec![
                vec![
                    "Alice".to_string(),
                    "Engineer".to_string(),
                    "Active".to_string(),
                ],
                vec![
                    "Bob".to_string(),
                    "Designer".to_string(),
                    "Away".to_string(),
                ],
                vec![
                    "Charlie".to_string(),
                    "Manager".to_string(),
                    "Active".to_string(),
                ],
            ],
        };

        let row_count = provider.row_count();
        let state = TableState::new("demo_table_id", row_count);

        // Group operations to establish separators
        let operations = TableOperations::new()
            .with_group(vec![Box::new(SelectAll), Box::new(DeSelectAll)])
            .with_group(vec![
                Box::new(TagSelected::default()),
                Box::new(UntagSelected),
            ])
            .with_group(vec![Box::new(CopyRows::default())]);

        Self {
            provider,
            state,
            operations,
        }
    }
}

impl eframe::App for TableApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(egui::RichText::new("egui-table-kit Explorer").strong());
            ui.add_space(10.0);

            // Grouped action toolbar
            ui.horizontal(|ui| {
                let _ = self
                    .operations
                    .gui(ui, &self.provider, &mut self.state, false);
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(self.state.counts_header(self.provider.row_count()));
                ui.weak("|  💡 Left-click headers for sorts, right-click headers for filters.");
            });
            ui.add_space(6.0);

            let org_colors = [[219, 58, 58], [58, 219, 112]];
            let user_colors = [];

            // Refresh the filtered/sorted view only when something changed; this is a
            // cheap no-op on idle frames instead of a full O(N) filter + sort pass.
            let _ = self.state.refresh_view(&self.provider);

            // Expose table settings with modern, virtualized columns
            let columns = (0..self.provider.column_count())
                .map(|col_idx| {
                    let is_last = col_idx == self.provider.column_count() - 1;
                    let initial_width = if is_last { 150.0 } else { 120.0 };
                    egui_table_kit::layout::Column::new(initial_width)
                        .range(15.0..=f32::INFINITY)
                        .resizable(true)
                })
                .collect::<Vec<_>>();

            let row_height = 28.0;
            let table = egui_table_kit::layout::Table::new()
                .id_salt("stable_explorer_table")
                .num_rows(self.state.active_rows.len() as u64)
                .columns(columns)
                .headers([egui_table_kit::layout::HeaderRow::new(row_height)]);

            let active_rows_count = self.state.active_rows.len();

            let Self {
                provider, state, ..
            } = self;

            let mut collected_responses = Vec::new();
            let mut halt_error = None;
            let mut item_clicked = None;
            let mut secondary_clicked = None;

            // Clone lightweight structures to completely separate mutable borrow Lifetimes
            let highlights = state.highlights.clone();
            let active_rows = state.active_rows.clone();

            let custom_cell_ui = Box::new(
                move |ui: &mut egui::Ui,
                      cell_info: &egui_table_kit::layout::CellInfo,
                      row_data: &dyn Row,
                      text_color: egui::Color32| {
                    let row_idx = active_rows[cell_info.row_nr as usize];

                    let tag_color = if let Some(color_idx) = highlights.get_usize(row_idx)
                        && let Ok(rgb) = select_color(color_idx, &org_colors, &user_colors)
                    {
                        Some(egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                    } else {
                        None
                    };

                    let response = ui
                        .horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;

                            if cell_info.col_nr == 0
                                && let Some(color) = tag_color
                            {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(rect, 1.5, color);
                            }

                            if let Some((cell_val, _)) = row_data.cell(cell_info.col_nr) {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(cell_val.as_ref()).color(text_color),
                                    )
                                    .selectable(false)
                                    .wrap_mode(
                                        if ui.is_sizing_pass() {
                                            ui.wrap_mode()
                                        } else {
                                            egui::TextWrapMode::Truncate
                                        },
                                    ),
                                );
                            }
                        })
                        .response;

                    Some(response)
                },
            );

            let mut delegate = egui_table_kit::delegate::TableKitDelegate::new(
                provider,
                state,
                &org_colors,
                &user_colors,
                &mut collected_responses,
                &mut halt_error,
                Some(custom_cell_ui),
                &mut item_clicked,
                &mut secondary_clicked,
            );

            // Configure menu popup anchoring: can be HeaderMenuAnchor::Cursor or HeaderMenuAnchor::Cell
            delegate.header_menu_anchor = HeaderMenuAnchor::Cursor;

            // Make the separator lines brighter by temporarily overriding noninteractive stroke color
            let bright_gray = egui::Color32::from_rgb(180, 180, 180);
            ui.visuals_mut().widgets.noninteractive.bg_stroke.color = bright_gray;

            // Ensure the delegate's row height configuration is set as desired
            delegate.row_height = row_height;

            #[allow(clippy::cast_precision_loss)]
            let total_height = (active_rows_count + 1) as f32 * delegate.row_height;
            let table_height = total_height.min(ui.available_height());

            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), table_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    table.show(ui, &mut delegate);
                },
            );

            // Explicitly release the borrows held by the delegate
            drop(delegate);

            let _ = state.process_responses(provider, collected_responses);
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "egui-table-kit Explorer",
        options,
        Box::new(|_cc| Ok(Box::<TableApp>::default())),
    )
}
