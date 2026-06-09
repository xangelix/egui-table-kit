use egui_extras::{Table, TableBuilder, TableRow};
use fluent_zero::t;

use super::{
    error::TableError,
    filter::{Filter, highlight::HighlightFilter, search::SearchBar},
    state::TableState,
};

#[derive(Clone, Debug, Default)]
pub struct ColumnState {
    pub response: ColResponse,
    pub sort_up: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct ColResponse {
    pub to_sort: bool,
    pub hovered: bool,

    pub filtering: Filter,
    pub secondary_clicked: bool,
}

pub trait TableHeaderRowExt {
    fn header_cell(
        &mut self,
        text: &str,
        sort_up: &Option<bool>,
        previous_response: &ColResponse,
        org_colors: &[[u8; 3]],
        user_colors: &[[u8; 3]],
    ) -> Result<ColResponse, TableError>;
}

impl TableHeaderRowExt for TableRow<'_, '_> {
    #[allow(clippy::too_many_lines)]
    fn header_cell(
        &mut self,
        text: &str,
        sort_up: &Option<bool>,
        previous_response: &ColResponse,
        org_colors: &[[u8; 3]],
        user_colors: &[[u8; 3]],
    ) -> Result<ColResponse, TableError> {
        let mut response = ColResponse {
            filtering: previous_response.filtering.clone(),
            ..Default::default()
        };
        let mut halt_error: Option<TableError> = None;
        let mut is_hovered = false;

        let col_response = self
            .col(|ui| {
                is_hovered = ui.rect_contains_pointer(ui.max_rect());

                let item_spacing = ui.spacing().item_spacing;
                let gapless_rect = ui.max_rect().expand2(0.5 * item_spacing);
                ui.painter().rect_filled(
                    gapless_rect,
                    egui::CornerRadius::ZERO,
                    ui.visuals().widgets.noninteractive.bg_stroke.color,
                );

                ui.horizontal(|ui| {
                    ui.strong(text);

                    if let Some(sort_up) = sort_up {
                        if *sort_up {
                            ui.strong("🔻");
                        } else {
                            ui.strong("🔺");
                        }
                    }

                    if previous_response.hovered && sort_up.is_none() {
                        ui.label("🔻");
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(5.0);

                        let popup_id = ui.make_persistent_id(ui.id().with("filter"));
                        let popup_open = if previous_response.secondary_clicked {
                            egui::Popup::toggle_id(ui.ctx(), popup_id);
                            true
                        } else {
                            egui::Popup::is_id_open(ui.ctx(), popup_id)
                        };

                        // Cleanup empty filters when the popup closes
                        if !popup_open {
                            if response.filtering.is_empty() {
                                // Reset to a clean default if effectively empty
                                response.filtering = Filter::default();
                            } else if response.filtering.search.text().is_empty()
                                && response.filtering.search.is_active()
                            {
                                // If search text is empty but active, deactivate it to be clean
                                response.filtering.search.clear();
                            }
                        }

                        // Show ellipsis if: filtering is active, hovering, or popup is open
                        if !response.filtering.is_empty() || previous_response.hovered || popup_open
                        {
                            let ellipsis_response = draw_vertical_ellipsis(ui);

                            let width = 150.0;
                            egui::Popup::menu(&ellipsis_response)
                                .id(popup_id)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .show(|ui| {
                                    ui.set_width(width);
                                    set_menu_style(ui.style_mut());

                                    ui.strong(t!("column-options"));
                                    ui.separator();

                                    // 1. Text Search
                                    SearchBar::new(&t!("filter-text"))
                                        .ui(ui, &mut response.filtering.search);

                                    // 2. Highlight Filter (Render ONLY if color palettes are provided)
                                    if (!org_colors.is_empty() || !user_colors.is_empty())
                                        && HighlightFilter::new(&t!("new-highlight-filter"))
                                            .ui(
                                                ui,
                                                &mut response.filtering.highlight,
                                                org_colors,
                                                user_colors,
                                            )
                                            .is_err()
                                    {
                                        halt_error = Some(TableError::CorruptedState);
                                    }

                                    // Auto-cleanup while editing:
                                    // If both are cleared by the user in the UI, we might want to know,
                                    // but checking is_empty() at the start of next frame/close is safer.

                                    ui.separator();

                                    // 3. Sorting Options
                                    let sort_desc = match sort_up {
                                        Some(true) => format!(" {}", t!("current-ascending")),
                                        Some(false) => format!(" {}", t!("current-descending")),
                                        None => String::new(),
                                    };
                                    if ui
                                        .button(format!("{} {sort_desc}", t!("toggle-sort")))
                                        .clicked()
                                    {
                                        response.to_sort = true;
                                    }
                                });
                        }
                    });
                });
            })
            .1;

