use super::{response::Response, streamed::Streamed};

/// What answers one request: a whole response written at once, or a stream
/// that writes its own response until it ends.
pub enum Answer {
    /// A whole response.
    Whole(Response),
    /// A response written over time.
    Stream(Box<dyn Streamed>),
}

impl From<Response> for Answer {
    fn from(response: Response) -> Self {
        Self::Whole(response)
    }
}
