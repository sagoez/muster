use getset::{CopyGetters, Getters};

use super::args::{positive_duration, state_goal_parser};
use crate::{
    adapter::service::StateGoal,
    domain::value::{LocalPort, LocalUrl, OutputCursor, PositiveDuration, ProcessRef},
};

/// `muster wait`: a process and what to wait for.
#[derive(clap::Args, Getters, CopyGetters)]
pub struct WaitArgs {
    /// The process, as `muster ps` lists it; quote a name with spaces.
    /// Chosen from a list at a terminal when absent.
    #[getset(get = "pub")]
    name: Option<ProcessRef>,
    /// Wait for an output line containing this text.
    #[arg(long, group = "until", value_name = "TEXT")]
    #[getset(get = "pub")]
    log: Option<String>,
    /// Read --log as a regular expression.
    #[arg(long, requires = "log")]
    #[getset(get_copy = "pub")]
    regex: bool,
    /// Wait until a TCP connection to this localhost port succeeds.
    #[arg(long, group = "until", value_name = "PORT")]
    #[getset(get = "pub")]
    port: Option<LocalPort>,
    /// Wait until this localhost URL answers below 500.
    #[arg(long, group = "until", value_name = "URL")]
    #[getset(get = "pub")]
    http: Option<LocalUrl>,
    /// With --http, the status to wait for instead.
    #[arg(long, requires = "http", value_name = "CODE")]
    #[getset(get = "pub")]
    status: Option<u16>,
    /// Wait for a lifecycle state; `stopped` means not running at all.
    #[arg(long, group = "until", value_parser = state_goal_parser())]
    #[getset(get = "pub")]
    state: Option<StateGoal>,
    /// The longest to wait, such as 60s; unbounded when absent.
    #[arg(long, value_name = "DURATION", value_parser = positive_duration)]
    #[getset(get = "pub")]
    timeout: Option<PositiveDuration>,
    /// Count output after this cursor, from an earlier wait's result, as
    /// well as output after the command started.
    #[arg(long, value_name = "CURSOR")]
    #[getset(get = "pub")]
    since: Option<OutputCursor>,
    /// Print the result as JSON.
    #[arg(long)]
    #[getset(get_copy = "pub")]
    json: bool,
}
