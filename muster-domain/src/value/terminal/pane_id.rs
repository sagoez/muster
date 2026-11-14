use nutype::nutype;

/// Stable identifier for a managed process and its terminal pane.
#[nutype(derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display))]
pub struct PaneId(u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_ids_are_not_equal() {
        assert_ne!(PaneId::new(1), PaneId::new(2));
    }
}
