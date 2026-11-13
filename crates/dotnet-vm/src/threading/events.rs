//! VM-owned unnamed event state.

#[cfg(feature = "multithreading")]
pub(crate) use dotnet_utils::sync::Arc as EventRef;
#[cfg(feature = "multithreading")]
use dotnet_utils::sync::Condvar;
use dotnet_utils::sync::Mutex;
#[cfg(not(feature = "multithreading"))]
pub(crate) use std::rc::Rc as EventRef;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct Events {
    events: Mutex<Vec<EventRef<Event>>>,
}

impl Events {
    pub(crate) fn create(&self, manual: bool, signaled: bool) -> Option<isize> {
        let mut events = self.events.lock();
        let id = isize::try_from(events.len()).ok()?.checked_add(1)?;
        events.push(EventRef::new(Event {
            manual,
            state: Mutex::new(State {
                signaled,
                next: 0,
                epoch: 0,
                waiters: VecDeque::new(),
            }),
            #[cfg(feature = "multithreading")]
            changed: Condvar::new(),
        }));
        Some(id)
    }

    pub(crate) fn get(&self, id: isize) -> Option<EventRef<Event>> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.events.lock().get(index).cloned()
    }
}

struct Waiter {
    id: u64,
    deadline: Option<Instant>,
    result: Option<bool>,
}
struct State {
    signaled: bool,
    next: u64,
    epoch: u64,
    waiters: VecDeque<Waiter>,
}
impl State {
    // INVARIANT: F6.EventWaitRegistration — callers hold the event lock across expiration and signal selection.
    fn expire_waiters(&mut self, now: Instant) {
        for waiter in &mut self.waiters {
            if waiter.result.is_none() && waiter.deadline.is_some_and(|d| now >= d) {
                waiter.result = Some(false);
            }
        }
    }

