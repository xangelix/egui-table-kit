use std::{
    collections::{BTreeMap, btree_map::Entry},
    ops::{Range, RangeInclusive},
};

use egui::{
    Align, Context, Id, IdMap, IdSalt, Layout, NumExt as _, Rangef, Rect, Response, Ui, UiBuilder,
    Vec2, Vec2b, vec2,
};

use super::{
    SplitScroll, SplitScrollDelegate,
    columns::{Column, ColumnFlags},
};

// TODO: fix the functionality of this
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum AutoSizeMode {
    /// Never auto-size the columns.
    #[default]
    Never,

    /// Always auto-size the columns
    Always,

    /// Auto-size the columns if the parents' width changes
    OnParentResize,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct TableState {
    // Maps columns ids to their widths.
    pub col_widths: IdMap<f32>,

    pub parent_width: Option<f32>,

    #[serde(default)]
    pub scroll_offset: Option<Vec2>,
}

impl TableState {
    #[must_use]
    pub fn load(ctx: &egui::Context, id: Id) -> Option<Self> {
        ctx.data_mut(|d| d.get_persisted(id))
    }

    pub fn store(self, ctx: &egui::Context, id: Id) {
        ctx.data_mut(|d| d.insert_persisted(id, self));
    }

    #[must_use]
    pub fn id(ui: &Ui, id_salt: IdSalt) -> Id {
        ui.make_persistent_id(id_salt)
    }

    pub fn reset(ctx: &egui::Context, id: Id) {
        ctx.data_mut(|d| {
            d.remove::<Self>(id);
        });
    }
}

/// Describes one of potentially many header rows.
///
/// Each header row has a fixed height.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct HeaderRow {
    pub height: f32,

    /// If empty, it is ignored.
    ///
    /// Contains non-overlapping ranges of column indices to group together.
    /// For instance: `vec![(0..3), (3..5), (5..6)]`.
    pub groups: Vec<Range<usize>>,
}

impl HeaderRow {
    #[must_use]
    pub const fn new(height: f32) -> Self {
        Self {
            height,
            groups: Vec::new(),
        }
    }
}

/// A table viewer.
///
/// Designed to be fast when there are millions of rows, but only hundreds of columns.
///
/// ## Sticky columns and rows
/// You can designate a certain number of column and rows as being "sticky".
/// These won't scroll with the rest of the table.
///
/// The sticky rows are always the first ones at the top, and are usually used for the column headers.
/// The sticky columns are always the first ones on the left, useful for special columns like
/// table row number or similar.
/// A sticky column is sometimes called a "gutter".
///
/// ## Batteries not included
/// * You need to specify the `Table` size beforehand
/// * Does not add any margins to cells. Add it yourself with [`egui::Frame`].
/// * Does not wrap cells in scroll areas. Do that yourself.
/// * Doesn't paint any guide-lines for the rows. Paint them yourself.
pub struct Table {
    /// The columns of the table.
    columns: Vec<Column>,

    /// Salt added to the parent [`Ui::id`] to produce an [`Id`] that is unique
    /// within the parent [`Ui`].
    ///
    /// You need to set this to something unique if you have multiple tables in the same ui.
    id_salt: IdSalt,

    /// Which columns are sticky (non-scrolling)?
    num_sticky_cols: usize,

    /// The count and parameters of the sticky (non-scrolling) header rows.
    headers: Vec<HeaderRow>,

    /// Total number of rows (sticky + non-sticky).
    num_rows: u64,

    /// How to do auto-sizing of columns, if at all.
    auto_size_mode: AutoSizeMode,

    scroll_to_columns: Option<(RangeInclusive<usize>, Option<Align>)>,
    scroll_to_rows: Option<(RangeInclusive<u64>, Option<Align>)>,

    /// If true, the vertical scrollbar will stick to the bottom as the content grows.
    ///
    /// Useful for log views or terminal emulation.
    stick_to_bottom: bool,
    max_height: Option<f32>,
    max_rows: Option<u64>,
}

