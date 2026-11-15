use std::path::Path;

use getset::Getters;
use muster_domain::{process::ProcessIcon, project::Project, value::ProcessName};
use muster_os::path::home_relative;

use super::{
    occupant::Occupant,
    reading::ProjectReading,
    view::{FloorView, NoteView},
};

/// One project on the floor and what the floor keeps of it between readings.
#[derive(Getters)]
pub(super) struct Tenant {
    /// The project.
    #[getset(get = "pub(super)")]
    project: Project,
    /// The icon it wears.
    icon: ProcessIcon,
    /// Whether its stack did not read at the last reading.
    unreadable: bool,
    /// Its processes, in stack order.
    #[getset(get = "pub(super)")]
    occupants: Vec<Occupant>,
    /// Its notes, as last read.
    #[getset(get = "pub(super)")]
    notes: Vec<NoteView>,
}

impl Tenant {
    /// The project as `reading` shows it, keeping what `before`, the same
    /// project as the floor last held it, knew of its notes and of each
    /// process still in it.
    pub(super) fn of(reading: &ProjectReading, before: Option<Self>) -> Self {
        let (mut kept, notes) =
            before.map_or_else(Default::default, |tenant| (tenant.occupants, tenant.notes));
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
            unreadable: *reading.unreadable(),
            occupants,
            notes,
        }
    }

    /// Whether the page draws it as it would draw `reading`: the same folder,
    /// name, icon and readability, and the same processes in the same order
    /// wearing the same icons.
    pub(super) fn shows_as(&self, reading: &ProjectReading) -> bool {
        self.project.root() == reading.project().root()
            && self.project.name() == reading.project().name()
            && self.icon == *reading.icon()
            && self.unreadable == *reading.unreadable()
            && self.occupants.len() == reading.processes().len()
            && self
                .occupants
                .iter()
                .zip(reading.processes())
                .all(|(occupant, process)| {
                    occupant.name() == process.name() && occupant.icon() == process.icon()
                })
    }

    /// Its processes, to change.
    pub(super) fn occupants_mut(&mut self) -> &mut [Occupant] {
        &mut self.occupants
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
    /// is under `home`, and each process marked muted unless its status is
    /// `confirmed`. Its processes are `held` back, none drawn, while no
    /// status reading has placed them yet, so none walks in from Off.
    pub(super) fn view(&self, home: Option<&Path>, confirmed: bool, held: bool) -> FloorView {
        let processes = if held {
            Vec::new()
        } else {
            self.occupants
                .iter()
                .map(|occupant| occupant.face(self.project.name(), confirmed))
                .collect()
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
