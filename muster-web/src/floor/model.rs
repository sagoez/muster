use std::{
    mem,
    path::PathBuf,
    time::{Duration, Instant},
};

use muster_client::listening_ports;
use muster_domain::{
    coordination::Scratchpad,
    value::{ProcessName, ProjectName, ProjectRoot},
};
use muster_protocol::{holder::HolderDirs, service::ProcessStatus};

use super::{
    message::{FloatKind, FloorMessage, Health, Pin, Sequenced},
    occupant::Occupant,
    reading::{LogReading, LogWatch, ProjectReading, ProjectsReading, StatusReading},
    tenant::Tenant,
    view::{FloorView, NoteView},
};

/// How long a process speaks after its log last changed.
const SPEAKING_FOR: Duration = Duration::from_secs(2);
/// Most ports one process floats in one reading, so a server opening many at
/// once does not bury its face.
const MAX_PORT_FLOATS: usize = 3;

/// Every project's floor as the page should show it, and what each new
/// reading changes of it, as numbered messages. Pure: it does no I/O and
/// is told the time.
pub struct Floor {
    /// The folder the view was started in, whose project is `here`.
    cwd: PathBuf,
    /// The home folder, which paths show as `~`.
    home: Option<PathBuf>,
    /// Where each process's log lives.
    holder_dirs: HolderDirs,
    /// The sequence number of the last message.
    seq: u64,
    /// How the service stood at the last status reading.
    service: Health,
    /// What is wrong with the state, if anything.
    store: Option<Health>,
    /// Whether the next status reading is a baseline, sent whole as a
    /// snapshot with no pin or float for what changed.
    baseline: bool,
    /// The projects, in the store's order.
    tenants: Vec<Tenant>,
}

impl Floor {
    /// An empty floor for a view started in `cwd`, that shows paths under
    /// `home` from `~` and finds logs through `holder_dirs`. Until a status
    /// reading says otherwise, no service runs.
    #[must_use]
    pub fn new(cwd: PathBuf, home: Option<PathBuf>, holder_dirs: HolderDirs) -> Self {
        Self {
            cwd,
            home,
            holder_dirs,
            seq: 0,
            service: Health::Absent,
            store: None,
            baseline: true,
            tenants: Vec::new(),
        }
    }

    /// How the view's sources stand: what is wrong with the state, else how
    /// the service stands.
    #[must_use]
    pub fn health(&self) -> Health {
        self.store.unwrap_or(self.service)
    }