impl Default for Table {
    fn default() -> Self {
        Self {
            columns: vec![],
            id_salt: IdSalt::new("table"),
            num_sticky_cols: 0,
            headers: vec![HeaderRow::new(16.0)],
            num_rows: 0,
            auto_size_mode: AutoSizeMode::default(),
            scroll_to_columns: None,
            scroll_to_rows: None,
            stick_to_bottom: false,
            max_height: None,
            max_rows: None,
        }
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CellInfo {
    pub col_nr: usize,

    pub row_nr: u64,

    /// The unique [`Id`] of this table.
    pub table_id: Id,

    /// Is the row hovered?
    pub row_hovered: bool,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HeaderCellInfo {
    pub group_index: usize,

    pub col_range: Range<usize>,

    /// Header row
    pub row_nr: usize,

    /// The unique [`Id`] of this table.
    pub table_id: Id,
}

/// Data given to the delegate containing information about what is about to be rendered.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct PrefetchInfo {
    /// The sticky columns are always visible.
    pub num_sticky_columns: usize,

    /// This range of columns are currently visible, in addition to the sticky ones.
    pub visible_columns: Range<usize>,

    /// These rows are currently visible.
    pub visible_rows: Range<u64>,

    /// The unique [`Id`] of this table.
    pub table_id: Id,
}

/// The interface that the user needs to implement to display a table.
///
/// The [`Table`] calls functions on the delegate to render the table.
pub trait TableDelegate {
    /// Called before any call to [`Self::cell_ui`] to communicate the range of visible columns and rows.
    ///
    /// You can use this to only load the data required to be viewed.
    fn prepare(&mut self, _info: &PrefetchInfo) {}

    /// The contents of a header cell in the table.
    ///
    /// The [`CellInfo::row_nr`] is which header row (usually 0).
    fn header_cell_ui(&mut self, ui: &mut Ui, cell: &HeaderCellInfo);

    /// The contents of a row.
    ///
    /// Individual cell [`Ui`]s will be children of the ui passed to this fn, so you can e.g. use
    /// [`Ui::style_mut`] to style the whole row.
    ///
    /// This might be called multiple times per row (e.g. for sticky and non-sticky columns).
    fn row_ui(&mut self, _ui: &mut Ui, _row_nr: u64) {}

    /// The contents of a cell in the table.
    ///
    /// The [`CellInfo::row_nr`] is ignoring header rows.
    fn cell_ui(&mut self, ui: &mut Ui, cell: &CellInfo);

    /// Compute the offset for the top of the given row.
    ///
    /// Implement this for arbitrary row heights. The default implementation uses
    /// [`Self::default_row_height`].
    ///
    /// Note: must always return 0.0 for `row_nr = 0`.
    #[allow(clippy::cast_precision_loss)]
    fn row_top_offset(&self, _ctx: &Context, _table_id: Id, row_nr: u64) -> f32 {
        row_nr as f32 * self.default_row_height()
    }

    /// Default row height.
    ///
    /// This is used by the default implementation of [`Self::row_top_offset`].
    fn default_row_height(&self) -> f32 {
        20.0
    }

    fn uniform_row_height(&self) -> Option<f32> {
        Some(self.default_row_height())
    }
}

impl Table {
    /// Create a new table, with no columns and no headers, and zero rows.
    #[must_use]
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Salt added to the parent [`Ui::id`] to produce an [`Id`] that is unique
    /// within the parent [`Ui`].
    ///
    /// You need to set this to something unique if you have multiple tables in the same ui.
    #[must_use]
    #[inline]
    pub fn id_salt(mut self, id_salt: impl egui::AsIdSalt) -> Self {
        self.id_salt = IdSalt::new(id_salt);
        self
    }

    #[must_use]
    #[inline]
    pub const fn max_rows(mut self, max_rows: u64) -> Self {
        self.max_rows = Some(max_rows);
        self
    }

    /// Total number of rows (sticky + non-sticky).
    #[must_use]
    #[inline]
    pub const fn num_rows(mut self, num_rows: u64) -> Self {
        self.num_rows = num_rows;
        self
    }

    /// The columns of the table.
    #[must_use]
    #[inline]
    pub fn columns(mut self, columns: impl Into<Vec<Column>>) -> Self {
        self.columns = columns.into();
        self
    }

    /// How many columns are sticky (non-scrolling)?
    ///
    /// Default is 0.
    #[must_use]
    #[inline]
    pub const fn num_sticky_cols(mut self, num_sticky_cols: usize) -> Self {
        self.num_sticky_cols = num_sticky_cols;
        self
    }

    /// The count and parameters of the sticky (non-scrolling) header rows.
    #[must_use]
    #[inline]
    pub fn headers(mut self, headers: impl Into<Vec<HeaderRow>>) -> Self {
        self.headers = headers.into();
        self
    }

    /// How to do auto-sizing of columns, if at all.
    #[must_use]
    #[inline]
    pub const fn auto_size_mode(mut self, auto_size_mode: AutoSizeMode) -> Self {
        self.auto_size_mode = auto_size_mode;
        self
    }

    #[must_use]
    #[inline]
    pub const fn max_height(mut self, max_height: f32) -> Self {
        self.max_height = Some(max_height);
        self
    }

    /// The scroll handle will stick to the bottom position even while the content size
    /// changes dynamically.
    ///
    /// This can be useful to simulate terminal UIs or log/info scrollers.
    /// The scroll handle remains stuck until user manually changes position. Once "unstuck"
    /// it will remain focused on whatever content viewport the user left it on.
    #[must_use]
    #[inline]
    pub const fn stick_to_bottom(mut self, stick: bool) -> Self {
        self.stick_to_bottom = stick;
        self
    }

    /// Read the globally unique id, based on the current [`Self::id_salt`]
    /// and the parent id.
    #[must_use]
    #[inline]
    pub fn get_id(&self, ui: &Ui) -> Id {
        TableState::id(ui, self.id_salt)
    }

    /// Set a row to scroll to.
    ///
    /// `align` specifies if the row should be positioned in the top, center, or bottom of the view
    /// (using [`Align::TOP`], [`Align::Center`] or [`Align::BOTTOM`]).
    /// If `align` is `None`, the table will scroll just enough to bring the cursor into view.
    ///
    /// See also: [`Self::scroll_to_column`].
    #[must_use]
    #[inline]
    pub const fn scroll_to_row(self, row: u64, align: Option<Align>) -> Self {
        self.scroll_to_rows(row..=row, align)
    }

    /// Scroll to a range of rows.
    ///
    /// See [`Self::scroll_to_row`] for details.
    #[must_use]
    #[inline]
    pub const fn scroll_to_rows(mut self, rows: RangeInclusive<u64>, align: Option<Align>) -> Self {
        self.scroll_to_rows = Some((rows, align));
        self
    }

    /// Set a column to scroll to.
    ///
    /// `align` specifies if the column should be positioned in the left, center, or right of the view
    /// (using [`Align::LEFT`], [`Align::Center`] or [`Align::RIGHT`]).
    /// If `align` is `None`, the table will scroll just enough to bring the cursor into view.
    ///
    /// See also: [`Self::scroll_to_row`].
    #[must_use]
    #[inline]
    pub const fn scroll_to_column(self, column: usize, align: Option<Align>) -> Self {
        self.scroll_to_columns(column..=column, align)
    }

    /// Scroll to a range of columns.
    ///
    /// See [`Self::scroll_to_column`] for details.
    #[must_use]
    #[inline]
    pub const fn scroll_to_columns(
        mut self,
        columns: RangeInclusive<usize>,
        align: Option<Align>,
    ) -> Self {
        self.scroll_to_columns = Some((columns, align));
        self
    }

    /// The top y coordinate offset of a specific row nr.
    ///
    /// `get_row_top_offset(0)` should always return 0.0.
    #[expect(clippy::unused_self)] // for uniformity
    fn get_row_top_offset(
        &self,
        ctx: &Context,
        table_id: Id,
        table_delegate: &dyn TableDelegate,
        row_nr: u64,
    ) -> f32 {
        table_delegate.row_top_offset(ctx, table_id, row_nr)
    }

    /// Which row contains the given y offset (from the top)?
    fn get_row_nr_at_y_offset(
        &self,
        ctx: &Context,
        table_id: Id,
        table_delegate: &dyn TableDelegate,
        y_offset: f32,
    ) -> u64 {
        if let Some(height) = table_delegate.uniform_row_height()
            && height > 0.0
        {
            return ((y_offset / height) as u64).at_most(self.num_rows.saturating_sub(1));
        }

        // Fall back to binary search for variable heights
        partition_point(0..=self.num_rows, |row_nr| {
            y_offset <= self.get_row_top_offset(ctx, table_id, table_delegate, row_nr)
        })
        .saturating_sub(1)
    }

    pub fn show(mut self, ui: &mut Ui, table_delegate: &mut dyn TableDelegate) -> Response {
        self.num_sticky_cols = self.num_sticky_cols.at_most(self.columns.len());

        let id = TableState::id(ui, self.id_salt);
        let state = TableState::load(ui, id);
        let is_new = state.is_none();
        let mut state = state.unwrap_or_default();

        for (i, column) in self.columns.iter_mut().enumerate() {
            let column_id = column.id_for(i);
            let cached_width = state.col_widths.get(&column_id).copied();
            if let Some(existing_width) = cached_width {
                column.current = existing_width;
            } else {
                // If it is a new column and configured for auto-fitting, trigger sizing pass
                if column.is_auto_fit() {
                    column.flags.set(ColumnFlags::AUTO_SIZE_THIS_FRAME, true);
                }
            }
            column.current = column.range.clamp(column.current);

            // Only run the initial sizing pass on columns configured for auto-fitting
            if is_new && column.is_auto_fit() {
                column.flags.set(ColumnFlags::AUTO_SIZE_THIS_FRAME, true);
            }
        }

        // Apply active column dragging at the start of the frame so that col_x,
        // sticky_size, quadrant clip rects, and cell layouts are immediately computed
        // with the new dragged width instead of lagging by 1 frame or clipping sticky separators.
        if ui.input(|i| i.pointer.primary_down())
            && let Some(pointer) = ui.ctx().pointer_latest_pos()
        {
            let table_min_x = ui.cursor().min.x;
            let scroll_x = state.scroll_offset.unwrap_or(Vec2::ZERO).x;
            let mut col_left = table_min_x;

            for (i, column) in self.columns.iter_mut().enumerate() {
                let column_id = column.id_for(i);
                let header_resize_id = id.with(column_id).with("resize_header");
                let body_resize_id = id.with(column_id).with("resize_body");
                let is_dragged = ui.ctx().is_being_dragged(header_resize_id)
                    || ui.ctx().is_being_dragged(body_resize_id);

                if is_dragged {
                    let screen_col_left = if i < self.num_sticky_cols {
                        col_left
                    } else {
                        col_left - scroll_x
                    };
                    let new_width = pointer.x - screen_col_left;
                    let clamped_width = column.range.clamp(new_width);
                    state.col_widths.insert(column_id, clamped_width);
                    column.current = clamped_width;
                }

                col_left += column.current;
            }
        }

        // Only do full sizing pass if there are any columns that actually need to be auto-fitted
        let do_full_sizing_pass = is_new
            && self
                .columns
                .iter()
                .any(super::columns::Column::is_auto_size_this_frame);

        let parent_width = ui.available_width();
        let auto_size = match self.auto_size_mode {
            AutoSizeMode::Never => false,
            AutoSizeMode::Always => true,
            AutoSizeMode::OnParentResize => state.parent_width != Some(parent_width),
        };
        if auto_size {
            Column::auto_size(&mut self.columns, parent_width);
        }
        state.parent_width = Some(parent_width);

        let col_x = {
            let mut x = ui.cursor().min.x;
            let mut col_x = Vec::with_capacity(self.columns.len() + 1);
            col_x.push(x);
            for column in &self.columns {
                x += column.current;
                col_x.push(x);
            }
            col_x
        };

        let header_row_y = {
            let mut y = ui.cursor().min.y;
            let mut sticky_row_y = Vec::with_capacity(self.headers.len() + 1);
            sticky_row_y.push(y);
            for header in &self.headers {
                y += header.height;
                sticky_row_y.push(y);
            }
            sticky_row_y
        };

        let sticky_size = Vec2::new(
            self.columns[..self.num_sticky_cols]
                .iter()
                .map(|c| c.current)
                .sum(),
            self.headers.iter().map(|h| h.height).sum(),
        );

        let mut ui_builder = UiBuilder::new().layout(Layout::top_down(Align::Min));
        if do_full_sizing_pass {
            ui_builder = ui_builder.sizing_pass().invisible();
            ui.request_discard("Full egui_table sizing");
        }
        let response = ui
            .scope_builder(ui_builder, |ui| {
                // Don't wrap text in the table cells.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);

                let num_columns = self.columns.len();

                for (col_nr, column) in self.columns.iter_mut().enumerate() {
                    if column.is_resizable() {
                        let header_resize_id = id.with(column.id_for(col_nr)).with("resize_header");
                        let body_resize_id = id.with(column.id_for(col_nr)).with("resize_body");
                        let double_clicked = ui
                            .read_response(header_resize_id)
                            .is_some_and(|r| r.double_clicked())
                            || ui
                                .read_response(body_resize_id)
                                .is_some_and(|r| r.double_clicked());
                        if double_clicked {
                            column.flags.set(ColumnFlags::AUTO_SIZE_THIS_FRAME, true);
                        }
                    }
                    if column.is_auto_size_this_frame() {
                        ui.request_discard("egui_table column sizing");
                    }
                }

                SplitScroll {
                    scroll_enabled: Vec2b::new(true, true),
                    fixed_size: sticky_size,
                    scroll_outer_size: {
                        // Calculate the combined height of the headers and rows
                        let total_rows_height =
                            self.get_row_top_offset(ui, id, table_delegate, self.num_rows);
                        let total_content_height = sticky_size.y + total_rows_height;

                        // Ensure a minimum height of up to 10 rows (or self.num_rows if smaller)
                        // to prevent collapsing to header-only height during animations/sizing passes.
                        let min_rows = self.num_rows.min(10);
                        let min_rows_height =
                            self.get_row_top_offset(ui, id, table_delegate, min_rows);
                        let min_table_height = sticky_size.y + min_rows_height;

                        // Calculate the maximum allowed height based on row limits, max pixel limits, or the visible clip rect.
                        let max_height_limit = if let Some(max_r) = self.max_rows {
                            let max_rows_height =
                                self.get_row_top_offset(ui, id, table_delegate, max_r);
                            sticky_size.y + max_rows_height
                        } else {
                            self.max_height.unwrap_or_else(|| ui.clip_rect().height())
                        };

                        let available_height = ui
                            .available_height()
                            .at_most(max_height_limit)
                            .max(min_table_height);

                        let allocated_height = total_content_height.min(available_height);

                        Vec2::new(
                            (ui.available_width() - sticky_size.x).max(0.0),
                            (allocated_height - sticky_size.y).max(0.0),
                        )
                    },
                    scroll_content_size: Vec2::new(
                        self.columns[self.num_sticky_cols..]
                            .iter()
                            .map(|c| c.current)
                            .sum(),
                        self.get_row_top_offset(ui, id, table_delegate, self.num_rows),
                    ),
                    stick_to_bottom: self.stick_to_bottom,
                }
                .show(
                    ui,
                    &mut TableSplitScrollDelegate {
                        id,
                        table_delegate,
                        state: &mut state,
                        table: &mut self,
                        col_x,
                        header_row_y,
                        max_column_widths: vec![0.0; num_columns],
                        visible_column_lines: BTreeMap::default(),
                        do_full_sizing_pass,
                        has_prefetched: false,
                        egui_ctx: ui.ctx().clone(),
                        dragging_col: None,
                    },
                );
            })
            .response;

        state.store(ui, id);
        response
    }
}

#[derive(Clone, Copy, Debug)]
struct ColumnResizer {
    scroll_offset: Vec2,

    top: f32,
}

fn update(map: &mut BTreeMap<usize, ColumnResizer>, key: usize, value: ColumnResizer) {
    match map.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(value);
        }
        Entry::Occupied(mut entry) => {
            entry.get_mut().top = entry.get_mut().top.min(value.top);
        }
    }
}

