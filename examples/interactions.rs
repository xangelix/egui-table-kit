//! Stable selection, keyboard focus, row events and drag/drop without application dependencies.

use std::borrow::Cow;

use eframe::egui;
use egui_table_kit::{
    error::TableError,
    interaction::{RowEventKind, SelectionMode},
    operations::{BorrowedRow, HeaderIter, RowCallback, TableCell, TableProvider},
    state::TableState,
    table::TableKit,
};

struct Names(Vec<(u64, String)>);

impl TableProvider for Names {
    fn row_key(&self, row: usize) -> egui::Id {
        egui::Id::new(self.0[row].0)
    }

    fn row_count(&self) -> usize {
        self.0.len()
    }

    fn column_count(&self) -> usize {
        1
    }

    fn header(&self, column: usize) -> Option<Cow<'_, str>> {
        (column == 0).then_some(Cow::Borrowed("Name"))
    }

    fn headers(&self) -> HeaderIter<'_> {
        HeaderIter::new(self)
    }

    fn cell_at(&self, row: usize, _: usize) -> Result<Option<TableCell<'_>>, TableError> {
        Ok(self
            .0
            .get(row)
            .map(|(_, name)| (Cow::Borrowed(name.as_str()), None)))
    }

    fn for_row_at(&self, row_index: usize, f: &mut RowCallback<'_>) -> Result<(), TableError> {
        if row_index < self.row_count() {
            f(&BorrowedRow {
                provider: self,
                row_index,
            })?;
        }
        Ok(())
    }

    fn for_all_rows(&self, f: &mut RowCallback<'_>) -> Result<(), TableError> {
        for row in 0..self.row_count() {
            self.for_row_at(row, f)?;
        }
        Ok(())
    }

    fn for_selected_rows(
        &self,
        state: &TableState,
        f: &mut RowCallback<'_>,
    ) -> Result<(), TableError> {
        for row in &state.selected_rows {
            self.for_row_at(row as usize, f)?;
        }
        Ok(())
    }
}

struct Demo {
    names: Names,
    state: TableState,
    message: String,
}

impl Default for Demo {
    fn default() -> Self {
        let names = Names(
            ["Ada", "Grace", "Margaret", "Barbara"]
                .into_iter()
                .enumerate()
                .map(|(id, name)| (id as u64, name.into()))
                .collect(),
        );

        let mut state = TableState::new("names", names.row_count());
        state.selection_mode = SelectionMode::Explorer;

        Self {
            names,
            state,
            message: String::new(),
        }
    }
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        ui.heading("Table interaction example");
        ui.label("Click, Ctrl/Cmd-toggle, Shift-range; Up/Down move focus. Drag the selection to the box.");

        if ui
            .button("Replace snapshot with reversed storage order")
            .clicked()
        {
            let identities = self.state.capture_identity(&self.names);
            self.names.0.reverse();
            if let Err(error) = self.state.restore_identity(&self.names, &identities) {
                self.message = error.to_string();
            }
        }

        let mut scroll = None;
        let focus_id = egui::Id::new("list_focus");

        if ui.memory(|m| m.focused().is_none_or(|id| id == focus_id)) {
            for (key, delta) in [(egui::Key::ArrowUp, -1), (egui::Key::ArrowDown, 1)] {
                if ui.input(|i| i.key_pressed(key)) {
                    scroll = self
                        .state
                        .move_focus(&self.names, delta, ui.input(|i| i.modifiers));
                }
            }
        }

        let mut table = TableKit::new("names", &self.names, &mut self.state)
            .with_drag_enabled(true)
            .with_row_height(28.0)
            .with_max_height(Some(240.0));

        if let Some(row) = scroll {
            table = table.with_scroll_to_row(row as u64, egui::Align::Center);
        }

        match table.show_with_output(ui, |_, _, _, _| None) {
            Ok(output) => {
                ui.interact(
                    output.response.rect,
                    focus_id,
                    egui::Sense::focusable_noninteractive(),
                );

                for event in output.events {
                    self.message = format!("{:?}: {:?}", event.kind, event.key);
                    ui.memory_mut(|m| m.request_focus(focus_id));

                    if event.kind == RowEventKind::DragStarted {
                        egui::DragAndDrop::set_payload(ui.ctx(), event.dragged_keys.clone());
                    }
                }
            }
            Err(error) => {
                self.message = error.to_string();
            }
        }

        let (_, dropped) =
            ui.dnd_drop_zone::<Vec<egui::Id>, _>(egui::Frame::group(ui.style()), |ui| {
                ui.set_min_size(egui::vec2(300.0, 80.0));
                if egui::DragAndDrop::has_payload_of_type::<Vec<egui::Id>>(ui.ctx()) {
                    ui.label("Release to drop selected rows here");
                } else {
                    ui.label("Drop selected rows here");
                }
            });

        if let Some(keys) = dropped {
            let names: Vec<&str> = keys
                .iter()
                .filter_map(|key| {
                    self.names
                        .0
                        .iter()
                        .find(|(id, _)| egui::Id::new(*id) == *key)
                        .map(|(_, name)| name.as_str())
                })
                .collect();
            self.message = format!("Dropped: {}", names.join(", "));
        }

        if let Some(keys) = egui::DragAndDrop::payload::<Vec<egui::Id>>(ui.ctx())
            && ui.input(|i| i.pointer.is_decidedly_dragging())
            && let Some(pointer_pos) = ui.ctx().pointer_interact_pos()
        {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("drag_preview"),
            ));

            let label = if keys.len() == 1 {
                self.names
                    .0
                    .iter()
                    .find(|(id, _)| egui::Id::new(*id) == keys[0])
                    .map_or_else(|| "1 item".to_string(), |(_, name)| format!("📄 {name}"))
            } else {
                format!("📄 {} items", keys.len())
            };

            let font_id = egui::FontId::proportional(14.0);
            let galley = painter.layout_no_wrap(label, font_id, ui.visuals().strong_text_color());
            let padding = egui::vec2(10.0, 6.0);
            let rect = egui::Rect::from_min_size(
                pointer_pos + egui::vec2(16.0, 16.0),
                galley.size() + padding * 2.0,
            );

            painter.rect_filled(
                rect,
                egui::CornerRadius::same(6),
                ui.visuals().window_fill.gamma_multiply(0.85),
            );
            painter.rect_stroke(
                rect,
                egui::CornerRadius::same(6),
                egui::Stroke::new(1.0, ui.visuals().selection.stroke.color.gamma_multiply(0.8)),
                egui::StrokeKind::Inside,
            );
            painter.galley(rect.min + padding, galley, ui.visuals().strong_text_color());
        }

        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                focus_id,
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });

        ui.label(&self.message);
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "Interactions",
        eframe::NativeOptions::default(),
        Box::new(|_| Ok(Box::<Demo>::default())),
    )
}