        if let Some(err) = halt_error {
            return Err(err);
        }

        if is_hovered {
            response.hovered = true;
        }
        if col_response.clicked() {
            response.to_sort = true;
        }
        if col_response.secondary_clicked() {
            response.secondary_clicked = true;
        }

        Ok(response)
    }
}

fn draw_vertical_ellipsis(ui: &mut egui::Ui) -> egui::Response {
    // Define size parameters
    let text_height = ui.text_style_height(&egui::TextStyle::Body) - 3.0;
    let dot_radius = text_height * 0.13; // Adjust based on visual preference
    let spacing = dot_radius * 2.0;
    let ellipsis_height = (3.0 * dot_radius).mul_add(2.0, 2.0 * spacing); // Total height of ellipsis

    // Reserve space for the ellipsis
    let (rect, response) = ui.allocate_exact_size(
        egui::Vec2::new(dot_radius * 2.0, ellipsis_height),
        egui::Sense::click(),
    );

    // Draw the ellipsis
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let color = if response.hovered() {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            ui.visuals().widgets.inactive.bg_fill
        };

        for i in 0..3 {
            #[allow(clippy::cast_precision_loss)]
            let center = rect.center()
                + egui::Vec2::new(0.0, (i as f32 - 1.0) * dot_radius.mul_add(2.0, spacing));
            painter.circle_filled(center, dot_radius, color);
        }
    }

    response
}

pub trait HeaderTrait<'a> {
    fn archived_headers(
        self,
        data: &TableState,
        headers: impl IntoIterator<Item = &'a str>,
        height: f32,
        org_colors: &[[u8; 3]],
        user_colors: &[[u8; 3]],
    ) -> Result<(Vec<ColResponse>, Table<'a>), TableError>;
}

impl<'a> HeaderTrait<'a> for TableBuilder<'a> {
    fn archived_headers(
        self,
        data: &TableState,
        headers: impl IntoIterator<Item = &'a str>,
        height: f32,
        org_colors: &[[u8; 3]],
        user_colors: &[[u8; 3]],
    ) -> Result<(Vec<ColResponse>, Table<'a>), TableError> {
        let headers = headers.into_iter();
        let headers_count = headers.size_hint().0;
        let mut messages = Vec::with_capacity(headers_count);

        let default_response = ColResponse::default();
        let mut halt_error: Option<TableError> = None;
        let table = self.header(height, |mut header| {
            for (i, title) in headers.enumerate() {
                // Direct reference extraction; completely allocation-free
                let (previous_response, sort_up) = data
                    .columns
                    .get(i)
                    .map_or((&default_response, None), |col| {
                        (&col.response, col.sort_up)
                    });

                if let Ok(message) =
                    header.header_cell(title, &sort_up, previous_response, org_colors, user_colors)
                {
                    messages.push(message);
                } else {
                    halt_error = Some(TableError::CorruptedState);
                    return;
                }
            }
        });

        halt_error.map_or(Ok((messages, table)), Err)
    }
}

pub const fn set_menu_style(style: &mut egui::Style) {
    style.wrap_mode = Some(egui::TextWrapMode::Extend);

    style.spacing.button_padding = egui::vec2(2.0, 0.0);
    style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;

    style.visuals.selection.bg_fill = egui::Color32::from_gray(50);
    style.visuals.selection.stroke.color = egui::Color32::from_rgb(86, 92, 128);
}