    fn deliver_auto_reset_signal(&mut self) {
        if let Some(waiter) = self.waiters.iter_mut().find(|w| w.result.is_none()) {
            waiter.result = Some(true);
        } else {
            self.signaled = true;
        }
    }
}
pub(crate) struct Event {
    manual: bool,
    state: Mutex<State>,
    #[cfg(feature = "multithreading")]
    changed: Condvar,
}
impl Event {
    pub(crate) fn set(&self) {
        let mut state = self.state.lock();
        state.expire_waiters(Instant::now());
        if self.manual {
            state.signaled = true;
            for waiter in &mut state.waiters {
                if waiter.result.is_none() {
                    waiter.result = Some(true);
                }
            }
        } else {
            state.deliver_auto_reset_signal();
        }
        state.epoch = state
            .epoch
            .checked_add(1)
            .expect("event notification epoch exhausted");
        #[cfg(feature = "multithreading")]
        self.changed.notify_all();
    }
    pub(crate) fn reset(&self) {
        self.state.lock().signaled = false;
    }
    // INVARIANT: F6.EventWaitRegistration — the state lock publishes a unique registration and consumes any stored signal atomically.
    pub(crate) fn begin(self: &EventRef<Self>, deadline: Option<Instant>) -> EventWait {
        let mut state = self.state.lock();
        let id = state.next;
        state.next = state
            .next
            .checked_add(1)
            .expect("event waiter IDs exhausted");
        let result = if state.signaled {
            if !self.manual {
                state.signaled = false;
            }
            Some(true)
        } else if deadline.is_some_and(|d| Instant::now() >= d) {
            Some(false)
        } else {
            None
        };
        state.waiters.push_back(Waiter {
            id,
            deadline,
            result,
        });
        EventWait {
            event: self.clone(),
            id: Some(id),
            deadline,
        }
    }
    pub(crate) fn epoch(&self) -> u64 {
        self.state.lock().epoch
    }
    #[cfg(all(test, feature = "multithreading"))]
    pub(crate) fn notify_spurious(&self) {
        let _state = self.state.lock();
        self.changed.notify_all();
    }
    #[cfg(all(test, feature = "multithreading"))]
    pub(crate) fn expire_waiters_for_test(&self) {
        let mut state = self.state.lock();
        for waiter in &mut state.waiters {
            if waiter.deadline.is_some() {
                waiter.deadline = Some(Instant::now() - Duration::from_secs(1));
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn waiter_count(&self) -> usize {
        self.state.lock().waiters.len()
    }
    #[cfg(feature = "multithreading")]
    pub(crate) fn wait_for_change(&self, epoch: u64, duration: Duration) {
        let mut state = self.state.lock();
        if state.epoch == epoch {
            #[cfg(test)]
            crate::sync::WAIT_TEST_HOOK.with(|hook| {
                if let Some(callback) = hook.borrow_mut().as_mut() {
                    callback();
                }
            });
            self.changed.wait_for(&mut state, duration);
        }
    }
    #[cfg(not(feature = "multithreading"))]
    pub(crate) fn wait_for_change(&self, _epoch: u64, duration: Duration) {
        std::thread::sleep(duration);
    }
}
pub(crate) struct EventWait {
    pub(crate) event: EventRef<Event>,
    id: Option<u64>,
    pub(crate) deadline: Option<Instant>,
}
impl EventWait {
    pub(crate) fn poll(&mut self) -> Option<bool> {
        let mut state = self.event.state.lock();
        let id = self.id.expect("live event waiter");
        let index = state
            .waiters
            .iter()
            .position(|w| w.id == id)
            .expect("live event waiter");
        let waiter = &mut state.waiters[index];
        if waiter.result.is_none() && waiter.deadline.is_some_and(|d| Instant::now() >= d) {
            waiter.result = Some(false);
        }
        let result = waiter.result;
        if result.is_some() {
            state.waiters.remove(index);
            self.id = None;
        }
        result
    }
}
impl Drop for EventWait {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let mut state = self.event.state.lock();
        if let Some(index) = state.waiters.iter().position(|w| w.id == id) {
            let granted = state
                .waiters
                .remove(index)
                .is_some_and(|w| w.result == Some(true));
            if granted && !self.event.manual {
                state.expire_waiters(Instant::now());
                state.deliver_auto_reset_signal();
                state.epoch = state
                    .epoch
                    .checked_add(1)
                    .expect("event notification epoch exhausted");
                #[cfg(feature = "multithreading")]
                self.event.changed.notify_all();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_grants_deadlines_and_cancellation_are_serialized() {
        let registry = Events::default();
        let auto_id = registry.create(false, true).unwrap();
        let auto = registry.get(auto_id).unwrap();
        assert_eq!(auto.begin(None).poll(), Some(true));
        let mut expired = auto.begin(Some(Instant::now() - Duration::from_secs(1)));
        let mut live = auto.begin(None);
        auto.set();
        assert_eq!(expired.poll(), Some(false));
        assert_eq!(live.poll(), Some(true));
        let mut granted = auto.begin(Some(Instant::now() + Duration::from_secs(60)));
        auto.set();
        auto.reset();
        // A grant remains valid even if the clock advances and reset follows it.
        auto.state
            .lock()
            .waiters
            .iter_mut()
            .find(|w| Some(w.id) == granted.id)
            .unwrap()
            .deadline = Some(Instant::now() - Duration::from_secs(1));
        assert_eq!(granted.poll(), Some(true));
        drop(auto.begin(None));
        let cancelled = auto.begin(None);
        let mut successor = auto.begin(None);
        auto.set();
        drop(cancelled);
        assert_eq!(successor.poll(), Some(true));
        auto.set();
        assert_eq!(auto.begin(None).poll(), Some(true));
        assert_eq!(auto.begin(Some(Instant::now())).poll(), Some(false));
        let manual = registry.get(registry.create(true, false).unwrap()).unwrap();
        let mut a = manual.begin(None);
        let mut b = manual.begin(None);
        manual.set();
        manual.reset();
        assert_eq!(a.poll(), Some(true));
        assert_eq!(b.poll(), Some(true));
        assert_eq!(manual.begin(Some(Instant::now())).poll(), Some(false));
        assert!(registry.get(0).is_none());
        assert!(registry.get(-1).is_none());
    }
}
