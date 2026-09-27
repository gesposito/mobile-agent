//! The receiving end of a frame subscription, independent of its source.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use crate::model::{Frame, FrameStats};

/// How many undelivered frames a subscription keeps before dropping the
/// oldest: half a second at 30 fps.
pub const QUEUE_CAPACITY: usize = 16;

/// What produces a stream's frames.
pub(crate) trait FrameSource: Send {
    /// The next frame, waiting up to `timeout`, or forever with `None`.
    /// `None` once the timeout passed or the source ended.
    fn next(&self, timeout: Option<Duration>) -> Option<Frame>;

    fn stats(&self) -> FrameStats;
}

/// A frame subscription: a frame of the display as it was when the stream
/// started, then one each time the display's pixels change. Dropping it
/// unsubscribes.
pub struct FrameStream {
    source: Box<dyn FrameSource>,
}

impl FrameStream {
    pub(crate) fn new(source: Box<dyn FrameSource>) -> FrameStream {
        FrameStream { source }
    }

    /// The next frame, waiting up to `timeout`; `None` when none came.
    pub fn next(&self, timeout: Duration) -> Option<Frame> {
        self.source.next(Some(timeout))
    }

    /// The next frame if one is waiting.
    pub fn try_next(&self) -> Option<Frame> {
        self.source.next(Some(Duration::ZERO))
    }

    pub fn stats(&self) -> FrameStats {
        self.source.stats()
    }
}

/// Frames between the reading thread and the consumer. When the consumer
/// falls `capacity` frames behind, the oldest frame is dropped, so what it
/// reads next is always recent.
pub(crate) struct FrameQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
    capacity: usize,
}

struct QueueState {
    frames: VecDeque<Frame>,
    dropped: u64,
    closed: bool,
}

impl FrameQueue {
    pub fn new(capacity: usize) -> FrameQueue {
        let _ = capacity;
        unimplemented!("task 6.2: frame queue")
    }

    pub fn push(&self, frame: Frame) {
        let _ = frame;
        unimplemented!("task 6.2: frame queue")
    }

    /// The oldest frame, waiting up to `timeout` (forever with `None`).
    /// After `close` it drains what is left and then returns `None`.
    pub fn pop(&self, timeout: Option<Duration>) -> Option<Frame> {
        let _ = timeout;
        unimplemented!("task 6.2: frame queue")
    }

    /// Ends the queue and wakes a waiting consumer.
    pub fn close(&self) {
        unimplemented!("task 6.2: frame queue")
    }

    pub fn dropped(&self) -> u64 {
        unimplemented!("task 6.2: frame queue")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::{Instant, SystemTime};

    use super::*;

    fn frame(sequence: u64) -> Frame {
        Frame {
            sequence,
            captured_at: SystemTime::now(),
            captured: Instant::now(),
            width: 1,
            height: 1,
            jpeg: None,
            hash: sequence,
            content_hash: 0,
            torn: false,
        }
    }

    #[test]
    fn frames_come_out_in_order() {
        let queue = FrameQueue::new(4);
        queue.push(frame(1));
        queue.push(frame(2));
        assert_eq!(queue.pop(Some(Duration::ZERO)).map(|f| f.sequence), Some(1));
        assert_eq!(queue.pop(Some(Duration::ZERO)).map(|f| f.sequence), Some(2));
        assert!(queue.pop(Some(Duration::ZERO)).is_none());
    }

    #[test]
    fn a_full_queue_drops_its_oldest_frame() {
        let queue = FrameQueue::new(2);
        for sequence in 1..=5 {
            queue.push(frame(sequence));
        }
        assert_eq!(queue.dropped(), 3);
        assert_eq!(queue.pop(Some(Duration::ZERO)).map(|f| f.sequence), Some(4));
        assert_eq!(queue.pop(Some(Duration::ZERO)).map(|f| f.sequence), Some(5));
    }

    #[test]
    fn pop_waits_for_a_frame_and_gives_up_after_its_timeout() {
        let queue = Arc::new(FrameQueue::new(2));
        let start = Instant::now();
        assert!(queue.pop(Some(Duration::from_millis(30))).is_none());
        assert!(start.elapsed() >= Duration::from_millis(30));

        let producer = {
            let queue = queue.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                queue.push(frame(7));
            })
        };
        assert_eq!(
            queue.pop(Some(Duration::from_secs(5))).map(|f| f.sequence),
            Some(7)
        );
        producer.join().expect("the producer finished");
    }

    #[test]
    fn close_wakes_a_waiting_consumer_after_the_queue_drains() {
        let queue = Arc::new(FrameQueue::new(2));
        queue.push(frame(1));
        let closer = {
            let queue = queue.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                queue.close();
            })
        };
        assert_eq!(queue.pop(None).map(|f| f.sequence), Some(1));
        assert!(queue.pop(None).is_none(), "closed and empty");
        closer.join().expect("the closer finished");
    }
}
