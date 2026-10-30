use std::{thread, time::Duration};

use crossbeam_channel::{Sender, bounded};

use super::event::ServiceEvent;
use crate::domain::{port::ListeningPorts, value::Pid};

/// How often the listening ports of running processes are resampled.
pub const PORT_SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

/// Samples which ports each running process tree listens on, on its own
/// thread, and posts each sample back as [`ServiceEvent::PortsSampled`]. A
/// request while one is still being answered is dropped, so a slow scan never
/// queues behind itself. The thread ends once the sampler is dropped.
pub struct PortSampler {
    /// Where the loop sends the root pids to sample.
    requests: Sender<Vec<Pid>>,
}

impl PortSampler {
    /// Starts sampling through `listening`, posting samples to `events`.
    pub fn spawn(
        mut listening: Box<dyn ListeningPorts + Send>,
        events: Sender<ServiceEvent>,
    ) -> Self {
        let (requests, received) = bounded::<Vec<Pid>>(1);
        thread::spawn(move || {
            for roots in received {
                let ports = listening.listening(&roots);
                if events.send(ServiceEvent::PortsSampled { ports }).is_err() {
                    return;
                }
            }
        });
        Self { requests }
    }

    /// Asks for a sample of the trees rooted at `roots`, unless one is in
    /// flight.
    pub fn sample(&self, roots: Vec<Pid>) {
        let _ = self.requests.try_send(roots);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::fixed_listening_ports::{FixedListeningPorts, HELD_PORT},
        *,
    };

    /// Longest a test waits for a sample.
    const SAMPLE_WAIT: Duration = Duration::from_secs(5);

    /// Each sample reaches the loop as an event.
    #[test]
    fn posts_each_sample_to_the_loop() {
        let (sender, events) = bounded(4);
        let sampler = PortSampler::spawn(Box::new(FixedListeningPorts), sender);

        let root = Pid::try_new(41).unwrap();
        sampler.sample(vec![root]);

        let ServiceEvent::PortsSampled { ports } = events.recv_timeout(SAMPLE_WAIT).unwrap() else {
            panic!("expected a sample");
        };
        assert_eq!(ports[&root][0].port(), HELD_PORT);
        assert_eq!(ports[&root][0].pid(), Pid::try_new(42).unwrap());
    }
}
