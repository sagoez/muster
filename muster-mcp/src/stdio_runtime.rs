//! The one async runtime muster runs, which `rmcp` needs for `muster mcp`.

use std::future::Future;

use tokio::runtime::Builder;

use super::{mcp_deps::McpDeps, mcp_error::McpError, server::serve_stdio};

/// Serves `deps` over stdio until the client disconnects. A tool call still
/// running on a blocking thread then is left behind rather than waited out,
/// so the server never outlives its client by a call's bound.
///
/// # Errors
/// Returns [`McpError`] when the runtime cannot be built or the server fails.
pub fn run_stdio(deps: McpDeps) -> Result<(), McpError> {
    run_detached(serve_stdio(deps))?
}

/// Runs `work` on a fresh current-thread runtime, then shuts the runtime down
/// without waiting for blocking tasks it spawned that still run.
///
/// # Errors
/// Returns [`McpError::Runtime`] when the runtime cannot be built.
fn run_detached<F: Future>(work: F) -> Result<F::Output, McpError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(McpError::Runtime)?;
    let output = runtime.block_on(work);
    runtime.shutdown_background();
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    use tokio::task;

    use super::*;

    /// How long the abandoned blocking task would run.
    const ABANDONED: Duration = Duration::from_secs(10);
    /// Well under [`ABANDONED`], and generous for a loaded machine.
    const PROMPT: Duration = Duration::from_secs(3);

    /// A blocking task still running when the served future ends does not
    /// hold the caller until it finishes.
    #[test]
    fn a_running_blocking_task_is_left_behind() {
        let started = Instant::now();

        let answer = run_detached(async {
            drop(task::spawn_blocking(|| thread::sleep(ABANDONED)));
            7
        })
        .unwrap();

        assert_eq!(answer, 7);
        assert!(started.elapsed() < PROMPT, "took {:?}", started.elapsed());
    }
}
