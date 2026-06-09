use std::sync::{Arc, LazyLock};

use parking_lot::RwLock;

use crate::error::TableError;

pub static SELECT_COLOR: LazyLock<Arc<RwLock<u8>>> =
    LazyLock::new(|| Arc::new(RwLock::new(Default::default())));

pub fn select_color_meta(
    index: u8,
    org: &[[u8; 3]],
    user: &[[u8; 3]],
) -> Result<(usize, usize, [u8; 3]), TableError> {
    let index = index as usize;
    let org_color_len = org.len();

    let color = if index < org_color_len {
        Some(*org.get(index).ok_or(TableError::CorruptedState)?)
    } else {
        user.get(index - org_color_len).copied()
    };
    Ok((index, org_color_len, color.unwrap_or_else(Default::default)))
}

pub fn select_color(index: u8, org: &[[u8; 3]], user: &[[u8; 3]]) -> Result<[u8; 3], TableError> {
    select_color_meta(index, org, user).map(|(_, _, color)| color)
}

pub fn color_select_button(
    ui: &mut egui::Ui,
    color: egui::Color32,
    selected: bool,
    skinny: bool,
) -> egui::Response {
    let size = ui.spacing().interact_size;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(if skinny { size.x / 2.0 } else { size.x }, size.y),
        egui::Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::new(egui::WidgetType::ColorButton));

    if ui.is_rect_visible(rect) {
        let visuals = if selected {
            let mut selected_visuals = ui.visuals().widgets.open;
            selected_visuals.bg_stroke.color = egui::Color32::DARK_BLUE;
            selected_visuals.bg_stroke.width = 2.0;
            selected_visuals
        } else {
            *ui.style().interact(&response)
        };

        let rect = rect.expand(visuals.expansion);
        egui::color_picker::show_color_at(ui.painter(), color, rect);

        let rounding = visuals.corner_radius.at_most(2);
        ui.painter()
            .rect_stroke(rect, rounding, visuals.bg_stroke, egui::StrokeKind::Middle);
    }

    response
}

pub fn opt_color_select_button(
    ui: &mut egui::Ui,
    label: &str,
    color: &mut Option<u8>,
    selected: bool,
    skinny: bool,
    highlight_options_r1: &[[u8; 3]],
    highlight_options_r2: &[[u8; 3]],
) -> Result<(), TableError> {
    ui.label(label);

    match color {
        Some(inner) => {
            let [r, g, b] = select_color(*inner, highlight_options_r1, highlight_options_r2)?;
            if color_select_button(ui, egui::Color32::from_rgb(r, g, b), selected, skinny).clicked()
            {
                *color = None;
            }
        }
        None => {
            if ui.button("➕").clicked() {
                *color = Some(*SELECT_COLOR.read());
            }
        }
    }

    Ok(())
}

pub struct HighlightFilter<'a> {
    label: &'a str,
}

impl<'a> HighlightFilter<'a> {
    #[must_use]
    pub const fn new(label: &'a str) -> Self {
        Self { label }
    }

    pub fn ui(
        self,
        ui: &mut egui::Ui,
        highlight_select: &mut Option<Option<u8>>,
        highlight_options_r1: &[[u8; 3]],
        highlight_options_r2: &[[u8; 3]],
    ) -> Result<(), TableError> {
        ui.horizontal(|ui| {
            if let Some(highlight_filter) = highlight_select {
                opt_color_select_button(
                    ui,
                    self.label,
                    highlight_filter,
                    false,
                    false,
                    highlight_options_r1,
                    highlight_options_r2,
                )?;

                if ui.button("❌").on_hover_text("Remove Filter").clicked() {
                    *highlight_select = None;
                }
            } else if ui.button(self.label).clicked() {
                *highlight_select = Some(None);
            }

            Ok(())
        })
        .inner
    }
}
