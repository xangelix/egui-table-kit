use std::borrow::Cow;

use compact_str::ToCompactString as _;

use super::{error::TableError, filter::Filter, state::TableState};

/// A single cell representation containing a primary value and an optional hover/display override.
pub type TableCell<'a> = (Cow<'a, str>, Option<Cow<'a, str>>);

/// A row slice reference passed sequentially to callbacks during iteration.
pub type RowSlice<'a, 'b> = &'b [TableCell<'a>];

/// The callback signature used to process streamed row data.
/// - `'b` represents the lifetime of any local variables captured by the closure.
/// - `for<'a, 'c>` unifies the string content lifetime `'a` and reference lifetime `'c`,
///   allowing the callback to process rows with short-lived, local lifetimes.
pub type RowCallback<'b> = dyn for<'a, 'c> FnMut(RowSlice<'a, 'c>) -> Result<(), TableError> + 'b;

#[derive(Debug, Copy, Clone, serde::Serialize, serde::Deserialize)]
pub struct RowHierarchy {
    pub indent_level: usize,
    pub has_children: bool,
    pub is_expanded: bool,
}

pub trait TableProvider {
    fn headers(&self) -> &[&str];
    fn row_count(&self) -> usize;

    fn for_selected_rows(
        &self,
        state: &TableState,
        f: &mut RowCallback<'_>,
    ) -> Result<(), TableError>;

    fn for_all_rows(&self, f: &mut RowCallback<'_>) -> Result<(), TableError>;

    /// Sorts the active row indices by the specified column.
    /// Uses a generic string-based fallback sorting implementation, but can be overridden.
    fn sort_active_rows(
        &self,
        active_rows: &mut Vec<usize>,
        col_index: usize,
        ascending: bool,
    ) -> Result<(), TableError> {
        // Collect string values for all rows at `col_index`
        let mut values = Vec::with_capacity(self.row_count());
        self.for_all_rows(&mut |row| {
            let val = row
                .get(col_index)
                .map(|(v, _)| v.to_compact_string())
                .unwrap_or_default();
            values.push(val);
            Ok(())
        })?;

        // Sort active_rows using the collected values
        active_rows.sort_by(|&a, &b| {
            let val_a = values.get(a);
            let val_b = values.get(b);
            if ascending {
                val_a.cmp(&val_b)
            } else {
                val_b.cmp(&val_a)
            }
        });

        Ok(())
    }

    /// Filters all rows sequentially. Override this to implement custom parallel filtering (e.g. Rayon).
    fn filter_rows(
        &self,
        state: &TableState,
        filters: &[(usize, Filter)],
    ) -> Result<Vec<usize>, TableError> {
        if filters.is_empty() {
            return Ok((0..self.row_count()).collect());
        }

        let mut passing_indices = Vec::with_capacity(self.row_count());
        let mut row_idx = 0;

        self.for_all_rows(&mut |row| {
            let highlight = state.highlights.get_usize(row_idx);
            let mut matches = true;

            for &(col_idx, ref filter) in filters {
                if let Some(cell) = row.get(col_idx) {
                    if !filter.matches(&cell.0, highlight) {
                        matches = false;
                        break;
                    }
                } else {
                    matches = false;
                    break;
                }
            }

            if matches {
                passing_indices.push(row_idx);
            }
            row_idx += 1;
            Ok(())
        })?;

        Ok(passing_indices)
    }

    /// Returns tree nesting parameters for a given row.
    /// Evaluates to `None` by default (representing traditional non-hierarchical flat tables).
    fn row_hierarchy(&self, _state: &TableState, _row_index: usize) -> Option<RowHierarchy> {
        None
    }

    /// Returns whether this provider represents a hierarchical tree table.
    /// Returns `false` by default.
    fn is_tree(&self) -> bool {
        false
    }

    /// Returns the active parent row index for a given row (if any).
    fn row_parent(&self, _row_index: usize) -> Option<usize> {
        None
    }

    /// Returns the child row indices nested immediately under the specified row.
    fn row_children(&self, _row_index: usize) -> Vec<usize> {
        Vec::new()
    }

    /// Returns whether an individual row matches the currently active column filters.
    fn row_matches(
        &self,
        _state: &TableState,
        _row_index: usize,
        _filters: &[(usize, Filter)],
        _highlight: Option<u8>,
    ) -> bool {
        true
    }
}

impl dyn TableProvider + '_ {
    /// Maps over each selected row with a closure and collects the results into a flat Vector.
    pub fn map_selected_rows<T, F>(
        &self,
        state: &TableState,
        mut f: F,
    ) -> Result<Vec<T>, TableError>
    where
        F: FnMut(RowSlice<'_, '_>) -> Result<T, TableError>,
    {
        let mut results = Vec::with_capacity(state.selected_rows.len() as usize);
        self.for_selected_rows(state, &mut |row| {
            results.push(f(row)?);
            Ok(())
        })?;
        Ok(results)
    }

