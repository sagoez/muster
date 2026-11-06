use getset::Getters;
use typed_builder::TypedBuilder;

/// A point-in-time resource-usage reading aggregated over a process subtree.
#[derive(Clone, Copy, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProcessSample {
    /// Total CPU use across the tree as a percentage, where 100 is one core.
    cpu_percent: f32,
    /// Resident memory across the tree, in bytes.
    memory_bytes: u64,
}