struct TableSplitScrollDelegate<'a> {
    id: Id,
    table_delegate: &'a mut dyn TableDelegate,
    table: &'a mut Table,
    state: &'a mut TableState,

    /// The x coordinate for the start of each column, plus the end of the last column.
    col_x: Vec<f32>,

    /// The y coordinate for the start of each header row, plus the end of the last header row.
    header_row_y: Vec<f32>,

    /// Actual width of the widest element in each column
    max_column_widths: Vec<f32>,

    /// Key is column number. The resizer is to the right of the column.
    visible_column_lines: BTreeMap<usize, ColumnResizer>,

    do_full_sizing_pass: bool,

    has_prefetched: bool,

    egui_ctx: Context,

    dragging_col: Option<usize>,
}

impl TableSplitScrollDelegate<'_> {
    /// Helper wrapper around [`Table::get_row_top_offset`].
    fn get_row_top_offset(&self, row_nr: u64) -> f32 {
        self.table
            .get_row_top_offset(&self.egui_ctx, self.id, self.table_delegate, row_nr)
    }

    /// Helper wrapper around [`Table::get_row_nr_at_y_offset`].
    fn get_row_nr_at_y_offset(&self, y_offset: f32) -> u64 {
        self.table
            .get_row_nr_at_y_offset(&self.egui_ctx, self.id, self.table_delegate, y_offset)
    }

    fn header_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2) {
        // Compute the visible column range for the current quadrant viewport
        let viewport = ui.clip_rect().translate(scroll_offset);

        #[allow(clippy::float_cmp)]
        let col_range = if self.table.columns.is_empty() || viewport.left() == viewport.right() {
            0..0
        } else if self.do_full_sizing_pass {
            // Render all columns during a sizing pass to measure layout constraints
            0..self.table.columns.len()
        } else {
            let col_idx_at = |x: f32| -> usize {
                self.col_x
                    .partition_point(|&col_x| col_x < x)
                    .saturating_sub(1)
                    .at_most(self.table.columns.len() - 1)
            };

            col_idx_at(viewport.min.x)..col_idx_at(viewport.max.x) + 1
        };

        let last_header_row_y = self.header_row_y.last().copied().unwrap_or(0.0);

        for (row_nr, header_row) in self.table.headers.iter().enumerate() {
            let groups = if header_row.groups.is_empty() {
                (0..self.table.columns.len()).map(|i| i..i + 1).collect()
            } else {
                header_row.groups.clone()
            };

            let y_range = Rangef::new(self.header_row_y[row_nr], self.header_row_y[row_nr + 1]);

            for (group_index, col_range_group) in groups.into_iter().enumerate() {
                let start = col_range_group.start;
                let end = col_range_group.end;

                // Skip processing and rendering if this group is outside the quadrant's visible column span
                if end <= col_range.start || start >= col_range.end {
                    continue;
                }

                let mut header_rect =
                    Rect::from_x_y_ranges(self.col_x[start]..=self.col_x[end], y_range)
                        .translate(-scroll_offset);

                if 0 < start
                    && self.table.columns[start - 1].is_resizable()
                    && ui.clip_rect().x_range().contains(header_rect.left())
                {
                    // The previous column is resizable, so make sure the resize line goes to above this heading:
                    update(
                        &mut self.visible_column_lines,
                        start - 1,
                        ColumnResizer {
                            scroll_offset,
                            top: header_rect.top(),
                        },
                    );
                }

                let clip_rect = header_rect;

                let last_column = &self.table.columns[end - 1];
                let auto_size_this_frame = last_column.is_auto_size_this_frame();

                if auto_size_this_frame {
                    header_rect.max.x = header_rect.min.x
                        + self.table.columns[start..end]
                            .iter()
                            .map(|column| column.range.min)
                            .sum::<f32>();
                }

                let mut ui_builder = UiBuilder::new()
                    .max_rect(header_rect)
                    .id_salt(("header", row_nr, group_index))
                    .layout(egui::Layout::left_to_right(egui::Align::Center));
                if auto_size_this_frame {
                    ui_builder = ui_builder.sizing_pass();
                }
                let mut cell_ui = ui.new_child(ui_builder);
                cell_ui.shrink_clip_rect(clip_rect);

                self.table_delegate.header_cell_ui(
                    &mut cell_ui,
                    &HeaderCellInfo {
                        group_index,
                        col_range: col_range_group,
                        row_nr,
                        table_id: self.id,
                    },
                );

                if start + 1 == end {
                    // normal single-column group
                    let col_nr = start;
                    let column = &self.table.columns[start];
                    let width = &mut self.max_column_widths[col_nr];
                    *width = width.max(cell_ui.min_size().x);

                    // Save column lines for later interaction:
                    if column.is_resizable()
                        && ui.clip_rect().x_range().contains(header_rect.right())
                    {
                        update(
                            &mut self.visible_column_lines,
                            col_nr,
                            ColumnResizer {
                                scroll_offset,
                                top: header_rect.top(),
                            },
                        );
                    }
                }
            }
        }

        let is_sticky_quadrant = self.table.num_sticky_cols > 0
            && ui.clip_rect().max.x <= self.col_x[self.table.num_sticky_cols] + 5.0;

        // Repaint separator lines over the headers and handle resize interaction in the header
        for (col_nr, ColumnResizer { scroll_offset, top }) in &self.visible_column_lines {
            let col_nr = *col_nr;
            if is_sticky_quadrant {
                if col_nr >= self.table.num_sticky_cols {
                    continue;
                }
            } else if col_nr < self.table.num_sticky_cols {
                continue;
            }

            let Some(column) = self.table.columns.get(col_nr) else {
                continue;
            };
            if !column.is_resizable() {
                continue;
            }

            let column_id = column.id_for(col_nr);
            let range = column.range;
            let current = column.current;
            let mut column_width = self
                .state
                .col_widths
                .get(&column_id)
                .copied()
                .unwrap_or(current);

            let mut x = self.col_x[col_nr + 1] - scroll_offset.x + (column_width - current);
            let yrange = Rangef::new(*top, last_header_row_y);

            let line_rect = egui::Rect::from_x_y_ranges(x..=x, yrange.min..=yrange.max)
                .expand(ui.style().interaction.resize_grab_radius_side);

            let header_resize_id = self.id.with(column_id).with("resize_header");
            let body_resize_id = self.id.with(column_id).with("resize_body");
            let resize_response =
                ui.interact(line_rect, header_resize_id, egui::Sense::click_and_drag());

            let hovered = resize_response.hovered();
            let is_dragged = resize_response.dragged()
                || ui.ctx().is_being_dragged(header_resize_id)
                || ui.ctx().is_being_dragged(body_resize_id)
                || self.dragging_col == Some(col_nr);

            if is_dragged && let Some(pointer) = ui.pointer_latest_pos() {
                let new_width = column_width + pointer.x - x;
                let clamped_width = range.clamp(new_width);
                self.state.col_widths.insert(column_id, clamped_width);
                self.dragging_col = Some(col_nr);
                column_width = clamped_width;
                x = self.col_x[col_nr + 1] - scroll_offset.x + (column_width - current);
            }

            let hovered_col_id = self.id.with("hovered_col_resize");
            let current_frame = ui.ctx().cumulative_frame_nr();
            if hovered {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(hovered_col_id, (current_frame, col_nr)));
            }

            let is_hovered = if let Some((frame, hovered_col)) = ui
                .ctx()
                .data(|d| d.get_temp::<(u64, usize)>(hovered_col_id))
            {
                hovered_col == col_nr
                    && (frame == current_frame || frame == current_frame.saturating_sub(1))
            } else {
                false
            };

            if is_hovered || is_dragged {
                ui.set_cursor_icon(egui::CursorIcon::ResizeColumn);
            }

            let stroke = if is_dragged {
                ui.style().visuals.widgets.active.bg_stroke
            } else if is_hovered {
                ui.style().visuals.widgets.hovered.bg_stroke
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke
            };

            ui.painter().vline(x, yrange, stroke);
        }
    }

    fn region_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2, do_prefetch: bool) {
        // Used to find the visible range of columns and rows:
        let viewport = ui.clip_rect().translate(scroll_offset);
        let last_header_row_y = self.header_row_y.last().copied().unwrap_or(0.0);

        #[allow(clippy::float_cmp)]
        let col_range = if self.table.columns.is_empty() || viewport.left() == viewport.right() {
            0..0
        } else if self.do_full_sizing_pass {
            // We do the UI for all columns during a sizing pass, so we can auto-size ALL columns
            0..self.table.columns.len()
        } else {
            // Only paint the visible columns:
            let col_idx_at = |x: f32| -> usize {
                self.col_x
                    .partition_point(|&col_x| col_x < x)
                    .saturating_sub(1)
                    .at_most(self.table.columns.len() - 1)
            };

            col_idx_at(viewport.min.x)..col_idx_at(viewport.max.x) + 1
        };

        #[allow(clippy::float_cmp)]
        let row_range = if self.table.num_rows == 0 || viewport.top() == viewport.bottom() {
            0..0
        } else if self.do_full_sizing_pass {
            // We do the UI for all rows during a sizing pass, so we can auto-size rows and columns
            0..self.table.num_rows
        } else {
            let row_idx_at = |y: f32| -> u64 { self.get_row_nr_at_y_offset(y - last_header_row_y) };

            row_idx_at(viewport.min.y)..row_idx_at(viewport.max.y) + 1
        };

        if do_prefetch {
            self.table_delegate.prepare(&PrefetchInfo {
                visible_rows: row_range.clone(),
                num_sticky_columns: self.table.num_sticky_cols,
                visible_columns: col_range.clone(),
                table_id: self.id,
            });
            self.has_prefetched = true;
        } else {
            debug_assert!(
                self.has_prefetched,
                "TableSplitScrollDelegate::region_ui called without prefetch having happened"
            );
        }

        // If the table layout has not been properly initialized, don't display
        if !self.do_full_sizing_pass && self.col_x.len() != self.table.columns.len() + 1 {
            ui.ctx().request_discard(
                "SplitScrollDelegate: Table col_x length does not match self.table.columns.len() + 1",
            );
            return;
        }

        if self.header_row_y.len() != self.table.headers.len() + 1 {
            ui.ctx().request_discard(
                "SplitScrollDelegate: Table header_row_y length does not match self.table.headers.len() + 1",
            );
            return;
        }

        if last_header_row_y != self.header_row_y[self.header_row_y.len() - 1] {
            ui.ctx()
                .request_discard("SplitScroll delegate methods called in unexpected order");
        }

        let pointer_pos = ui.ctx().pointer_latest_pos();
        let current_frame = ui.ctx().cumulative_frame_nr();
        let hovered_row_id = self.id.with("hovered_row");

        for row_nr in row_range {
            let y_range = Rangef::new(
                last_header_row_y + self.get_row_top_offset(row_nr),
                last_header_row_y + self.get_row_top_offset(row_nr + 1),
            );

            let row_x_range = self.col_x[0]..=self.col_x[self.col_x.len() - 1];
            let row_rect = Rect::from_x_y_ranges(row_x_range, y_range).translate(-scroll_offset);

            // Check if the cursor is hovering over the visible portion of this row
            if let Some(pos) = pointer_pos {
                let visible_row_rect = row_rect.intersect(ui.clip_rect());

                // Exclusive check on bottom and right edges to prevent multi-row highlights
                let contains_exclusive = visible_row_rect.min.x <= pos.x
                    && pos.x < visible_row_rect.max.x
                    && visible_row_rect.min.y <= pos.y
                    && pos.y < visible_row_rect.max.y;

                if contains_exclusive && ui.rect_contains_pointer(visible_row_rect) {
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(hovered_row_id, (current_frame, row_nr)));
                }
            }

            // Determine if the current row was hovered on this frame or the previous one
            let row_hovered = if let Some((frame, hovered_row)) =
                ui.ctx().data(|d| d.get_temp::<(u64, u64)>(hovered_row_id))
            {
                hovered_row == row_nr
                    && (frame == current_frame || frame == current_frame.saturating_sub(1))
            } else {
                false
            };

            let mut row_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(row_rect)
                    .id_salt(("row", row_nr))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            row_ui.set_min_size(row_rect.size());

            self.table_delegate.row_ui(&mut row_ui, row_nr);

            for col_nr in col_range.clone() {
                let column = &self.table.columns[col_nr];
                let mut cell_rect =
                    Rect::from_x_y_ranges(self.col_x[col_nr]..=self.col_x[col_nr + 1], y_range)
                        .translate(-scroll_offset);
                let clip_rect = cell_rect;
                let auto_size_this_frame = column.is_auto_size_this_frame();
                if auto_size_this_frame {
                    cell_rect.max.x = cell_rect.min.x + column.range.min;
                }

                let mut ui_builder = UiBuilder::new()
                    .max_rect(cell_rect)
                    .id_salt((row_nr, col_nr))
                    .layout(egui::Layout::left_to_right(egui::Align::Center));
                if auto_size_this_frame {
                    ui_builder = ui_builder.sizing_pass();
                }
                let mut cell_ui = row_ui.new_child(ui_builder);
                cell_ui.shrink_clip_rect(clip_rect);

                self.table_delegate.cell_ui(
                    &mut cell_ui,
                    &CellInfo {
                        col_nr,
                        row_nr,
                        table_id: self.id,
                        row_hovered,
                    },
                );

                let width = &mut self.max_column_widths[col_nr];
                *width = width.max(cell_ui.min_size().x);
            }
        }

        // Save column lines for later interaction:
        for col_nr in col_range {
            let column = &self.table.columns[col_nr];
            if column.is_resizable() {
                update(
                    &mut self.visible_column_lines,
                    col_nr,
                    ColumnResizer {
                        scroll_offset,
                        top: last_header_row_y,
                    },
                );
            }
        }
    }
}

