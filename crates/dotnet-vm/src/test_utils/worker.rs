//! Owns a test worker and the driver-side resources that can keep it blocked.
//!
//! Install immediately after spawning, before any fallible handshake. Resources
//! must include every sender/lease whose drop releases the worker. Never keep
//! extra sender clones outside this owner. Cycle/STW guards must be dropped before
//! this owner, and dependent workers must be owned in reverse join order.

use std::thread::{JoinHandle, Result};

pub(crate) struct Worker<T, R> {
    handle: Option<JoinHandle<T>>,
    resources: Option<R>,
}

impl<T, R> Worker<T, R> {
    pub(crate) fn new(handle: JoinHandle<T>, resources: R) -> Self {
        Self {
            handle: Some(handle),
            resources: Some(resources),
        }
    }

    pub(crate) fn resources(&self) -> &R {
        self.resources.as_ref().expect("worker resources released")
    }

    pub(crate) fn resources_mut(&mut self) -> &mut R {
        self.resources.as_mut().expect("worker resources released")
    }

    pub(crate) fn release(&mut self) {
        drop(self.resources.take());
    }

    /// Disconnect/release before joining, and report a worker panic to the driver.
    pub(crate) fn finish(mut self) -> Result<T> {
        self.release();
        self.handle.take().expect("worker already joined").join()
    }
}

impl<T, R> Drop for Worker<T, R> {
    fn drop(&mut self) {
        self.release();
        if let Some(handle) = self.handle.take() {
            // During driver unwind do not introduce a second panic from a worker
            // result or panic payload. All users supply non-panicking resources.
            if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                drop(handle.join());
            })) {
                std::mem::forget(payload);
            }
        }
    }
}