    /// Maps only the first selected row (if any) and returns the result, stopping iteration immediately.
    pub fn map_first_selected_row<T, F>(
        &self,
        state: &TableState,
        f: F,
    ) -> Result<Option<T>, TableError>
    where
        F: FnOnce(RowSlice<'_, '_>) -> Result<T, TableError>,
    {
        let mut result = None;
        let mut f_opt = Some(f);

        self.for_selected_rows(state, &mut |row| {
            if let Some(f_once) = f_opt.take() {
                result = Some(f_once(row)?);
            }
            Ok(())
        })?;

        Ok(result)
    }
}

pub trait RowSliceExt {
    /// Extracts the primary text at the specified column index.
    fn get_primary(&self, col_index: usize) -> Result<&str, TableError>;

    /// Extracts the hover/alternate text at the specified column index.
    fn get_hover(&self, col_index: usize) -> Result<&str, TableError>;

    /// Parses the primary text at the specified column index into type `T`.
    fn parse_primary<T>(&self, col_index: usize) -> Result<T, TableError>
    where
        T: std::str::FromStr,
        <T as std::str::FromStr>::Err: std::fmt::Display;

    /// Parses the hover text at the specified column index into type `T`.
    fn parse_hover<T>(&self, col_index: usize) -> Result<T, TableError>
    where
        T: std::str::FromStr,
        <T as std::str::FromStr>::Err: std::fmt::Display;
}

impl RowSliceExt for RowSlice<'_, '_> {
    fn get_primary(&self, col_index: usize) -> Result<&str, TableError> {
        self.get(col_index)
            .map(|(val, _)| val.as_ref())
            .ok_or(TableError::CorruptedState)
    }

    fn get_hover(&self, col_index: usize) -> Result<&str, TableError> {
        self.get(col_index)
            .and_then(|(_, hover)| hover.as_ref().map(AsRef::as_ref))
            .ok_or(TableError::CorruptedState)
    }

    fn parse_primary<T>(&self, col_index: usize) -> Result<T, TableError>
    where
        T: std::str::FromStr,
        <T as std::str::FromStr>::Err: std::fmt::Display,
    {
        T::from_str(self.get_primary(col_index)?).map_err(|e| TableError::Generic(e.to_string()))
    }

    fn parse_hover<T>(&self, col_index: usize) -> Result<T, TableError>
    where
        T: std::str::FromStr,
        <T as std::str::FromStr>::Err: std::fmt::Display,
    {
        T::from_str(self.get_hover(col_index)?).map_err(|e| TableError::Generic(e.to_string()))
    }
}

pub struct OperationContext<'a, 'b> {
    pub ui: &'a mut egui::Ui,
    pub data: &'a mut TableState,
    pub provider: &'b dyn TableProvider,
}

#[derive(Debug, Default)]
pub struct TableOperations(pub Vec<Vec<Box<dyn TableOperation>>>);

impl TableOperations {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_group(mut self, group: Vec<Box<dyn TableOperation>>) -> Self {
        self.0.push(group);
        self
    }

    #[must_use]
    pub fn with_operation(mut self, op: impl TableOperation + 'static) -> Self {
        if let Some(group) = self.0.last_mut() {
            group.push(Box::new(op));
        } else {
            self.0.push(vec![Box::new(op)]);
        }
        self
    }