impl SplitScrollDelegate for TableSplitScrollDelegate<'_> {
    fn left_top_ui(&mut self, ui: &mut Ui) {
        self.header_ui(ui, Vec2::ZERO);
    }

    fn right_top_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2) {
        let horizontal_scroll_offset = vec2(scroll_offset.x, 0.0);
        self.header_ui(ui, horizontal_scroll_offset);
    }

    fn right_bottom_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2) {
        if self.table.scroll_to_columns.is_some() || self.table.scroll_to_rows.is_some() {
            let mut target_rect = ui.clip_rect(); // no scrolling
            let mut target_align = None;

            if let Some((column_range, align)) = &self.table.scroll_to_columns {
                // Use the first scrollable column as the base, so that offsets start
                // at 0 for the first non-sticky column — mirroring how row_top_offset
                // starts at 0 for the first data row.
                let scrollable_col_x_base = self.col_x[self.table.num_sticky_cols];
                let x_from_column_nr = |col_nr: usize| -> f32 {
                    ui.min_rect().left() + (self.col_x[col_nr] - scrollable_col_x_base)
                };

                let sticky_width = scrollable_col_x_base - self.col_x[0];

                // Subtract sticky_width from the left of the target rect so that when
                // scroll_to_rect aligns the left of the target to the viewport left, the
                // actual column lands just right of the sticky columns (not behind them).
                target_rect.min.x = x_from_column_nr(*column_range.start()) - sticky_width;
                target_rect.max.x = x_from_column_nr(*column_range.end() + 1);
                target_align = target_align.or(*align);
            }

            if let Some((row_range, align)) = &self.table.scroll_to_rows {
                let y_from_row_nr =
                    |row_nr: u64| -> f32 { ui.min_rect().top() + self.get_row_top_offset(row_nr) };

                let last_header_row_y = self.header_row_y.last().copied().unwrap_or(0.0);
                let sticky_height = last_header_row_y - self.header_row_y[0];

                // Subtract sticky_height from the top of the target rect so that when
                // scroll_to_rect aligns the top of the target to the viewport top, the
                // actual row lands just below the sticky header (not behind it).
                target_rect.min.y = y_from_row_nr(*row_range.start()) - sticky_height;
                target_rect.max.y = y_from_row_nr(*row_range.end() + 1);
                target_align = target_align.or(*align);
            }

            ui.scroll_to_rect(target_rect, target_align);
        }

        self.state.scroll_offset = Some(scroll_offset);
        self.region_ui(ui, scroll_offset, true);
    }

    fn left_bottom_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2) {
        let vertical_scroll_offset = vec2(0.0, scroll_offset.y);
        self.region_ui(ui, vertical_scroll_offset, false);
    }

    fn paint_overlays(&mut self, ui: &mut Ui) {
        let total_rows_height = self.get_row_top_offset(self.table.num_rows);
        let header_bottom = self.header_row_y.last().copied().unwrap_or(0.0);
        let clip_bottom = ui.clip_rect().bottom();

        let is_sticky_quadrant = self.table.num_sticky_cols > 0
            && ui.clip_rect().max.x <= self.col_x[self.table.num_sticky_cols] + 5.0;

        // Interaction and painting for body lines
        for (col_nr, ColumnResizer { scroll_offset, top }) in &self.visible_column_lines {
            let col_nr = *col_nr;
            if is_sticky_quadrant {
                if col_nr >= self.table.num_sticky_cols {
                    continue;
                }
            } else if col_nr < self.table.num_sticky_cols {
                continue;
            }

            let Some(column) = self.table.columns.get(col_nr) else {
                continue;
            };
            if !column.is_resizable() {
                continue;
            }

            let column_id = column.id_for(col_nr);
            let range = column.range;
            let current = column.current;
            let mut column_width = self
                .state
                .col_widths
                .get(&column_id)
                .copied()
                .unwrap_or(current);

            let mut x = self.col_x[col_nr + 1] - scroll_offset.x + (column_width - current);
            let content_bottom = header_bottom + total_rows_height - scroll_offset.y;
            let line_bottom = clip_bottom.min(content_bottom);
            let yrange = Rangef::new(*top, line_bottom);

            let line_rect = egui::Rect::from_x_y_ranges(x..=x, yrange.min..=yrange.max)
                .expand(ui.style().interaction.resize_grab_radius_side);

            let header_resize_id = self.id.with(column_id).with("resize_header");
            let body_resize_id = self.id.with(column_id).with("resize_body");
            let resize_response =
                ui.interact(line_rect, body_resize_id, egui::Sense::click_and_drag());

            let hovered = resize_response.hovered();
            let is_dragged = resize_response.dragged()
                || ui.ctx().is_being_dragged(header_resize_id)
                || ui.ctx().is_being_dragged(body_resize_id)
                || self.dragging_col == Some(col_nr);

            if is_dragged && let Some(pointer) = ui.pointer_latest_pos() {
                let new_width = column_width + pointer.x - x;
                let clamped_width = range.clamp(new_width);
                self.state.col_widths.insert(column_id, clamped_width);
                self.dragging_col = Some(col_nr);
                column_width = clamped_width;
                x = self.col_x[col_nr + 1] - scroll_offset.x + (column_width - current);
            }

            let hovered_col_id = self.id.with("hovered_col_resize");
            let current_frame = ui.ctx().cumulative_frame_nr();
            if hovered {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(hovered_col_id, (current_frame, col_nr)));
            }

            let is_hovered = if let Some((frame, hovered_col)) = ui
                .ctx()
                .data(|d| d.get_temp::<(u64, usize)>(hovered_col_id))
            {
                hovered_col == col_nr
                    && (frame == current_frame || frame == current_frame.saturating_sub(1))
            } else {
                false
            };

            if is_hovered || is_dragged {
                ui.set_cursor_icon(egui::CursorIcon::ResizeColumn);
            }

            let stroke = if is_dragged {
                ui.style().visuals.widgets.active.bg_stroke
            } else if is_hovered {
                ui.style().visuals.widgets.hovered.bg_stroke
            } else {
                ui.visuals().widgets.noninteractive.bg_stroke
            };

            ui.painter().vline(x, yrange, stroke);
        }
    }

    fn update_col_widths(&mut self, ui: &mut Ui) {
        for col_nr in 0..self.table.columns.len() {
            // Skip auto-sizing if the user is actively dragging this column
            if self.dragging_col == Some(col_nr) {
                continue;
            }

            let column = self.table.columns.get(col_nr);
            let Some(column) = column else {
                continue;
            };
            if !column.is_resizable() {
                continue;
            }

            let column_id = column.id_for(col_nr);
            let used_width = column.range.clamp(self.max_column_widths[col_nr]);
            let old_width = self
                .state
                .col_widths
                .get(&column_id)
                .copied()
                .unwrap_or(column.current);

            // Copy flags to avoid borrow checker issues
            let auto_size_this_frame = column.is_auto_size_this_frame();
            let auto_fit = column.is_auto_fit();

            if auto_size_this_frame {
                self.table.columns[col_nr]
                    .flags
                    .set(ColumnFlags::AUTO_SIZE_THIS_FRAME, false);
            }

            let mut new_width = old_width;
            if auto_size_this_frame || (ui.is_sizing_pass() && auto_fit) {
                new_width = used_width;
            } else if auto_fit {
                new_width = old_width.max(used_width);
            }

            self.state.col_widths.insert(column_id, new_width);
        }

        // Clear the drag state when the mouse button is released
        if !ui.input(|i| i.pointer.primary_down()) {
            self.dragging_col = None;
        }
    }
}

/// Returns the index of the first element that returns `true` using binary search.
fn partition_point(range: RangeInclusive<u64>, second_partition: impl Fn(u64) -> bool) -> u64 {
    let mut min = *range.start();
    let mut max = *range.end();

    debug_assert!(min < max, "Bad call to partition_point");

    while min < max {
        let mid = min + (max - min) / 2;

        if second_partition(mid) {
            max = mid;
        } else {
            min = mid + 1;
        }
    }

    min
}

#[cfg(test)]
mod tests {
    use super::partition_point;

    #[test]
    fn test_partition_point() {
        assert_eq!(partition_point(0..=17, |i| 8 <= i), 8);
        assert_eq!(partition_point(0..=17, |i| 9 <= i), 9);
        assert_eq!(partition_point(10..=17, |_| true), 10);
        assert_eq!(partition_point(10..=17, |_| false), 17);
    }
}
