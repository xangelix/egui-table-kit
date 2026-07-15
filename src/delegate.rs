use egui::{Color32, Margin, Response, Sense, Ui};

use super::{
    error::TableError,
    filter::highlight::select_color,
    header::{ColResponse, HeaderMenuAnchor, show_header_cell_contents},
    layout::{CellInfo, HeaderCellInfo, PrefetchInfo, TableDelegate},
    operations::{BorrowedRow, Row, TableProvider},
    state::TableState,
};

/// Shared callback definition used to render custom interactive cellular components.
pub type CustomCellCallback<'a> =
    dyn FnMut(&mut Ui, &CellInfo, &dyn Row, Color32) -> Option<Response> + 'a;

/// The central delegate translating kit properties to the underlying `egui_table` layout.
pub struct TableKitDelegate<'a> {
    pub provider: &'a dyn TableProvider,
    pub state: &'a mut TableState,
    pub org_colors: &'a [[u8; 3]],
    pub user_colors: &'a [[u8; 3]],
    pub collected_responses: &'a mut Vec<ColResponse>,
    pub halt_error: &'a mut Option<TableError>,
    pub custom_cell_ui: Option<Box<CustomCellCallback<'a>>>,
    pub item_clicked: &'a mut Option<usize>,
    pub secondary_clicked: &'a mut Option<usize>,
    pub is_new_pass: bool,

    // Configurations
    pub cell_padding: Margin,
    pub highlight_entire_row: bool,
    pub hovered_row: Option<u64>,
    pub header_menu_anchor: HeaderMenuAnchor,
    pub row_height: f32,
    pub header_bg_color: Option<Color32>,

    pub striped: bool,
    pub striping_color: Option<Color32>,
    pub hover_color: Option<Color32>,
}

impl<'a> TableKitDelegate<'a> {
    /// Creates a new delegate instance with robust visual defaults.
    pub fn new(
        provider: &'a dyn TableProvider,
        state: &'a mut TableState,
        org_colors: &'a [[u8; 3]],
        user_colors: &'a [[u8; 3]],
        collected_responses: &'a mut Vec<ColResponse>,
        halt_error: &'a mut Option<TableError>,
        custom_cell_ui: Option<Box<CustomCellCallback<'a>>>,
        item_clicked: &'a mut Option<usize>,
        secondary_clicked: &'a mut Option<usize>,
    ) -> Self {
        Self {
            provider,
            state,
            org_colors,
            user_colors,
            collected_responses,
            halt_error,
            custom_cell_ui,
            item_clicked,
            secondary_clicked,
            is_new_pass: true,
            cell_padding: Margin::symmetric(8, 0),
            highlight_entire_row: true,
            hovered_row: None,
            header_menu_anchor: HeaderMenuAnchor::Cursor,
            row_height: 16.0,
            header_bg_color: None,
            striped: false,
            striping_color: None,
            hover_color: None,
        }
    }
}

