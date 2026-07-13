//! Logic for constrained column auto-sizing.

use egui::Rangef;

bitflags::bitflags! {
    /// Configuration flags for a column's layout behavior.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    #[serde(transparent)]
    pub struct ColumnFlags: u8 {
        /// Can the user resize this column?
        const RESIZABLE = 1 << 0;
        /// If set, the column will be automatically sized based on the content this frame.
        const AUTO_SIZE_THIS_FRAME = 1 << 1;
        /// Set whether the column should auto-fit to content width on first load.
        const AUTO_FIT = 1 << 2;
        /// Set whether the column should absorb remaining horizontal space on layout passes.
        const REMAINDER = 1 << 3;
    }
}

#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
pub struct Column {
    pub current: f32,
    pub range: Rangef,
    pub id: Option<egui::Id>,
    pub flags: ColumnFlags,
}

impl Default for Column {
    fn default() -> Self {
        Self {
            current: 100.0,
            range: Rangef::new(4.0, f32::INFINITY),
            id: None,
            flags: ColumnFlags::RESIZABLE,
        }
    }
}

impl Column {
    /// Current and/or initial column width.
    #[must_use]
    #[inline]
    pub fn new(current: f32) -> Self {
        Self {
            current,
            ..Default::default()
        }
    }

    /// Allowed width range.
    #[must_use]
    #[inline]
    pub fn range(mut self, range: impl Into<Rangef>) -> Self {
        self.range = range.into();
        self
    }

    /// Optional unique id within the parent table.
    #[must_use]
    #[inline]
    pub const fn id(mut self, id: egui::Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Can the user resize this column?
    #[must_use]
    #[inline]
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.flags.set(ColumnFlags::RESIZABLE, resizable);
        self
    }

    /// If set, the column will be automatically sized based on the content this frame.
    #[must_use]
    #[inline]
    pub fn auto_size_this_frame(mut self, auto_size_this_frame: bool) -> Self {
        self.flags
            .set(ColumnFlags::AUTO_SIZE_THIS_FRAME, auto_size_this_frame);
        self
    }

    /// Set whether the column should auto-fit to content width on first load.
    #[must_use]
    #[inline]
    pub fn auto_fit(mut self, auto_fit: bool) -> Self {
        self.flags.set(ColumnFlags::AUTO_FIT, auto_fit);
        self
    }

    /// Set whether the column should absorb remaining horizontal space on layout passes.
    #[must_use]
    #[inline]
    pub fn remainder(mut self, remainder: bool) -> Self {
        self.flags.set(ColumnFlags::REMAINDER, remainder);
        self
    }

    /// Getter for whether the column is resizable.
    #[inline]
    #[must_use]
    pub const fn is_resizable(&self) -> bool {
        self.flags.contains(ColumnFlags::RESIZABLE)
    }

    /// Getter for whether the column should auto-size this frame.
    #[inline]
    #[must_use]
    pub const fn is_auto_size_this_frame(&self) -> bool {
        self.flags.contains(ColumnFlags::AUTO_SIZE_THIS_FRAME)
    }

    /// Getter for whether the column should auto-fit.
    #[inline]
    #[must_use]
    pub const fn is_auto_fit(&self) -> bool {
        self.flags.contains(ColumnFlags::AUTO_FIT)
    }

    /// Getter for whether the column acts as a remainder.
    #[inline]
    #[must_use]
    pub const fn is_remainder(&self) -> bool {
        self.flags.contains(ColumnFlags::REMAINDER)
    }

    #[must_use]
    #[inline]
    pub fn id_for(&self, col_idx: usize) -> egui::Id {
        self.id.unwrap_or_else(|| egui::Id::new(col_idx))
    }

    /// Resize columns to fit the total width.
    pub fn auto_size(columns: &mut [Self], target_width: f32) {
        if columns.is_empty() {
            return;
        }

        // Make sure all columns have a valid range.
        let mut max_width = 0.0;
        let mut current_width = 0.0;
        for column in columns.iter_mut() {
            column.current = column.range.clamp(column.current);
            max_width += column.range.max;
            current_width += column.current;
        }

        // If the total width of all columns already meets or exceeds the target parent width,
        // we should not attempt to shrink them (as horizontal scrolling handles the overflow).
        if current_width >= target_width {
            return;
        }

        let sign = 1.0;

        if max_width <= current_width {
            return; // Can't grow
        }

        // Only grow columns that are explicitly configured as remainder columns (matching egui_extras)
        let mut can_change: Vec<(f32, usize)> = columns
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if c.flags.contains(ColumnFlags::REMAINDER) && c.current < c.range.max {
                    let room = if c.range.max == f32::INFINITY {
                        10000.0 - c.current // Cap infinity comparisons at a large finite limit
                    } else {
                        c.range.max - c.current
                    };
                    return Some((room, i));
                }
                None
            })
            .collect();

        if can_change.is_empty() {
            return; // No remainder columns to grow, keep everything compact
        }

        // Put the columns that has the most room to change first:
        can_change.sort_by(|a, b| b.0.partial_cmp(&a.0).expect("NaN or Inf in column code"));
        debug_assert!(
            can_change[0].0 >= can_change.last().expect("Can't be empty").0,
            "The sort is broken"
        );

        let mut remaining_abs = (target_width - current_width).abs();

        while let Some((room_in_least, least_idx)) = can_change.pop() {
            #[allow(clippy::cast_precision_loss)]
            let evenly_distributed = remaining_abs / (can_change.len() as f32 + 1.0);

            if evenly_distributed <= room_in_least {
                columns[least_idx].current += sign * evenly_distributed;
                for (_, i) in can_change {
                    columns[i].current += sign * evenly_distributed;
                }
                return;
            }

            columns[least_idx].current = columns[least_idx].range.max;
            remaining_abs -= room_in_least;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(c: i32, range: std::ops::RangeInclusive<i32>) -> Column {
        #[allow(clippy::cast_precision_loss)]
        Column::new(c as f32).range(Rangef::new(*range.start() as f32, *range.end() as f32))
    }

    fn widths(columns: &[Column]) -> Vec<f32> {
        columns.iter().map(|c| c.current).collect()
    }

    #[test]
    fn test_single_column() {
        let mut columns = [col(0, 100..=200).remainder(true)];
        Column::auto_size(&mut columns, 50.0);
        assert_eq!(widths(&columns), [100.0]);
        Column::auto_size(&mut columns, 132.0);
        assert_eq!(widths(&columns), [132.0]);
        Column::auto_size(&mut columns, 500.0);
        assert_eq!(widths(&columns), [200.0]);
    }

    #[test]
    fn test_many_columns() {
        let mut columns = [
            col(15, 10..=20).remainder(true),
            col(25, 10..=100).remainder(true),
            col(150, 100..=200).remainder(true),
        ];

        Column::auto_size(&mut columns, 190.0);
        assert_eq!(
            widths(&columns),
            [15.0, 25.0, 150.0],
            "They have no need to grow"
        );

        Column::auto_size(&mut columns, 207.0);
        assert_eq!(
            widths(&columns),
            [20.0, 31.0, 156.0],
            "They should grow equally"
        );
    }
}