    /// Everything, as one snapshot for a page that has just connected.
    pub fn snapshot(&mut self) -> Sequenced {
        let notes = self
            .tenants
            .iter()
            .map(|tenant| (tenant.project.name().to_string(), tenant.notes.clone()))
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
    /// state's health changed, else the projects when one was added or
    /// removed, a stack or an icon changed, or `here` moved.
    pub fn projects(&mut self, reading: ProjectsReading) -> Vec<Sequenced> {
        let (health, here) = (self.health(), self.here());
        let changed = match reading {
            ProjectsReading::Read(projects) => {
                self.store = None;
                self.settle(&projects)
            },
            ProjectsReading::NoState => self.clear(Health::NoState),
            ProjectsReading::NewerSchema => self.clear(Health::NewerSchema),
            ProjectsReading::Unreadable => {
                self.store = Some(Health::Unreadable);
                false
            },
        };
        if self.health() != health {
            return vec![self.snapshot()];
        }
        if !changed && self.here() == here {
            return Vec::new();
        }
        let projects = FloorMessage::Projects {
            here: self.here(),
            projects: self.views(),
        };
        vec![self.sequenced(projects)]
    }

    /// Takes in what asking the service for status found: one snapshot for
    /// a baseline, else each process that changed, its pin and its floats.
    /// The first reading, a gap, and a change of the service's health make
    /// a baseline. A failed ask says nothing, so it changes nothing.
    pub fn status(&mut self, reading: StatusReading) -> Vec<Sequenced> {
        let (service, statuses) = match reading {
            StatusReading::Failed => return Vec::new(),
            StatusReading::Answered(statuses) => (Health::Running, statuses),
            StatusReading::Absent => (Health::Absent, Vec::new()),
            StatusReading::Starting => (Health::Starting, Vec::new()),
            StatusReading::OtherVersion => (Health::OtherVersion, Vec::new()),
        };
        let baseline = mem::take(&mut self.baseline) || service != self.service;
        self.service = service;
        let mut messages = Vec::new();
        for tenant in &mut self.tenants {
            let project = statuses
                .iter()
                .find(|status| status.project().root() == tenant.project.root());
            for occupant in &mut tenant.occupants {
                let key = occupant.name.key();
                let status = project.and_then(|project| {
                    project
                        .processes()
                        .iter()
                        .find(|status| status.name().key() == key)
                });
                let before = occupant.face();
                let was = mem::replace(&mut occupant.status, status.cloned());
                let seen = mem::replace(&mut occupant.seen, true);
                if baseline {
                    continue;
                }
                let name = tenant.project.name();
                let after = occupant.face();
                if after != before {
                    messages.push(FloorMessage::Process {
                        project: name.clone(),
                        process: after,
                    });
                }
                if seen {
                    messages.extend(changes(name, &occupant.name, was.as_ref(), status));
                }
            }
        }
        if baseline {
            return vec![self.snapshot()];
        }
        messages
            .into_iter()
            .map(|message| self.sequenced(message))
            .collect()
    }

    /// Makes the next status reading a baseline: events were missed, or the
    /// service restarted, so the changes it shows are not ones anyone saw.
    pub fn gap(&mut self) {
        self.baseline = true;
    }

    /// Takes in the notes of the project at `root`: its notes when they
    /// changed, nothing for a project not on the floor.
    pub fn notes(&mut self, root: &ProjectRoot, notes: &[Scratchpad]) -> Vec<Sequenced> {
        let notes: Vec<NoteView> = notes.iter().map(NoteView::from).collect();
        let Some(tenant) = self
            .tenants
            .iter_mut()
            .find(|tenant| tenant.project.root() == root)
        else {
            return Vec::new();
        };
        if tenant.notes == notes {
            return Vec::new();
        }
        tenant.notes.clone_from(&notes);
        let project = tenant.project.name().clone();
        vec![self.sequenced(FloorMessage::Notes { project, notes })]
    }

    /// Takes in the logs' stats at `now`: a running process whose log
    /// changed since its last stat speaks until [`SPEAKING_FOR`] after, and
    /// one whose time is up falls quiet. Called each time the logs are
    /// statted, even with none to stat, so speaking ends on time.
    pub fn logs(&mut self, now: Instant, readings: &[LogReading]) -> Vec<Sequenced> {
        let until = now.checked_add(SPEAKING_FOR).unwrap_or(now);
        let mut messages = Vec::new();
        for reading in readings {
            let Some(tenant) = self
                .tenants
                .iter_mut()
                .find(|tenant| tenant.project.name() == reading.project())
            else {
                continue;
            };
            let project = tenant.project.name().clone();
            let Some(occupant) = tenant.occupant_mut(reading.name()) else {
                continue;
            };
            let before = occupant.log.replace(*reading.stat());
            let printed = before.is_some_and(|before| before != *reading.stat());
            if printed && occupant.runs() && occupant.speaking_until.replace(until).is_none() {
                messages.push(speaking(&project, occupant, true));
            }
        }
        for tenant in &mut self.tenants {
            for occupant in &mut tenant.occupants {
                if occupant.speaking_until.is_some_and(|until| until <= now) {
                    occupant.speaking_until = None;
                    messages.push(speaking(tenant.project.name(), occupant, false));
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
    pub fn log_watches(&self) -> Vec<LogWatch> {
        self.tenants
            .iter()
            .flat_map(|tenant| {
                tenant
                    .occupants
                    .iter()
                    .filter(|occupant| occupant.runs())
                    .map(|occupant| {
                        LogWatch::builder()
                            .project(tenant.project.name().clone())
                            .name(occupant.name.clone())
                            .path(
                                self.holder_dirs
                                    .process_log(tenant.project.root(), &occupant.name),
                            )
                            .build()
                    })
            })
            .collect()
    }

    /// The log of the process named `name` in the project named `project`,
    /// both as a request wrote them; none unless that process is on the
    /// floor. The path comes from the floor's own project and process, never
    /// from the request's text.
    #[must_use]
    pub fn log_path(&self, project: &str, name: &str) -> Option<PathBuf> {
        let name = ProcessName::try_new(name).ok()?;
        let tenant = self
            .tenants
            .iter()
            .find(|tenant| tenant.project.name().as_ref() == project)?;
        let occupant = tenant.occupant(&name)?;
        Some(
            self.holder_dirs
                .process_log(tenant.project.root(), &occupant.name),
        )
    }

    /// Replaces the projects with `projects`, keeping what the floor knew of
    /// each one still there, and says whether the page draws them otherwise.
    fn settle(&mut self, projects: &[ProjectReading]) -> bool {
        let changed = self.tenants.len() != projects.len()
            || self
                .tenants
                .iter()
                .zip(projects)
                .any(|(tenant, reading)| !tenant.shows_as(reading));
        let mut before = mem::take(&mut self.tenants);
        self.tenants = projects
            .iter()
            .map(|reading| {
                let kept = before
                    .iter()
                    .position(|tenant| tenant.project.root() == reading.project().root())
                    .map(|at| before.swap_remove(at));
                Tenant::of(reading, kept)
            })
            .collect();
        changed
    }

    /// Empties the floor for a state that holds no projects it can read,
    /// marked `trouble`, and says whether any project left.
    fn clear(&mut self, trouble: Health) -> bool {
        self.store = Some(trouble);
        let had = !self.tenants.is_empty();
        self.tenants.clear();
        had
    }

    /// The project holding the folder the view was started in: the deepest
    /// one, when projects nest.
    fn here(&self) -> Option<ProjectName> {
        self.tenants
            .iter()
            .filter(|tenant| tenant.project.root().contains(&self.cwd))
            .max_by_key(|tenant| tenant.project.root().as_ref().components().count())
            .map(|tenant| tenant.project.name().clone())
    }

    /// Every project's floor as the page draws it.
    fn views(&self) -> Vec<FloorView> {
        let home = self.home.as_deref();
        self.tenants
            .iter()
            .map(|tenant| tenant.view(home))
            .collect()
    }

    /// `message` with the next sequence number.
    fn sequenced(&mut self, message: FloorMessage) -> Sequenced {
        self.seq += 1;
        Sequenced::builder().seq(self.seq).message(message).build()
    }
}

/// The pin and floats process `name` of `project` earns going from `was` to
/// `now`: each port it newly listens on, up to [`MAX_PORT_FLOATS`], and a
/// restart when its count rose.
fn changes(
    project: &ProjectName,
    name: &ProcessName,
    was: Option<&ProcessStatus>,
    now: Option<&ProcessStatus>,
) -> Vec<FloorMessage> {
    let pin = Pin::between(was, now).map(|pin| FloorMessage::Pin {
        project: project.clone(),
        name: name.clone(),
        pin,
    });
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
    pin.into_iter()
        .chain(ports)
        .chain(restarted.then(|| float(FloatKind::Restart, None)))
        .collect()
}

/// That `occupant` of `project` started or stopped speaking.
fn speaking(project: &ProjectName, occupant: &Occupant, on: bool) -> FloorMessage {
    FloorMessage::Speaking {
        project: project.clone(),
        name: occupant.name.clone(),
        on,
    }
}

#[cfg(test)]
mod tests {
    use std::{slice, time::SystemTime};

    use muster_domain::{
        coordination::{Author, ScratchpadKey},
        process::{ProcessIcon, ProcessState, Readiness},
        value::EpochSeconds,
    };
    use serde_json::{Value, json};

    use super::*;
    use crate::floor::{
        fixture::{
            HOME, floor, holder_dirs, process_status, process_status_with, project_reading,
            project_reading_with, project_status, root,
        },
        reading::LogStat,
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
    /// answers it.
    fn shop_answers(processes: Vec<ProcessStatus>) -> StatusReading {
        StatusReading::Answered(vec![project_status(SHOP, processes)])
    }

    /// The projects read as `projects`.
    fn read(projects: Vec<ProjectReading>) -> ProjectsReading {
        ProjectsReading::Read(projects)
    }

    /// `web` in `state` with readiness `ready`.
    fn web(state: ProcessState, ready: Readiness) -> ProcessStatus {
        process_status("web", state, ready)
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
        floor.projects(read(vec![project_reading(SHOP, &names)]));
        floor.status(shop_answers(statuses));
        floor
    }

    /// What each step of `web` walking through `steps` sends, past the
    /// baseline of its first step.
    fn walk(steps: &[(ProcessState, Readiness)]) -> Vec<Vec<Sequenced>> {
        let ((state, ready), rest) = steps.split_first().unwrap();
        let mut floor = settled(vec![web(*state, *ready)]);
        rest.iter()
            .map(|(state, ready)| floor.status(shop_answers(vec![web(*state, *ready)])))
            .collect()
    }

    /// A stat of a log `size` bytes long, last written at `modified`.
    fn stat(size: u64, modified: Option<SystemTime>) -> LogStat {
        LogStat::builder().size(size).modified(modified).build()
    }

    /// Shop's process `name` found its log as `stat`.
    fn logged(name: &str, stat: LogStat) -> LogReading {
        LogReading::builder()
            .project(ProjectName::try_new(SHOP).unwrap())
            .name(process_name(name))
            .stat(Some(stat))
            .build()
    }

    /// The first status is a baseline: one snapshot, with no pin or float.
    #[test]
    fn the_first_status_is_a_baseline() {
        let mut floor = floor();
        floor.projects(read(vec![project_reading(SHOP, &["web"])]));

        let sent = floor.status(shop_answers(vec![process_status_with(
            "web",
            ProcessState::Running,
            Readiness::None,
            &[8080],
            2,
        )]));

        assert_eq!(kinds(&sent), ["snapshot"]);
        let snapshot = json_of(&sent[0]);
        assert_eq!(snapshot["health"], "running");
        assert_eq!(snapshot["here"], "shop");
        let process = &snapshot["projects"][0]["processes"][0];
        assert_eq!(process["room"], "running");
        assert_eq!(process["ports"], json!([8080]));
    }

    /// Starting to Running sends the process in its new room, then an ok pin.
    #[test]
    fn a_state_change_sends_the_process_then_its_pin() {
        let sent = walk(&[
            (ProcessState::Starting, Readiness::None),
            (ProcessState::Running, Readiness::None),
        ])
        .remove(0);

        assert_eq!(kinds(&sent), ["process", "pin"]);
        assert_eq!(moved_to(&sent), Some(Room::Running));
        assert_eq!(pins(&sent), [("web".to_string(), Pin::Ok)]);
    }

    /// Into Failed pins fail; into Starting or Off pins nothing.
    #[test]
    fn into_failed_pins_fail_and_into_starting_or_off_nothing() {
        let steps = walk(&[
            (ProcessState::Running, Readiness::None),
            (ProcessState::Crashed, Readiness::None),
            (ProcessState::Starting, Readiness::None),
            (ProcessState::Exited, Readiness::None),
        ]);

        assert_eq!(moved_to(&steps[0]), Some(Room::Failed));
        assert_eq!(pins(&steps[0]), [("web".to_string(), Pin::Fail)]);
        assert_eq!(moved_to(&steps[1]), Some(Room::Starting));
        assert!(pins(&steps[1]).is_empty());
        assert_eq!(moved_to(&steps[2]), Some(Room::Off));
        assert!(pins(&steps[2]).is_empty());
    }

    /// A restart pins again, also when it walks through stopping; a stop
    /// walking through Restarting's room pins nothing.
    #[test]
    fn a_restart_pins_again_but_a_stop_pins_nothing() {
        let restart = walk(&[
            (ProcessState::Running, Readiness::None),
            (ProcessState::Restarting, Readiness::None),
        ]);
        assert_eq!(pins(&restart[0]), [("web".to_string(), Pin::Again)]);

        let through_stopping = walk(&[
            (ProcessState::Running, Readiness::None),
            (ProcessState::Stopping, Readiness::None),
            (ProcessState::Restarting, Readiness::None),
        ]);
        assert!(pins(&through_stopping[0]).is_empty());
        assert_eq!(pins(&through_stopping[1]), [(
            "web".to_string(),
            Pin::Again
        )]);

        let stop = walk(&[
            (ProcessState::Running, Readiness::None),
            (ProcessState::Stopping, Readiness::None),
            (ProcessState::Exited, Readiness::None),
        ]);
        assert_eq!(moved_to(&stop[0]), Some(Room::Restarting));
        assert_eq!(moved_to(&stop[1]), Some(Room::Off));
        assert!(stop.iter().all(|sent| pins(sent).is_empty()), "{stop:?}");
    }

    /// A ready check running out of time pins late, and the process stands
    /// in Running.
    #[test]
    fn a_ready_timeout_pins_late_in_running() {
        let sent = walk(&[
            (ProcessState::Running, Readiness::Waiting),
            (ProcessState::Running, Readiness::TimedOut),
        ])
        .remove(0);

        assert_eq!(moved_to(&sent), Some(Room::Running));
        assert_eq!(pins(&sent), [("web".to_string(), Pin::Late)]);
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

        let opened = floor.status(shop_answers(vec![running(&[8080], 2)]));
        assert_eq!(floats(&opened), [
            (FloatKind::Port, Some(8080)),
            (FloatKind::Restart, None)
        ]);

        let many = floor.status(shop_answers(vec![running(
            &[8080, 9001, 9002, 9003, 9004],
            2,
        )]));
        assert_eq!(floats(&many), [
            (FloatKind::Port, Some(9001)),
            (FloatKind::Port, Some(9002)),
            (FloatKind::Port, Some(9003)),
        ]);
        assert!(pins(&many).is_empty());
    }

    /// A gap sends exactly one snapshot, with no pin for what it missed.
    #[test]
    fn a_gap_sends_one_snapshot() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        floor.gap();

        let sent = floor.status(shop_answers(vec![web(
            ProcessState::Crashed,
            Readiness::None,
        )]));
        let after = floor.status(shop_answers(vec![web(
            ProcessState::Crashed,
            Readiness::None,
        )]));

        assert_eq!(kinds(&sent), ["snapshot"]);
        assert!(after.is_empty(), "{after:?}");
    }

    /// A service restart is a baseline: a snapshot when it goes, another
    /// when it is back with changes, and no pin.
    #[test]
    fn a_service_restart_is_a_baseline() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);

        let gone = floor.status(StatusReading::Absent);
        let back = floor.status(shop_answers(vec![web(
            ProcessState::Crashed,
            Readiness::None,
        )]));

        assert_eq!(kinds(&gone), ["snapshot"]);
        let gone = json_of(&gone[0]);
        assert_eq!(gone["health"], "absent");
        assert_eq!(gone["projects"][0]["processes"][0]["room"], "off");
        assert_eq!(kinds(&back), ["snapshot"]);
        let back = json_of(&back[0]);
        assert_eq!(back["health"], "running");
        assert_eq!(back["projects"][0]["processes"][0]["room"], "failed");
    }

    /// Each other health of the service is a snapshot once, and a failed
    /// ask changes nothing.
    #[test]
    fn each_service_health_is_a_snapshot_and_a_failed_ask_nothing() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);

        assert!(floor.status(StatusReading::Failed).is_empty());
        let starting = floor.status(StatusReading::Starting);
        let other = floor.status(StatusReading::OtherVersion);
        let again = floor.status(StatusReading::OtherVersion);

        assert_eq!(json_of(&starting[0])["health"], "starting");
        assert_eq!(json_of(&other[0])["health"], "other-version");
        assert!(again.is_empty(), "{again:?}");
    }

    /// A project the service has not loaded shows every process off, and a
    /// status of a project not on the floor is ignored.
    #[test]
    fn an_unloaded_project_shows_every_process_off() {
        let mut floor = floor();
        floor.projects(read(vec![
            project_reading(SHOP, &["web"]),
            project_reading(BLOG, &["api", "worker"]),
        ]));
        let with_ghost = |api: ProcessState| {
            StatusReading::Answered(vec![
                project_status(SHOP, vec![web(ProcessState::Running, Readiness::None)]),
                project_status("ghost", vec![process_status("api", api, Readiness::None)]),
            ])
        };

        let sent = floor.status(with_ghost(ProcessState::Running));
        let later = floor.status(with_ghost(ProcessState::Crashed));

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
    /// pin and no float.
    #[test]
    fn a_process_seen_for_the_first_time_earns_no_pin() {
        let mut floor = settled(vec![web(ProcessState::Running, Readiness::None)]);
        let added = floor.projects(read(vec![project_reading(SHOP, &["web", "worker"])]));

        let sent = floor.status(shop_answers(vec![
            web(ProcessState::Running, Readiness::None),
            process_status_with("worker", ProcessState::Running, Readiness::None, &[8080], 1),
        ]));

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
            "attached", "cpu", "ended", "icon", "mark", "memory", "name", "ports", "quoted",
            "restarts", "room", "speaking", "state", "uptime",
        ]);
        assert_eq!(process["name"], "web");
        assert_eq!(process["quoted"], "'web'");
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

        let first = floor.logs(at(0), &[
            logged("web", stat(10, None)),
            logged("worker", stat(10, None)),
        ]);
        let grew = floor.logs(at(1), &[
            logged("web", stat(20, None)),
            logged("worker", stat(20, None)),
        ]);
        let speaking =
            json_of(&floor.snapshot())["projects"][0]["processes"][0]["speaking"].clone();
        let still = floor.logs(at(2), &[logged("web", stat(20, None))]);
        let quiet = floor.logs(at(3), &[logged("web", stat(20, None))]);
        let touched = floor.logs(at(4), &[logged(
            "web",
            stat(20, Some(SystemTime::UNIX_EPOCH)),
        )]);

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
        floor.projects(read(vec![
            project_reading(SHOP, &["web"]),
            project_reading(BLOG, &["api"]),
        ]));
        let note = Scratchpad::builder()
            .key(ScratchpadKey::try_new("plan").unwrap())
            .body("ship <b>it</b>".to_string())
            .author(Author::agent("claude-code@builder"))
            .updated_at(EpochSeconds::new(100))
            .build();

        let sent = floor.notes(&root(SHOP), slice::from_ref(&note));
        let same = floor.notes(&root(SHOP), slice::from_ref(&note));
        let blog = floor.notes(&root(BLOG), &[]);
        let ghost = floor.notes(&root("ghost"), slice::from_ref(&note));
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
    /// found; a log is found only for a process on the floor.
    #[test]
    fn a_removed_project_leaves_the_floor() {
        let mut floor = floor();
        floor.projects(read(vec![
            project_reading(SHOP, &["web"]),
            project_reading(BLOG, &["api"]),
        ]));
        let before = floor.log_path(BLOG, "api");

        let sent = floor.projects(read(vec![project_reading(SHOP, &["web"])]));

        assert!(before.is_some());
        assert_eq!(kinds(&sent), ["projects"]);
        let projects = json_of(&sent[0])["projects"].clone();
        assert_eq!(projects.as_array().unwrap().len(), 1);
        assert_eq!(projects[0]["name"], "shop");
        assert_eq!(floor.log_path(BLOG, "api"), None);
        assert_eq!(
            floor.log_path(SHOP, "web"),
            Some(holder_dirs().process_log(&root(SHOP), &process_name("web")))
        );
        assert_eq!(floor.log_path(SHOP, "../web"), None);
        assert_eq!(floor.log_path(SHOP, ""), None);
    }

    /// A stack or an icon change sends the projects; an unchanged reading
    /// sends nothing.
    #[test]
    fn a_stack_or_icon_change_sends_the_projects() {
        let mut floor = floor();

        let first = floor.projects(read(vec![project_reading(SHOP, &["web"])]));
        let same = floor.projects(read(vec![project_reading(SHOP, &["web"])]));
        let icon = floor.projects(read(vec![project_reading_with(
            SHOP,
            &["web"],
            ProcessIcon::Rust,
        )]));
        let stack = floor.projects(read(vec![project_reading_with(
            SHOP,
            &["web", "db"],
            ProcessIcon::Rust,
        )]));

        assert_eq!(kinds(&first), ["projects"]);
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
        floor.projects(read(vec![project_reading(SHOP, &["web"])]));

        let missing = floor.projects(ProjectsReading::NoState);
        let newer = floor.projects(ProjectsReading::NewerSchema);
        let back = floor.projects(read(vec![project_reading(SHOP, &["web"])]));
        let unreadable = floor.projects(ProjectsReading::Unreadable);

        assert_eq!(kinds(&missing), ["snapshot"]);
        assert_eq!(json_of(&missing[0])["health"], "no-state");
        assert_eq!(json_of(&missing[0])["projects"], json!([]));
        assert_eq!(json_of(&newer[0])["health"], "newer-schema");
        assert_eq!(json_of(&back[0])["health"], "absent");
        assert_eq!(json_of(&back[0])["projects"][0]["name"], "shop");
        assert_eq!(json_of(&unreadable[0])["health"], "unreadable");
        assert_eq!(json_of(&unreadable[0])["projects"][0]["name"], "shop");
    }

    /// Every message takes the next sequence number, a page's own snapshot
    /// too.
    #[test]
    fn sequence_numbers_rise_by_one() {
        let mut floor = floor();

        let mut sent = floor.projects(read(vec![project_reading(SHOP, &["web"])]));
        sent.extend(floor.status(shop_answers(vec![web(
            ProcessState::Starting,
            Readiness::None,
        )])));
        sent.push(floor.snapshot());
        sent.extend(floor.status(shop_answers(vec![web(
            ProcessState::Running,
            Readiness::None,
        )])));

        let seqs: Vec<u64> = sent.iter().map(Sequenced::seq).collect();
        assert_eq!(seqs, [1, 2, 3, 4, 5]);
    }

    /// A project under the home folder shows its path from `~`, and one
    /// elsewhere shows it whole; a view started outside every project has
    /// no `here`.
    #[test]
    fn the_home_folder_shows_as_a_tilde() {
        let mut floor = floor();
        floor.projects(read(vec![project_reading(SHOP, &["web"])]));
        let mut elsewhere = Floor::new(PathBuf::from(HOME), None, holder_dirs());
        elsewhere.projects(read(vec![project_reading(SHOP, &["web"])]));

        let home = json_of(&floor.snapshot());
        let whole = json_of(&elsewhere.snapshot());

        assert_eq!(home["projects"][0]["path"], "~/shop");
        assert_eq!(whole["projects"][0]["path"], "/home/me/shop");
        assert_eq!(whole["here"], Value::Null);
    }
}
