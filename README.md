# egui-table-kit

`egui-table-kit` is an extension library for [`egui`](https://github.com/emilk/egui) and [`egui_extras`](https://docs.rs/egui_extras) designed to simplify implementing table state management, high-performance interactions, and custom action workflows. It extends `egui_extras::TableBuilder` by providing unified filtering, sorting, row color tagging, hierarchical tree structures, and multi-row selection patterns.

---

## Features

- **Text Search & Regex Filtering**: Advanced per-column text filters supporting raw literal substrings, case insensitivity, and regular expressions with integrated error feedback for invalid regex patterns.
- **Color Highlighting & Tagging**: Categorize rows into up to 10 color-based highlight groups (backed by compressed bitmaps) and filter rows matching specific colors.
- **Hierarchical Tree Support**: Native rendering of nested trees, custom guideline drawing with dashed line segments, parent-child expansion/collapse toggle buttons, and cached subtree flattening.
- **Optimized Selection Storage**: Selection state tracking utilizing `roaring` bitmaps for efficient range queries and a small memory footprint at any scale (the bitmaps hold rows `0..2^32`; see [Lazy tables](#lazy-tables)).
- **Lazy Tables**: Billions of rows that aren't in memory, loaded as the user scrolls: the provider is told what each frame draws, cells can be loading or failed, and nothing walks every row.
- **Convenience Extractors (`RowSliceExt`)**: Simple helper trait on raw row data to parse, extract, or fall back between primary cell text and optional alternate/hover text.
- **Action Dispatch Framework (`TableOperations`)**: Build toolbar button groups or context menus bound to row selection limits. Built-in support for long-running operations via poll loops, modal dialogues, progress spinners, and operation errors.
- **Default Actions**: Shipped out-of-the-box with operations for select all, deselect all, filter-based selections, and clipboard-copy utilities (with and without header data).

---

## Installation

Add the following to your `Cargo.toml`:

```toml
[dependencies]
egui = "0.36"
egui_extras = { version = "0.36", default-features = false }
egui-table-kit = "0.8"
```

---

## Core Concepts

### 1. `TableProvider`

Implement the `TableProvider` trait to adapt your dataset for the table view. This trait handles headers, total row counts, streaming over selections, custom sorting fallback implementations, and hierarchical tree navigation.

```rust
pub trait TableProvider {
    fn column_count(&self) -> usize;
    fn header(&self, index: usize) -> Option<std::borrow::Cow<'_, str>>;
    fn row_count(&self) -> usize;

    /// Stream selected rows sequentially to a captured closure.
    fn for_selected_rows(
        &self,
        state: &TableState,
        f: &mut RowCallback<'_>,
    ) -> Result<(), TableError>;

    /// Stream all rows sequentially to a captured closure.
    fn for_all_rows(&self, f: &mut RowCallback<'_>) -> Result<(), TableError>;

    /// Sort the active row indices by a column. Contains a default string-based fallback.
    fn sort_active_rows(
        &self,
        active_rows: &mut Vec<usize>,
        col_index: usize,
        ascending: bool,
    ) -> Result<(), TableError>;

    /// Sequential fallback filtering. Override for custom or parallel filtering (e.g., using Rayon).
    fn filter_rows(
        &self,
        state: &TableState,
        filters: &[(usize, Filter)],
    ) -> Result<Vec<usize>, TableError>;

    // --- Lazy Table Methods (see "Lazy tables" below) ---

    /// Called once per show pass, before any cell is asked for, with what is drawn.
    fn prepare(&self, _view: &Viewport<'_>) {}

    /// Whether a cell's value can be drawn yet (only ready cells are asked for).
    fn cell_status(&self, _row: usize, _col: usize) -> CellStatus<'_> { CellStatus::Ready }

    /// Whether the rows aren't all in memory.
    fn is_lazy(&self) -> bool { false }

    /// Whether a column can be sorted or filtered (its controls are hidden if not).
    fn can_sort(&self, _col: usize) -> bool { true }
    fn can_filter(&self, _col: usize) -> bool { true }

    /// A description shown while the pointer rests on a column's header (asked only then).
    fn header_tooltip(&self, _col: usize) -> Option<std::borrow::Cow<'_, str>> { None }

    // --- Hierarchical Tree Table Methods ---

    /// Returns the tree structural context for a row. Defaults to None (flat table).
    fn row_hierarchy(&self, _state: &TableState, _row_index: usize) -> Option<RowHierarchy> { None }

    /// Returns whether this table displays nested tree structures.
    fn is_tree(&self) -> bool { false }

    /// Gets the immediate parent index of a row.
    fn row_parent(&self, _row_index: usize) -> Option<usize> { None }

    /// Gets the immediate child indices of a parent row.
    fn row_children(&self, _row_index: usize) -> Vec<usize> { Vec::new() }

    /// Checks whether an individual row matches active filtering requirements.
    fn row_matches(
        &self,
        _state: &TableState,
        _row_index: usize,
        _filters: &[(usize, Filter)],
        _highlight: Option<u8>,
    ) -> bool { true }
}
```

### 2. `TableState`

`TableState` stores active runtime view settings for your table. It maintains:

- The persistent table ID.
- Dynamic column states (hover highlights, active search criteria, and active sort orientations).
- The rows shown, in order (`active_rows`, an `ActiveRows`): `ActiveRows::All(n)` while every row is shown in the provider's order (no list, however many rows), or `ActiveRows::Rows(list)` once filtered, sorted, or flattened from a tree.
- Selected and expanded nodes (powered by `RoaringBitmap`).
- Tree guidelines and expansion toggle state indicators.
- Caches for sibling sorting to minimize CPU overhead on frame redraws.

### 3. `TableOperations`

Compose context menus or top-aligned toolbar panels. By grouping actions together, `TableOperations::gui` handles enabling/disabling triggers, generating keyboard shortcuts, displaying state-dependent menu prompts, or spawning modal workflows.

```rust
use egui_table_kit::operations::{TableOperations, SelectAll, DeSelectAll, CopyRows};

let operations = TableOperations::new()
    .with_group(vec![
        Box::new(SelectAll::new()),
        Box::new(DeSelectAll::new()),
    ])
    .with_group(vec![
        Box::new(CopyRows::new()),
    ]);
```

You can define custom operations by implementing the `TableOperation` trait:

```rust
pub trait TableOperation: std::any::Any + std::fmt::Debug + Send + Sync {
    fn name(&self) -> Cow<'_, str>;
    fn icon(&self) -> &'static str { "X" }
    fn enabled(&self) -> TableOperationEnablement;
    fn exec(&mut self, ctx: &mut OperationContext<'_, '_>) -> Result<(), TableError>;

    // Optional modal UI and background polling hooks:
    fn pollable(&self) -> bool { false }
    fn poll(&mut self, _ui: &mut egui::Ui, _data: &mut TableState) -> Result<(), TableError> { Ok(()) }
    fn is_pending(&mut self) -> bool { false }
    fn just_completed(&mut self) -> bool { false }
    fn error(&self) -> Option<&str> { None }
    fn clear_error(&mut self) {}
}
```

---

## Example Usage

The following example is a simplified version of `examples/minimal.rs`, illustrating how to define a data provider, construct your state, render headers using the `HeaderTrait`, and process filtering/sorting events.

```rust
use std::borrow::Cow;

use eframe::egui;
use egui_table_kit::{
    error::TableError,
    header::HeaderTrait,
    operations::{CopyRows, DeSelectAll, HeaderIter, Row, RowCallback, SelectAll, TableCell, TableOperations, TableProvider},
    state::TableState,
};

struct SimpleDataset {
    headers: Vec<&'static str>,
    records: Vec<Vec<String>>,
}

struct SimpleRow<'a> {
    record: &'a [String],
}

impl<'a> Row for SimpleRow<'a> {
    fn cell(&self, col_index: usize) -> Option<TableCell<'_>> {
        self.record.get(col_index).map(|s| (Cow::Borrowed(s.as_str()), None))
    }
    fn column_count(&self) -> usize { self.record.len() }
}

impl TableProvider for SimpleDataset {
    fn column_count(&self) -> usize { self.headers.len() }
    fn header(&self, index: usize) -> Option<Cow<'_, str>> {
        self.headers.get(index).map(|&s| Cow::Borrowed(s))
    }
    fn headers(&self) -> HeaderIter<'_> { HeaderIter::new(self) }
    fn row_count(&self) -> usize { self.records.len() }

    fn for_all_rows(&self, f: &mut RowCallback<'_>) -> Result<(), TableError> {
        for record in &self.records {
            f(&SimpleRow { record })?;
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
                f(&SimpleRow { record })?;
            }
        }
        Ok(())
    }
}

struct DemoApp {
    provider: SimpleDataset,
    state: TableState,
    operations: TableOperations,
}

impl Default for DemoApp {
    fn default() -> Self {
        let provider = SimpleDataset {
            headers: vec!["Name", "Role", "Department"],
            records: vec![
                vec!["Alice".into(), "Engineer".into(), "Platform".into()],
                vec!["Bob".into(), "Designer".into(), "Product".into()],
            ],
        };
        let row_count = provider.row_count();
        let state = TableState::new("demo_table_unique_id", row_count);
        let operations = TableOperations::new()
            .with_group(vec![
                Box::new(SelectAll::new()),
                Box::new(DeSelectAll::new()),
                Box::new(CopyRows::new()),
            ]);

        Self { provider, state, operations }
    }
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Render action toolbar
            ui.horizontal(|ui| {
                let _ = self.operations.gui(ui, &self.provider, &mut self.state, false);
            });

            ui.add_space(5.0);
            ui.label(self.state.counts_header(self.provider.row_count()));

            // Setup the egui_extras TableBuilder
            let builder = egui_extras::TableBuilder::new(ui)
                .resizable(true)
                .sense(egui::Sense::click())
                .column(egui_extras::Column::initial(150.0))
                .column(egui_extras::Column::initial(150.0))
                .column(egui_extras::Column::remainder());

            // Build table header with built-in filter and sort support
            if let Ok((responses, table)) = builder.archived_headers(
                &self.state,
                self.provider.headers(),
                22.0, // Header height
                &[],  // Highlight palette Row 1
                &[],  // Highlight palette Row 2
            ) {
                // Update table state filters/sorting with responses from user clicks
                let _ = self.state.process_responses(&self.provider, responses);

                // Run active column filters on flat dataset
                let filter_state = self.state.get_filter_state();
                let _ = self.state.apply_all_filters(&self.provider, &filter_state);

                // Apply active column sorting
                if let Some((sort_col, sort_up)) = self.state.get_sort_state() {
                    let _ = self.provider.sort_active_rows(
                        self.state.active_rows.to_mut(),
                        sort_col,
                        sort_up,
                    );
                }

                let active_rows = self.state.active_rows.clone();

                table.body(|mut body| {
                    for row_idx in &active_rows {
                        let is_selected = self.state.is_selected(row_idx);
                        body.row(18.0, |mut row| {
                            for col_idx in 0..self.provider.column_count() {
                                row.col(|ui| {
                                    let rect = ui.max_rect().expand2(0.5 * ui.spacing().item_spacing);
                                    let response = ui.interact(rect, ui.id().with((row_idx, col_idx)), egui::Sense::click());

                                    if response.clicked() {
                                        self.state.handle_row_selection(ui.input(|i| i.modifiers), row_idx);
                                    }

                                    if is_selected {
                                        ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, ui.visuals().selection.bg_fill);
                                    }

                                    let cell_val = &self.provider.records[row_idx][col_idx];
                                    ui.add(egui::Label::new(cell_val).selectable(false));
                                });
                            }
                        });
                    }
                });
            }
        });
    }
}
```

---

## Lazy tables

A provider whose rows aren't all in memory (a Parquet file of billions of rows read as the user scrolls) renders through `TableKit` as any other. Only the rows and columns in view are touched:

- **No per-row state.** `TableState::new(id, row_count)` holds every row as `ActiveRows::All(row_count)`, without a list. Filtering, sorting, and flattening a tree make a list (`ActiveRows::Rows`); clearing the filters and the sort of a flat table returns to `All`. Read it with `get`, `position` (immediate for `All`), `iter`, and `iter_range`; take a list to change with `to_mut()`, which makes one from `All`:

  ```rust
  provider.sort_active_rows(state.active_rows.to_mut(), column, ascending)?;
  ```

- **Say what is drawn.** `TableProvider::prepare(&Viewport)` is called once per show pass, before any cell is asked for, with the visible indices (`visible`), the provider rows at them (`rows`: `VisibleRows::Identity(range)` or `VisibleRows::Mapped(&[usize])`; `span()` gives the lowest and highest), and the provider columns drawn (`columns`: sticky ones, then the visible range, through the column map). Start loading there.
- **Draw what isn't there yet.** `TableProvider::cell_status(row, col)` returns `CellStatus::Ready`, `Loading`, or `Failed(message)`. Only ready cells are asked for with `cell_at` (and given to a custom cell renderer). A loading cell is drawn as a faint bar, a failed one as a short error with the message on hover; both are labelled for screen readers, and their rows are selected and clicked like any other. The table doesn't repaint on its own while cells load: call `Context::request_repaint` when data arrives.
- **Never walk every row.** `TableProvider::is_lazy() -> true` limits the first frame's full sizing pass (columns fitted to their contents) to the visible rows, at most `LAZY_SIZING_ROWS` (256; `TableKit::with_max_sizing_rows` or `layout::Table::max_sizing_rows` set it explicitly). Columns that can't be sorted or filtered without reading every row return `false` from `can_sort`/`can_filter`: the header hides those controls and the table never sorts or filters by them. A lazy provider's rows aren't searched for a row's key either (keyboard focus falls back to the first row instead).
- **Pin and describe columns.** `TableKit::with_sticky_columns(n)` keeps the first `n` columns (after the column map) in place while the others scroll sideways, for a row-number gutter; `prepare` lists them first. `TableProvider::header_tooltip` describes a column (its type, sizes, statistics) while the pointer rests on its header, and is asked only then.
- **Rows past 2^32.** Selection, expansion, highlights, and a tree's filter matches are `RoaringBitmap`s of `u32`, so they hold rows `0..2^32` (`state::BITMAP_ROWS`). A row past them is never selected, highlighted, or expanded (selecting it does nothing; Select All selects the first 2^32) and is never taken for another row: every bitmap access goes through `state::row_bit(row) -> Option<u32>`. `TableState::is_selected(row)` and `is_expanded(row)` answer for any row.

`examples/lazy.rs` shows a billion rows of sixty columns served in 1,024-row blocks by a loader thread (`cargo run --release --example lazy`).

Upgrading from 0.7: `TableState::active_rows` is an `ActiveRows`, not a `Vec<usize>`. Where a `&mut Vec<usize>` was taken (to sort it), use `state.active_rows.to_mut()`; `active_rows.get(i)` returns `Option<usize>`; build one from a list with `vec![…].into()`. The provider's new methods all have defaults. `TableProvider::filter_rows` is no longer called with an empty filter list: every row is then shown in order.

---

## Technical Details

### 1. Guideline dashed rendering in Tree views

When configuring hierarchical tree tables, `TableState::show_tree_cell` draws guidelines linking nested child entities to parents. Calculations avoid allocating extra buffers by evaluating coordinates on-the-fly and issuing direct draw calls to the active painter.

### 2. Double-text fallback with `RowSliceExt`

`dyn Row + '_` yields `TableCell<'_>` containing `(Cow<'_, str>, Option<Cow<'_, str>>)`. The first string represents the primary display value, while the second serves as an alternate or hover string. The `RowSliceExt` trait enables parsing either source value:

- `get_primary(col)` / `get_hover(col)`: Retrieve raw string values.
- `parse_primary::<T>(col)` / `parse_hover::<T>(col)`: Attempt to parse cellular strings into specific typed values (e.g. `f32`, `DateTime`, etc.) with generic error propagation.
