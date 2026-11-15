use std::path::Path;

use getset::{CopyGetters, Getters, MutGetters};
use muster_domain::{process::ProcessIcon, project::Project, value::ProcessName};
use muster_os::path::home_relative;

use super::{
    faces_shown::FacesShown,
    occupant::Occupant,
    reading::ProjectReading,
    view::{FloorView, NoteView},
};

/// One project on the floor and what the floor keeps of it between readings.
#[derive(Getters, CopyGetters, MutGetters)]
pub(super) struct Tenant {
    /// The project.
    #[getset(get = "pub(super)")]
    project: Project,
    /// The icon it wears.
    icon: ProcessIcon,
    /// Whether its stack did not read at the last reading.
    #[getset(get_copy = "pub(super)")]
    unreadable: bool,
    /// Its processes, in stack order: while its stack does not read, those
    /// it held when it last read.
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    occupants: Vec<Occupant>,
    /// Its notes, as last read.
    #[getset(get = "pub(super)")]
    notes: Vec<NoteView>,
}

impl Tenant {
    /// The project as `reading` shows it, keeping what `before`, the same
    /// project as the floor last held it, knew of its notes and of each
    /// process still in it. A stack that did not read keeps every process
    /// and the icon `before` held, since it says nothing of them.
    pub(super) fn of(reading: &ProjectReading, before: Option<Self>) -> Self {
        let (icon, mut kept, notes) = before.map_or_else(
            || (*reading.icon(), Vec::new(), Vec::new()),
            |tenant| (tenant.icon, tenant.occupants, tenant.notes),
        );
        let unreadable = *reading.unreadable();
        if unreadable {
            return Self {
                project: reading.project().clone(),
                icon,
                unreadable,
                occupants: kept,
                notes,
            };
        }
        let occupants = reading
            .processes()
            .iter()
            .map(|process| {
                let key = process.name().key();
                match kept
                    .iter()
                    .position(|occupant| occupant.name().key() == key)
                {
                    Some(at) => kept.swap_remove(at).renamed(process),
                    None => Occupant::new(process),
                }
            })
            .collect();
        Self {
            project: reading.project().clone(),
            icon: *reading.icon(),
            unreadable,
            occupants,
            notes,
        }
    }

    /// Whether the page draws it as it would draw `reading`: the same folder,
    /// name and readability, and, for a stack that read, the same icon and
    /// the same processes in the same order wearing the same icons.
    pub(super) fn shows_as(&self, reading: &ProjectReading) -> bool {
        let same_project = self.project.root() == reading.project().root()
            && self.project.name() == reading.project().name()
            && self.unreadable == *reading.unreadable();
        if *reading.unreadable() {
            return same_project;
        }
        same_project
            && self.icon == *reading.icon()
            && self.occupants.len() == reading.processes().len()
            && self
                .occupants
                .iter()
                .zip(reading.processes())
                .all(|(occupant, process)| {
                    occupant.name() == process.name() && occupant.icon() == process.icon()
                })
    }

    /// Whether its processes' statuses stand confirmed when the service's
    /// are `confirmed`: never while its stack does not read, since then
    /// it may no longer hold them.
    pub(super) fn confirms(&self, confirmed: bool) -> bool {
        confirmed && !self.unreadable
    }

    /// Its process whose name has the same key as `name`.
    pub(super) fn occupant(&self, name: &ProcessName) -> Option<&Occupant> {
        let key = name.key();
        self.occupants
            .iter()
            .find(|occupant| occupant.name().key() == key)
    }

    /// Its process whose name has the same key as `name`, to change.
    pub(super) fn occupant_mut(&mut self, name: &ProcessName) -> Option<&mut Occupant> {
        let key = name.key();
        self.occupants
            .iter_mut()
            .find(|occupant| occupant.name().key() == key)
    }

    /// Takes in its notes as just read; says whether they changed.
    pub(super) fn read_notes(&mut self, notes: Vec<NoteView>) -> bool {
        if self.notes == notes {
            return false;
        }
        self.notes = notes;
        true
    }

    /// The project as the page draws it, its folder written from `~` when it
    /// is under `home`, and its processes as `shown`: held back while no
    /// status reading has placed them, so none walks in from Off, else each
    /// drawn unconfirmed unless it [`confirms`](Self::confirms) them.
    pub(super) fn view(&self, home: Option<&Path>, shown: FacesShown) -> FloorView {
        let processes = match shown {
            FacesShown::Held => Vec::new(),
            FacesShown::Unconfirmed | FacesShown::Confirmed => {
                let confirmed = self.confirms(shown == FacesShown::Confirmed);
                self.occupants
                    .iter()
                    .map(|occupant| occupant.face(self.project.name(), confirmed))
                    .collect()
            },
        };
        FloorView::builder()
            .name(self.project.name().clone())
            .path(home_relative(self.project.root().as_ref().as_path(), home))
            .icon(self.icon)
            .processes(processes)
            .unreadable(self.unreadable)
            .build()
    }
}
