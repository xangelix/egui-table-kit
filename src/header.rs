//! Layout and options structures for column header interaction overlays.

use fluent_zero::t;

use super::{
    error::TableError,
    filter::{Filter, highlight::HighlightFilter, search::SearchBar},
};

/// Configurable anchor positions for the column options context menus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HeaderMenuAnchor {
    /// Anchors the popup menu cleanly below the header cell boundaries.
    Cell,
    /// Anchors the popup menu directly at the cursor's exact click position.
    #[default]
    Cursor,
}

/// Tracks structural and interactive view states inside individual columns.
#[derive(Clone, Debug, Default)]
pub struct ColumnState {
    pub response: ColResponse,
    pub sort_up: Option<bool>,
}

/// Dynamic event triggers collected during header cell render steps.
#[derive(Clone, Debug, Default)]
pub struct ColResponse {
    pub to_sort: bool,
    pub hovered: bool,
    pub filtering: Filter,
    pub secondary_clicked: bool,
}

/// Applies styling adjustments to the column options popups.
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

/// Interactive header cell renderer.
pub fn show_header_cell_contents(
    ui: &mut egui::Ui,
    text: &str,
    sort_up: &Option<bool>,
    previous_response: &ColResponse,
    org_colors: &[[u8; 3]],
    user_colors: &[[u8; 3]],
    anchor_mode: HeaderMenuAnchor,
) -> Result<ColResponse, TableError> {
    let mut response = ColResponse {
        filtering: previous_response.filtering.clone(),
        ..Default::default()
    };
    let mut halt_error = None;
    let is_hovered = ui.rect_contains_pointer(ui.max_rect());

    let item_spacing = ui.spacing().item_spacing;
    let gapless_rect = ui.max_rect().expand2(0.5 * item_spacing);

    // Apply background fill for the header cell
    let bg_color = ui.visuals().widgets.noninteractive.weak_bg_fill;
    ui.painter()
        .rect_filled(gapless_rect, egui::CornerRadius::ZERO, bg_color);

    // Use the hierarchical ID instead of a thread-local counter.
    // This is stable and collision-free across sizing/painting passes.
    let unique_id = ui.id();

    // Passive hover sensor to anchor the popup correctly
    let cell_interact = ui.interact(
        gapless_rect,
        unique_id.with("header_interact"),
        egui::Sense::hover(),
    );

    let popup_id = ui.make_persistent_id(unique_id.with("filter"));
    let anchor_pos_id = unique_id.with("popup_anchor_pos");

    // Retrieve or create a stable click coordinate anchor
    let mut click_anchor = ui.data_mut(|d| d.get_temp::<egui::Pos2>(anchor_pos_id));

    // Direct pointer tracking for right-clicks to guarantee context menu activation
    let is_cell_hovered = ui.rect_contains_pointer(gapless_rect);
    let right_clicked = is_cell_hovered && ui.input(|i| i.pointer.secondary_clicked());
    if right_clicked {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }

    let popup_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let mut ellipsis_clicked = false;

    // Determine Overlay Dimensions & Active States
    let has_filter = !response.filtering.is_empty();
    let is_sorted = sort_up.is_some();
    let show_ellipsis = has_filter || is_hovered || popup_open;
    let show_sort = is_sorted || is_hovered;

    let right_padding = 8.0f32;
    let mut solid_width = 0.0f32;
    let mut ellipsis_center_x = None;

    let cell_width = gapless_rect.width();
    let right_x = gapless_rect.max.x;

    // Only reserve solid width for the ellipsis on the far right
    if !ui.is_sizing_pass() && cell_width > 20.0 && show_ellipsis {
        let current_offset = right_padding;
        ellipsis_center_x = Some(right_x - current_offset - 6.0); // 12px width
        solid_width = current_offset + 12.0;
    }

    ui.horizontal(|ui| {
        ui.add_space(8.0);

        if ui.is_sizing_pass() {
            ui.add(
                egui::Label::new(egui::RichText::new(text).strong())
                    .selectable(false)
                    .wrap_mode(ui.wrap_mode()),
            );
            if show_sort {
                let sort_text = match sort_up {
                    Some(false) => "🔺",
                    Some(true) | None => "🔻",
                };
                let font_id = egui::FontId::proportional(11.0);
                ui.add(
                    egui::Label::new(egui::RichText::new(sort_text).font(font_id))
                        .selectable(false),
                );
            }
            ui.add_space(8.0);
        } else {
            // Allocate the text container with a restricted layout width.
            // Left padding is 8.0, safety gap is 4.0. Total non-text offset from left is 12.0.
            let max_container_width = (cell_width - solid_width - 12.0).max(0.0);
            ui.allocate_ui(
                egui::vec2(max_container_width, ui.available_height()),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;

                        let sort_width = if show_sort { 14.0 } else { 0.0 };
                        let max_text_width = (max_container_width - sort_width).max(0.0);

                        ui.allocate_ui(egui::vec2(max_text_width, ui.available_height()), |ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(text).strong())
                                    .selectable(false)
                                    .wrap_mode(egui::TextWrapMode::Truncate),
                            );
                        });

                        if show_sort {
                            let sort_text = match sort_up {
                                Some(false) => "🔺",
                                Some(true) | None => "🔻",
                            };
                            let sort_color = if sort_up.is_some() {
                                ui.visuals().widgets.active.text_color()
                            } else {
                                ui.visuals()
                                    .widgets
                                    .inactive
                                    .text_color()
                                    .linear_multiply(0.4)
                            };

                            let font_id = egui::FontId::proportional(11.0);
                            let sort_response = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(sort_text)
                                        .font(font_id)
                                        .color(sort_color),
                                )
                                .selectable(false),
                            );

                            // Keep the sorting click interaction alive for the inline indicator icon
                            let sort_rect = sort_response.rect;
                            let sort_interact_response = ui.interact(
                                sort_rect,
                                unique_id.with("sort_interact"),
                                egui::Sense::click(),
                            );
                            if sort_interact_response.clicked() {
                                response.to_sort = true;
                            }
                        }
                    });
                },
            );
        }
    });

    if !ui.is_sizing_pass() && show_ellipsis && cell_width > 20.0 {
        let center_y = gapless_rect.center().y;

        // Render & Interact with Vertical Ellipsis Button
        if let Some(cx) = ellipsis_center_x {
            let ellipsis_rect =
                egui::Rect::from_center_size(egui::pos2(cx, center_y), egui::vec2(12.0, 18.0));

            let ellipsis_response = ui.interact(
                ellipsis_rect,
                unique_id.with("ellipsis_interact"),
                egui::Sense::click(),
            );

            if ellipsis_response.clicked() {
                ellipsis_clicked = true;
            }

            // Paint vertical ellipsis dots
            let center = ellipsis_rect.center();
            let dot_radius = 1.3;
            let spacing = 3.5;
            let dot_color = if ellipsis_response.hovered() {
                ui.visuals().widgets.hovered.text_color()
            } else {
                ui.visuals()
                    .widgets
                    .inactive
                    .text_color()
                    .linear_multiply(0.7)
            };
            for i in -1..=1 {
                #[allow(clippy::cast_precision_loss)]
                let dot_center = center + egui::vec2(0.0, (i as f32) * spacing);
                ui.painter()
                    .circle_filled(dot_center, dot_radius, dot_color);
            }
        }
    }

    // Toggle popup if ellipsis was left-clicked
    if ellipsis_clicked {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }

    // Capture precise pointer location upon activation
    if (right_clicked || ellipsis_clicked)
        && click_anchor.is_none()
        && let Some(pointer_pos) = ui.ctx().pointer_latest_pos()
    {
        ui.data_mut(|d| d.insert_temp(anchor_pos_id, pointer_pos));
        click_anchor = Some(pointer_pos);
    }

    // Direct, collision-free left-click sorting
    if is_cell_hovered && ui.input(|i| i.pointer.primary_clicked()) && !ellipsis_clicked {
        response.to_sort = true;
    }

    // Resolve target response layout reference to attach the menu
    let menu_anchor_response = match anchor_mode {
        HeaderMenuAnchor::Cursor if let Some(pos) = click_anchor => {
            let anchor_rect = egui::Rect::from_center_size(pos, egui::Vec2::splat(1.0));
            ui.interact(
                anchor_rect,
                unique_id.with("dummy_popup_anchor"),
                egui::Sense::hover(),
            )
        }
        _ => cell_interact,
    };

    // Render the options popup below the resolved anchor
    egui::Popup::menu(&menu_anchor_response)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(150.0);
            set_menu_style(ui.style_mut());

            ui.strong(t!("column-options"));
            ui.separator();

            SearchBar::new(&t!("filter-text")).ui(ui, &mut response.filtering.search);

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

            ui.separator();

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
                ui.close_kind(egui::UiKind::Menu);
            }
        });

    // Cleanup coordinates once the popup is closed
    if !popup_open {
        ui.data_mut(|d| {
            d.remove::<egui::Pos2>(anchor_pos_id);
        });
    }

    if let Some(err) = halt_error {
        return Err(err);
    }

    if is_hovered {
        response.hovered = true;
    }
    if ellipsis_clicked {
        response.secondary_clicked = true;
    }

    Ok(response)
}
