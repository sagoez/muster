//! The horizontal tab stops a child has set, mirrored from what it wrote
//! because the emulator backend keeps its own copy private.

/// Columns between the default tab stops.
const TAB_INTERVAL: usize = 8;

/// One flag per column, set where a tab stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabStops {
    stops: Vec<bool>,
}

impl TabStops {
    /// The default stops, every eighth column, for `columns` columns.
    pub fn new(columns: usize) -> Self {
        Self {
            stops: (0..columns).map(is_default_stop).collect(),
        }
    }

    /// Fits the stops to `columns`, keeping those already set and giving new
    /// columns the default stops, the way the emulator does.
    pub fn resize(&mut self, columns: usize) {
        let mut column = self.stops.len();
        self.stops.resize_with(columns, || {
            let stop = is_default_stop(column);
            column += 1;
            stop
        });
    }

    /// Sets a stop at `column` (HTS).
    pub fn set(&mut self, column: usize) {
        if let Some(stop) = self.stops.get_mut(column) {
            *stop = true;
        }
    }

    /// Clears the stop at `column` (TBC 0).
    pub fn clear(&mut self, column: usize) {
        if let Some(stop) = self.stops.get_mut(column) {
            *stop = false;
        }
    }

    /// Clears every stop (TBC 3).
    pub fn clear_all(&mut self) {
        self.stops.fill(false);
    }

    /// Whether the stops are the defaults a reset terminal has.
    pub fn is_default(&self) -> bool {
        *self == Self::new(self.stops.len())
    }

    /// The zero-based columns that hold a stop.
    pub fn columns(&self) -> impl Iterator<Item = usize> + '_ {
        self.stops
            .iter()
            .enumerate()
            .filter(|(_, stop)| **stop)
            .map(|(column, _)| column)
    }
}

/// Whether `column` holds a stop on a reset terminal.
fn is_default_stop(column: usize) -> bool {
    column.is_multiple_of(TAB_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::TabStops;

    /// Growing keeps the stops already set and gives new columns the
    /// defaults by their absolute position.
    #[test]
    fn growing_keeps_set_stops_and_defaults_the_new_columns() {
        let mut stops = TabStops::new(10);
        stops.clear_all();
        stops.set(3);
        stops.resize(20);
        assert_eq!(stops.columns().collect::<Vec<_>>(), vec![3, 16]);
        assert!(!stops.is_default());
        assert!(TabStops::new(20).is_default());
    }
}
