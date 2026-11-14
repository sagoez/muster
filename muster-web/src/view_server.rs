use std::sync::Arc;

use super::{
    http::{Gate, HttpServer, ViewToken},
    routes::Site,
    view_deps::ViewDeps,
    web_error::WebError,
};

/// `muster view`, bound and ready: its server listening on loopback and the
/// gate that admits only its token.
pub struct ViewServer {
    /// The bound server.
    server: HttpServer,
    /// What admits a request.
    gate: Gate,
}

impl ViewServer {
    /// Listens on loopback, on the port `deps` names or a free one, behind a
    /// fresh token.
    ///
    /// # Errors
    /// Returns [`WebError::Bind`] when the port cannot be listened on, or
    /// [`WebError::Token`] when no token can be made.
    pub fn bind(deps: ViewDeps) -> Result<Self, WebError> {
        let server = HttpServer::bind(*deps.port()).map_err(WebError::Bind)?;
        let gate = Gate::builder()
            .port(server.port().map_err(WebError::Bind)?)
            .token(ViewToken::generate().map_err(WebError::Token)?)
            .build();
        Ok(Self { server, gate })
    }

    /// The address to open, carrying the token.
    pub fn url(&self) -> String {
        self.gate.first_url()
    }

    /// Opens [`Self::url`] in the default browser.
    ///
    /// # Errors
    /// Returns [`WebError::Browser`] when no browser could be opened.
    pub fn open_in_browser(&self) -> Result<(), WebError> {
        opener::open(self.url()).map_err(WebError::Browser)
    }

    /// Serves the view until the process ends.
    ///
    /// # Errors
    /// Returns [`WebError::Start`] when the server cannot start, or
    /// [`WebError::Stopped`] when it stops.
    pub fn serve(self) -> Result<(), WebError> {
        let site = Site::builder().gate(self.gate).build();
        self.server
            .start(Arc::new(site))
            .map_err(WebError::Start)?
            .join()
            .map_err(|_| WebError::Stopped)
    }
}
