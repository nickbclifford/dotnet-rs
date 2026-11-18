//! Canonical allocation service queue. Multithreaded builds only.
//!
//! A non-owner activation can request a shared object from the dedicated finalizer executor,
//! whose arena is retained until shutdown. The selected producer suspends without holding its
//! arena mutation while the service materializes the key. The queue below owns only plain data: keys, request
//! identities and completion events. It never carries a `Gc` handle, a borrowed UTF-16 slice or
//! an allocation; completion means "retry the rooted authoritative table lookup".
use dotnet_utils::{
    ArenaId,
    sync::{Arc, Condvar, Mutex},
};
use dotnet_vm_ops::effects::{CanonicalKey, NativeInitSleeper, NativeInitWait};
use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

/// Terminal outcome of a request that did not publish its key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CanonicalError {
    /// Every interested activation dropped the request before the service started it.
    Canceled,
    /// Admission closed (VM shutdown or abort) before the request was published.
    Shutdown,
    /// The owner could not build the object; the failure is terminal for this key.
    Materialization(String),
}

impl CanonicalError {
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Canceled => "canonical request canceled".into(),
            Self::Shutdown => "canonical service shut down".into(),
            Self::Materialization(message) => {
                format!("canonical materialization failed: {message}")
            }
        }
    }
}

/// One completion event per request, shared by every activation waiting on its key.
pub(crate) struct Completion {
    result: Mutex<Option<Result<(), CanonicalError>>>,
    changed: Condvar,
    interest: AtomicUsize,
}

impl Completion {
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            changed: Condvar::new(),
            interest: AtomicUsize::new(0),
        }
    }

    /// The first terminal result wins; later completions (shutdown after publication) are no-ops.
    fn complete(&self, outcome: Result<(), CanonicalError>) {
        let mut result = self.result.lock();
        if result.is_none() {
            *result = Some(outcome);
        }
        self.changed.notify_all();
    }

    pub(crate) fn outcome(&self) -> Option<Result<(), CanonicalError>> {
        self.result.lock().clone()
    }

    fn wait(&self, duration: Duration) {
        let mut result = self.result.lock();
        if result.is_none() {
            self.changed.wait_for(&mut result, duration);
        }
    }
}

/// An activation's interest in a request. Dropping it before the service starts the request
/// cancels the request; it never rolls back an already-published winner.
pub(crate) struct CanonicalWait {
    completion: Arc<Completion>,
}

impl Drop for CanonicalWait {
    fn drop(&mut self) {
        self.completion.interest.fetch_sub(1, Ordering::AcqRel);
    }
}

impl NativeInitWait for CanonicalWait {
    fn ready(&self) -> bool {
        self.completion.outcome().is_some()
    }
    fn sleeper(&self) -> Box<dyn NativeInitSleeper> {
        Box::new(CanonicalSleeper(self.completion.clone()))
    }
}

struct CanonicalSleeper(Arc<Completion>);

impl NativeInitSleeper for CanonicalSleeper {
    fn wait(&self, duration: Duration) {
        self.0.wait(duration);
    }
}

/// An owned request. `id` orders requests and identifies them in diagnostics.
pub(crate) struct CanonicalRequest {
    pub(crate) id: u64,
    pub(crate) key: CanonicalKey,
    completion: Arc<Completion>,
}

#[derive(Default)]
struct State {
    owner: Option<ArenaId>,
    open: bool,
    closed: bool,
    next_id: u64,
    queue: VecDeque<CanonicalRequest>,
    /// Requests that are queued, running, or failed. A success is removed after publication.
    pending: HashMap<CanonicalKey, Arc<Completion>>,
}

/// The queue shared by every executor. One mutex orders admission, shutdown and batch removal;
/// the table lookups that hit never touch it.
#[derive(Default)]
pub(crate) struct CanonicalService {
    state: Mutex<State>,
    /// Signaled by `request`, `close` and `open` so the owner idles without polling latency.
    changed: Condvar,
}

impl CanonicalService {
    /// Start admitting requests for `owner`. The owner has installed its prerequisites first.
    pub(crate) fn open(&self, owner: ArenaId) {
        let mut state = self.state.lock();
        if !state.closed {
            state.owner = Some(owner);
            state.open = true;
        }
        self.changed.notify_all();
    }

    /// Whether requests are currently served. Executors fall back to local allocation when the
    /// service never started (no finalizer thread) or has shut down.
    pub(crate) fn is_active(&self) -> bool {
        let state = self.state.lock();
        state.open && !state.closed
    }