    pub fn gui(
        &mut self,
        ui: &mut egui::Ui,
        provider: &dyn TableProvider,
        data: &mut TableState,
        context_menu: bool,
    ) -> Result<bool, TableError> {
        let mut refresh = false;
        let mut any_clicked = false;

        for op_group in &mut self.0 {
            for op in op_group {
                let is_pending = op.is_pending();
                if op.just_completed() && op.refresh_on_completion() {
                    refresh = true;
                }

                if op.pollable() {
                    op.poll(ui, data)?;
                }

                let (enabled, reason): (bool, &'static str) = if is_pending {
                    (false, "Operation pending...")
                } else {
                    match op.enabled() {
                        TableOperationEnablement::Always => (true, ""),
                        TableOperationEnablement::AtLeastOneSelected => {
                            (!data.selected_rows.is_empty(), "At least one row required")
                        }
                        TableOperationEnablement::OneSelected => {
                            (data.selected_rows.len() == 1, "Exactly one row required")
                        }
                        TableOperationEnablement::AtLeastOneFiltered => (
                            !data.active_rows.is_empty(),
                            "At least one filtered row required",
                        ),
                    }
                };

                if !context_menu {
                    op.extra_ui(ui, data)?;
                }

                ui.add_enabled_ui(enabled, |ui| {
                    let mut button = ui
                        .button(op.get_name(context_menu).as_ref())
                        .on_hover_text(op.name());

                    // Only format and allocate the error message string when disabled
                    if !enabled {
                        button = button.on_disabled_hover_text(format!("{}\n{reason}", op.name()));
                    }

                    if button.clicked() {
                        any_clicked = true;
                        let mut ctx = OperationContext { ui, data, provider };
                        op.exec(&mut ctx)
                    } else {
                        Ok(())
                    }
                })
                .inner?;
            }
            ui.separator();
        }

        if any_clicked && context_menu {
            ui.close_kind(egui::UiKind::Menu);
        }

        Ok(refresh)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TableOperationEnablement {
    #[default]
    Always,
    AtLeastOneFiltered,
    AtLeastOneSelected,
    OneSelected,
}

pub trait TableOperation: std::any::Any + std::fmt::Debug + Send + Sync {
    fn name(&self) -> Cow<'_, str>;
    fn icon(&self) -> &'static str {
        "X"
    }
    fn get_name(&self, full: bool) -> Cow<'_, str> {
        if full {
            Cow::Owned(format!("{} {}", self.name(), self.icon()))
        } else {
            Cow::Borrowed(self.icon())
        }
    }
    fn refresh_on_completion(&self) -> bool {
        false
    }
    fn pollable(&self) -> bool {
        false
    }
    fn is_first_page(&self) -> bool {
        true
    }
    fn is_last_page(&self) -> bool {
        true
    }
    fn enabled(&self) -> TableOperationEnablement;
    fn exec(&mut self, ctx: &mut OperationContext<'_, '_>) -> Result<(), TableError>;
    fn extra_ui(&mut self, _ui: &mut egui::Ui, _data: &mut TableState) -> Result<(), TableError> {
        Ok(())
    }
    fn is_pending(&mut self) -> bool {
        false
    }
    fn just_completed(&mut self) -> bool {
        false
    }
    /// Routine tick loop, natively fired if `pollable()` evaluates to true.
    fn poll(&mut self, _ui: &mut egui::Ui, _data: &mut TableState) -> Result<(), TableError> {
        Ok(())
    }
    fn consume(&mut self) -> Result<(), TableError> {
        Ok(())
    }
    fn error(&self) -> Option<&str> {
        None
    }
    fn clear_error(&mut self) {}
    fn is_modal_open(&self) -> bool {
        false
    }
    fn set_modal_open(&mut self, _open: bool) {}
    fn new() -> Self
    where
        Self: Sized;
    fn reset(&mut self)
    where
        Self: Sized,
    {
        *self = Self::new();
    }

