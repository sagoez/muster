use std::time::{Instant, SystemTime};

use crossbeam_channel::Sender;
use muster_domain::{coordination::Scratchpad, value::ProjectRoot};

use super::subscription::Subscription;
use crate::floor::{
    EventsReading, LocatedLog, LogReading, LogWatch, ProjectsReading, StatusReading,
};

/// What reaches the hub, the one owner of the floor: a reading from a
/// poller, or an ask from a connection with where to send the answer.
pub enum HubEvent {
    /// What reading the projects found.
    Projects(ProjectsReading),
    /// The status poller is about to ask the service, so the answer that
    /// follows shows every event heard before this.
    Asking,
    /// What asking the service for status found.
    Status {
        /// The answer.
        reading: StatusReading,
        /// When it was asked for.
        now: Instant,
        /// When it was asked for, by the clock.
        wall: SystemTime,
    },
    /// A page of the service's events, read after a cursor, or a gap
    /// standing for events lost while it could not be read.
    Events(EventsReading),
    /// One project's notes.
    Notes {
        /// The project's folder.
        root: ProjectRoot,
        /// Its notes, with their bodies.
        notes: Vec<Scratchpad>,
    },
    /// The watched logs' stats.
    Logs {
        /// When they were statted.
        now: Instant,
        /// Each watched log's stat.
        readings: Vec<LogReading>,
    },
    /// A page asks to stream; answered with its stream, or none when too
    /// many are open.
    Subscribe(Sender<Option<Subscription>>),
    /// A connection asks where a process's log is, by the names its request
    /// wrote; none unless the process is on the floor.
    LocateLog {
        /// The project's name, as written.
        project: String,
        /// The process's name, as written.
        name: String,
        /// Where the answer goes.
        reply: Sender<Option<LocatedLog>>,
    },
    /// The logs poller asks which logs to stat.
    LogWatches(Sender<Vec<LogWatch>>),
    /// The view is ending.
    Stop,
}
