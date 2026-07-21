use std::collections::VecDeque;
use std::sync::{Condvar, LockResult, Mutex, MutexGuard};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PushError<T> {
    Full(T),
    Closed(T),
    Poisoned(T),
}

#[derive(Debug)]
struct QueueState<T> {
    items: VecDeque<T>,
    closed: bool,
}

#[derive(Debug)]
pub(crate) struct BoundedQueue<T> {
    capacity: usize,
    state: Mutex<QueueState<T>>,
    available: Condvar,
}

impl<T> BoundedQueue<T> {
    pub(crate) fn new(capacity: usize) -> Option<Self> {
        if capacity == 0 {
            return None;
        }

        let mut items = VecDeque::new();
        items.try_reserve_exact(capacity).ok()?;

        Some(Self {
            capacity,
            state: Mutex::new(QueueState {
                items,
                closed: false,
            }),
            available: Condvar::new(),
        })
    }

    pub(crate) fn try_push(&self, item: T) -> Result<(), PushError<T>> {
        let mut state = recover_lock(self.state.lock());

        if state.closed {
            return Err(PushError::Closed(item));
        }
        if state.items.len() == self.capacity {
            return Err(PushError::Full(item));
        }

        state.items.push_back(item);
        drop(state);
        self.available.notify_one();
        Ok(())
    }

    pub(crate) fn pop(&self) -> Option<T> {
        let mut state = recover_lock(self.state.lock());

        loop {
            if let Some(item) = state.items.pop_front() {
                return Some(item);
            }
            if state.closed {
                return None;
            }

            state = recover_lock(self.available.wait(state));
        }
    }

    pub(crate) fn close_and_drain(&self) -> Vec<T> {
        let drained = {
            let mut state = recover_lock(self.state.lock());
            state.closed = true;
            state.items.drain(..).collect()
        };
        self.available.notify_all();
        drained
    }
}

fn recover_lock<T>(
    result: LockResult<MutexGuard<'_, QueueState<T>>>,
) -> MutexGuard<'_, QueueState<T>> {
    match result {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{BoundedQueue, PushError};

    #[test]
    fn push_is_fifo_and_full_is_non_blocking() {
        let queue = BoundedQueue::new(2).expect("valid capacity");
        queue.try_push("a").expect("first item");
        queue.try_push("b").expect("second item");
        assert!(matches!(queue.try_push("c"), Err(PushError::Full("c"))));
        assert_eq!(queue.pop(), Some("a"));
        assert_eq!(queue.pop(), Some("b"));
    }

    #[test]
    fn close_drains_pending_items_and_wakes_waiters() {
        let queue = Arc::new(BoundedQueue::<&str>::new(2).expect("valid capacity"));
        let waiting = Arc::clone(&queue);
        let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            result_sender.send(waiting.pop()).expect("send pop result");
        });
        assert!(queue.close_and_drain().is_empty());
        assert_eq!(
            result_receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("closed queue wakes worker"),
            None
        );
        worker.join().expect("worker");

        let pending = BoundedQueue::new(2).expect("valid pending queue");
        pending.try_push("not-started").expect("pending item");
        assert_eq!(pending.close_and_drain(), vec!["not-started"]);
        assert_eq!(queue.pop(), None);
        assert!(matches!(
            queue.try_push("late"),
            Err(PushError::Closed("late"))
        ));
    }
}
