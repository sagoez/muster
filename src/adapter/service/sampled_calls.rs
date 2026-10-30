use std::{cell::RefCell, rc::Rc};

use crate::domain::value::Pid;

/// The pids of every call a [`FakeMetrics`](super::fake_metrics::FakeMetrics)
/// answered, in order.
pub(super) type SampledCalls = Rc<RefCell<Vec<Vec<Pid>>>>;
