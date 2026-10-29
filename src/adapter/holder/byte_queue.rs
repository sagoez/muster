use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crossbeam_channel::{Receiver, Sender, unbounded};

/// A queue to one consumer thread, bounded by the bytes it holds rather than
/// its item count, so a producer can refuse work without ever blocking.
pub(super) struct ByteQueue<T> {
    /// Feeds the consumer; dropping the queue ends the consumer's iteration.
    sender: Sender<(T, usize)>,
    /// A second receiver the producer uses to discard what is queued.
    backlog: Receiver<(T, usize)>,
    /// Bytes queued and not yet taken by the consumer.
    queued: Arc<AtomicUsize>,
    /// Bytes the queue holds before it refuses more.
    cap: usize,
}

impl<T: Send + 'static> ByteQueue<T> {
    /// Creates an empty queue that holds up to `cap` bytes.
    pub(super) fn new(cap: usize) -> Self {
        let (sender, backlog) = unbounded();
        Self {
            sender,
            backlog,
            queued: Arc::new(AtomicUsize::new(0)),
            cap,
        }
    }

    /// Queues `item` weighing `bytes` when it fits. An empty queue takes any
    /// item, so one larger than the cap still passes on its own. Only the one
    /// producer adds bytes, so the check cannot be overtaken; the consumer only
    /// frees room.
    pub(super) fn try_push(&self, item: T, bytes: usize) -> bool {
        let queued = self.queued.load(Ordering::SeqCst);
        if queued != 0 && queued.saturating_add(bytes) > self.cap {
            return false;
        }
        self.queued.fetch_add(bytes, Ordering::SeqCst);
        self.enqueue(item, bytes)
    }

    /// Queues `item` regardless of the cap, for a message that must follow a
    /// [`Self::clear`].
    pub(super) fn force_push(&self, item: T, bytes: usize) -> bool {
        self.queued.fetch_add(bytes, Ordering::SeqCst);
        self.enqueue(item, bytes)
    }

    /// Discards everything queued that the consumer has not taken yet.
    pub(super) fn clear(&self) {
        for (_, bytes) in self.backlog.try_iter() {
            self.queued.fetch_sub(bytes, Ordering::SeqCst);
        }
    }

    /// The consumer's side: yields items in order, releasing their bytes, and
    /// ends once the queue is dropped and drained.
    pub(super) fn consumer(&self) -> impl Iterator<Item = T> + Send + 'static {
        let queued = Arc::clone(&self.queued);
        self.backlog.clone().into_iter().map(move |(item, bytes)| {
            queued.fetch_sub(bytes, Ordering::SeqCst);
            item
        })
    }

    /// Sends a reserved item, returning its bytes when no consumer remains.
    fn enqueue(&self, item: T, bytes: usize) -> bool {
        let sent = self.sender.send((item, bytes)).is_ok();
        if !sent {
            self.queued.fetch_sub(bytes, Ordering::SeqCst);
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cap of the queues under test.
    const CAP: usize = 10;

    #[test]
    fn refuses_past_the_cap_but_takes_one_oversized_item_when_empty() {
        let queue = ByteQueue::new(CAP);
        assert!(queue.try_push("big", CAP * 2));
        assert!(!queue.try_push("small", 1));

        let mut consumer = queue.consumer();
        assert_eq!(consumer.next(), Some("big"));
        assert!(queue.try_push("a", CAP / 2));
        assert!(queue.try_push("b", CAP / 2));
        assert!(!queue.try_push("c", 1));
    }

    #[test]
    fn clear_discards_the_backlog_and_frees_its_bytes() {
        let queue = ByteQueue::new(CAP);
        assert!(queue.try_push("stale", CAP));
        queue.clear();
        assert!(queue.force_push("fresh", CAP * 2));

        let consumer = queue.consumer();
        drop(queue);
        assert_eq!(consumer.collect::<Vec<_>>(), vec!["fresh"]);
    }
}
