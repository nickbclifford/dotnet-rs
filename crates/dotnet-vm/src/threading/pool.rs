//! Request accounting for CoreLib's managed work queue.

#[cfg(feature = "multithreading")]
use dotnet_utils::sync::Condvar;
use dotnet_utils::sync::{AtomicU64, Mutex, Ordering};

struct PoolCounts {
    requests: usize,
    workers: usize,
    live: usize,
    #[cfg(feature = "multithreading")]
    idle: usize,
    min_worker: i32,
    min_io: i32,
    max_worker: i32,
    max_io: i32,
}

pub(crate) struct ThreadPool {
    counts: Mutex<PoolCounts>,
    #[cfg(feature = "multithreading")]
    cap: usize,
    completed: AtomicU64,
    #[cfg(feature = "multithreading")]
    changed: Condvar,
}

impl Default for ThreadPool {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism().map_or(1, usize::from);
        Self {
            counts: Mutex::new(PoolCounts {
                requests: 0,
                workers: 0,
                live: 0,
                #[cfg(feature = "multithreading")]
                idle: 0,
                min_worker: parallelism.min(32767) as i32,
                min_io: 1,
                max_worker: 32767,
                max_io: 1000,
            }),
            #[cfg(feature = "multithreading")]
            cap: parallelism.max(2),
            completed: AtomicU64::new(0),
            #[cfg(feature = "multithreading")]
            changed: Condvar::new(),
        }
    }
}

impl ThreadPool {
    /// Reserve a worker under the same lock as the request and idle count.
    #[cfg(feature = "multithreading")]
    pub(crate) fn request(&self) -> bool {
        let mut state = self.counts.lock();
        state.requests = state
            .requests
            .checked_add(1)
            .expect("pool request count exhausted");
        let spawn = if state.requests > state.idle && state.workers < self.cap {
            state.workers += 1;
            true
        } else {
            false
        };
        self.changed.notify_one();
        spawn
    }

    #[cfg(not(feature = "multithreading"))]
    pub(crate) fn request(&self) {
        let mut state = self.counts.lock();
        state.requests = state
            .requests
            .checked_add(1)
            .expect("pool request count exhausted");
    }

    #[cfg(feature = "multithreading")]
    pub(crate) fn claim_or_wait(&self) -> bool {
        {
            let mut state = self.counts.lock();
            if state.requests != 0 {
                state.requests -= 1;
                return true;
            }
        }
        #[cfg(test)]
        crate::stack::native_effects::wait_test_hook(
            crate::stack::native_effects::WaitTestPoint::Probed,
        );
        {
            let mut state = self.counts.lock();
            if state.requests != 0 {
                return false;
            }
            state.idle += 1;
        }
        #[cfg(test)]
        {
            crate::stack::native_effects::wait_test_hook(
                crate::stack::native_effects::WaitTestPoint::Registered,
            );
            crate::executor::WAIT_TEST_HOOK.with(|hook| {
                if let Some(callback) = hook.borrow_mut().take() {
                    callback();
                }
            });
        }
        let mut state = self.counts.lock();
        if state.requests == 0 {
            self.changed
                .wait_for(&mut state, std::time::Duration::from_millis(10));
        }
        state.idle -= 1;
        false
    }

    #[cfg(feature = "multithreading")]
    pub(crate) fn worker_started(&self) {
        self.counts.lock().live += 1;
    }

    #[cfg(feature = "multithreading")]
    pub(crate) fn worker_exited(&self) {
        let mut state = self.counts.lock();
        state.workers -= 1;
        state.live -= 1;
    }

    #[cfg(all(test, feature = "multithreading"))]
    pub(crate) fn pending_requests_for_test(&self) -> usize {
        self.counts.lock().requests
    }

    pub(crate) fn completed(&self) {
        self.completed.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn completed_count(&self) -> i64 {
        self.completed.load(Ordering::Relaxed) as i64
    }
    pub(crate) fn worker_count(&self) -> i32 {
        self.counts.lock().live as i32
    }
    pub(crate) fn set_min(&self, worker: i32, io: i32) -> bool {
        let mut s = self.counts.lock();
        if worker < 0 || io < 0 || worker > s.max_worker || io > s.max_io {
            return false;
        }
        s.min_worker = worker.max(1);
        s.min_io = io.max(1);
        true
    }
    pub(crate) fn set_max(&self, worker: i32, io: i32) -> bool {
        let mut s = self.counts.lock();
        if worker < s.min_worker || io < s.min_io || worker < 1 || io < 1 {
            return false;
        }
        s.max_worker = worker.min(32767);
        s.max_io = io.min(32767);
        true
    }
    pub(crate) fn min(&self) -> (i32, i32) {
        let s = self.counts.lock();
        (s.min_worker, s.min_io)
    }
    pub(crate) fn max(&self) -> (i32, i32) {
        let s = self.counts.lock();
        (s.max_worker, s.max_io)
    }
    pub(crate) fn available(&self) -> (i32, i32) {
        let s = self.counts.lock();
        (s.max_worker.saturating_sub(s.workers as i32), s.max_io)
    }
}

#[cfg(all(test, feature = "multithreading"))]
mod tests {
    use super::ThreadPool;
    use crate::stack::native_effects::{WAIT_TEST_HOOK, WaitTestPoint};
    use dotnet_utils::sync::Arc;

    #[test]
    fn reservations_are_capped_and_idle_workers_are_reused() {
        let pool = ThreadPool {
            cap: 2,
            ..ThreadPool::default()
        };
        let pool = Arc::new(pool);
        assert!(pool.request());
        assert!(pool.request());
        assert_eq!(pool.worker_count(), 0);
        pool.worker_started();
        pool.worker_started();
        for _ in 0..8 {
            assert!(!pool.request());
        }
        assert_eq!(pool.worker_count(), 2);
        for _ in 0..10 {
            assert!(pool.claim_or_wait());
        }
        let other = pool.clone();
        WAIT_TEST_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |point| {
                if matches!(point, WaitTestPoint::Registered) {
                    assert!(!other.request());
                }
            }))
        });
        assert!(!pool.claim_or_wait());
        WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
        assert!(pool.claim_or_wait());
        assert_eq!(pool.worker_count(), 2);
    }

    #[test]
    fn requests_between_probe_registration_and_park_are_not_lost() {
        for at_registration in [false, true] {
            let pool = Arc::new(ThreadPool::default());
            pool.counts.lock().workers = 1;
            let other = pool.clone();
            let mut fired = false;
            WAIT_TEST_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |point| {
                    if !fired
                        && matches!(
                            (at_registration, point),
                            (false, WaitTestPoint::Probed) | (true, WaitTestPoint::Registered)
                        )
                    {
                        fired = true;
                        other.request();
                    }
                }))
            });
            assert!(!pool.claim_or_wait());
            WAIT_TEST_HOOK.with(|hook| drop(hook.borrow_mut().take()));
            assert!(pool.claim_or_wait());
            assert_eq!(pool.counts.lock().requests, 0);
        }
    }
}