    fn pollable_modal(
        &mut self,
        ui: &mut egui::Ui,
        centered: bool,
        action: Cow<'_, str>,
        action_progressive: Cow<'_, str>,
        input_ui: impl FnOnce(&mut egui::Ui, &mut Self) -> Result<(), TableError>,
    ) -> Result<(), TableError>
    where
        Self: Sized,
    {
        if self.is_modal_open() {
            egui::Modal::new(ui.id().with("pollable_modal"))
                .show(ui.ctx(), |ui| {
                    ui.scope_builder(
                        egui::UiBuilder::new().layout(egui::Layout::top_down(if centered {
                            egui::Align::Center
                        } else {
                            egui::Align::Min
                        })),
                        |ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            ui.heading(
                                egui::RichText::new(format!("{} {}", self.name(), self.icon()))
                                    .strong(),
                            );
                            ui.separator();
                            ui.spacing_mut().item_spacing.y = 5.0;

                            if self.just_completed() && self.error().is_none() {
                                self.reset();
                                return Ok(());
                            }

                            let is_pending = self.is_pending();
                            ui.add_enabled_ui(!is_pending, |ui| input_ui(ui, self))
                                .inner?;
                            ui.add_space(10.0);

                            if let Some(error) = self.error() {
                                ui.colored_label(egui::Color32::RED, "Error");
                                ui.colored_label(egui::Color32::RED, error);
                            }

                            if is_pending {
                                ui.label(action_progressive);
                                ui.add_space(5.0);
                                ui.spinner();
                            } else {
                                if self.is_last_page() {
                                    let is_allowed = self.poll_allow_execution();
                                    if ui
                                        .add_enabled(is_allowed, egui::Button::new(action))
                                        .clicked()
                                    {
                                        self.clear_error();
                                        self.consume()?;
                                    }
                                }
                                if self.is_first_page() && ui.button("Cancel").clicked() {
                                    self.reset();
                                }
                            }
                            Ok(())
                        },
                    )
                    .inner
                })
                .inner
        } else {
            Ok(())
        }
    }

    fn polled_modal(
        &mut self,
        ui: &mut egui::Ui,
        heading: Cow<'_, str>,
        action_progressive: Cow<'_, str>,
        input_ui: impl FnOnce(&mut egui::Ui, &mut Self) -> Result<(), TableError>,
    ) -> Result<(), TableError>
    where
        Self: Sized,
    {
        if self.is_modal_open() {
            egui::Modal::new(ui.id().with("polled_modal"))
                .show(ui.ctx(), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.heading(heading);
                        ui.separator();
                        ui.spacing_mut().item_spacing.y = 5.0;

                        if self.is_pending() {
                            ui.label(action_progressive);
                            ui.add_space(5.0);
                            ui.spinner();
                        } else if let Some(error) = self.error() {
                            ui.colored_label(egui::Color32::RED, "Error");
                            ui.colored_label(egui::Color32::RED, error);
                        } else {
                            input_ui(ui, self)?;
                        }

                        ui.add_space(10.0);
                        if ui.button("Close").clicked() {
                            self.reset();
                        }
                        Ok::<_, TableError>(())
                    })
                })
                .inner
                .inner?;
        }
        Ok(())
    }

    fn poll_allow_execution(&self) -> bool {
        true
    }
}

// Default Operations

#[derive(Debug, Default)]
pub struct CopyRows {
    pub prioritize_hovers: bool,
}

impl TableOperation for CopyRows {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self::default()
    }
    fn name(&self) -> Cow<'_, str> {
        if self.prioritize_hovers {
            Cow::Borrowed("Copy hovered rows")
        } else {
            Cow::Borrowed("Copy rows")
        }
    }
    fn icon(&self) -> &'static str {
        if self.prioritize_hovers {
            "📁"
        } else {
            "📋"
        }
    }
    fn enabled(&self) -> TableOperationEnablement {
        TableOperationEnablement::AtLeastOneSelected
    }
    fn exec(&mut self, ctx: &mut OperationContext<'_, '_>) -> Result<(), TableError> {
        // Pre-allocate a default chunk size to minimize system allocator pressure
        let mut output = String::with_capacity(2048);

        ctx.provider.for_selected_rows(ctx.data, &mut |row| {
            if !output.is_empty() {
                output.push('\n');
            }
            for (i, (val, hover)) in row.iter().enumerate() {
                if i > 0 {
                    output.push(',');
                }
                let cell_text = if self.prioritize_hovers {
                    hover.as_deref().unwrap_or(val)
                } else {
                    val
                };
                output.push_str(cell_text);
            }
            Ok(())
        })?;

        ctx.ui.ctx().copy_text(output);
        Ok(())
    }
    fn just_completed(&mut self) -> bool {
        true
    }
}

#[derive(Debug, Default)]
pub struct CopyHeadersRows {
    pub prioritize_hovers: bool,
}

