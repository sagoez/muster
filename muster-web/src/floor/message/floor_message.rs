use std::collections::BTreeMap;

use muster_domain::value::{ProcessName, ProjectName};
use serde::Serialize;
use strum::IntoStaticStr;

use super::{float_kind::FloatKind, health::Health, pin::Pin};
use crate::floor::view::{FaceView, FloorView, NoteView};

/// One change the page applies, named by its `kind` both on the stream's
/// `event:` line and inside its data.
#[derive(Clone, Debug, PartialEq, Serialize, IntoStaticStr)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum FloorMessage {
    /// Everything, which replaces whatever the page held.
    Snapshot {
        /// How the view's sources stand.
        health: Health,
        /// The project holding the folder the view was started in.
        here: Option<ProjectName>,
        /// Every project's floor, in the store's order.
        projects: Vec<FloorView>,
        /// Each project's notes, by project name.
        notes: BTreeMap<String, Vec<NoteView>>,
    },
    /// The projects, after one was added or removed or a stack or an icon
    /// changed.
    Projects {
        /// The project holding the folder the view was started in.
        here: Option<ProjectName>,
        /// Every project's floor, in the store's order.
        projects: Vec<FloorView>,
    },
    /// One process, after anything the page shows of it changed.
    Process {
        /// Its project.
        project: ProjectName,
        /// The process as it stands now.
        process: FaceView,
    },
    /// A pin dropping on a process that changed room.
    Pin {
        /// Its project.
        project: ProjectName,
        /// The process.
        name: ProcessName,
        /// The pin.
        pin: Pin,
    },
    /// Something rising off a process's face.
    Float {
        /// Its project.
        project: ProjectName,
        /// The process.
        name: ProcessName,
        /// What rises.
        float: FloatKind,
        /// The port that opened, for a port float.
        port: Option<u16>,
    },
    /// A process starting or stopping speaking: its log grew, or stayed
    /// still long enough.
    Speaking {
        /// Its project.
        project: ProjectName,
        /// The process.
        name: ProcessName,
        /// Whether it speaks now.
        on: bool,
    },
    /// One project's notes, after any of them changed.
    Notes {
        /// The project.
        project: ProjectName,
        /// Its notes.
        notes: Vec<NoteView>,
    },
}
