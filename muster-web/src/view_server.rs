use std::{ffi::c_int, sync::Arc, time::Instant};

use opener::OpenError;
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};

use super::{
    floor::Floor,
    http::{Gate, HttpServer, ViewToken},
    live::{Hub, start_pollers},
    routes::Site,
    view_deps::ViewDeps,
    web_error::WebError,
};

/// The signals that end the view: Ctrl-C, a kill or a logout. They are
/// caught so the caller ends the view as each one asks.
const EXIT_SIGNALS: [c_int; 3] = [SIGINT, SIGTERM, SIGHUP];

/// `muster view`, bound and ready: its server listening on loopback and the
/// gate that admits only its token.
pub struct ViewServer {
    /// The bound server.
    server: HttpServer,
    /// What admits a request.
    gate: Gate,
    /// The signals that end the view, caught from the moment it binds.
    exit_signals: Signals,
    /// What the view reads, once it serves.
    deps: ViewDeps,
}

impl ViewServer {
    /// Listens on loopback, on the port `deps` names or a free one, behind a
    /// fresh token, catching the exit signals from now on.
    ///
    /// # Errors
    /// Returns [`WebError::Signals`] when the exit signals cannot be caught,
    /// [`WebError::Bind`] when the port cannot be listened on, or
    /// [`WebError::Token`] when no token can be made.
    pub fn bind(deps: ViewDeps) -> Result<Self, WebError> {
        let exit_signals = Signals::new(EXIT_SIGNALS).map_err(WebError::Signals)?;
        let server = HttpServer::bind(*deps.port()).map_err(WebError::Bind)?;
        let gate = Gate::builder()
            .port(server.port().map_err(WebError::Bind)?)
            .token(ViewToken::generate().map_err(WebError::Token)?)
            .build();
        Ok(Self {
            server,
            gate,
            exit_signals,
            deps,
        })
    }

    /// The address that opens the view, carrying the token in its path.
    pub fn url(&self) -> String {
        self.gate.url()
    }

    /// Opens the view in the default browser at a one-time address, so the
    /// token never reaches a process's arguments.
    ///
    /// # Errors
    /// Returns [`WebError::Code`] when no code can be made, or
    /// [`WebError::Browser`] when no browser could be opened.
    pub fn open_in_browser(&self) -> Result<(), WebError> {
        open_with(&self.gate, Instant::now(), |url| opener::open_browser(url))
    }

    /// Serves the view until an exit signal arrives, and returns that
    /// signal's number: starts the hub over an empty floor, feeds it the
    /// first readings and starts its pollers, then serves. The signal stops
    /// the hub, which ends every stream and, through it, every poller.
    ///
    /// # Errors
    /// Returns [`WebError::Start`] when the hub, a poller or the server
    /// cannot start.
    pub fn serve(self) -> Result<c_int, WebError> {
        let Self {
            server,
            gate,
            mut exit_signals,
            deps,
        } = self;
        let floor = Floor::new(
            deps.cwd().clone(),
            deps.home().clone(),
            deps.holder_dirs().clone(),
        );
        let (hub, handle) = Hub::new(floor);
        let (life, wake) = (hub.life(), hub.wake());
        let hub_thread = hub.spawn().map_err(WebError::Start)?;
        let site = Site::builder().gate(gate).hub(handle.clone()).build();
        let started = start_pollers(&deps, &handle, &life, wake)
            .and_then(|()| server.start(Arc::new(site)).map(drop));
        let ended = started.map(|()| exit_signals.forever().next().unwrap_or(SIGINT));
        handle.stop();
        let _ = hub_thread.join();
        ended.map_err(WebError::Start)
    }
}

/// Mints a one-time code on `gate` at `now` and hands `open` the address
/// that redeems it.
///
/// # Errors
/// Returns [`WebError::Code`] when no code can be made, or
/// [`WebError::Browser`] when `open` fails.
fn open_with(
    gate: &Gate,
    now: Instant,
    open: impl FnOnce(&str) -> Result<(), OpenError>,
) -> Result<(), WebError> {
    open(&gate.open_url(now).map_err(WebError::Code)?).map_err(WebError::Browser)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use muster_domain::value::LocalPort;

    use super::*;

    /// The browser is handed the one-time address under `/open/`, never the
    /// token.
    #[test]
    fn the_browser_is_handed_the_code_not_the_token() {
        let gate = Gate::builder()
            .port(LocalPort::try_new(4173).unwrap())
            .token(ViewToken::generate().unwrap())
            .build();
        let handed = RefCell::new(String::new());

        open_with(&gate, Instant::now(), |url| {
            handed.replace(url.to_string());
            Ok(())
        })
        .unwrap();

        let handed = handed.into_inner();
        assert!(
            handed.starts_with("http://127.0.0.1:4173/open/"),
            "{handed}"
        );
        assert!(!handed.contains(gate.token().secret()), "{handed}");
    }
}
