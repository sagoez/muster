use std::path::Path;

use getset::{Getters, MutGetters};
use muster_client::project_option_for;
use muster_domain::{process::ProcessIcon, project::Project, value::ProcessName};
use muster_os::path::home_relative;
use typed_builder::TypedBuilder;

use super::{
    faces_shown::FacesShown,
    occupant::Occupant,
    reading::ProjectReading,
    view::{FloorView, NoteView, Unconfirmed},
};

/// One project on the floor and what the floor keeps of it between readings.
#[derive(Getters, MutGetters, TypedBuilder)]
pub(super) struct Tenant {
    /// The project.
    #[getset(get = "pub(super)")]
    project: Project,
    /// The icon it wears.
    icon: ProcessIcon,
    /// Whether its stack did not read at the last reading.
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
            return Self::builder()
                .project(reading.project().clone())
                .icon(icon)
                .unreadable(unreadable)
                .occupants(kept)
                .notes(notes)
                .build();
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
        Self::builder()
            .project(reading.project().clone())
            .icon(*reading.icon())
            .unreadable(unreadable)
            .occupants(occupants)
            .notes(notes)
            .build()
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

    /// Why its processes' statuses stand unconfirmed when the service's are
    /// unconfirmed for `service`: always while its stack does not read,
    /// since then it may no longer hold them.
    pub(super) fn unconfirmed(&self, service: Option<Unconfirmed>) -> Option<Unconfirmed> {
        if self.unreadable {
            Some(Unconfirmed::UnreadableStack)
        } else {
            service
        }
    }

    /// The `-p` words its cards' commands name it with, which reach it from
    /// any folder: its folder written from `~` under `home` when its name
    /// reads as a folder.
    pub(super) fn project_option(&self, home: Option<&Path>) -> Option<String> {
        project_option_for(&self.project, home)
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
    /// drawn unconfirmed for the reason [`unconfirmed`](Self::unconfirmed)
    /// gives, if any.
    pub(super) fn view(&self, home: Option<&Path>, shown: FacesShown) -> FloorView {
        let processes = match shown {
            FacesShown::Held => Vec::new(),
            FacesShown::Unconfirmed(_) | FacesShown::Confirmed => {
                let unconfirmed = self.unconfirmed(shown.unconfirmed());
                let option = self.project_option(home);
                self.occupants
                    .iter()
                    .map(|occupant| occupant.face(option.as_deref(), unconfirmed))
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
