//! Rejected region-call alternatives, retained only for the compiled gallery.
//!
//! An awaitable request must still return only unbranded results.
//! ```compile_fail
//! use effect_ops::*;
//! async fn bad(mut fx: VmEffects, root: Stashed) {
//!     let value = fx.vm_ready(|cx| root.get(cx)).await;
//!     fx.safe_point().await.unwrap();
//!     drop(value);
//! }
//! ```
//! ```compile_fail
//! use effect_ops::*;
//! async fn bad(mut fx: VmEffects) {
//!     let guard = fx.vm_ready(|cx| cx.scope()).await;
//!     fx.safe_point().await.unwrap();
//!     drop(guard);
//! }
//! ```
//! ```compile_fail
//! use effect_ops::*;
//! async fn bad(mut fx: VmEffects, root: Stashed) {
//!     let value = fx.vm_async(async |cx| root.get(cx)).await;
//!     fx.safe_point().await.unwrap();
//!     drop(value);
//! }
//! ```

use crate::{Region, RegionAccess, VmEffects};
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

impl VmEffects {
    /// Synchronous candidate: scoped VM access, including GC-branded state.
    #[doc(hidden)]
    pub fn with_vm<R>(&mut self, f: impl for<'gc> FnOnce(&mut Region<'gc, 'gc>) -> R) -> R {
        self.enter(f)
    }

    /// Awaitable candidate: runs the synchronous region callback during this poll.
    /// Always Ready on its first poll; this operation never parks or collects.
    #[doc(hidden)]
    pub async fn vm_ready<R>(&mut self, f: impl for<'gc> FnOnce(&mut Region<'gc, 'gc>) -> R) -> R {
        self.enter(f)
    }

    /// Async-closure trial. Borrowing async helpers work, but actual suspension panics.
    /// The callback future stays inside the region and is dropped before region exit.
    #[doc(hidden)]
    pub async fn vm_async<R>(
        &mut self,
        f: impl for<'gc> AsyncFnOnce(&mut Region<'gc, 'gc>) -> R,
    ) -> R {
        self.enter(|cx| {
            let mut future = pin!(f(cx));
            match future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
            {
                Poll::Ready(value) => value,
                Poll::Pending => panic!("async VM region attempted to suspend"),
            }
        })
    }
}

impl VmEffects {
    /// Awaitable region which first honors a pending GC-stop request.
    /// The callback is synchronous and fallible; no branded value crosses suspension.
    /// Checkpointing does not allow a branded result to escape either.
    /// ```compile_fail
    /// use effect_ops::*;
    /// async fn bad(mut fx: VmEffects, root: Stashed) -> Outcome {
    ///     let value = fx.checkpoint_vm(|cx| Ok(root.get(cx))).await?;
    ///     fx.safe_point().await?;
    ///     drop(value);
    ///     Ok(Stashed::Void)
    /// }
    /// ```
    #[doc(hidden)]
    pub async fn checkpoint_vm<R>(
        &mut self,
        f: impl for<'gc> FnOnce(&mut Region<'gc, 'gc>) -> Result<R, crate::Failure>,
    ) -> Result<R, crate::Failure> {
        if self.enter(|cx| cx.requested()) {
            self.safe_point().await?;
        }
        self.enter(f)
    }
}