impl TableDelegate for TableKitDelegate<'_> {
    fn default_row_height(&self) -> f32 {
        self.row_height
    }

    fn prepare(&mut self, _info: &PrefetchInfo) {
        if self.is_new_pass {
            self.is_new_pass = false;
            self.collected_responses.clear();
            self.hovered_row = None;
        }
    }

    fn header_cell_ui(&mut self, ui: &mut Ui, cell: &HeaderCellInfo) {
        let col_idx = cell.col_range.start;
        let title = self.provider.header(col_idx).unwrap_or_default();

        let default_response = ColResponse::default();
        let (previous_response, sort_up) = self
            .state
            .columns
            .get(col_idx)
            .map_or((&default_response, None), |col| {
                (&col.response, col.sort_up)
            });

        // Apply custom header background color if configured
        let bg_color = self.header_bg_color.unwrap_or_else(|| {
            if ui.visuals().dark_mode {
                Color32::from_gray(38)
            } else {
                Color32::from_gray(230)
            }
        });
        ui.visuals_mut().widgets.noninteractive.weak_bg_fill = bg_color;

        // Isolate ID namespace per column to prevent sort/ellipsis widget ID collisions
        ui.push_id(col_idx, |ui| {
            match show_header_cell_contents(
                ui,
                title.as_ref(),
                &sort_up,
                previous_response,
                self.org_colors,
                self.user_colors,
                self.header_menu_anchor,
            ) {
                Ok(response) => {
                    if col_idx >= self.collected_responses.len() {
                        self.collected_responses
                            .resize_with(col_idx + 1, ColResponse::default);
                    }
                    self.collected_responses[col_idx] = response;
                }
                Err(e) => {
                    *self.halt_error = Some(e);
                }
            }
        });
    }

    fn cell_ui(&mut self, ui: &mut Ui, cell: &CellInfo) {
        let current_visible_idx = cell.row_nr as usize;
        let Some(&row_idx) = self.state.active_rows.get(current_visible_idx) else {
            return;
        };

        let is_selected = self.state.selected_rows.contains(row_idx as u32);
        let highlight = self.state.highlights.get_usize(row_idx);

        let item_spacing = ui.spacing().item_spacing;
        let cell_rect = ui.max_rect().expand2(0.5 * item_spacing);

        // Read the hover state computed by the layout pass
        let row_hovered = if self.highlight_entire_row {
            cell.row_hovered
        } else {
            ui.rect_contains_pointer(cell_rect)
        };

        // Resolve background highlight color
        let highlight_rgb = if let Some(color_idx) = highlight {
            select_color(color_idx, self.org_colors, self.user_colors).ok()
        } else {
            None
        };

        // Paint background layers
        if is_selected {
            ui.painter().rect_filled(
                cell_rect,
                egui::CornerRadius::ZERO,
                ui.visuals().selection.bg_fill,
            );
        } else {
            // 1. Base layer
            let base_color = if let Some(rgb) = highlight_rgb {
                Some(Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
            } else if self.striped && cell.row_nr % 2 == 1 {
                Some(
                    self.striping_color
                        .unwrap_or_else(|| ui.visuals().faint_bg_color),
                )
            } else {
                None
            };

            if let Some(color) = base_color {
                ui.painter()
                    .rect_filled(cell_rect, egui::CornerRadius::ZERO, color);
            }

            // 2. Hover layer
            if row_hovered {
                let hover_fill = self
                    .hover_color
                    .unwrap_or_else(|| ui.visuals().widgets.hovered.weak_bg_fill);

                ui.painter()
                    .rect_filled(cell_rect, egui::CornerRadius::ZERO, hover_fill);
            }
        }

        // Render visual guidelines inside the tree cells
        let is_tree_changed = if cell.col_nr == 0 && self.provider.is_tree() {
            if let Some(hierarchy) = self.provider.row_hierarchy(self.state, row_idx) {
                self.state.show_tree_cell(ui, row_idx, hierarchy)
            } else {
                false
            }
        } else {
            false
        };

        if is_tree_changed {
            ui.ctx().request_repaint();
        }

        // Set up cell text color matching selection state
        let text_color = if is_selected {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().widgets.inactive.text_color()
        };

        // Adjust the click-sensing area on Column 0 to protect expand/collapse button hits
        let mut interact_rect = cell_rect;
        if cell.col_nr == 0
            && self.provider.is_tree()
            && let Some(hierarchy) = self.provider.row_hierarchy(self.state, row_idx)
        {
            #[allow(clippy::cast_precision_loss)]
            let indent_width = (hierarchy.indent_level as f32).mul_add(22.0, 22.0);
            interact_rect.min.x = (interact_rect.min.x + indent_width).min(interact_rect.max.x);
        }

        // Set up cell interaction triggers using layout coordinates to prevent transition collisions
        let response = ui.interact(
            interact_rect,
            ui.id().with((cell.row_nr, cell.col_nr)),
            Sense::click(),
        );
        if response.clicked() {
            *self.item_clicked = Some(row_idx);
            self.state
                .handle_row_selection(ui.input(|i| i.modifiers), row_idx);
        }
        if response.secondary_clicked() {
            *self.secondary_clicked = Some(row_idx);
        }

        // Construct the zero-allocation borrowed row representation
        let borrowed_row = BorrowedRow {
            provider: self.provider,
            row_index: row_idx,
        };

        let mut rendered = false;

        // Custom padding configuration to pull Column 0 snug to the expand arrow ONLY when rendering a tree
        let mut padding = self.cell_padding;
        if cell.col_nr == 0 && self.provider.is_tree() {
            padding.left = 0;
            padding.right = 4;
        }

        // Wrap everything inside an inner-margin Frame to provide clean cell margins
        egui::Frame::NONE.inner_margin(padding).show(ui, |ui| {
            ui.push_id((cell.row_nr, cell.col_nr), |ui| {
                let custom_renderer = self.custom_cell_ui.as_mut();

                if let Some(renderer) = custom_renderer
                    && renderer(ui, cell, &borrowed_row as &dyn Row, text_color).is_some()
                {
                    rendered = true;
                }

                if !rendered && let Some((val, _)) = borrowed_row.cell(cell.col_nr) {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(val.as_ref()).color(text_color))
                                .selectable(false)
                                .wrap_mode(if ui.is_sizing_pass() {
                                    ui.wrap_mode()
                                } else {
                                    egui::TextWrapMode::Truncate
                                }),
                        );
                    });
                }
            });
        });
    }
}
