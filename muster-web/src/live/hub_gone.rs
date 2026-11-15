use thiserror::Error;

/// The hub ended, so nothing it owned can be asked of it any more.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("the view's hub has ended")]
pub struct HubGone;
