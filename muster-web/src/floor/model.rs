use std::{
    iter, mem,
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

use muster_client::listening_ports;
use muster_domain::{
    coordination::Scratchpad,
    value::{ProcessName, ProjectName, ProjectRoot},
};
use muster_protocol::{
    holder::HolderDirs,
    service::{EventSeq, ProcessStatus, ProjectStatus, StackEvent, StatusAnswer},
};
use typed_builder::TypedBuilder;

use super::{
    faces_shown::FacesShown,
    located_log::LocatedLog,
    message::{FloatKind, FloorMessage, Pin, Sequenced},
    reading::{
        EventsReading, Health, LogReading, LogWatch, ProjectReading, ProjectsReading, StateTrouble,
        StatusReading,
    },
    tenant::Tenant,
    view::{FloorView, NoteView, Unconfirmed},
};

/// How long a process speaks after its log last changed.
const SPEAKING_FOR: Duration = Duration::from_secs(2);
/// Failed asks in a row after which a service that holds its socket reads
/// as unresponsive.
pub(crate) const MAX_FAILED_ASKS: u32 = 5;
/// Most ports one process floats in one reading, so a server opening many at
/// once does not bury its face.
const MAX_PORT_FLOATS: usize = 3;
/// Most events the floor holds while a baseline is due, so a long replay
/// after a park keeps only its newest, the only ones that may follow the
/// baseline's cursor.
const MAX_HELD_EVENTS: usize = 64;

/// Every project's floor as the page should show it, and what each new
/// reading changes of it, as numbered messages. Pure: it does no I/O and
/// is told the time. Its builder takes only where the view was started and
/// where logs live, so every floor starts as one that has read nothing. It
/// has no getters: what it holds reaches a page only as the messages it
/// sends, and the hub asks of it through its methods.
#[derive(TypedBuilder)]
pub(crate) struct Floor {
    /// The folder the view was started in, whose project is `here`.
    cwd: PathBuf,
    /// The home folder, which paths show as `~`.
    home: Option<PathBuf>,
    /// Where each process's log lives.
    holder_dirs: HolderDirs,
    /// The sequence number of the last message.
    #[builder(default, setter(skip))]
    seq: u64,
    /// How the service stood at the last status reading.
    #[builder(default = Health::Loading, setter(skip))]
    service: Health,
    /// Whether the floor rested since that reading, nobody watching while
    /// its pollers parked: until the next reading what it holds is not
    /// confirmed, and its health reads as loading.
    #[builder(default, setter(skip))]
    rested: bool,
    /// The last answer of the service, and when it was asked for: a project
    /// or a process that arrives after it stands where it said, so none
    /// walks in from Off.
    #[builder(default, setter(skip))]
    last_answer: Option<(StatusAnswer, SystemTime)>,
    /// The event a heard one must follow to pin: the cursor of the status
    /// answer that made the last baseline, then each event heard after it.
    /// One at or before it, or of another service's log, is history or
    /// heard twice, so it pins nothing; none before the first baseline.
    #[builder(default, setter(skip))]
    heard_to: Option<EventSeq>,
    /// The events heard while a baseline was due, oldest first, at most
    /// [`MAX_HELD_EVENTS`], for that baseline to pin those after its cursor.
    #[builder(default, setter(skip))]
    held: Vec<StackEvent>,
    /// Asks of the service that failed since the last that did not.
    #[builder(default, setter(skip))]
    failed_asks: u32,
    /// What is wrong with the state, if anything.
    #[builder(default, setter(skip))]
    store: Option<Health>,
    /// Whether the next status reading is a baseline, sent whole as a
    /// snapshot with no float for what changed.
    #[builder(default = true, setter(skip))]
    baseline: bool,
    /// Whether the projects have been read yet; until then every status
    /// reading is a baseline, so no process walks in from Off once they are.
    #[builder(default, setter(skip))]
    projects_read: bool,
    /// The projects, in the store's order.
    #[builder(default, setter(skip))]
    tenants: Vec<Tenant>,
}

impl Floor {
    /// An empty floor for a view started in `cwd`, that shows paths under
    /// `home` from `~` and finds logs through `holder_dirs`. Until the first
    /// status reading it is loading.
    #[must_use]
    pub(crate) fn new(cwd: PathBuf, home: Option<PathBuf>, holder_dirs: HolderDirs) -> Self {
        Self::builder()
            .cwd(cwd)
            .home(home)
            .holder_dirs(holder_dirs)
            .build()
    }

    /// How the view's sources stand: what is wrong with the state, else how
    /// the service stands, loading while the floor rested.
    #[must_use]
    pub(crate) fn health(&self) -> Health {
        let service = if self.rested {
            Health::Loading
        } else {
            self.service
        };
        self.store.unwrap_or(service)
    }

    /// Everything, as one snapshot for a page that has just connected.
    pub(crate) fn snapshot(&mut self) -> Sequenced {
        let notes = self
            .tenants
            .iter()
            .map(|tenant| (tenant.project().name().to_string(), tenant.notes().clone()))
            .collect();
        let snapshot = FloorMessage::Snapshot {
            health: self.health(),
            here: self.here(),
            projects: self.views(),
            notes,
        };
        self.sequenced(snapshot)
    }

    /// Takes in what reading the projects found: a snapshot when the
    /// state's health changed, which also makes the next status reading a
    /// baseline; else the projects when one was added or removed, a stack or
    /// an icon changed, or `here` moved, then the notes of each project new
    /// or newly named: a new one's are none, so a page drops any it kept of
    /// a project of that name before, and a renamed one's are those it
    /// held, which the page keeps by name and dropped with the old. A state
    /// that could not be read this time keeps the projects; any other
    /// trouble empties the floor. A process no status reading has placed
    /// yet stands where the service last said it did.
    pub(crate) fn read_projects(&mut self, reading: ProjectsReading) -> Vec<Sequenced> {
        let (health, here) = (self.health(), self.here());
        self.projects_read = true;
        let (changed, named) = match reading {
            ProjectsReading::Read(projects) => {
                self.store = None;
                self.settle(&projects)
            },
            ProjectsReading::Trouble(StateTrouble::Unreadable) => {
                self.store = Some(Health::Unreadable);
                (false, Vec::new())
            },
            ProjectsReading::Trouble(
                trouble @ (StateTrouble::NoState
                | StateTrouble::OlderSchema
                | StateTrouble::NewerSchema),
            ) => (self.clear(trouble.into()), Vec::new()),
        };
        self.place_unseen();
        if self.health() != health {
            self.baseline = true;
            self.show_all(None);
            return vec![self.snapshot()];
        }
        if !changed && self.here() == here {
            return Vec::new();
        }
        self.show_all(None);
        let projects = FloorMessage::Projects {
            here: self.here(),
            projects: self.views(),
        };
        let notes = named
            .into_iter()
            .map(|(project, notes)| FloorMessage::Notes { project, notes });
        let messages: Vec<FloorMessage> = iter::once(projects).chain(notes).collect();
        messages
            .into_iter()
            .map(|message| self.sequenced(message))
            .collect()
    }

    /// Takes in what asking the service for status found, asked at `now` and
    /// at `wall` by the clock: one snapshot for a baseline, then the pins of
    /// the events held for it that follow its cursor; else each process
    /// whose face changed, then its floats. A status earns no pin: only a
    /// heard event does. A change of usage alone is sent at most once every
    /// few seconds. The first reading, a gap, the first after the floor
    /// rested, a change of the service's health and an answer of another
    /// service's log make a baseline. A failed ask says nothing of the
    /// processes, so it changes none of them, but [`MAX_FAILED_ASKS`] in a
    /// row make the service unresponsive. A service that is absent,
    /// starting or of another version says nothing of them either: their
    /// children run on under their holders, so each keeps what was last
    /// read of it, drawn unconfirmed.
    pub(crate) fn read_status(
        &mut self,
        reading: StatusReading,
        now: Instant,
        wall: SystemTime,
    ) -> Vec<Sequenced> {
        let answer = match reading {
            StatusReading::Failed => return self.failed_ask(now),
            StatusReading::Down(down) => return self.unconfirmed(down.into(), now),
            StatusReading::Answered(answer) => answer,
        };
        self.failed_asks = 0;
        let (statuses, cursor) = (answer.projects(), answer.cursor());
        let other_log = self
            .heard_to
            .is_some_and(|heard_to| heard_to.epoch() != cursor.epoch());
        let baseline = self.baseline_due() || other_log;
        self.rested = false;
        self.baseline = !self.projects_read;
        self.service = Health::Running;
        let mut messages = Vec::new();
        for tenant in &mut self.tenants {
            let project = status_of_project(statuses, tenant);
            let name = tenant.project().name().clone();
            let unconfirmed = tenant.unconfirmed(None);
            let option = tenant.project_option(self.home.as_deref());
            for occupant in tenant.occupants_mut() {
                let status = status_of_process(project, occupant.name());
                let was = occupant.place(status.cloned(), wall);
                let seen = occupant.see();
                if baseline {
                    continue;
                }
                let face = occupant.face(option.as_deref(), unconfirmed);
                if occupant.face_due(&face, now) {
                    occupant.show(face.clone(), Some(now));
                    messages.push(FloorMessage::Process {
                        project: name.clone(),
                        process: face,
                    });
                }
                if seen {
                    messages.extend(floats(&name, occupant.name(), was.as_ref(), status));
                }
            }
        }
        self.last_answer = Some((answer, wall));
        if baseline {
            self.heard_to = Some(match self.heard_to {
                Some(heard_to) if heard_to.is_after(cursor) => heard_to,
                _ => cursor,
            });
            self.show_all(Some(now));
            let snapshot = self.snapshot();
            let held = mem::take(&mut self.held);
            let pins: Vec<FloorMessage> =
                held.iter().filter_map(|event| self.pin_of(event)).collect();
            let pins: Vec<Sequenced> = pins
                .into_iter()
                .map(|message| self.sequenced(message))
                .collect();
            return iter::once(snapshot).chain(pins).collect();
        }
        messages
            .into_iter()
            .map(|message| self.sequenced(message))
            .collect()
    }

    /// Whether the next status answer makes a baseline, so an event heard
    /// until then waits for its cursor: one is due, the floor rested, or
    /// the service did not answer the last reading.
    fn baseline_due(&self) -> bool {
        self.baseline || self.rested || self.service != Health::Running
    }

    /// The pin `event` drops, when it follows the last event heard, or the
    /// last baseline's cursor, in the same service's log, of a process on
    /// the floor, and is of a kind that pins. An event that follows moves
    /// that mark, so one heard twice pins once.
    fn pin_of(&mut self, event: &StackEvent) -> Option<FloorMessage> {
        let seq = event.seq();
        if !self.heard_to.is_some_and(|heard_to| seq.is_after(heard_to)) {
            return None;
        }
        self.heard_to = Some(seq);
        let (root, name) = (event.project().as_ref()?, event.process().as_ref()?);
        let tenant = self
            .tenants
            .iter()
            .find(|tenant| tenant.project().root() == root)?;
        let occupant = tenant.occupant(name)?;
        let pin = Pin::of_event(event.kind(), occupant.checked())?;
        Some(FloorMessage::Pin {
            project: tenant.project().name().clone(),
            name: occupant.name().clone(),
            pin,
        })
    }

    /// Counts a failed ask at `now`: from [`MAX_FAILED_ASKS`] in a row on,
    /// the service is unresponsive, sent as a snapshot that keeps every
    /// process as last read when that is news or the floor rested since;
    /// anything else sends nothing.
    fn failed_ask(&mut self, now: Instant) -> Vec<Sequenced> {
        self.failed_asks = self.failed_asks.saturating_add(1);
        if self.failed_asks < MAX_FAILED_ASKS {
            return Vec::new();
        }
        self.unconfirmed(Health::Unresponsive, now)
    }

    /// Marks the service as `health`, which confirms no status, at `now`:
    /// a change of it, or the first reading after the floor rested, is sent
    /// as a snapshot that keeps every process as last read, drawn
    /// unconfirmed; the same health again sends nothing.
    fn unconfirmed(&mut self, health: Health, now: Instant) -> Vec<Sequenced> {
        if health != Health::Unresponsive {
            self.failed_asks = 0;
        }
        if self.service == health && !self.rested {
            return Vec::new();
        }
        self.rested = false;
        self.service = health;
        self.show_all(Some(now));
        vec![self.snapshot()]
    }

    /// Makes the next status reading a baseline: events were missed, or the
    /// service restarted, so the changes it shows are not ones anyone saw.
    pub(crate) fn gap(&mut self) {
        self.baseline = true;
    }

    /// Rests the floor, as when its pollers parked while no page watched: no
    /// status it holds is confirmed until the next status reading, so a
    /// page sees it as loading with every process drawn unconfirmed. Every
    /// process falls quiet and forgets its log's stat, since the logs were
    /// not statted: the first stat after takes each log as it finds it, so
    /// none speaks for what it printed meanwhile. It sends nothing.
    pub(crate) fn rest(&mut self) {
        self.rested = true;
        for tenant in &mut self.tenants {
            for occupant in tenant.occupants_mut() {
                occupant.forget_log();
            }
        }
    }

    /// Takes in a page of the service's events: the pin of each that pins,
    /// sent as it is heard, wherever its process's face stands. Each event
    /// pins once, and only when it follows the cursor of the status answer
    /// that made the last baseline: one at or before it is history that
    /// answer already drew. While a baseline is due, a gap making one, the
    /// events are held for it instead, up to [`MAX_HELD_EVENTS`], and pin
    /// once its answer says where its cursor stands. Events of a project or
    /// process not on the floor, and of kinds that pin nothing, are
    /// dropped.
    pub(crate) fn hear_events(&mut self, reading: &EventsReading) -> Vec<Sequenced> {
        if reading.gap() {
            self.gap();
        }
        if self.baseline_due() {
            // An unchecked start pins, so this holds every kind that may.
            let may_pin = |event: &&StackEvent| {
                event.process().is_some() && Pin::of_event(event.kind(), false).is_some()
            };
            for event in reading.events().iter().filter(may_pin) {
                if self.held.len() == MAX_HELD_EVENTS {
                    self.held.remove(0);
                }
                self.held.push(event.clone());
            }
            return Vec::new();
        }
        let pins: Vec<FloorMessage> = reading
            .events()
            .iter()
            .filter_map(|event| self.pin_of(event))
            .collect();
        pins.into_iter()
            .map(|message| self.sequenced(message))
            .collect()
    }

    /// Takes in the notes of the project at `root`: its notes when they
    /// changed, nothing for a project not on the floor.
    pub(crate) fn read_notes(
        &mut self,
        root: &ProjectRoot,
        notes: &[Scratchpad],
    ) -> Vec<Sequenced> {
        let notes: Vec<NoteView> = notes.iter().map(NoteView::from).collect();
        let Some(tenant) = self
            .tenants
            .iter_mut()
            .find(|tenant| tenant.project().root() == root)
        else {
            return Vec::new();
        };
        if !tenant.read_notes(notes.clone()) {
            return Vec::new();
        }
        let project = tenant.project().name().clone();
        vec![self.sequenced(FloorMessage::Notes { project, notes })]
    }

    /// Takes in the logs' stats at `now`: a running process whose log
    /// changed since its last stat speaks until [`SPEAKING_FOR`] after, and
    /// one whose time is up falls quiet. Called each time the logs are
    /// statted, even with none to stat, so speaking ends on time.
    pub(crate) fn read_logs(&mut self, now: Instant, readings: &[LogReading]) -> Vec<Sequenced> {
        let until = now.checked_add(SPEAKING_FOR).unwrap_or(now);
        let mut messages = Vec::new();
        for reading in readings {
            let Some(tenant) = self
                .tenants
                .iter_mut()
                .find(|tenant| tenant.project().root() == reading.root())
            else {
                continue;
            };
            let project = tenant.project().name().clone();
            let Some(occupant) = tenant.occupant_mut(reading.name()) else {
                continue;
            };
            if occupant.stat_log(*reading.stat(), until) {
                messages.push(speaking(&project, occupant.name(), true));
            }
        }
        for tenant in &mut self.tenants {
            let project = tenant.project().name().clone();
            for occupant in tenant.occupants_mut() {
                if occupant.quiet_if_due(now) {
                    messages.push(speaking(&project, occupant.name(), false));
                }
            }
        }
        messages
            .into_iter()
            .map(|message| self.sequenced(message))
            .collect()
    }

    /// The running processes whose logs to stat, each with its log.
    #[must_use]
    pub(crate) fn log_watches(&self) -> Vec<LogWatch> {
        self.tenants
            .iter()
            .flat_map(|tenant| {
                tenant
                    .occupants()
                    .iter()
                    .filter(|occupant| occupant.runs())
                    .map(|occupant| {
                        let root = tenant.project().root();
                        LogWatch::builder()
                            .root(root.clone())
                            .name(occupant.name().clone())
                            .path(self.holder_dirs.process_log(root, occupant.name()))
                            .build()
                    })
            })
            .collect()
    }

    /// The log of the process named `name` in the project named `project`,
    /// both as a request wrote them: the project matched by its name as the
    /// store keeps it unique and the page names it, the process by its key,
    /// as the store keeps a stack's names unique. None unless that process
    /// is on the floor. The path comes from the floor's own project and
    /// process, never from the request's text.
    #[must_use]
    pub(crate) fn locate_log(&self, project: &str, name: &str) -> Option<LocatedLog> {
        let name = ProcessName::try_new(name).ok()?;
        let project = ProjectName::try_new(project).ok()?;
        let tenant = self
            .tenants
            .iter()
            .find(|tenant| *tenant.project().name() == project)?;
        let occupant = tenant.occupant(&name)?;
        let root = tenant.project().root();
        Some(
            LocatedLog::builder()
                .project(tenant.project().clone())
                .name(occupant.name().clone())
                .path(self.holder_dirs.process_log(root, occupant.name()))
                .build(),
        )
    }

    /// Replaces the projects with `projects`, keeping what the floor knew of
    /// each one still there; says whether the page draws them otherwise, and
    /// names, with its notes, each project that was not there before or was
    /// there under another name.
    fn settle(&mut self, projects: &[ProjectReading]) -> (bool, Vec<(ProjectName, Vec<NoteView>)>) {
        let changed = self.tenants.len() != projects.len()
            || self
                .tenants
                .iter()
                .zip(projects)
                .any(|(tenant, reading)| !tenant.shows_as(reading));
        let mut before = mem::take(&mut self.tenants);
        let mut named = Vec::new();
        self.tenants = projects
            .iter()
            .map(|reading| {
                let kept = before
                    .iter()
                    .position(|tenant| tenant.project().root() == reading.project().root())
                    .map(|at| before.swap_remove(at));
                let newly_named = kept
                    .as_ref()
                    .is_none_or(|tenant| tenant.project().name() != reading.project().name());
                let tenant = Tenant::of(reading, kept);
                if newly_named {
                    named.push((tenant.project().name().clone(), tenant.notes().clone()));
                }
                tenant
            })
            .collect();
        (changed, named)
    }

    /// Empties the floor for a state that holds no projects it can read,
    /// marked `trouble`, and says whether any project left.
    fn clear(&mut self, trouble: Health) -> bool {
        self.store = Some(trouble);
        let had = !self.tenants.is_empty();
        self.tenants.clear();
        had
    }

    /// Places each process no status reading has placed yet where the last
    /// answer of the service said it stood, so a project or a process that
    /// arrives after that answer is drawn in its room at once.
    fn place_unseen(&mut self) {
        let Some((answer, wall)) = &self.last_answer else {
            return;
        };
        for tenant in &mut self.tenants {
            let project = status_of_project(answer.projects(), tenant);
            for occupant in tenant.occupants_mut() {
                if !occupant.seen() {
                    let status = status_of_process(project, occupant.name());
                    occupant.place(status.cloned(), *wall);
                }
            }
        }
    }

    /// How the floor draws its processes: held back while the service has
    /// not been asked, so none walks in from Off; confirmed while a service
    /// of this version answered the last status reading since the floor
    /// last rested; else unconfirmed, as resting until a status is read
    /// after the floor rested, then for the service being down.
    fn faces_shown(&self) -> FacesShown {
        match self.service {
            Health::Loading => FacesShown::Held,
            _ if self.rested => FacesShown::Unconfirmed(Unconfirmed::Resting),
            Health::Running => FacesShown::Confirmed,
            _ => FacesShown::Unconfirmed(Unconfirmed::ServiceDown),
        }
    }

    /// Records that every page was just sent every process's face, its
    /// usage at `now` when known.
    fn show_all(&mut self, now: Option<Instant>) {
        let service = self.faces_shown().unconfirmed();
        for tenant in &mut self.tenants {
            let option = tenant.project_option(self.home.as_deref());
            let unconfirmed = tenant.unconfirmed(service);
            for occupant in tenant.occupants_mut() {
                let face = occupant.face(option.as_deref(), unconfirmed);
                occupant.show(face, now);
            }
        }
    }

    /// The project holding the folder the view was started in: the deepest
    /// one, when projects nest.
    fn here(&self) -> Option<ProjectName> {
        self.tenants
            .iter()
            .filter(|tenant| tenant.project().root().contains(&self.cwd))
            .max_by_key(|tenant| tenant.project().root().as_ref().components().count())
            .map(|tenant| tenant.project().name().clone())
    }

    /// Every project's floor as the page draws it.
    fn views(&self) -> Vec<FloorView> {
        let home = self.home.as_deref();
        let shown = self.faces_shown();
        self.tenants
            .iter()
            .map(|tenant| tenant.view(home, shown))
            .collect()
    }

    /// `message` with the next sequence number.
    fn sequenced(&mut self, message: FloorMessage) -> Sequenced {
        self.seq += 1;
        Sequenced::builder().seq(self.seq).message(message).build()
    }
}

/// What `statuses` say of `tenant`'s project, found by its folder.
fn status_of_project<'a>(
    statuses: &'a [ProjectStatus],
    tenant: &Tenant,
) -> Option<&'a ProjectStatus> {
    statuses
        .iter()
        .find(|status| status.project().root() == tenant.project().root())
}

