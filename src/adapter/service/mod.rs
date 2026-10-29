//! The headless service side of muster: one service per user that supervises
//! every project's processes through holders, the protocol clients reach it
//! by, the events that drive it and the notices it raises.

mod acceptor;
mod batch_outcome;
mod event;
mod event_draft;
mod event_kind;
mod event_log;
mod event_page;
mod event_seq;
mod event_waiter;
mod exit_reason;
#[cfg(test)]
mod fake_pool;
mod flow;
#[cfg(test)]
mod gated_launcher;
mod holder_pool;
#[cfg(test)]
mod holders;
mod last_exit;
mod launch_environment;
mod located;
mod matched_line;
#[cfg(test)]
mod memory_store;
mod notice;
mod notice_level;
mod notices;
#[cfg(test)]
mod null_sink;
mod output_tap;
mod port_block;
mod port_holder;
mod port_owner;
mod port_sampler;
mod probe_command;
mod probe_target;
mod probe_task;
mod probe_worker;
mod process_action;
mod process_status;
mod process_usage;
mod project_ports;
mod project_status;
pub mod protocol;
mod refusal;
mod reply;
mod run_pin;
mod run_record;
mod run_recorder;
#[cfg(test)]
mod running;
mod runtime;
#[cfg(test)]
mod runtime_fixture;
#[cfg(test)]
mod runtime_testing;
mod service_deps;
mod service_error;
mod service_error_body;
mod service_error_kind;
mod service_request;
mod service_response;
mod service_version;
mod shared_event_log;
mod shared_holders;
mod shared_notices;
mod singleton;
mod stack_event;
mod stack_url;
#[cfg(test)]
mod stalled_handle;
mod state_dir;
mod state_goal;
pub mod supervisor;
mod tap_line;
#[cfg(test)]
mod test_ports;
#[cfg(test)]
mod thread_launcher;
mod url_source;
mod usage_sampler;
mod wait_condition;
mod wait_group;
mod wait_outcome;
mod wait_reply;
mod wait_spec;
mod wait_status;
mod waiter;

pub use batch_outcome::*;
pub use event::*;
pub use event_draft::*;
pub use event_kind::*;
pub use event_log::*;
pub use event_page::*;
pub use event_seq::*;
pub use exit_reason::*;
pub use holder_pool::*;
pub use last_exit::*;
pub use launch_environment::*;
pub use located::*;
pub use matched_line::*;
pub use notice::*;
pub use notice_level::*;
pub use notices::*;
pub use output_tap::*;
pub use port_block::*;
pub use port_holder::*;
pub use port_owner::*;
pub use port_sampler::*;
pub use probe_target::*;
pub use probe_worker::*;
pub use process_action::*;
pub use process_status::*;
pub use process_usage::*;
pub use project_ports::*;
pub use project_status::*;
pub use refusal::*;
pub use run_pin::*;
pub use run_recorder::*;
pub use runtime::*;
pub use service_deps::*;
pub use service_error::*;
pub use service_error_body::*;
pub use service_error_kind::*;
pub use service_request::*;
pub use service_response::*;
pub use service_version::*;
pub use shared_event_log::*;
pub use shared_holders::*;
pub use shared_notices::*;
pub use singleton::*;
pub use stack_event::*;
pub use stack_url::*;
pub use state_dir::*;
pub use state_goal::*;
pub use tap_line::*;
pub use url_source::*;
pub use usage_sampler::*;
pub use wait_condition::*;
pub use wait_group::*;
pub use wait_outcome::*;
pub use wait_reply::*;
pub use wait_spec::*;
pub use wait_status::*;
pub use waiter::*;
