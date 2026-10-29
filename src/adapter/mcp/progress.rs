use std::future::Future;

use rmcp::{RoleServer, model::ProgressNotificationParam, service::RequestContext};
use tokio::time;

use super::bounds::PROGRESS_INTERVAL;

/// Runs `work`, telling the client it still runs every
/// [`PROGRESS_INTERVAL`] when the request carries a progress token, so a
/// client that resets its timeout on progress keeps waiting.
pub async fn with_progress<T>(
    context: &RequestContext<RoleServer>,
    work: impl Future<Output = T>,
) -> T {
    let Some(token) = context.meta.get_progress_token() else {
        return work.await;
    };
    tokio::pin!(work);
    let mut ticks = time::interval(PROGRESS_INTERVAL);
    ticks.tick().await;
    let mut progress = 0.0;
    loop {
        tokio::select! {
            done = &mut work => return done,
            _ = ticks.tick() => {
                progress += 1.0;
                let notice = ProgressNotificationParam::new(token.clone(), progress);
                // A client that cannot hear progress still gets the result.
                let _ = context.peer.notify_progress(notice).await;
            },
        }
    }
}
