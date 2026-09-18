use egui::{Color32, Margin, Response, Sense, Ui};

use super::{
    error::TableError,
    filter::highlight::select_color,
    header::{ColResponse, HeaderMenuAnchor, show_header_cell_contents},
    layout::{CellInfo, HeaderCellInfo, PrefetchInfo, TableDelegate},
    operations::{BorrowedRow, Row, TableProvider},
    state::TableState,
};
use crate::interaction::{RowEvent, RowEventKind, RowGeometry, SelectionInput};

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

    /// Opt-in drag sensing; existing consumers keep click-only interaction.
    pub drag_enabled: bool,
    /// Whether to collect `RowEvent`s during this pass.
    pub track_events: bool,
    /// Whether to calculate and collect `RowGeometry`s during this pass.
    pub track_geometry: bool,
    pub events: Vec<RowEvent>,
    pub rows: Vec<RowGeometry>,

    /// Layout-column to provider-column mapping. Keep a tree's column 0 first.
    pub column_map: Option<Vec<usize>>,
}

impl<'a> TableKitDelegate<'a> {
    /// Creates a new delegate instance with robust visual defaults.
    #[expect(
        clippy::too_many_arguments,
        reason = "Preserve the public delegate constructor; TableKit provides the builder API"
    )]
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
            drag_enabled: false,
            track_events: false,
            track_geometry: false,
            events: Vec::new(),
            rows: Vec::new(),
            column_map: None,
        }
    }

    #[must_use]
    pub const fn with_track_events(mut self, track: bool) -> Self {
        self.track_events = track;
        self
    }

    #[must_use]
    pub const fn with_track_geometry(mut self, track: bool) -> Self {
        self.track_geometry = track;
        self
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
            self.events.clear();
            self.rows.clear();
        }
    }

    fn header_cell_ui(&mut self, ui: &mut Ui, cell: &HeaderCellInfo) {
        let col_idx = match &self.column_map {
            Some(map) => map.get(cell.col_range.start).copied(),
            None => Some(cell.col_range.start),
        };
        let Some(col_idx) = col_idx.filter(|&c| c < self.provider.column_count()) else {
            *self.halt_error = Some(TableError::CorruptedState);
            return;
        };
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
        let layout_col_nr = cell.col_nr;
        let mapped;
        let cell = match &self.column_map {
            Some(map) => {
                let Some(&column) = map
                    .get(layout_col_nr)
                    .filter(|&&c| c < self.provider.column_count())
                else {
                    *self.halt_error = Some(TableError::CorruptedState);
                    return;
                };
                mapped = CellInfo {
                    col_nr: column,
                    ..*cell
                };
                &mapped
            }
            None => cell,
        };
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
            if self.track_events {
                let key = self.provider.row_key(row_idx);
                let kind = RowEventKind::ExpansionChanged(
                    self.state.expanded_rows.contains(row_idx as u32),
                );
                if self
                    .state
                    .accept_event(ui.ctx().cumulative_frame_nr(), key, kind)
                {
                    self.events.push(RowEvent {
                        key,
                        row_index: row_idx,
                        kind,
                        modifiers: ui.input(|i| i.modifiers),
                        dragged_keys: vec![],
                    });
                }
            }
        }

        // Set up cell text color matching selection state
        let text_color = if is_selected {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().widgets.inactive.text_color()
        };

        // Adjust the click-sensing area on Column 0 to protect expand/collapse button hits.
        // Only the arrow itself (14px) plus its surrounding spacing is carved out; the
        // indent strip left of it keeps its own interact zone so clicks there still select
        // the row. Rows without children have a disabled placeholder arrow, so no
        // carve-out is needed.
        let mut interact_rect = cell_rect;
        let mut indent_strip_rect = None;
        if cell.col_nr == 0
            && self.provider.is_tree()
            && let Some(hierarchy) = self.provider.row_hierarchy(self.state, row_idx)
            && hierarchy.has_children
        {
            #[allow(clippy::cast_precision_loss)]
            let indent_width = hierarchy.indent_level as f32 * 22.0;
            let strip_end = (interact_rect.min.x + indent_width).min(interact_rect.max.x);
            if strip_end > interact_rect.min.x {
                indent_strip_rect = Some(egui::Rect::from_min_max(
                    interact_rect.min,
                    egui::pos2(strip_end, interact_rect.max.y),
                ));
            }
            // Item spacing (8px) + arrow (14px) + trailing gap (4px)
            interact_rect.min.x = (strip_end + 26.0).min(interact_rect.max.x);
        }

        // Column separators own their resize hit regions, including body handles.
        let resize_margin = ui.style().interaction.resize_grab_radius_side;
        interact_rect.max.x = (interact_rect.max.x - resize_margin).max(interact_rect.min.x);
        if layout_col_nr > 0 {
            interact_rect.min.x = (interact_rect.min.x + resize_margin).min(interact_rect.max.x);
        }

        let key = self.provider.row_key(row_idx);
        if self.track_geometry && !ui.is_sizing_pass() {
            let clipped = cell_rect.intersect(ui.clip_rect());
            if clipped.is_positive() {
                let interact_clipped = interact_rect.intersect(ui.clip_rect());
                let strip_clipped = indent_strip_rect.map(|strip| strip.intersect(ui.clip_rect()));

                let existing_row = if let Some(last) = self.rows.last_mut()
                    && last.key == key
                {
                    Some(last)
                } else {
                    self.rows.iter_mut().find(|row| row.key == key)
                };

                if let Some(row) = existing_row {
                    row.rect = row.rect.union(clipped);
                    if interact_clipped.is_positive() {
                        row.hit_regions.push(interact_clipped);
                    }
                    if let Some(strip) = strip_clipped
                        && strip.is_positive()
                    {
                        row.hit_regions.push(strip);
                    }
                } else {
                    let mut hit_regions = Vec::with_capacity(self.provider.column_count() + 1);
                    if interact_clipped.is_positive() {
                        hit_regions.push(interact_clipped);
                    }
                    if let Some(strip) = strip_clipped
                        && strip.is_positive()
                    {
                        hit_regions.push(strip);
                    }
                    self.rows.push(RowGeometry {
                        key,
                        row_index: row_idx,
                        visible_index: current_visible_idx,
                        rect: clipped,
                        hit_regions,
                    });
                }
            }
        }

        let sensing = if self.drag_enabled && self.provider.row_selectable(row_idx) {
            Sense::click_and_drag()
        } else {
            Sense::click()
        };

        let mut handle_row_click = |response: &egui::Response| {
            if ui.is_sizing_pass() {
                return;
            }

            let is_interacted = response.clicked()
                || response.double_clicked()
                || response.secondary_clicked()
                || (self.drag_enabled
                    && (response.drag_started_by(egui::PointerButton::Primary)
                        || response.drag_stopped_by(egui::PointerButton::Primary)));
            if !is_interacted {
                return;
            }

            let modifiers = ui.input(|i| i.modifiers);
            let frame = ui.ctx().cumulative_frame_nr();
            if response.clicked() && self.state.accept_event(frame, key, RowEventKind::Clicked) {
                *self.item_clicked = Some(row_idx);
                self.state.select_row_at_visible(
                    self.provider,
                    row_idx,
                    Some(current_visible_idx),
                    modifiers,
                    SelectionInput::Click,
                );
                if self.track_events {
                    self.events.push(RowEvent {
                        key,
                        row_index: row_idx,
                        kind: RowEventKind::Clicked,
                        modifiers,
                        dragged_keys: vec![],
                    });
                }
            }
            if response.double_clicked()
                && self.state.accept_event(frame, key, RowEventKind::Activated)
                && self.track_events
            {
                self.events.push(RowEvent {
                    key,
                    row_index: row_idx,
                    kind: RowEventKind::Activated,
                    modifiers,
                    dragged_keys: vec![],
                });
            }
            if response.secondary_clicked()
                && self
                    .state
                    .accept_event(frame, key, RowEventKind::SecondaryClicked)
            {
                *self.secondary_clicked = Some(row_idx);
                self.state.select_row_at_visible(
                    self.provider,
                    row_idx,
                    Some(current_visible_idx),
                    modifiers,
                    SelectionInput::Context,
                );
                if self.track_events {
                    self.events.push(RowEvent {
                        key,
                        row_index: row_idx,
                        kind: RowEventKind::SecondaryClicked,
                        modifiers,
                        dragged_keys: vec![],
                    });
                }
            }
            if response.drag_started_by(egui::PointerButton::Primary)
                && self
                    .state
                    .accept_event(frame, key, RowEventKind::DragStarted)
            {
                self.state.select_row_at_visible(
                    self.provider,
                    row_idx,
                    Some(current_visible_idx),
                    modifiers,
                    SelectionInput::Drag,
                );
                if self.track_events {
                    let mut dragged_keys =
                        Vec::with_capacity(self.state.selected_rows.len() as usize);
                    dragged_keys.extend(
                        self.state
                            .selected_rows
                            .iter()
                            .map(|row| row as usize)
                            .filter(|&row| {
                                row < self.provider.row_count() && self.provider.row_selectable(row)
                            })
                            .map(|row| self.provider.row_key(row)),
                    );
                    self.events.push(RowEvent {
                        key,
                        row_index: row_idx,
                        kind: RowEventKind::DragStarted,
                        modifiers,
                        dragged_keys,
                    });
                }
            }
            if response.drag_stopped_by(egui::PointerButton::Primary)
                && self
                    .state
                    .accept_event(frame, key, RowEventKind::DragStopped)
                && self.track_events
            {
                self.events.push(RowEvent {
                    key,
                    row_index: row_idx,
                    kind: RowEventKind::DragStopped,
                    modifiers,
                    dragged_keys: vec![],
                });
            }
        };

        let response = ui.interact(
            interact_rect,
            cell.table_id.with((key, cell.col_nr)),
            sensing,
        );
        handle_row_click(&response);

        if let Some(strip_rect) = indent_strip_rect {
            let strip_response = ui.interact(
                strip_rect,
                cell.table_id.with((key, cell.col_nr, "indent_strip")),
                sensing,
            );
            handle_row_click(&strip_response);
        }

        // Construct the zero-allocation borrowed row representation
        let borrowed_row = BorrowedRow {
            provider: self.provider,
            row_index: row_idx,
        };
        let value = match borrowed_row.try_cell(cell.col_nr) {
            Ok(value) => value,
            Err(error) => {
                *self.halt_error = Some(error);
                return;
            }
        };
        if let Some((label, _)) = &value {
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    ui.is_enabled(),
                    is_selected,
                    label.as_ref(),
                )
            });
        }
        if self.state.focused_key == Some(key) {
            let clipped = cell_rect.intersect(ui.clip_rect());
            if clipped.is_positive() {
                ui.painter().rect_stroke(
                    clipped.shrink(1.0),
                    0.0,
                    ui.visuals().selection.stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }

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

                if !rendered && let Some((val, _)) = &value {
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
