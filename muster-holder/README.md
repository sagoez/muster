# muster-holder

An internal crate of [muster](https://github.com/sagoez/muster), the supervisor and control plane behind your terminal: the holder process that owns one child's PTY, screen and log and outlives the service. It has no stable API and is published only so `muster-workspace` can be installed from crates.io. To use muster, run `cargo install muster-workspace`.
