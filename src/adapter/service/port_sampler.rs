use std::{thread, time::Duration};

use crossbeam_channel::{Sender, bounded};

use super::event::ServiceEvent;
use crate::domain::port::ListeningPorts;

/// How often the listening ports of running processes are resampled.
pub const PORT_SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

/// Samples which ports each running process tree listens on, on its own
/// thread, and posts each sample back as [`ServiceEvent::PortsSampled`]. A
/// request while one is still being answered is dropped, so a slow scan never
/// queues behind itself. The thread ends once the sampler is dropped.
pub struct PortSampler {
    /// Where the loop sends the root pids to sample.
    requests: Sender<Vec<u32>>,
}

impl PortSampler {
    /// Starts sampling through `listening`, posting samples to `events`.
    pub fn spawn(
        mut listening: Box<dyn ListeningPorts + Send>,
        events: Sender<ServiceEvent>,
    ) -> Self {
        let (requests, received) = bounded::<Vec<u32>>(1);
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
    pub fn sample(&self, roots: Vec<u32>) {
        let _ = self.requests.try_send(roots);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        net::{IpAddr, Ipv4Addr},
    };

    use super::*;
    use crate::domain::port::ListeningSocket;

    /// Longest a test waits for a sample.
    const SAMPLE_WAIT: Duration = Duration::from_secs(5);

    /// Reports port 3000 under every root it is asked about.
    struct EveryRootOn3000;

    impl ListeningPorts for EveryRootOn3000 {
        fn listening(&mut self, roots: &[u32]) -> HashMap<u32, Vec<ListeningSocket>> {
            roots
                .iter()
                .map(|root| {
                    (*root, vec![
                        ListeningSocket::builder()
                            .port(3000)
                            .address(IpAddr::V4(Ipv4Addr::LOCALHOST))
                            .pid(*root + 1)
                            .build(),
                    ])
                })
                .collect()
        }
    }

    #[test]
    fn posts_each_sample_to_the_loop() {
        let (sender, events) = bounded(4);
        let sampler = PortSampler::spawn(Box::new(EveryRootOn3000), sender);

        sampler.sample(vec![41]);

        match events.recv_timeout(SAMPLE_WAIT).unwrap() {
            ServiceEvent::PortsSampled { ports } => {
                assert_eq!(ports[&41][0].port(), 3000);
                assert_eq!(ports[&41][0].pid(), 42);
            },
            _ => panic!("expected a sample"),
        }
    }
}
