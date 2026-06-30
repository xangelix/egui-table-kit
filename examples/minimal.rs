use std::borrow::Cow;

use eframe::egui;
use egui_table_kit::{
    error::TableError,
    filter::highlight::select_color,
    header::HeaderTrait,
    operations::{
        CopyRows, DeSelectAll, HeaderIter, OperationContext, RowCallback, SelectAll, TableOperation,
        TableOperationEnablement, TableOperations, TableProvider,
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

    fn for_all_rows(&self, f: &mut RowCallback<'_>) -> Result<(), TableError> {
        for record in &self.records {
            let cells: Vec<(Cow<'_, str>, Option<Cow<'_, str>>)> = record
                .iter()
                .map(|val| (Cow::Borrowed(val.as_str()), None))
                .collect();
            f(&cells)?;
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
                let cells: Vec<(Cow<'_, str>, Option<Cow<'_, str>>)> = record
                    .iter()
                    .map(|val| (Cow::Borrowed(val.as_str()), None))
                    .collect();
                f(&cells)?;
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

            let row_height = 18.0; // Reduced row vertical spacing

            let builder = egui_extras::TableBuilder::new(ui)
                .id_salt("stable_explorer_table")
                .sense(egui::Sense::click())
                .striped(true)
                .resizable(true)
                .column(egui_extras::Column::initial(140.0))
                .column(egui_extras::Column::initial(110.0))
                .column(egui_extras::Column::remainder());

            if let Ok((responses, table)) = builder.archived_headers(
                &self.state,
                self.provider.headers(),
                18.0, // Reduced header vertical height
                &org_colors,
                &user_colors,
            ) {
                let _ = self.state.process_responses(&self.provider, responses);

                // Explicitly run column filters
                let filter_state = self.state.get_filter_state();
                let _ = self.state.apply_all_filters(&self.provider, &filter_state);

                // Explicitly run active sorting configuration
                if let Some((sort_col, sort_up)) = self.state.get_sort_state() {
                    let _ = self.provider.sort_active_rows(
                        &mut self.state.active_rows,
                        sort_col,
                        sort_up,
                    );
                }

                let active_rows = self.state.active_rows.clone();

                table.body(|mut body| {
                    for &row_idx in &active_rows {
                        let is_selected = self.state.selected_rows.contains(row_idx as u32);

                        let tag_color = if let Some(color_idx) =
                            self.state.highlights.get_usize(row_idx)
                            && let Ok(rgb) = select_color(color_idx, &org_colors, &user_colors)
                        {
                            Some(egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                        } else {
                            None
                        };

                        body.row(row_height, |mut row| {
                            for col_idx in 0..self.provider.column_count() {
                                row.col(|ui| {
                                    let item_spacing = ui.spacing().item_spacing;
                                    let gapless_rect = ui.max_rect().expand2(0.5 * item_spacing);

                                    let is_row_hovered =
                                        ui.ctx().pointer_latest_pos().is_some_and(|pos| {
                                            gapless_rect.y_range().contains(pos.y)
                                                && ui.clip_rect().x_range().contains(pos.x)
                                        });

                                    let response = ui.interact(
                                        gapless_rect,
                                        ui.id().with((row_idx, col_idx)),
                                        egui::Sense::click(),
                                    );

                                    if response.clicked() {
                                        self.state.handle_row_selection(
                                            ui.input(|i| i.modifiers),
                                            row_idx,
                                        );
                                    }

                                    let bg_color = if is_selected {
                                        Some(ui.visuals().selection.bg_fill)
                                    } else if is_row_hovered {
                                        Some(ui.visuals().widgets.hovered.weak_bg_fill)
                                    } else {
                                        None
                                    };

                                    if let Some(bg) = bg_color {
                                        ui.painter().rect_filled(
                                            gapless_rect,
                                            egui::CornerRadius::ZERO,
                                            bg,
                                        );
                                    }

                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 4.0;

                                        if col_idx == 0
                                            && let Some(color) = tag_color
                                        {
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::vec2(8.0, 8.0),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(rect, 1.5, color);
                                        }

                                        let cell_val = &self.provider.records[row_idx][col_idx];
                                        let text_color = if is_selected {
                                            ui.visuals().selection.stroke.color
                                        } else {
                                            ui.visuals().widgets.inactive.text_color()
                                        };
                                        ui.colored_label(text_color, cell_val);
                                    });
                                });
                            }
                        });
                    }
                });
            }
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
