use super::request::Request;
use crate::http::reply::response::Response;

/// What answers each request the server reads whole.
pub trait Responder: Send + Sync {
    /// The response to `request`.
    fn respond(&self, request: &Request) -> Response;
}