    #[cfg(test)]
    pub(crate) fn owner(&self) -> Option<ArenaId> {
        self.state.lock().owner
    }

    /// Check the recorded owner even if shutdown has closed admission during materialization.
    #[cfg(debug_assertions)]
    pub(crate) fn is_recorded_owner(&self, arena: ArenaId) -> bool {
        self.state.lock().owner == Some(arena)
    }

    /// Whether `arena` is the active service owner. It materializes directly, never queuing.
    pub(crate) fn is_active_owner(&self, arena: ArenaId) -> bool {
        let state = self.state.lock();
        state.open && !state.closed && state.owner == Some(arena)
    }

    /// Whether a request from `arena` must queue instead of allocating locally.
    pub(crate) fn must_queue(&self, arena: ArenaId) -> bool {
        let state = self.state.lock();
        state.open && !state.closed && state.owner != Some(arena)
    }

    /// Decide whether an already-started activation must wait. Unlike an initial fast-path
    /// decision, a closed service is terminal for a request that has not published.
    pub(crate) fn request_needed(&self, arena: ArenaId) -> Result<bool, CanonicalError> {
        let state = self.state.lock();
        if state.closed {
            Err(CanonicalError::Shutdown)
        } else {
            Ok(state.open && state.owner != Some(arena))
        }
    }

    pub(crate) fn is_settled(&self) -> bool {
        let state = self.state.lock();
        state.open || state.closed
    }

    /// Block until the service is open or closed, or `limit` elapses.
    pub(crate) fn wait_settled(&self, limit: Duration) {
        let mut state = self.state.lock();
        if !state.open && !state.closed {
            self.changed.wait_for(&mut state, limit);
        }
    }

    pub(crate) fn has_work(&self) -> bool {
        !self.state.lock().queue.is_empty()
    }

    /// Owner idle wait: returns when a request, shutdown or `limit` arrives.
    pub(crate) fn wait_for_work(&self, limit: Duration) {
        let mut state = self.state.lock();
        if state.queue.is_empty() && !state.closed {
            self.changed.wait_for(&mut state, limit);
        }
    }

    /// Queue `key`, or join its existing request. A terminal failure is shared by all waiters.
    pub(crate) fn request(&self, key: &CanonicalKey) -> Result<CanonicalWait, CanonicalError> {
        let mut state = self.state.lock();
        if state.closed {
            return Err(CanonicalError::Shutdown);
        }
        if let Some(completion) = state.pending.get(key) {
            if let Some(Err(error)) = completion.outcome() {
                return Err(error);
            }
            completion.interest.fetch_add(1, Ordering::AcqRel);
            return Ok(CanonicalWait {
                completion: completion.clone(),
            });
        }
        let completion = Arc::new(Completion::new());
        completion.interest.fetch_add(1, Ordering::AcqRel);
        let id = state.next_id;
        state.next_id += 1;
        state.pending.insert(key.clone(), completion.clone());
        state.queue.push_back(CanonicalRequest {
            id,
            key: key.clone(),
            completion: completion.clone(),
        });
        self.changed.notify_all();
        Ok(CanonicalWait { completion })
    }

    /// Remove up to `max` live requests. Requests nobody wants any longer are canceled here, and
    /// the lock is released before the caller materializes anything.
    pub(crate) fn take_batch(&self, max: usize) -> Vec<CanonicalRequest> {
        let mut state = self.state.lock();
        let mut batch = Vec::new();
        while batch.len() < max {
            let Some(request) = state.queue.pop_front() else {
                break;
            };
            if request.completion.interest.load(Ordering::Acquire) == 0 {
                state.pending.remove(&request.key);
                request.completion.complete(Err(CanonicalError::Canceled));
                continue;
            }
            batch.push(request);
        }
        batch
    }

    /// Complete `request` after its object is published (or its construction failed).
    pub(crate) fn finish(&self, request: CanonicalRequest, outcome: Result<(), CanonicalError>) {
        if outcome.is_ok() {
            self.state.lock().pending.remove(&request.key);
        }
        request.completion.complete(outcome);
    }