/// What `project` says of its process whose name has the same key as `name`.
fn status_of_process<'a>(
    project: Option<&'a ProjectStatus>,
    name: &ProcessName,
) -> Option<&'a ProcessStatus> {
    let key = name.key();
    project.and_then(|project| {
        project
            .processes()
            .iter()
            .find(|status| status.name().key() == key)
    })
}

/// The floats process `name` of `project` earns going from `was` to `now`:
/// each port it newly listens on, up to [`MAX_PORT_FLOATS`], and a restart
/// when its count rose.
fn floats(
    project: &ProjectName,
    name: &ProcessName,
    was: Option<&ProcessStatus>,
    now: Option<&ProcessStatus>,
) -> Vec<FloorMessage> {
    let float = |float: FloatKind, port: Option<u16>| FloorMessage::Float {
        project: project.clone(),
        name: name.clone(),
        float,
        port,
    };
    let known = was.map(listening_ports).unwrap_or_default();
    let ports = now
        .map(listening_ports)
        .unwrap_or_default()
        .into_iter()
        .filter(|port| !known.contains(port))
        .take(MAX_PORT_FLOATS)
        .map(|port| float(FloatKind::Port, Some(port)));
    let restarted = was
        .zip(now)
        .is_some_and(|(was, now)| now.restarts() > was.restarts());
    ports
        .chain(restarted.then(|| float(FloatKind::Restart, None)))
        .collect()
}

