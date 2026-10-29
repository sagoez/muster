use std::{
    path::PathBuf,
    thread::{self, JoinHandle},
    time::Instant,
};

use getset::{Getters, MutGetters};
use typed_builder::TypedBuilder;

use super::{
    notice::Notice,
    process_action::ProcessAction,
    project_status::ProjectStatus,
    protocol,
    runtime_testing::{POLL, WAIT, find, no_env, wait_until},
    service_error::ServiceError,
    service_request::ServiceRequest,
    service_response::ServiceResponse,
};
use crate::domain::{
    process::ProcessState,
    value::{ProcessName, ProjectRoot},
};

/// A service running on a thread of the test.
#[derive(Getters, MutGetters, TypedBuilder)]
pub(super) struct Running {
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    socket: PathBuf,
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    thread: Option<JoinHandle<Result<(), ServiceError>>>,
}

impl Running {
    /// Sends `request` as a client would and returns the answer.
    pub(super) fn send(&self, request: ServiceRequest) -> ServiceResponse {
        protocol::send(&self.socket, &request).unwrap()
    }

    /// Brings `project` up with no client environment.
    pub(super) fn up(&self, project: &ProjectRoot) -> ServiceResponse {
        self.send(ServiceRequest::Up {
            project: project.clone(),
            environment: no_env(),
            wait: None,
        })
    }

    /// Applies `action` to `name` of `project` with no client environment.
    pub(super) fn control(
        &self,
        project: &ProjectRoot,
        name: &str,
        action: ProcessAction,
    ) -> ServiceResponse {
        self.send(ServiceRequest::Control {
            project: project.clone(),
            name: ProcessName::try_new(name).unwrap(),
            action,
            environment: no_env(),
            wait: None,
        })
    }

    /// Sends `Stop`, then waits for the service to exit.
    pub(super) fn stop(mut self) {
        assert_eq!(self.send(ServiceRequest::Stop), ServiceResponse::Ok);
        self.join();
    }

    /// Waits, bounded, for the service thread to finish and checks it
    /// exited cleanly.
    pub(super) fn join(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        wait_until("the service to exit", || thread.is_finished());
        thread.join().unwrap().unwrap();
    }

    /// Polls `ps` until the process `name` of `project` is in `state`,
    /// failing at [`WAIT`], and returns that status.
    pub(super) fn wait_for(
        &self,
        project: &ProjectRoot,
        name: &str,
        state: ProcessState,
    ) -> ProjectStatus {
        let mut status = self.status(project);
        wait_until("the process to reach its state", || {
            status = self.status(project);
            find(&status, name).is_some_and(|process| *process.state() == state)
        });
        status
    }

    /// The one project status a `ps` of `project` reports.
    pub(super) fn status(&self, project: &ProjectRoot) -> ProjectStatus {
        let ServiceResponse::Status(mut statuses) = self.send(ServiceRequest::Ps {
            project: Some(project.clone()),
        }) else {
            panic!("ps answers a status");
        };
        statuses.remove(0)
    }

    /// The notices of `project`.
    pub(super) fn notices(&self, project: &ProjectRoot) -> Vec<Notice> {
        let ServiceResponse::Notices(notices) = self.send(ServiceRequest::Notices {
            project: project.clone(),
        }) else {
            panic!("notices answer notices");
        };
        notices
    }
}

impl Drop for Running {
    /// Stops a service a failing test left running.
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = protocol::send(&self.socket, &ServiceRequest::Stop);
            if let Some(thread) = self.thread.take() {
                let started = Instant::now();
                while !thread.is_finished() && started.elapsed() < WAIT {
                    thread::sleep(POLL);
                }
            }
        }
    }
}
