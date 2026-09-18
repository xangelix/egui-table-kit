//! Stable row events and opt-in file-explorer selection. No application payload types are required.

use ahash::AHashMap;
use egui::{Id, Modifiers, Rect, Response};

use crate::error::TableError;
use crate::operations::TableProvider;
use crate::state::TableState;

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
                    .map(|&r| provider.row_key(r))
            }),
        }
    }

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
            .map(|r| r as u32)
            .collect();
        self.expanded_rows = snapshot
            .expanded
            .iter()
            .filter_map(|key| indices.get(key))
            .map(|&r| r as u32)
            .collect();
        self.focused_key = snapshot.focused.filter(|key| indices.contains_key(key));
        let anchor_row = snapshot.anchor.and_then(|key| indices.get(&key).copied());
        self.anchor_key = snapshot.anchor.filter(|key| indices.contains_key(key));

        self.filter_cache_dirty = true;
        self.sorted_children_cache.clear();
        self.refresh_view(provider)?;

        self.last_clicked_visible_index = anchor_row.and_then(|row| {
            if row < self.active_rows.len() && self.active_rows[row] == row {
                Some(row)
            } else {
                self.active_rows.iter().position(|&r| r == row)
            }
        });

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

    pub fn select_row_at_visible(
        &mut self,
        provider: &dyn TableProvider,
        row: usize,
        visible_index: Option<usize>,
        modifiers: Modifiers,
        input: SelectionInput,
    ) {
        if row >= provider.row_count() || !provider.row_selectable(row) {
            return;
        }

        let key = provider.row_key(row);
        self.focused_key = Some(key);

        let selected = self.selected_rows.contains(row as u32);
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
            .filter(|&idx| idx < self.active_rows.len() && self.active_rows[idx] == row)
            .or_else(|| {
                if row < self.active_rows.len() && self.active_rows[row] == row {
                    Some(row)
                } else {
                    self.active_rows.iter().position(|&r| r == row)
                }
            });
        let toggle = modifiers.command || modifiers.ctrl;

        if matches!(input, SelectionInput::Context) {
            self.selected_rows.clear();
            self.selected_rows.insert(row as u32);
            self.anchor_key = Some(key);
            self.last_clicked_visible_index = current;
        } else if modifiers.shift {
            let anchor = self.anchor_key.and_then(|key| {
                if let Some(idx) = self.last_clicked_visible_index
                    && idx < self.active_rows.len()
                    && provider.row_key(self.active_rows[idx]) == key
                {
                    Some(idx)
                } else {
                    self.active_rows
                        .iter()
                        .position(|&r| provider.row_key(r) == key)
                }
            });

            if !toggle {
                self.selected_rows.clear();
            }

            if let (Some(anchor), Some(current)) = (anchor, current) {
                for &row in &self.active_rows[anchor.min(current)..=anchor.max(current)] {
                    if provider.row_selectable(row) {
                        self.selected_rows.insert(row as u32);
                    }
                }
                self.last_clicked_visible_index = Some(anchor);
            } else {
                self.selected_rows.insert(row as u32);
                self.anchor_key = Some(key);
                self.last_clicked_visible_index = current;
            }
        } else if toggle && matches!(input, SelectionInput::Keyboard) {
            // Ctrl/Cmd+arrows move the focus without changing the selection.
        } else if toggle {
            if selected {
                self.selected_rows.remove(row as u32);
            } else {
                self.selected_rows.insert(row as u32);
            }
            self.anchor_key = Some(key);
            self.last_clicked_visible_index = current;
        } else {
            self.selected_rows.clear();
            self.selected_rows.insert(row as u32);
            self.anchor_key = Some(key);
            self.last_clicked_visible_index = current;
        }
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

        let current_vis_idx = self.focused_key.and_then(|key| {
            if let Some(last) = self.last_clicked_visible_index
                && last < self.active_rows.len()
                && provider.row_key(self.active_rows[last]) == key
            {
                Some(last)
            } else {
                self.active_rows
                    .iter()
                    .position(|&r| provider.row_key(r) == key)
            }
        });

        let target_vis_idx = match delta {
            isize::MIN => self
                .active_rows
                .iter()
                .position(|&r| provider.row_selectable(r)),
            isize::MAX => self
                .active_rows
                .iter()
                .rposition(|&r| provider.row_selectable(r)),
            0 => current_vis_idx,
            _ => {
                let start = match current_vis_idx {
                    Some(cur) => cur,
                    None => {
                        let pos = if delta < 0 {
                            self.active_rows
                                .iter()
                                .rposition(|&r| provider.row_selectable(r))?
                        } else {
                            self.active_rows
                                .iter()
                                .position(|&r| provider.row_selectable(r))?
                        };
                        let row = self.active_rows[pos];
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
                        self.active_rows[target + 1..]
                            .iter()
                            .position(|&r| provider.row_selectable(r))
                            .map(|rel| target + 1 + rel)
                    } else {
                        self.active_rows[..target]
                            .iter()
                            .rposition(|&r| provider.row_selectable(r))
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
        let row = self.active_rows[target_vis_idx];
        self.select_row_at_visible(
            provider,
            row,
            Some(target_vis_idx),
            modifiers,
            SelectionInput::Keyboard,
        );
        Some(target_vis_idx)
    }

    pub fn select_all_visible(&mut self, provider: &dyn TableProvider) {
        self.selected_rows = self
            .active_rows
            .iter()
            .copied()
            .filter(|&r| provider.row_selectable(r))
            .map(|r| r as u32)
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
