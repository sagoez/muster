use std::path::Path;

use muster_domain::{process::ProcessIcon, project::Project, value::ProcessName};

use super::{
    home_path::home_relative,
    occupant::Occupant,
    reading::ProjectReading,
    view::{FloorView, NoteView},
};

/// One project on the floor and what the floor keeps of it between readings.
pub(super) struct Tenant {
    /// The project.
    pub(super) project: Project,
    /// The icon it wears.
    pub(super) icon: ProcessIcon,
    /// Its processes, in stack order.
    pub(super) occupants: Vec<Occupant>,
    /// Its notes, as last read.
    pub(super) notes: Vec<NoteView>,
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
                match kept.iter().position(|occupant| occupant.name.key() == key) {
                    Some(at) => Occupant {
                        name: process.name().clone(),
                        icon: *process.icon(),
                        ..kept.swap_remove(at)
                    },
                    None => Occupant::new(process),
                }
            })
            .collect();
        Self {
            project: reading.project().clone(),
            icon: *reading.icon(),
            occupants,
            notes,
        }
    }

    /// Whether the page draws it as it would draw `reading`: the same folder,
    /// name and icon, and the same processes in the same order wearing the
    /// same icons.
    pub(super) fn shows_as(&self, reading: &ProjectReading) -> bool {
        self.project.root() == reading.project().root()
            && self.project.name() == reading.project().name()
            && self.icon == *reading.icon()
            && self.occupants.len() == reading.processes().len()
            && self
                .occupants
                .iter()
                .zip(reading.processes())
                .all(|(occupant, process)| {
                    occupant.name == *process.name() && occupant.icon == *process.icon()
                })
    }

    /// Its process whose name has the same key as `name`.
    pub(super) fn occupant(&self, name: &ProcessName) -> Option<&Occupant> {
        let key = name.key();
        self.occupants
            .iter()
            .find(|occupant| occupant.name.key() == key)
    }

    /// Its process whose name has the same key as `name`, to change.
    pub(super) fn occupant_mut(&mut self, name: &ProcessName) -> Option<&mut Occupant> {
        let key = name.key();
        self.occupants
            .iter_mut()
            .find(|occupant| occupant.name.key() == key)
    }

    /// The project as the page draws it, its folder written from `~` when it
    /// is under `home`.
    pub(super) fn view(&self, home: Option<&Path>) -> FloorView {
        FloorView::builder()
            .name(self.project.name().clone())
            .quoted(self.project.name().shell_quoted())
            .path(home_relative(self.project.root().as_ref().as_path(), home))
            .icon(self.icon)
            .processes(self.occupants.iter().map(Occupant::face).collect())
            .build()
    }
}