impl TableOperation for CopyHeadersRows {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self::default()
    }
    fn name(&self) -> Cow<'_, str> {
        if self.prioritize_hovers {
            Cow::Borrowed("Copy hovered rows with headers")
        } else {
            Cow::Borrowed("Copy rows with headers")
        }
    }
    fn icon(&self) -> &'static str {
        if self.prioritize_hovers {
            "🗄"
        } else {
            "📜"
        }
    }
    fn enabled(&self) -> TableOperationEnablement {
        TableOperationEnablement::AtLeastOneSelected
    }

    fn exec(&mut self, ctx: &mut OperationContext<'_, '_>) -> Result<(), TableError> {
        let headers = ctx.provider.headers();

        // Pre-allocate a reasonable capacity for headers and initial rows
        let mut output = String::with_capacity(2048);

        // 1. Write the headers directly into the buffer (replacing headers.join(","))
        for (i, header) in headers.iter().enumerate() {
            if i > 0 {
                output.push(',');
            }
            output.push_str(header);
        }

        // 2. Stream the selected rows sequentially into the same buffer
        ctx.provider.for_selected_rows(ctx.data, &mut |row| {
            output.push('\n');
            for (i, (val, hover)) in row.iter().enumerate() {
                if i > 0 {
                    output.push(',');
                }
                let cell_text = if self.prioritize_hovers {
                    hover.as_deref().unwrap_or(val)
                } else {
                    val
                };
                output.push_str(cell_text);
            }
            Ok(())
        })?;

        // 3. Send the single allocated string to the clip board
        ctx.ui.ctx().copy_text(output);
        Ok(())
    }

    fn just_completed(&mut self) -> bool {
        true
    }
}

#[derive(Debug, Default)]
pub struct FilterSelectAll;

impl TableOperation for FilterSelectAll {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self
    }
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("Select filtered")
    }
    fn icon(&self) -> &'static str {
        "☑"
    }
    fn enabled(&self) -> TableOperationEnablement {
        TableOperationEnablement::Always
    }
    fn exec(&mut self, ctx: &mut OperationContext<'_, '_>) -> Result<(), TableError> {
        let active_u32_iter = ctx.data.active_rows.iter().map(|&row| row as u32);
        ctx.data.selected_rows.extend(active_u32_iter);
        Ok(())
    }
    fn just_completed(&mut self) -> bool {
        true
    }
}

#[derive(Debug, Default)]
pub struct FilterDeSelectAll;

impl TableOperation for FilterDeSelectAll {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self
    }
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("Deselect filtered")
    }
    fn icon(&self) -> &'static str {
        "❎"
    }
    fn enabled(&self) -> TableOperationEnablement {
        TableOperationEnablement::Always
    }
    fn exec(&mut self, ctx: &mut OperationContext<'_, '_>) -> Result<(), TableError> {
        ctx.data.active_rows.iter().for_each(|row| {
            ctx.data.selected_rows.remove(*row as u32);
        });
        Ok(())
    }
    fn just_completed(&mut self) -> bool {
        true
    }
}

#[derive(Debug, Default)]
pub struct SelectAll;

impl TableOperation for SelectAll {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self
    }
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("Select all")
    }
    fn icon(&self) -> &'static str {
        "✔"
    }
    fn enabled(&self) -> TableOperationEnablement {
        TableOperationEnablement::Always
    }
    fn exec(&mut self, ctx: &mut OperationContext<'_, '_>) -> Result<(), TableError> {
        ctx.data.selected_rows.clear();
        ctx.data
            .selected_rows
            .insert_range(0..ctx.provider.row_count() as u32);
        Ok(())
    }
    fn just_completed(&mut self) -> bool {
        true
    }
}

#[derive(Debug, Default)]
pub struct DeSelectAll;

impl TableOperation for DeSelectAll {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self
    }
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("Deselect all")
    }
    fn icon(&self) -> &'static str {
        "❌"
    }
    fn enabled(&self) -> TableOperationEnablement {
        TableOperationEnablement::Always
    }
    fn exec(&mut self, ctx: &mut OperationContext<'_, '_>) -> Result<(), TableError> {
        ctx.data.selected_rows.clear();
        Ok(())
    }
    fn just_completed(&mut self) -> bool {
        true
    }
}
