use std::{ffi::c_int, sync::Arc};

use muster_domain::value::ProjectName;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};

use super::{
    floor::Floor,
    http::{Gate, HttpServer, ViewToken},
    live::{Hub, start_pollers},
    routes::Site,
    running_view::RunningView,
    view_deps::ViewDeps,
    web_error::WebError,
};

/// The signals that end the view: Ctrl-C, a kill or a logout. They are
/// caught so the caller ends the view as each one asks.
const EXIT_SIGNALS: [c_int; 3] = [SIGINT, SIGTERM, SIGHUP];
/// What starts the fragment that names the floor the page opens on.
const FLOOR_FRAGMENT: &str = "#/";

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

    /// Starts serving: the hub over an empty floor, the server, then the
    /// pollers, which read on threads of their own. Nothing slow runs
    /// before the server accepts, so the page opens at once and shows the
    /// floor loading until the first readings arrive.
    ///
    /// # Errors
    /// Returns [`WebError::Start`] when the hub, the server or a poller
    /// cannot start; a hub already started is stopped first.
    pub fn start(self) -> Result<RunningView, WebError> {
        let Self {
            server,
            gate,
            exit_signals,
            deps,
        } = self;
        let floor = Floor::new(
            deps.cwd().clone(),
            deps.home().clone(),
            deps.holder_dirs().clone(),
        );
        let (hub, handle) = Hub::new(floor);
        let pauses = hub.pauses();
        let hub_thread = hub.spawn().map_err(WebError::Start)?;
        let site = Site::builder()
            .gate(gate.clone())
            .hub(handle.clone())
            .build();
        let started = server
            .start(Arc::new(site))
            .map(drop)
            .and_then(|()| start_pollers(&deps, &handle, &pauses));
        if let Err(error) = started {
            handle.stop();
            let _ = hub_thread.join();
            return Err(WebError::Start(error));
        }
        Ok(RunningView::new(
            gate,
            floor_fragment(deps.floor().as_ref()),
            exit_signals,
            handle,
            hub_thread,
        ))
    }
}

/// The fragment naming the floor of `project`, `#/<project>` with the name
/// percent-encoded as the page decodes it; empty when none was named.
fn floor_fragment(project: Option<&ProjectName>) -> String {
    project.map_or_else(String::new, |project| {
        format!(
            "{FLOOR_FRAGMENT}{}",
            utf8_percent_encode(project.as_ref(), NON_ALPHANUMERIC)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fragment a floor named `name` opens on.
    fn fragment(name: Option<&str>) -> String {
        floor_fragment(
            name.map(|name| ProjectName::try_new(name).unwrap())
                .as_ref(),
        )
    }

    /// A named floor is the address's fragment, encoded so the page reads
    /// it back whole; with none named there is no fragment.
    #[test]
    fn a_named_floor_is_the_fragment() {
        assert_eq!(fragment(Some("shop")), "#/shop");
        assert_eq!(fragment(Some("my shop#1")), "#/my%20shop%231");
        assert_eq!(fragment(None), "");
    }
}
