use std::{ffi::c_int, path::PathBuf, sync::Arc};

use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};

use super::{
    http::{Gate, HttpServer, ViewToken},
    launch_page::LaunchPage,
    routes::Site,
    view_deps::ViewDeps,
    web_error::WebError,
};

/// The signals that end the view: Ctrl-C, a kill or a logout. They are
/// caught so the launch page holding the token is removed on the way out.
const EXIT_SIGNALS: [c_int; 3] = [SIGINT, SIGTERM, SIGHUP];

/// `muster view`, bound and ready: its server listening on loopback, the
/// gate that admits only its token, and the launch page a browser was handed.
pub struct ViewServer {
    /// The bound server.
    server: HttpServer,
    /// What admits a request.
    gate: Gate,
    /// The private directory the launch page goes in.
    state_dir: PathBuf,
    /// The page the browser was handed, removed when the view ends.
    launch_page: Option<LaunchPage>,
    /// The signals that end the view, caught from the moment it binds.
    exit_signals: Signals,
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
            state_dir: deps.state_dir().clone(),
            launch_page: None,
            exit_signals,
        })
    }

    /// The address that opens the view, carrying the token in its path.
    pub fn url(&self) -> String {
        self.gate.url()
    }

    /// Opens the view in the default browser through a private launch page,
    /// kept until the view ends, so the token never reaches a process's
    /// arguments.
    ///
    /// # Errors
    /// Returns [`WebError::Page`] when the page cannot be written, or
    /// [`WebError::Browser`] when no browser could be opened.
    pub fn open_in_browser(&mut self) -> Result<(), WebError> {
        self.launch_page = Some(LaunchPage::open(
            &self.state_dir,
            *self.gate.port(),
            &self.url(),
            |page| opener::open_browser(page),
        )?);
        Ok(())
    }

    /// Serves the view until an exit signal arrives, then returns, removing
    /// the launch page.
    ///
    /// # Errors
    /// Returns [`WebError::Start`] when the server cannot start.
    pub fn serve(self) -> Result<(), WebError> {
        let Self {
            server,
            gate,
            launch_page,
            mut exit_signals,
            ..
        } = self;
        server
            .start(Arc::new(Site::builder().gate(gate).build()))
            .map_err(WebError::Start)?;
        let _ = exit_signals.forever().next();
        drop(launch_page);
        Ok(())
    }
}