    /// Close admission atomically with queue ownership and resolve every unfinished request
    /// with `Shutdown`, waking sleepers. A request already published keeps its success.
    pub(crate) fn close(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        let drained: Vec<_> = state.queue.drain(..).collect();
        for request in drained {
            request.completion.complete(Err(CanonicalError::Shutdown));
        }
        for completion in state.pending.values() {
            completion.complete(Err(CanonicalError::Shutdown));
        }
        state.pending.clear();
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> CanonicalKey {
        CanonicalKey::String(text.encode_utf16().collect())
    }

    #[test]
    fn canonical_homing_cancel_before_service() {
        let service = CanonicalService::default();
        service.open(ArenaId::from(7));
        let wait = service.request(&key("a")).expect("admitted");
        let observer = service.request(&key("a")).expect("joined");
        drop(wait);
        // One interested activation remains, so the request is still live.
        assert_eq!(service.take_batch(8).len(), 1);

        let canceled = service.request(&key("b")).expect("admitted");
        let completion = canceled.completion.clone();
        drop(canceled);
        assert!(service.take_batch(8).is_empty(), "canceled work is skipped");
        assert_eq!(completion.outcome(), Some(Err(CanonicalError::Canceled)));
        // A later request for the same key starts fresh.
        let again = service.request(&key("b")).expect("admitted");
        assert!(!again.ready());
        drop(observer);
    }

    #[test]
    fn canonical_homing_cancel_after_publication() {
        let service = CanonicalService::default();
        service.open(ArenaId::from(7));
        let wait = service.request(&key("a")).expect("admitted");
        let request = service.take_batch(8).pop().expect("queued");
        service.finish(request, Ok(()));
        drop(wait);
        assert!(service.take_batch(8).is_empty());
        // The published success is not rolled back by the later drop, and a new request for
        // the key (a table miss would be the caller's retry) is admitted normally.
        let wait = service.request(&key("a")).expect("admitted");
        assert!(!wait.ready());
    }

    #[test]
    fn canonical_homing_simultaneous_requests_share_one_materialization() {
        let service = CanonicalService::default();
        service.open(ArenaId::from(7));
        let first = service.request(&key("a")).expect("admitted");
        let second = service.request(&key("a")).expect("joined");
        let batch = service.take_batch(8);
        assert_eq!(batch.len(), 1, "one queued request per key");
        let request = batch.into_iter().next().expect("queued");
        assert!(!first.ready() && !second.ready());
        service.finish(request, Ok(()));
        assert!(first.ready() && second.ready());
    }

    #[test]
    fn canonical_homing_shutdown() {
        let service = CanonicalService::default();
        service.open(ArenaId::from(7));
        let queued = service.request(&key("queued")).expect("admitted");
        let running = service.request(&key("running")).expect("admitted");
        let mut batch = service.take_batch(1);
        assert_eq!(batch.len(), 1);
        service.close();
        assert_eq!(
            queued.completion.outcome(),
            Some(Err(CanonicalError::Shutdown))
        );
        assert_eq!(
            running.completion.outcome(),
            Some(Err(CanonicalError::Shutdown))
        );
        // Finishing a request after shutdown cannot overwrite the terminal result.
        service.finish(batch.pop().expect("taken"), Ok(()));
        assert!(matches!(
            service.request(&key("late")),
            Err(CanonicalError::Shutdown)
        ));
        assert!(!service.is_active());
    }

    #[test]
    fn canonical_homing_failure_is_terminal_for_the_key() {
        let service = CanonicalService::default();
        service.open(ArenaId::from(7));
        let wait = service.request(&key("a")).expect("admitted");
        let request = service.take_batch(8).pop().expect("queued");
        service.finish(request, Err(CanonicalError::Materialization("boom".into())));
        assert!(wait.ready());
        assert!(matches!(
            service.request(&key("a")),
            Err(CanonicalError::Materialization(_))
        ));
    }

    #[test]
    fn canonical_homing_owner_never_queues() {
        let service = CanonicalService::default();
        assert!(!service.must_queue(ArenaId::from(1)), "inactive service");
        assert_eq!(service.request_needed(ArenaId::from(1)), Ok(false));
        service.open(ArenaId::from(7));
        assert!(service.must_queue(ArenaId::from(1)));
        assert!(!service.must_queue(ArenaId::from(7)));
        assert_eq!(service.request_needed(ArenaId::from(1)), Ok(true));
        assert_eq!(service.request_needed(ArenaId::from(7)), Ok(false));
        assert!(service.is_active_owner(ArenaId::from(7)));
        service.close();
        assert!(!service.must_queue(ArenaId::from(1)), "closed service");
        assert_eq!(
            service.request_needed(ArenaId::from(1)),
            Err(CanonicalError::Shutdown)
        );
    }
}
