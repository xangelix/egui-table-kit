use egui::{Rect, Ui, UiBuilder, Vec2, Vec2b, pos2, vec2};
/// A scroll area with some portion of its left and/or top side "stuck".
///
/// This produces four quadrants:
///
/// ```text
///               <-------LEFT-------> <---------RIGHT---------->
///
///              ------------------------------------------------
///          ^   |                    |   <----------------->   |
///    TOP   |   |       Fixed        |      Horizontally       |
///          V   |    fixed_size      |       scrollable        |
///              |--------------------|-------------------------|.................
///          ^   | ^                  |           ^             |                .
///  BOTTOM  |   | |   Vertically     | <-  Fully scrollable -> |                .
///          |   | |   scrollable     |    scroll_outer_size    |                .
///          V   | v                  |           v             |                .
///              |____________________|_________________________|                .
///                                   .                                          .
///                                   .                   scroll_content_size    .
///                                   .                                          .
///                                   ............................................
/// ```
///
/// The above shows the initial layout when the scroll offset is zero (no scrolling has occurred yet).
#[derive(Clone, Copy, Debug)]
pub struct SplitScroll {
    pub scroll_enabled: Vec2b,

    /// Width of the fixed left side, and height of the fixed top.
    pub fixed_size: Vec2,

    /// Size of the small container of the right bottom scrollable region.
    pub scroll_outer_size: Vec2,

    /// Size of the large contents of the right bottom region, ignoring the left/top fixed regions.
    pub scroll_content_size: Vec2,

    /// If true, the vertical scrollbar will stick to the bottom as the content grows.
    pub stick_to_bottom: bool,
}

/// The contents of a [`SplitScroll`].
/// The contents of a [`SplitScroll`].
pub trait SplitScrollDelegate {
    fn left_top_ui(&mut self, ui: &mut Ui);
    fn right_top_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2);
    fn left_bottom_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2);
    fn right_bottom_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2);

    /// Paints overlays (like separator lines).
    /// Called *inside* the `ScrollArea` viewport closure (clipped to bottom-right)
    /// so that scrollbars paint over it.
    /// Also called *outside* (clipped to left) to paint over sticky areas.
    fn paint_overlays(&mut self, _ui: &mut Ui) {}

    /// Updates state like auto-sized column widths and drag interaction.
    /// Called *after* all UI quadrants and `paint_overlays` have been rendered,
    /// ensuring `max_column_widths` is fully populated and visual lines match the current frame's layout.
    fn update_col_widths(&mut self, _ui: &mut Ui) {}
}

impl SplitScroll {
    pub fn show(self, ui: &mut Ui, delegate: &mut dyn SplitScrollDelegate) {
        let Self {
            scroll_enabled,
            fixed_size,
            scroll_outer_size,
            scroll_content_size,
            stick_to_bottom,
        } = self;

        ui.scope(|ui| {
            let mut rect = ui.cursor();
            rect.max = rect.min + fixed_size + scroll_outer_size;

            let total_columns_width = fixed_size.x + scroll_content_size.x;
            let columns_right_edge = rect.min.x + total_columns_width;
            let right_boundary = columns_right_edge.min(rect.max.x);
            rect.max.x = right_boundary;

            ui.shrink_clip_rect(rect);
            let rect = rect;

            let bottom_right_rect = Rect::from_min_max(rect.min + fixed_size, rect.max);

            let scroll_offset = {
                let mut scroll_ui = ui.new_child(UiBuilder::new().max_rect(rect));

                egui::ScrollArea::new(scroll_enabled)
                    .auto_shrink(false)
                    .scroll_bar_rect(bottom_right_rect)
                    .stick_to_bottom(stick_to_bottom)
                    .show_viewport(&mut scroll_ui, |ui, scroll_offset_rect| {
                        ui.set_min_size(fixed_size + scroll_content_size);
                        let scroll_offset = scroll_offset_rect.min.to_vec2();

                        // 1. RIGHT BOTTOM
                        let mut shrunk_rect = ui.max_rect();
                        shrunk_rect.min += fixed_size;
                        let mut shrunk_ui = ui.new_child(UiBuilder::new().max_rect(shrunk_rect));
                        shrunk_ui.shrink_clip_rect(bottom_right_rect);
                        delegate.right_bottom_ui(&mut shrunk_ui, scroll_offset);

                        // 2. PAINT OVERLAYS (Bottom-Right)
                        // Paint lines before scrollbars. Clipped to bottom_right_rect.
                        delegate.paint_overlays(&mut shrunk_ui);

                        scroll_offset_rect.min
                    })
                    .inner
            }
            .to_vec2();

            {
                // LEFT TOP: Fixed
                let left_top_rect = rect
                    .with_max_x(rect.left() + fixed_size.x)
                    .with_max_y(rect.top() + fixed_size.y);
                let mut left_top_ui = ui.new_child(UiBuilder::new().max_rect(left_top_rect));
                left_top_ui.shrink_clip_rect(left_top_rect);
                delegate.left_top_ui(&mut left_top_ui);
            }

            {
                // RIGHT TOP: Horizontally scrollable
                let right_top_outer_rect = rect
                    .with_min_x(rect.left() + fixed_size.x)
                    .with_max_y(rect.top() + fixed_size.y);
                let right_top_content_rect = Rect::from_min_size(
                    pos2(right_top_outer_rect.min.x - scroll_offset.x, rect.min.y),
                    vec2(scroll_content_size.x, fixed_size.y),
                );
                let mut right_top_ui =
                    ui.new_child(UiBuilder::new().max_rect(right_top_content_rect));
                right_top_ui.shrink_clip_rect(right_top_outer_rect);
                delegate.right_top_ui(&mut right_top_ui, scroll_offset);
            }

            {
                // LEFT BOTTOM: Vertically scrollable
                let left_bottom_outer_rect = rect
                    .with_max_x(rect.left() + fixed_size.x)
                    .with_min_y(rect.top() + fixed_size.y);
                let left_bottom_content_rect = Rect::from_min_size(
                    pos2(rect.min.x, left_bottom_outer_rect.min.y - scroll_offset.y),
                    vec2(fixed_size.x, scroll_content_size.y),
                );
                let mut left_bottom_ui =
                    ui.new_child(UiBuilder::new().max_rect(left_bottom_content_rect));
                left_bottom_ui.shrink_clip_rect(left_bottom_outer_rect);
                delegate.left_bottom_ui(&mut left_bottom_ui, scroll_offset);
            }

            // 3. PAINT OVERLAYS (Left)
            // Paint lines over sticky areas. Clipped to left_rect.
            let left_rect = rect.with_max_x(rect.left() + fixed_size.x);
            let mut left_ui = ui.new_child(UiBuilder::new().max_rect(left_rect));
            left_ui.shrink_clip_rect(left_rect);
            delegate.paint_overlays(&mut left_ui);

            // 4. UPDATE COL WIDTHS
            // Now that headers and body have populated `max_column_widths`, calculate final sizes.
            delegate.update_col_widths(ui);

            ui.advance_cursor_after_rect(rect);
        });
    }
}