/// That process `name` of `project` started or stopped speaking.
fn speaking(project: &ProjectName, name: &ProcessName, on: bool) -> FloorMessage {
    FloorMessage::Speaking {
        project: project.clone(),
        name: name.clone(),
        on,
    }
}

#[cfg(test)]
mod tests {
    use std::{slice, time::UNIX_EPOCH};

    use muster_domain::{
        coordination::{Author, ScratchpadKey},
        process::{ProcessIcon, ProcessState, Readiness},
        project::Project,
        value::EpochSeconds,
    };
    use muster_protocol::service::EventKind;
    use serde_json::{Value, json};

    use super::*;
    use crate::floor::{
        fixture::{
            HOME, answered, answered_at, checked_project_reading, floor, holder_dirs, newest_event,
            process_status, process_status_with, project, project_reading, project_reading_with,
            project_status, root, running_with_usage, stack_event,
        },
        reading::{LogStat, ServiceDown},
        view::Room,
    };

    /// The project most tests watch.
    const SHOP: &str = "shop";
    /// A second project.
    const BLOG: &str = "blog";

    /// The kind of each of `messages`, in order.
    fn kinds(messages: &[Sequenced]) -> Vec<&'static str> {
        messages
            .iter()
            .map(|message| message.message().into())
            .collect()
    }

    /// `message` as the JSON its frame carries.
    fn json_of(message: &Sequenced) -> Value {
        serde_json::to_value(message).unwrap()
    }

    /// The pins among `messages`, each with its process's name.
    fn pins(messages: &[Sequenced]) -> Vec<(String, Pin)> {
        messages
            .iter()
            .filter_map(|message| match message.message() {
                FloorMessage::Pin { name, pin, .. } => Some((name.to_string(), *pin)),
                _ => None,
            })
            .collect()
    }

    /// The pins among `messages`, without names.
    fn pins_only(messages: &[Sequenced]) -> Vec<Pin> {
        pins(messages).into_iter().map(|(_, pin)| pin).collect()
    }

    /// The floats among `messages`, each with its port.
    fn floats(messages: &[Sequenced]) -> Vec<(FloatKind, Option<u16>)> {
        messages
            .iter()
            .filter_map(|message| match message.message() {
                FloorMessage::Float { float, port, .. } => Some((*float, *port)),
                _ => None,
            })
            .collect()
    }

    /// The room the first process message among `messages` puts its process in.
    fn moved_to(messages: &[Sequenced]) -> Option<Room> {
        messages.iter().find_map(|message| match message.message() {
            FloorMessage::Process { process, .. } => Some(*process.room()),
            _ => None,
        })
    }

    /// `name` as a process name.
    fn process_name(name: &str) -> ProcessName {
        ProcessName::try_new(name).unwrap()
    }

    /// Shop's status holding `processes`, as a service of this version
    /// answers it now, after every event made so far.
    fn shop_answers(processes: Vec<ProcessStatus>) -> StatusReading {
        answered(vec![project_status(SHOP, processes)])
    }

    /// The projects read as `projects`.
    fn read(projects: Vec<ProjectReading>) -> ProjectsReading {
        ProjectsReading::Read(projects)
    }

    /// `web` in `state` with readiness `ready`.
    fn web(state: ProcessState, ready: Readiness) -> ProcessStatus {
        process_status("web", state, ready)
    }

    /// The wall clock `seconds` past the epoch's first million seconds.
    fn wall(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_000_000 + seconds)
    }

    /// What `floor` sends for `reading`, asked for now.
    fn poll(floor: &mut Floor, reading: StatusReading) -> Vec<Sequenced> {
        floor.read_status(reading, Instant::now(), SystemTime::now())
    }

    /// A floor holding shop with the processes `statuses` name, past the
    /// baseline that placed each as they say.
    fn settled(statuses: Vec<ProcessStatus>) -> Floor {
        let names: Vec<String> = statuses
            .iter()
            .map(|status| status.name().to_string())
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut floor = floor();
        floor.read_projects(read(vec![project_reading(SHOP, &names)]));
        poll(&mut floor, shop_answers(statuses));
        floor
    }

    /// What each step of `web` walking through `steps` sends, past the
    /// baseline of its first step.
    fn walk(steps: &[(ProcessState, Readiness)]) -> Vec<Vec<Sequenced>> {
        let ((state, ready), rest) = steps.split_first().unwrap();
        let mut floor = settled(vec![web(*state, *ready)]);
        rest.iter()
            .map(|(state, ready)| poll(&mut floor, shop_answers(vec![web(*state, *ready)])))
            .collect()
    }

    /// A page of shop's events after a cursor: `web` heard as each of
    /// `kinds`, in turn, numbered after every event heard before.
    fn heard(kinds: &[EventKind]) -> EventsReading {
        EventsReading::builder()
            .events(
                kinds
                    .iter()
                    .map(|kind| stack_event(SHOP, "web", *kind))
                    .collect(),
            )
            .gap(false)
            .build()
    }

    /// A stat of a log `size` bytes long, last written at `modified`.
    fn stat(size: u64, modified: Option<SystemTime>) -> LogStat {
        LogStat::builder().size(size).modified(modified).build()
    }

    /// Shop's process `name` found its log as `stat`.
    fn logged(name: &str, stat: LogStat) -> LogReading {
        LogReading::builder()
            .root(root(SHOP))
            .name(process_name(name))
            .stat(Some(stat))
            .build()
    }

    /// The first status is a baseline: one snapshot, with no pin or float.
    /// Before it the floor is loading.
    #[test]
    fn the_first_status_is_a_baseline() {
        let mut floor = floor();
        floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        let loading = json_of(&floor.snapshot());

        let sent = poll(
            &mut floor,
            shop_answers(vec![process_status_with(
                "web",
                ProcessState::Running,
                Readiness::None,
                &[8080],
                2,
            )]),
        );

        assert_eq!(loading["health"], "loading");
        assert_eq!(kinds(&sent), ["snapshot"]);
        let snapshot = json_of(&sent[0]);
        assert_eq!(snapshot["health"], "running");
        assert_eq!(snapshot["here"], "shop");
        let process = &snapshot["projects"][0]["processes"][0];
        assert_eq!(process["room"], "running");
        assert_eq!(process["ports"], json!([8080]));
        assert_eq!(
            process["next"],
            json!({"verb": "attach", "command": "muster -p shop attach web"})
        );
    }

    /// A status read before the projects were is a baseline, and so is the
    /// first after them, so no process walks in from Off at start.
    #[test]
    fn a_status_before_the_projects_keeps_the_next_a_baseline() {
        let mut floor = floor();
        let running = || shop_answers(vec![web(ProcessState::Running, Readiness::None)]);

        let early = poll(&mut floor, running());
        let projects = floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        let first = poll(&mut floor, running());
        let next = poll(&mut floor, running());

        assert_eq!(kinds(&early), ["snapshot"]);
        assert_eq!(kinds(&projects), ["projects", "notes"]);
        assert_eq!(
            json_of(&projects[0])["projects"][0]["processes"][0]["room"],
            "running",
            "the projects arrive placed as the earlier status said"
        );
        assert_eq!(kinds(&first), ["snapshot"]);
        assert_eq!(
            json_of(&first[0])["projects"][0]["processes"][0]["room"],
            "running"
        );
        assert!(next.is_empty(), "{next:?}");
    }

    /// A crash, its restart and the run after, all logged after the
    /// baseline, pin fail, again and ok once each, in turn, as they are
    /// heard: whether heard before the status answer that shows the process
    /// running again, after it, or split around one, and no status answer
    /// pins anything more.
    #[test]
    fn a_crash_loop_pins_each_event_once_however_it_is_heard_around_status() {
        let running = || shop_answers(vec![web(ProcessState::Running, Readiness::None)]);
        let restarting = || shop_answers(vec![web(ProcessState::Restarting, Readiness::None)]);
        let turn = [
            EventKind::Crashed,
            EventKind::Restarting,
            EventKind::Started,
        ];

        let mut before = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let mut sent_before = before.hear_events(&heard(&turn));
        sent_before.extend(poll(&mut before, running()));
        sent_before.extend(poll(&mut before, running()));

        let mut after = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let logged = heard(&turn);
        let mut sent_after = poll(&mut after, running());
        sent_after.extend(after.hear_events(&logged));
        sent_after.extend(poll(&mut after, running()));

        let mut split = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let mut sent_split = split.hear_events(&heard(&turn[..1]));
        let late = heard(&turn[1..]);
        sent_split.extend(poll(&mut split, restarting()));
        sent_split.extend(split.hear_events(&late));
        sent_split.extend(poll(&mut split, running()));

        for sent in [&sent_before, &sent_after, &sent_split] {
            assert_eq!(
                pins_only(sent),
                [Pin::Fail, Pin::Again, Pin::Ok],
                "{sent:?}"
            );
        }
        assert_eq!(kinds(&sent_before[..3]), ["pin", "pin", "pin"]);
    }

    /// Events at or before the cursor of the answer that made a baseline
    /// are history it drew, so they pin nothing: heard before the first
    /// status, after it, replayed after a park, or held while the service
    /// was down and it came back running. One logged after that cursor but
    /// heard while the baseline was due pins once its snapshot is sent.
    #[test]
    fn events_a_baseline_drew_pin_nothing() {
        let running = || shop_answers(vec![web(ProcessState::Running, Readiness::None)]);

        let mut first = floor();
        first.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        let early = first.hear_events(&heard(&[EventKind::Crashed]));
        let logged = heard(&[EventKind::Restarting]);
        let sighted = poll(&mut first, running());
        let late = first.hear_events(&logged);

        let mut parked = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let replay = heard(&[EventKind::Crashed, EventKind::Started]);
        parked.rest();
        parked.gap();
        let replayed = parked.hear_events(&replay);
        let woken = poll(&mut parked, running());
        let again = parked.hear_events(&replay);

        let mut down = settled(vec![web(ProcessState::Running, Readiness::None)]);
        poll(&mut down, StatusReading::Down(ServiceDown::Absent));
        let held = down.hear_events(&heard(&[EventKind::Crashed]));
        let back = poll(&mut down, running());

        let mut resting = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let answer = running();
        let after_cursor = heard(&[EventKind::Crashed]);
        resting.rest();
        let waiting = resting.hear_events(&after_cursor);
        let baseline = poll(&mut resting, answer);

        for quiet in [&early, &late, &replayed, &again, &held, &waiting] {
            assert!(quiet.is_empty(), "{quiet:?}");
        }
        for snapshot in [&sighted, &woken, &back] {
            assert_eq!(kinds(snapshot), ["snapshot"]);
        }
        assert_eq!(kinds(&baseline), ["snapshot", "pin"]);
        assert_eq!(pins_only(&baseline), [Pin::Fail]);
    }

    /// The same event heard twice, as a page read again, pins once.
    #[test]
    fn an_event_heard_twice_pins_once() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let crash = heard(&[EventKind::Crashed]);

        let first = floor.hear_events(&crash);
        let again = floor.hear_events(&crash);

        assert_eq!(pins_only(&first), [Pin::Fail]);
        assert!(again.is_empty(), "{again:?}");
    }

    /// An answer of the next service's log makes a baseline, after which an
    /// event of the previous log pins nothing, held or heard.
    #[test]
    fn an_old_epoch_event_after_a_service_restart_pins_nothing() {
        let running = || {
            vec![project_status(SHOP, vec![web(
                ProcessState::Running,
                Readiness::None,
            )])]
        };
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let old = newest_event();
        let next_epoch = EventSeq::builder()
            .epoch(old.epoch() + 1)
            .seq(old.seq())
            .build();
        let logged = heard(&[EventKind::Crashed, EventKind::Started]);

        let new_service = poll(&mut floor, answered_at(running(), next_epoch));
        let stale = floor.hear_events(&logged);
        let fresh = floor.hear_events(&heard(&[EventKind::Crashed]));

        assert_eq!(kinds(&new_service), ["snapshot"]);
        assert!(stale.is_empty(), "{stale:?}");
        assert!(fresh.is_empty(), "{fresh:?}");
    }

    /// Events of a project or process not on the floor, and of kinds that
    /// pin nothing, are dropped.
    #[test]
    fn a_process_missing_from_the_floor_gets_no_pin() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let reading = EventsReading::builder()
            .events(vec![
                stack_event("ghost", "web", EventKind::Crashed),
                stack_event(SHOP, "nobody", EventKind::Crashed),
                stack_event(SHOP, "web", EventKind::Stopped),
                stack_event(SHOP, "web", EventKind::Exited),
            ])
            .gap(false)
            .build();

        let sent = floor.hear_events(&reading);
        let polled = poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Running, Readiness::None)]),
        );

        assert!(sent.is_empty(), "{sent:?}");
        assert!(polled.is_empty(), "{polled:?}");
    }

    /// A start into a readiness check pins nothing; its passed check pins
    /// ok, and a check out of time late.
    #[test]
    fn a_checked_start_pins_only_its_check() {
        let mut floor = floor();
        floor.read_projects(read(vec![checked_project_reading(SHOP, &["web"])]));
        poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Exited, Readiness::None)]),
        );

        let started = floor.hear_events(&heard(&[EventKind::Started]));
        let ready = floor.hear_events(&heard(&[EventKind::Ready]));
        let late = floor.hear_events(&heard(&[EventKind::ReadyTimeout]));

        assert!(started.is_empty(), "{started:?}");
        assert_eq!(pins_only(&ready), [Pin::Ok]);
        assert_eq!(pins_only(&late), [Pin::Late]);
    }

    /// Five failed asks in a row mark the service unresponsive, in one
    /// snapshot that keeps every process; an answer brings it back.
    #[test]
    fn failed_asks_in_a_row_make_the_service_unresponsive() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);

        let quiet: Vec<Vec<Sequenced>> = (1..MAX_FAILED_ASKS)
            .map(|_| poll(&mut floor, StatusReading::Failed))
            .collect();
        let unresponsive = poll(&mut floor, StatusReading::Failed);
        let still = poll(&mut floor, StatusReading::Failed);
        let back = poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Running, Readiness::None)]),
        );

        assert!(quiet.iter().all(Vec::is_empty), "{quiet:?}");
        assert_eq!(kinds(&unresponsive), ["snapshot"]);
        let snapshot = json_of(&unresponsive[0]);
        assert_eq!(snapshot["health"], "unresponsive");
        assert_eq!(snapshot["projects"][0]["processes"][0]["room"], "running");
        assert!(still.is_empty(), "{still:?}");
        assert_eq!(json_of(&back[0])["health"], "running");
    }

    /// A floor that rested while the service was already unresponsive is
    /// told so by the next failed ask, rather than loading for good.
    #[test]
    fn a_rested_floor_hears_an_unresponsive_service_at_the_next_failed_ask() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        for _ in 0..MAX_FAILED_ASKS {
            poll(&mut floor, StatusReading::Failed);
        }

        floor.rest();
        let rested = json_of(&floor.snapshot());
        let woken = poll(&mut floor, StatusReading::Failed);
        let still = poll(&mut floor, StatusReading::Failed);

        assert_eq!(rested["health"], "loading");
        assert_eq!(kinds(&woken), ["snapshot"]);
        let woken = json_of(&woken[0]);
        assert_eq!(woken["health"], "unresponsive");
        assert_eq!(
            woken["projects"][0]["processes"][0]["unconfirmed"],
            "service-down"
        );
        assert_eq!(floor.health(), Health::Unresponsive);
        assert!(still.is_empty(), "{still:?}");
    }

    /// A state that comes back after it held no database is a baseline:
    /// its projects arrive in a snapshot, and so does the next status, so no
    /// face walks in from Off.
    #[test]
    fn a_state_that_comes_back_makes_a_baseline() {
        let running = || shop_answers(vec![web(ProcessState::Running, Readiness::None)]);
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);

        floor.read_projects(ProjectsReading::Trouble(StateTrouble::NoState));
        poll(&mut floor, running());
        let back = floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        let next = poll(&mut floor, running());

        assert_eq!(kinds(&back), ["snapshot"]);
        assert_eq!(
            json_of(&back[0])["projects"][0]["processes"][0]["room"],
            "running",
            "the projects come back placed as the last status said"
        );
        assert_eq!(kinds(&next), ["snapshot"]);
    }

    /// Until the first status reading, the projects arrive with their
    /// processes held back, none drawn in Off; that reading's snapshot
    /// draws them in their rooms.
    #[test]
    fn processes_are_held_back_until_the_first_status() {
        let mut floor = floor();

        let projects = floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        let loading = json_of(&floor.snapshot());
        let first = poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Running, Readiness::None)]),
        );

        assert_eq!(json_of(&projects[0])["projects"][0]["processes"], json!([]));
        assert_eq!(loading["projects"][0]["processes"], json!([]));
        assert_eq!(kinds(&first), ["snapshot"]);
        assert_eq!(
            json_of(&first[0])["projects"][0]["processes"][0]["room"],
            "running"
        );
    }

    /// A project whose stack did not read is a floor of its own, marked
    /// unreadable and with no processes, beside the others.
    #[test]
    fn a_project_whose_stack_did_not_read_is_marked_unreadable() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);

        let sent = floor.read_projects(read(vec![
            project_reading(SHOP, &["web"]),
            ProjectReading::unread(&project(BLOG)),
        ]));

        assert_eq!(kinds(&sent), ["projects", "notes"]);
        let projects = json_of(&sent[0])["projects"].clone();
        assert_eq!(projects[0]["unreadable"], false);
        assert_eq!(projects[0]["processes"][0]["room"], "running");
        assert_eq!(projects[1]["name"], BLOG);
        assert_eq!(projects[1]["unreadable"], true);
        assert_eq!(projects[1]["processes"], json!([]));
    }

    /// A project whose stack stops reading keeps the processes it held,
    /// each in its room but unconfirmed, and sends nothing more while it
    /// stays unreadable; once it reads again they are confirmed in place,
    /// and none walks in from Off.
    #[test]
    fn a_stack_that_stops_reading_keeps_its_processes_unconfirmed() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);

        let broken = floor.read_projects(read(vec![ProjectReading::unread(&project(SHOP))]));
        let still = floor.read_projects(read(vec![ProjectReading::unread(&project(SHOP))]));
        let polled = poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Running, Readiness::None)]),
        );
        let mended = floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));

        assert_eq!(kinds(&broken), ["projects"]);
        let shop = json_of(&broken[0])["projects"][0].clone();
        assert_eq!(shop["unreadable"], true);
        assert_eq!(shop["processes"][0]["room"], "running");
        assert_eq!(shop["processes"][0]["unconfirmed"], "unreadable-stack");
        assert!(still.is_empty(), "{still:?}");
        assert!(polled.is_empty(), "{polled:?}");
        assert_eq!(kinds(&mended), ["projects"]);
        let shop = json_of(&mended[0])["projects"][0].clone();
        assert_eq!(shop["unreadable"], false);
        assert_eq!(shop["processes"][0]["room"], "running");
        assert_eq!(shop["processes"][0]["unconfirmed"], Value::Null);
    }

    /// A project new to the floor arrives with its notes, empty, so a page
    /// drops any it kept of a project of that name before.
    #[test]
    fn a_new_project_arrives_with_its_notes() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);

        let sent = floor.read_projects(read(vec![
            project_reading(SHOP, &["web"]),
            project_reading(BLOG, &["api"]),
        ]));

        assert_eq!(kinds(&sent), ["projects", "notes"]);
        let notes = json_of(&sent[1]);
        assert_eq!(notes["project"], BLOG);
        assert_eq!(notes["notes"], json!([]));
    }

    /// A project renamed in the same folder keeps its notes and sends them
    /// under its new name, since a page keeps notes by name and drops the
    /// old name's with the old project.
    #[test]
    fn a_renamed_project_sends_its_notes_under_the_new_name() {
        let mut floor = floor();
        floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        let note = Scratchpad::builder()
            .key(ScratchpadKey::try_new("plan").unwrap())
            .body("ship it".to_string())
            .author(Author::agent("claude"))
            .updated_at(EpochSeconds::new(100))
            .build();
        floor.read_notes(&root(SHOP), slice::from_ref(&note));
        let renamed = ProjectReading::builder()
            .project(
                Project::builder()
                    .root(root(SHOP))
                    .name(ProjectName::try_new("store").unwrap())
                    .build(),
            )
            .icon(ProcessIcon::Terminal)
            .processes(project_reading(SHOP, &["web"]).processes().clone())
            .unreadable(false)
            .build();

        let sent = floor.read_projects(read(vec![renamed.clone()]));
        let again = floor.read_projects(read(vec![renamed]));
        let unchanged = floor.read_notes(&root(SHOP), slice::from_ref(&note));

        assert_eq!(kinds(&sent), ["projects", "notes"]);
        let notes = json_of(&sent[1]);
        assert_eq!(notes["project"], "store");
        assert_eq!(notes["notes"][0]["key"], "plan");
        assert!(again.is_empty(), "{again:?}");
        assert!(unchanged.is_empty(), "{unchanged:?}");
    }

    /// A log's project is found by its name as written, so two projects
    /// whose names differ only in how their accents are encoded stay apart;
    /// its process is found by its name's key, however the request encoded
    /// it.
    #[test]
    fn a_log_is_found_by_its_project_name_and_its_process_key() {
        let mut floor = floor();
        floor.read_projects(read(vec![
            project_reading("caf\u{e9}", &["cr\u{e8}me"]),
            project_reading("cafe\u{301}", &["cr\u{e8}me"]),
        ]));

        let composed = floor.locate_log("caf\u{e9}", "cre\u{300}me").unwrap();
        let decomposed = floor.locate_log("cafe\u{301}", "cr\u{e8}me").unwrap();

        assert_eq!(composed.project().root(), &root("caf\u{e9}"));
        assert_eq!(decomposed.project().root(), &root("cafe\u{301}"));
        assert_eq!(floor.locate_log("cafe", "cr\u{e8}me"), None);
    }

    /// A status reading moves a process's face from room to room but pins
    /// nothing: into Running, Failed, Starting, Off or Restarting, and a
    /// check running out of time.
    #[test]
    fn a_status_change_moves_the_face_but_pins_nothing() {
        let steps = walk(&[
            (ProcessState::Starting, Readiness::None),
            (ProcessState::Running, Readiness::None),
            (ProcessState::Crashed, Readiness::None),
            (ProcessState::Starting, Readiness::None),
            (ProcessState::Exited, Readiness::None),
            (ProcessState::Restarting, Readiness::None),
        ]);
        let late = walk(&[
            (ProcessState::Running, Readiness::Waiting),
            (ProcessState::Running, Readiness::TimedOut),
        ]);

        let rooms: Vec<Option<Room>> = steps.iter().map(|sent| moved_to(sent)).collect();
        assert_eq!(rooms, [
            Some(Room::Running),
            Some(Room::Failed),
            Some(Room::Starting),
            Some(Room::Off),
            Some(Room::Restarting),
        ]);
        assert_eq!(moved_to(&late[0]), Some(Room::Running));
        for sent in steps.iter().chain(&late) {
            assert_eq!(kinds(sent), ["process"]);
        }
    }

    /// A crash and its restart, both between two polls that each see the
    /// process running, pin fail then ok as they are heard, and the poll
    /// after floats the restart.
    #[test]
    fn a_crash_between_polls_pins_as_heard_and_floats_the_restart() {
        let running = |restarts| {
            process_status_with("web", ProcessState::Running, Readiness::None, &[], restarts)
        };
        let mut floor = settled(vec![running(0)]);

        let pinned = floor.hear_events(&heard(&[EventKind::Crashed, EventKind::Started]));
        let sent = poll(&mut floor, shop_answers(vec![running(1)]));

        assert_eq!(pins(&pinned), [
            ("web".to_string(), Pin::Fail),
            ("web".to_string(), Pin::Ok)
        ]);
        assert_eq!(kinds(&sent), ["process", "float"]);
        assert_eq!(floats(&sent), [(FloatKind::Restart, None)]);
    }

    /// A page of events with a gap makes the next status a snapshot, with
    /// no pin for what it heard.
    #[test]
    fn a_gap_in_the_events_makes_a_snapshot() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let reading = EventsReading::builder()
            .events(vec![stack_event(SHOP, "web", EventKind::Crashed)])
            .gap(true)
            .build();

        let held = floor.hear_events(&reading);
        let sent = poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Crashed, Readiness::None)]),
        );

        assert!(held.is_empty(), "{held:?}");
        assert_eq!(kinds(&sent), ["snapshot"]);
    }

    /// A newly opened port floats, at most three at a time, and a rising
    /// restart count floats a restart.
    #[test]
    fn opened_ports_and_restarts_float() {
        let running = |ports: &[u16], restarts| {
            process_status_with(
                "web",
                ProcessState::Running,
                Readiness::None,
                ports,
                restarts,
            )
        };
        let mut floor = settled(vec![running(&[], 1)]);

        let opened = poll(&mut floor, shop_answers(vec![running(&[8080], 2)]));
        assert_eq!(floats(&opened), [
            (FloatKind::Port, Some(8080)),
            (FloatKind::Restart, None)
        ]);

        let many = poll(
            &mut floor,
            shop_answers(vec![running(&[8080, 9001, 9002, 9003, 9004], 2)]),
        );
        assert_eq!(floats(&many), [
            (FloatKind::Port, Some(9001)),
            (FloatKind::Port, Some(9002)),
            (FloatKind::Port, Some(9003)),
        ]);
        assert!(pins(&many).is_empty());
    }

    /// The start is sent as a time the page counts from: polls a second
    /// apart whose uptimes round differently keep it, and a new run moves
    /// it.
    #[test]
    fn the_start_stays_put_across_polls() {
        let at = |uptime| running_with_usage("web", Duration::from_secs(uptime), 1.0, 10);
        let now = Instant::now();
        let mut floor = floor();
        floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));

        let first = floor.read_status(shop_answers(vec![at(10)]), now, wall(100));
        let next = floor.read_status(shop_answers(vec![at(12)]), now, wall(101));
        let rerun = floor.read_status(shop_answers(vec![at(1)]), now, wall(120));

        assert_eq!(
            json_of(&first[0])["projects"][0]["processes"][0]["started"],
            1_000_090
        );
        assert!(next.is_empty(), "{next:?}");
        assert_eq!(kinds(&rerun), ["process"]);
        assert_eq!(json_of(&rerun[0])["process"]["started"], 1_000_119);
    }

    /// A change of usage alone goes out at most once every five seconds,
    /// carrying the latest usage, while any other change goes out at once.
    #[test]
    fn usage_alone_is_sent_at_most_every_five_seconds() {
        // A running child's uptime grows with the clock; a fixed uptime would
        // be a new run each reading.
        let with_cpu = |seconds: u64, cpu| {
            running_with_usage("web", Duration::from_secs(60 + seconds), cpu, 10)
        };
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut floor = floor();
        floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        floor.read_status(shop_answers(vec![with_cpu(0, 1.0)]), at(0), wall(0));

        let soon = floor.read_status(shop_answers(vec![with_cpu(1, 2.0)]), at(1), wall(1));
        let later = floor.read_status(shop_answers(vec![with_cpu(3, 3.0)]), at(3), wall(3));
        let due = floor.read_status(shop_answers(vec![with_cpu(5, 3.0)]), at(5), wall(5));
        let again = floor.read_status(shop_answers(vec![with_cpu(6, 4.0)]), at(6), wall(6));
        let crashed = floor.read_status(
            shop_answers(vec![web(ProcessState::Crashed, Readiness::None)]),
            at(7),
            wall(7),
        );

        assert!(soon.is_empty(), "{soon:?}");
        assert!(later.is_empty(), "{later:?}");
        assert_eq!(kinds(&due), ["process"]);
        assert_eq!(json_of(&due[0])["process"]["cpu"], 3.0);
        assert!(again.is_empty(), "{again:?}");
        assert_eq!(kinds(&crashed), ["process"]);
    }

    /// A CPU use that is no number reads as null, and stays quiet.
    #[test]
    fn a_cpu_that_is_no_number_reads_as_null() {
        let nan = |uptime| running_with_usage("web", Duration::from_secs(uptime), f32::NAN, 10);
        let start = Instant::now();
        let mut floor = floor();
        floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));

        let first = floor.read_status(shop_answers(vec![nan(60)]), start, wall(0));
        let later = floor.read_status(
            shop_answers(vec![nan(70)]),
            start + Duration::from_secs(10),
            wall(10),
        );

        let process = &json_of(&first[0])["projects"][0]["processes"][0];
        assert_eq!(process["cpu"], Value::Null);
        assert_eq!(process["memory"], 10);
        assert!(later.is_empty(), "{later:?}");
    }

    /// A gap sends exactly one snapshot, with no pin for what it missed.
    #[test]
    fn a_gap_sends_one_snapshot() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        floor.gap();

        let sent = poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Crashed, Readiness::None)]),
        );
        let after = poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Crashed, Readiness::None)]),
        );

        assert_eq!(kinds(&sent), ["snapshot"]);
        assert!(after.is_empty(), "{after:?}");
    }

    /// A service restart is a baseline: a snapshot when it goes, which
    /// keeps every process where it was read, unconfirmed, as its child runs
    /// on under its holder, and another when it is back with changes, and no
    /// pin.
    #[test]
    fn a_service_restart_is_a_baseline() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);

        let gone = poll(&mut floor, StatusReading::Down(ServiceDown::Absent));
        let back = poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Crashed, Readiness::None)]),
        );

        assert_eq!(kinds(&gone), ["snapshot"]);
        let gone = json_of(&gone[0]);
        assert_eq!(gone["health"], "absent");
        let kept = &gone["projects"][0]["processes"][0];
        assert_eq!(kept["room"], "running");
        assert_eq!(kept["mark"], Value::Null);
        assert_eq!(kept["unconfirmed"], "service-down");
        assert_eq!(kept["next"]["verb"], "attach");
        assert_eq!(kinds(&back), ["snapshot"]);
        let back = json_of(&back[0]);
        assert_eq!(back["health"], "running");
        let back = &back["projects"][0]["processes"][0];
        assert_eq!(back["room"], "failed");
        assert_eq!(back["mark"], Value::Null);
        assert_eq!(back["unconfirmed"], Value::Null);
    }

    /// While no service answers, each process keeps its own room, word, mark
    /// and next command, only drawn unconfirmed: a paused one stays paused
    /// and is not taken for unsupervised, and one the service never placed
    /// stays off, unconfirmed too.
    #[test]
    fn an_absent_service_keeps_each_mark_and_confirms_nothing() {
        let mut floor = settled(vec![
            web(ProcessState::Running, Readiness::None),
            process_status("worker", ProcessState::Crashed, Readiness::None),
            process_status("cron", ProcessState::Paused, Readiness::None),
        ]);
        floor.read_projects(read(vec![
            project_reading(SHOP, &["web", "worker", "cron"]),
            project_reading(BLOG, &["api"]),
        ]));

        let down = poll(&mut floor, StatusReading::Down(ServiceDown::Absent));

        let snapshot = json_of(&down[0]);
        let shop = &snapshot["projects"][0]["processes"];
        let face = |at: usize| {
            let face = &shop[at];
            (
                face["room"].clone(),
                face["mark"].clone(),
                face["state"].clone(),
                face["next"]["verb"].clone(),
            )
        };
        assert_eq!(
            face(0),
            (
                json!("running"),
                Value::Null,
                json!("running"),
                json!("attach")
            )
        );
        assert_eq!(
            face(1),
            (
                json!("failed"),
                Value::Null,
                json!("crashed"),
                json!("logs")
            )
        );
        assert_eq!(
            face(2),
            (
                json!("off"),
                json!("paused"),
                json!("paused"),
                json!("resume")
            )
        );
        let blog = &snapshot["projects"][1]["processes"][0];
        assert_eq!(blog["room"], "off");
        for process in shop.as_array().unwrap().iter().chain([blog]) {
            assert_eq!(process["unconfirmed"], "service-down", "{process}");
        }
    }

    /// A floor that rested reads as loading with every process unconfirmed
    /// but in its room, until the next status reading, a baseline, confirms
    /// them again; a service down then is sent at once, though it is the
    /// health the floor last read.
    #[test]
    fn a_rested_floor_confirms_nothing_until_the_next_status() {
        let running = || shop_answers(vec![web(ProcessState::Running, Readiness::None)]);
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);

        floor.rest();
        let rested = json_of(&floor.snapshot());
        let woken = poll(&mut floor, running());
        poll(&mut floor, StatusReading::Down(ServiceDown::Absent));
        floor.rest();
        let down = poll(&mut floor, StatusReading::Down(ServiceDown::Absent));

        assert_eq!(rested["health"], "loading");
        let face = &rested["projects"][0]["processes"][0];
        assert_eq!(face["room"], "running");
        assert_eq!(face["unconfirmed"], "resting");
        assert_eq!(kinds(&woken), ["snapshot"]);
        let woken = json_of(&woken[0]);
        assert_eq!(woken["health"], "running");
        assert_eq!(
            woken["projects"][0]["processes"][0]["unconfirmed"],
            Value::Null
        );
        assert_eq!(kinds(&down), ["snapshot"]);
        let down = json_of(&down[0]);
        assert_eq!(down["health"], "absent");
        assert_eq!(
            down["projects"][0]["processes"][0]["unconfirmed"],
            "service-down"
        );
    }

    /// While the service starts or another version runs, no process walks:
    /// each keeps the room it was last read in, unconfirmed, and the logs of
    /// the running ones are still watched.
    #[test]
    fn a_service_down_keeps_every_process_where_it_was_read() {
        for health in [ServiceDown::Starting, ServiceDown::OtherVersion] {
            let mut floor = settled(vec![
                web(ProcessState::Running, Readiness::None),
                process_status("worker", ProcessState::Crashed, Readiness::None),
            ]);

            let down = poll(&mut floor, StatusReading::Down(health));

            let processes = json_of(&down[0])["projects"][0]["processes"].clone();
            assert_eq!(processes[0]["room"], "running", "{health:?}");
            assert_eq!(processes[1]["room"], "failed", "{health:?}");
            assert!(
                processes
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|process| process["unconfirmed"] == "service-down"),
                "{processes}"
            );
            assert_eq!(floor.log_watches().len(), 1, "{health:?}");
        }
    }

    /// Each other health of the service is a snapshot once, and a failed
    /// ask changes nothing.
    #[test]
    fn each_service_health_is_a_snapshot_and_a_failed_ask_nothing() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);

        assert!(poll(&mut floor, StatusReading::Failed).is_empty());
        let starting = poll(&mut floor, StatusReading::Down(ServiceDown::Starting));
        let other = poll(&mut floor, StatusReading::Down(ServiceDown::OtherVersion));
        let again = poll(&mut floor, StatusReading::Down(ServiceDown::OtherVersion));

        assert_eq!(json_of(&starting[0])["health"], "starting");
        assert_eq!(json_of(&other[0])["health"], "other-version");
        assert!(again.is_empty(), "{again:?}");
    }

    /// A project the service has not loaded shows every process off, and a
    /// status of a project not on the floor is ignored.
    #[test]
    fn an_unloaded_project_shows_every_process_off() {
        let mut floor = floor();
        floor.read_projects(read(vec![
            project_reading(SHOP, &["web"]),
            project_reading(BLOG, &["api", "worker"]),
        ]));
        let with_ghost = |api: ProcessState| {
            answered(vec![
                project_status(SHOP, vec![web(ProcessState::Running, Readiness::None)]),
                project_status("ghost", vec![process_status("api", api, Readiness::None)]),
            ])
        };

        let sent = poll(&mut floor, with_ghost(ProcessState::Running));
        let later = poll(&mut floor, with_ghost(ProcessState::Crashed));

        let snapshot = json_of(&sent[0]);
        let projects = snapshot["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0]["processes"][0]["room"], "running");
        let blog = projects[1]["processes"].as_array().unwrap();
        assert_eq!(blog.len(), 2);
        for process in blog {
            assert_eq!(process["room"], "off");
            assert_eq!(process["state"], "pending");
        }
        assert!(later.is_empty(), "{later:?}");
    }

    /// A process the first status reading after its arrival places earns no
    /// float.
    #[test]
    fn a_process_seen_for_the_first_time_floats_nothing() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let added = floor.read_projects(read(vec![project_reading(SHOP, &["web", "worker"])]));

        let sent = poll(
            &mut floor,
            shop_answers(vec![
                web(ProcessState::Running, Readiness::None),
                process_status_with("worker", ProcessState::Running, Readiness::None, &[8080], 1),
            ]),
        );

        assert_eq!(kinds(&added), ["projects"]);
        assert_eq!(kinds(&sent), ["process"]);
        assert_eq!(moved_to(&sent), Some(Room::Running));
    }

    /// A process message carries every field the page reads, as plain JSON.
    #[test]
    fn a_process_message_carries_the_page_fields() {
        let sent = walk(&[
            (ProcessState::Starting, Readiness::None),
            (ProcessState::Running, Readiness::None),
        ])
        .remove(0);

        let message = json_of(&sent[0]);
        assert_eq!(message["kind"], "process");
        assert_eq!(message["project"], "shop");
        let process = message["process"].as_object().unwrap();
        let mut keys: Vec<&str> = process.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, [
            "attached",
            "cpu",
            "ended",
            "icon",
            "mark",
            "memory",
            "name",
            "next",
            "ports",
            "restarts",
            "room",
            "speaking",
            "started",
            "state",
            "unconfirmed",
        ]);
        assert_eq!(process["unconfirmed"], Value::Null);
        assert_eq!(process["name"], "web");
        assert_eq!(process["next"]["command"], "muster -p shop attach web");
        assert_eq!(process["icon"], "terminal");
        assert_eq!(process["state"], "running");
        assert_eq!(process["mark"], Value::Null);
        assert_eq!(process["speaking"], false);
    }

    /// A running process whose log changed speaks, and falls quiet 2 s on;
    /// a log that stays still sends nothing, and one of a process that does
    /// not run never speaks.
    #[test]
    fn log_growth_speaks_for_two_seconds() {
        let mut floor = settled(vec![
            web(ProcessState::Running, Readiness::None),
            process_status("worker", ProcessState::Exited, Readiness::None),
        ]);
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);

        let first = floor.read_logs(at(0), &[
            logged("web", stat(10, None)),
            logged("worker", stat(10, None)),
        ]);
        let grew = floor.read_logs(at(1), &[
            logged("web", stat(20, None)),
            logged("worker", stat(20, None)),
        ]);
        let speaking =
            json_of(&floor.snapshot())["projects"][0]["processes"][0]["speaking"].clone();
        let still = floor.read_logs(at(2), &[logged("web", stat(20, None))]);
        let quiet = floor.read_logs(at(3), &[logged("web", stat(20, None))]);
        let touched = floor.read_logs(at(4), &[logged("web", stat(20, Some(UNIX_EPOCH)))]);

        assert!(first.is_empty(), "{first:?}");
        assert_eq!(kinds(&grew), ["speaking"]);
        assert_eq!(json_of(&grew[0])["name"], "web");
        assert_eq!(json_of(&grew[0])["on"], true);
        assert_eq!(speaking, true);
        assert!(still.is_empty(), "{still:?}");
        assert_eq!(kinds(&quiet), ["speaking"]);
        assert_eq!(json_of(&quiet[0])["on"], false);
        assert_eq!(json_of(&touched[0])["on"], true);
    }

    /// A floor at rest falls quiet and takes the next stat of each log as it
    /// finds it, so a log that grew while no page watched speaks of nothing.
    #[test]
    fn a_floor_at_rest_falls_quiet_and_forgets_its_logs() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let start = Instant::now();
        floor.read_logs(start, &[logged("web", stat(10, None))]);
        floor.read_logs(start, &[logged("web", stat(20, None))]);

        floor.rest();
        let snapshot = json_of(&floor.snapshot());
        let woken = floor.read_logs(start, &[logged("web", stat(90, None))]);

        assert_eq!(snapshot["projects"][0]["processes"][0]["speaking"], false);
        assert!(woken.is_empty(), "{woken:?}");
    }

    /// A process that starts speaking between polls sends no process
    /// message for it at the next poll.
    #[test]
    fn speaking_alone_sends_no_process_message() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let start = Instant::now();
        floor.read_logs(start, &[logged("web", stat(10, None))]);
        floor.read_logs(start + Duration::from_secs(1), &[logged(
            "web",
            stat(20, None),
        )]);

        let sent = poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Running, Readiness::None)]),
        );

        assert!(sent.is_empty(), "{sent:?}");
    }

    /// Only running processes are watched, each at its log.
    #[test]
    fn only_running_processes_are_watched() {
        let floor = settled(vec![
            web(ProcessState::Running, Readiness::None),
            process_status("worker", ProcessState::Exited, Readiness::None),
        ]);

        let watches = floor.log_watches();

        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0].name(), &process_name("web"));
        assert_eq!(
            watches[0].path(),
            &holder_dirs().process_log(&root(SHOP), &process_name("web"))
        );
    }

    /// A note change sends only its project's notes, once.
    #[test]
    fn notes_send_only_the_changed_project() {
        let mut floor = floor();
        floor.read_projects(read(vec![
            project_reading(SHOP, &["web"]),
            project_reading(BLOG, &["api"]),
        ]));
        let note = Scratchpad::builder()
            .key(ScratchpadKey::try_new("plan").unwrap())
            .body("ship <b>it</b>".to_string())
            .author(Author::agent("claude-code@builder"))
            .updated_at(EpochSeconds::new(100))
            .build();

        let sent = floor.read_notes(&root(SHOP), slice::from_ref(&note));
        let same = floor.read_notes(&root(SHOP), slice::from_ref(&note));
        let blog = floor.read_notes(&root(BLOG), &[]);
        let ghost = floor.read_notes(&root("ghost"), slice::from_ref(&note));
        let snapshot = json_of(&floor.snapshot());

        assert_eq!(kinds(&sent), ["notes"]);
        let message = json_of(&sent[0]);
        assert_eq!(message["project"], "shop");
        assert_eq!(
            message["notes"],
            json!([{
                "key": "plan",
                "body": "ship <b>it</b>",
                "author": "claude",
                "label": "claude-code@builder",
                "at": 100,
            }])
        );
        assert!(same.is_empty(), "{same:?}");
        assert!(blog.is_empty(), "{blog:?}");
        assert!(ghost.is_empty(), "{ghost:?}");
        assert_eq!(snapshot["notes"]["shop"][0]["key"], "plan");
        assert_eq!(snapshot["notes"]["blog"], json!([]));
    }

    /// A removed project leaves the floor, and its logs can no longer be
    /// found; a log is found only for a process on the floor, named as its
    /// stack names it.
    #[test]
    fn a_removed_project_leaves_the_floor() {
        let mut floor = floor();
        floor.read_projects(read(vec![
            project_reading(SHOP, &["web"]),
            project_reading(BLOG, &["api"]),
        ]));
        let before = floor.locate_log(BLOG, "api");

        let sent = floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));

        assert!(before.is_some());
        assert_eq!(kinds(&sent), ["projects"]);
        let projects = json_of(&sent[0])["projects"].clone();
        assert_eq!(projects.as_array().unwrap().len(), 1);
        assert_eq!(projects[0]["name"], "shop");
        assert_eq!(floor.locate_log(BLOG, "api"), None);
        let web = floor.locate_log(SHOP, "web").unwrap();
        assert_eq!(
            web.path(),
            &holder_dirs().process_log(&root(SHOP), &process_name("web"))
        );
        assert_eq!(web.project().root(), &root(SHOP));
        assert_eq!(web.name(), &process_name("web"));
        assert_eq!(floor.locate_log(SHOP, "../web"), None);
        assert_eq!(floor.locate_log(SHOP, ""), None);
    }

    /// A stack or an icon change sends the projects; an unchanged reading
    /// sends nothing.
    #[test]
    fn a_stack_or_icon_change_sends_the_projects() {
        let mut floor = floor();

        let first = floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        poll(&mut floor, StatusReading::Down(ServiceDown::Absent));
        let same = floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        let icon = floor.read_projects(read(vec![project_reading_with(
            SHOP,
            &["web"],
            ProcessIcon::Rust,
        )]));
        let stack = floor.read_projects(read(vec![project_reading_with(
            SHOP,
            &["web", "db"],
            ProcessIcon::Rust,
        )]));

        assert_eq!(kinds(&first), ["projects", "notes"]);
        assert_eq!(json_of(&first[0])["here"], "shop");
        assert!(same.is_empty(), "{same:?}");
        assert_eq!(kinds(&icon), ["projects"]);
        assert_eq!(json_of(&icon[0])["projects"][0]["icon"], "rust");
        assert_eq!(kinds(&stack), ["projects"]);
        assert_eq!(
            json_of(&stack[0])["projects"][0]["processes"][1]["name"],
            "db"
        );
    }

    /// State that cannot be read is a change of health, sent as a snapshot;
    /// a passing failure to read keeps the projects.
    #[test]
    fn trouble_with_the_state_is_a_snapshot() {
        let mut floor = floor();
        floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));

        let missing = floor.read_projects(ProjectsReading::Trouble(StateTrouble::NoState));
        let newer = floor.read_projects(ProjectsReading::Trouble(StateTrouble::NewerSchema));
        let back = floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        let unreadable = floor.read_projects(ProjectsReading::Trouble(StateTrouble::Unreadable));

        assert_eq!(kinds(&missing), ["snapshot"]);
        assert_eq!(json_of(&missing[0])["health"], "no-state");
        assert_eq!(json_of(&missing[0])["projects"], json!([]));
        assert_eq!(json_of(&newer[0])["health"], "newer-schema");
        assert_eq!(json_of(&back[0])["health"], "loading");
        assert_eq!(json_of(&back[0])["projects"][0]["name"], "shop");
        assert_eq!(json_of(&unreadable[0])["health"], "unreadable");
        assert_eq!(json_of(&unreadable[0])["projects"][0]["name"], "shop");
    }

    /// Every message takes the next sequence number, a page's own snapshot
    /// too.
    #[test]
    fn sequence_numbers_rise_by_one() {
        let mut floor = floor();

        let mut sent = floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        sent.extend(poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Starting, Readiness::None)]),
        ));
        sent.push(floor.snapshot());
        sent.extend(poll(
            &mut floor,
            shop_answers(vec![web(ProcessState::Running, Readiness::None)]),
        ));

        let seqs: Vec<u64> = sent.iter().map(Sequenced::seq).collect();
        assert_eq!(seqs, [1, 2, 3, 4, 5, 6]);
    }

    /// A project under the home folder shows its path from `~`, and one
    /// elsewhere shows it whole; a view started outside every project has
    /// no `here`.
    #[test]
    fn the_home_folder_shows_as_a_tilde() {
        let mut floor = floor();
        floor.read_projects(read(vec![project_reading(SHOP, &["web"])]));
        let mut elsewhere = Floor::new(PathBuf::from(HOME), None, holder_dirs());
        elsewhere.read_projects(read(vec![project_reading(SHOP, &["web"])]));

        let home = json_of(&floor.snapshot());
        let whole = json_of(&elsewhere.snapshot());

        assert_eq!(home["projects"][0]["path"], "~/shop");
        assert_eq!(whole["projects"][0]["path"], "/home/me/shop");
        assert_eq!(whole["here"], Value::Null);
    }

    /// A project whose name `-p` would take for a folder under wherever the
    /// command is pasted is named by its own folder, from `~`, in each card's
    /// command, while a plain name stays its name.
    #[test]
    fn a_project_named_like_a_folder_is_named_by_its_folder() {
        const FOLDER_LIKE: &str = ".devcontainer";
        let running = || vec![web(ProcessState::Running, Readiness::None)];
        let mut floor = floor();
        floor.read_projects(read(vec![
            project_reading(SHOP, &["web"]),
            project_reading(FOLDER_LIKE, &["web"]),
        ]));

        let sent = poll(
            &mut floor,
            answered(vec![
                project_status(SHOP, running()),
                project_status(FOLDER_LIKE, running()),
            ]),
        );

        let projects = json_of(&sent[0])["projects"].clone();
        let command = |at: usize| projects[at]["processes"][0]["next"]["command"].clone();
        assert_eq!(command(0), "muster -p shop attach web");
        assert_eq!(command(1), "muster -p '~/.devcontainer' attach web");
    }
}
