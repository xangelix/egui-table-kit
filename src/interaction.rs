//! Stable row events and opt-in file-explorer selection. No application payload types are required.

use ahash::AHashMap;
use egui::{Id, Modifiers, Rect, Response};

use crate::error::TableError;
use crate::operations::TableProvider;
use crate::state::{ActiveRows, TableState, row_bit, rows_with_bits};

/// Whether `row` can be selected: the provider allows it, and the bitmaps hold it
/// (see [`crate::state::BITMAP_ROWS`]).
fn selectable(provider: &dyn TableProvider, row: usize) -> bool {
    row_bit(row).is_some() && provider.row_selectable(row)
}

/// The visible indices whose rows could be selectable: all of them for a list, the ones the
/// bitmaps hold for every row in order (whose visible indices are their rows).
fn selectable_span(rows: &ActiveRows) -> std::ops::Range<usize> {
    match rows {
        ActiveRows::All(count) => 0..rows_with_bits(*count),
        ActiveRows::Rows(rows) => 0..rows.len(),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionMode {
    /// Retains the original toggle-on-second-click and additive Shift behavior.
    #[default]
    Legacy,
    Explorer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowEventKind {
    Clicked,
    Activated,
    SecondaryClicked,
    ExpansionChanged(bool),
    DragStarted,
    DragStopped,
}

#[derive(Clone, Debug)]
pub struct RowEvent {
    pub key: Id,
    /// Only valid for the provider snapshot used by this rendering call.
    pub row_index: usize,
    pub kind: RowEventKind,
    pub modifiers: Modifiers,
    /// Snapshot of all selected keys at drag start, including collapsed/filtered rows.
    pub dragged_keys: Vec<Id>,
}

#[derive(Clone, Debug)]
pub struct RowGeometry {
    pub key: Id,
    pub row_index: usize,
    pub visible_index: usize,
    /// Union of visible cell rectangles, clipped to the viewport.
    pub rect: Rect,
    /// Hit regions excluding tree expanders and column resize handles.
    pub hit_regions: Vec<Rect>,
}

#[derive(Debug)]
pub struct TableOutput {
    pub response: Response,
    pub events: Vec<RowEvent>,
    pub rows: Vec<RowGeometry>,
}

/// Capture before replacing a provider snapshot, then restore against the new provider.
#[derive(Clone, Debug, Default)]
pub struct IdentityState {
    pub selected: Vec<Id>,
    pub expanded: Vec<Id>,
    pub focused: Option<Id>,
    pub anchor: Option<Id>,
}

#[derive(Clone, Copy, Debug)]
pub enum SelectionInput {
    Click,
    Context,
    Drag,
    Keyboard,
}

impl TableState {
    pub fn capture_identity(&self, provider: &dyn TableProvider) -> IdentityState {
        let mut selected = Vec::with_capacity(self.selected_rows.len() as usize);
        selected.extend(
            self.selected_rows
                .iter()
                .map(|r| r as usize)
                .filter(|&r| r < provider.row_count())
                .map(|r| provider.row_key(r)),
        );

        let mut expanded = Vec::with_capacity(self.expanded_rows.len() as usize);
        expanded.extend(
            self.expanded_rows
                .iter()
                .map(|r| r as usize)
                .filter(|&r| r < provider.row_count())
                .map(|r| provider.row_key(r)),
        );

        IdentityState {
            selected,
            expanded,
            focused: self.focused_key,
            anchor: self.anchor_key.or_else(|| {
                self.last_clicked_visible_index
                    .and_then(|v| self.active_rows.get(v))
                    .map(|r| provider.row_key(r))
            }),
        }
    }

    /// Restores a [`Self::capture_identity`] snapshot against `provider`. It asks every row's
    /// key once, so it walks all of the provider's rows.
    pub fn restore_identity(
        &mut self,
        provider: &dyn TableProvider,
        snapshot: &IdentityState,
    ) -> Result<(), TableError> {
        let row_count = provider.row_count();
        let mut indices = AHashMap::with_capacity(row_count);
        for row in 0..row_count {
            if indices.insert(provider.row_key(row), row).is_some() {
                return Err(TableError::CorruptedState);
            }
        }

        self.selected_rows = snapshot
            .selected
            .iter()
            .filter_map(|key| indices.get(key))
            .copied()
            .filter(|&row| provider.row_selectable(row))
            .filter_map(row_bit)
            .collect();
        self.expanded_rows = snapshot
            .expanded
            .iter()
            .filter_map(|key| indices.get(key))
            .copied()
            .filter_map(row_bit)
            .collect();
        self.focused_key = snapshot.focused.filter(|key| indices.contains_key(key));
        let focused_row = snapshot.focused.and_then(|key| indices.get(&key).copied());
        let anchor_row = snapshot.anchor.and_then(|key| indices.get(&key).copied());
        self.anchor_key = snapshot.anchor.filter(|key| indices.contains_key(key));

        self.filter_cache_dirty = true;
        self.sorted_children_cache.clear();
        self.refresh_view(provider)?;

        self.last_clicked_visible_index = anchor_row.and_then(|row| self.active_rows.position(row));
        self.focused_visible_index = focused_row.and_then(|row| self.active_rows.position(row));

        Ok(())
    }

    pub fn select_row(
        &mut self,
        provider: &dyn TableProvider,
        row: usize,
        modifiers: Modifiers,
        input: SelectionInput,
    ) {
        self.select_row_at_visible(provider, row, None, modifiers, input);
    }

    /// Selects (or focuses) `row` as `input` and `modifiers` say. A row the provider doesn't let
    /// be selected, or one past the bitmaps ([`crate::state::BITMAP_ROWS`]), is left alone.
    pub fn select_row_at_visible(
        &mut self,
        provider: &dyn TableProvider,
        row: usize,
        visible_index: Option<usize>,
        modifiers: Modifiers,
        input: SelectionInput,
    ) {
        if row >= provider.row_count() || !selectable(provider, row) {
            return;
        }
        let Some(bit) = row_bit(row) else {
            return;
        };

        let key = provider.row_key(row);
        self.focused_key = Some(key);
        self.focused_visible_index = visible_index
            .filter(|&idx| self.active_rows.get(idx) == Some(row))
            .or_else(|| {
                // Immediate without a list; a list is searched only below, where it was before.
                self.active_rows
                    .is_identity()
                    .then(|| self.active_rows.position(row))
                    .flatten()
            });

        let selected = self.selected_rows.contains(bit);
        if matches!(input, SelectionInput::Drag | SelectionInput::Context) && selected {
            return;
        }

        if self.selection_mode == SelectionMode::Legacy {
            if !matches!(input, SelectionInput::Context) {
                self.handle_row_selection_at_visible(modifiers, row, visible_index);
            }
            return;
        }

        let current = visible_index
            .filter(|&idx| self.active_rows.get(idx) == Some(row))
            .or_else(|| self.active_rows.position(row));
        let toggle = modifiers.command || modifiers.ctrl;

        if matches!(input, SelectionInput::Context) {
            self.selected_rows.clear();
            self.selected_rows.insert(bit);
            self.anchor_key = Some(key);
            self.last_clicked_visible_index = current;
        } else if modifiers.shift {
            let anchor = self
                .anchor_key
                .and_then(|key| self.locate_key(provider, key));

            if !toggle {
                self.selected_rows.clear();
            }

            if let (Some(anchor), Some(current)) = (anchor, current) {
                let span = selectable_span(&self.active_rows);
                let start = anchor.min(current);
                let end = anchor.max(current).saturating_add(1).min(span.end);
                for row in self.active_rows.iter_range(start..end) {
                    if selectable(provider, row)
                        && let Some(bit) = row_bit(row)
                    {
                        self.selected_rows.insert(bit);
                    }
                }
                self.last_clicked_visible_index = Some(anchor);
            } else {
                self.selected_rows.insert(bit);
                self.anchor_key = Some(key);
                self.last_clicked_visible_index = current;
            }
        } else if toggle && matches!(input, SelectionInput::Keyboard) {
            // Ctrl/Cmd+arrows move the focus without changing the selection.
        } else if toggle {
            if selected {
                self.selected_rows.remove(bit);
            } else {
                self.selected_rows.insert(bit);
            }
            self.anchor_key = Some(key);
            self.last_clicked_visible_index = current;
        } else {
            self.selected_rows.clear();
            self.selected_rows.insert(bit);
            self.anchor_key = Some(key);
            self.last_clicked_visible_index = current;
        }
    }

    /// Where the row with `key` is shown: at the last clicked or the focused visible index if
    /// it's there, or else found by asking each shown row's key. A lazy provider's rows aren't
    /// walked ([`TableProvider::is_lazy`]): there, a row at neither index isn't found.
    fn locate_key(&self, provider: &dyn TableProvider, key: Id) -> Option<usize> {
        for hint in [self.last_clicked_visible_index, self.focused_visible_index]
            .into_iter()
            .flatten()
        {
            if self
                .active_rows
                .get(hint)
                .is_some_and(|row| provider.row_key(row) == key)
            {
                return Some(hint);
            }
        }
        if provider.is_lazy() {
            return None;
        }
        self.active_rows
            .iter()
            .position(|row| provider.row_key(row) == key)
    }

    /// The first (or, `forward` false, the last) visible index in `visible` whose row can be
    /// selected.
    fn find_selectable(
        &self,
        provider: &dyn TableProvider,
        visible: std::ops::Range<usize>,
        forward: bool,
    ) -> Option<usize> {
        let span = selectable_span(&self.active_rows);
        let start = visible.start.max(span.start);
        let end = visible.end.min(span.end);
        if start >= end {
            return None;
        }
        let mut rows = self.active_rows.iter_range(start..end);
        let found = if forward {
            rows.position(|r| selectable(provider, r))
        } else {
            rows.rposition(|r| selectable(provider, r))
        };
        found.map(|offset| start + offset)
    }

    /// Move relative to the focused row, or to an endpoint for `isize::MIN/MAX`.
    /// Returns the visible row index for scroll-to-row integration.
    pub fn move_focus(
        &mut self,
        provider: &dyn TableProvider,
        delta: isize,
        modifiers: Modifiers,
    ) -> Option<usize> {
        if self.active_rows.is_empty() {
            return None;
        }

        let current_vis_idx = self
            .focused_key
            .and_then(|key| self.locate_key(provider, key));

        let all = 0..self.active_rows.len();
        let target_vis_idx = match delta {
            isize::MIN => self.find_selectable(provider, all, true),
            isize::MAX => self.find_selectable(provider, all, false),
            0 => current_vis_idx,
            _ => {
                let start = match current_vis_idx {
                    Some(cur) => cur,
                    None => {
                        let pos = self.find_selectable(provider, all, delta > 0)?;
                        let row = self.active_rows.get(pos)?;
                        self.select_row_at_visible(
                            provider,
                            row,
                            Some(pos),
                            modifiers,
                            SelectionInput::Keyboard,
                        );
                        return Some(pos);
                    }
                };

                let mut target = start;
                let mut remaining = delta.unsigned_abs();
                while remaining > 0 {
                    let next = if delta > 0 {
                        self.find_selectable(provider, target + 1..all.end, true)
                    } else {
                        self.find_selectable(provider, 0..target, false)
                    };

                    match next {
                        Some(pos) => {
                            target = pos;
                            remaining -= 1;
                        }
                        None => break,
                    }
                }
                Some(target)
            }
        };

        let target_vis_idx = target_vis_idx?;
        let row = self.active_rows.get(target_vis_idx)?;
        self.select_row_at_visible(
            provider,
            row,
            Some(target_vis_idx),
            modifiers,
            SelectionInput::Keyboard,
        );
        Some(target_vis_idx)
    }

    /// Selects every shown row that can be selected (the bitmaps hold rows `0..2^32`, see
    /// [`crate::state::BITMAP_ROWS`]).
    pub fn select_all_visible(&mut self, provider: &dyn TableProvider) {
        let span = selectable_span(&self.active_rows);
        self.selected_rows = self
            .active_rows
            .iter_range(span)
            .filter(|&r| selectable(provider, r))
            .filter_map(row_bit)
            .collect();
    }

    pub(crate) fn accept_event(&mut self, frame: u64, key: Id, kind: RowEventKind) -> bool {
        if self.event_frame != Some(frame) {
            self.event_frame = Some(frame);
            self.frame_events.clear();
        }
        self.frame_events.insert((key, kind))
    }
}
