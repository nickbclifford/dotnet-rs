//! Chosen VM access API: async acquisition and RAII rooted views, never raw Gc references.
//!
//! A view keeps the exclusive effects loan live through its VM-access guard.
//! ```compile_fail
//! use effect_ops::*;
//! async fn bad(mut fx: VmEffects, source: StringRoot) {
//!     let mut vm = fx.vm().await.unwrap();
//!     let mut source = vm.string(&source);
//!     fx.safe_point().await.unwrap();
//!     let _ = source.substring_len(0, None);
//! }
//! ```
//! A view cannot outlive the guard, even if the view would only be used synchronously.
//! ```compile_fail
//! use effect_ops::*;
//! async fn bad(mut fx: VmEffects, source: StringRoot) {
//!     let mut source = {
//!         let mut vm = fx.vm().await.unwrap();
//!         vm.string(&source)
//!     };
//!     let _ = source.substring_len(0, None);
//! }
//! ```
//! The escape hatch still has the generative callback boundary.
//! ```compile_fail
//! use effect_ops::*;
//! async fn bad(mut fx: VmEffects, source: Stashed) {
//!     let mut vm = fx.vm().await.unwrap();
//!     let value = vm.with(|cx| source.get(cx));
//!     drop(value);
//! }
//! ```

use crate::{
    ArrayRoot, Failure, ObjectData, Outcome, Region, RegionAccess, Stashed, StaticAction,
    StringRoot, VmEffects,
};

/// An exclusive loan of the effect capability. Contains no erased/branded GC pointers.
/// Drop never accesses a poll-local context. All heap borrows are method-local regions.
pub struct VmAccess<'fx> {
    fx: &'fx mut VmEffects,
}
impl Drop for VmAccess<'_> {
    fn drop(&mut self) {
        // The Rust borrow ends here. There is no stored heap lock or raw context to release.
    }
}
impl VmEffects {
    /// Internal non-checkpointing constructor; exposed only for comparison/safety tests.
    #[doc(hidden)]
    pub fn borrow_vm(&mut self) -> VmAccess<'_> {
        VmAccess { fx: self }
    }

    /// Honor a pending stop or exhausted native quantum, then lend access until dropped.
    pub async fn vm(&mut self) -> Result<VmAccess<'_>, Failure> {
        let stop_requested = self.enter(|cx| cx.requested());
        if stop_requested || self.0.access_remaining.get() == 0 {
            self.safe_point().await?;
        }
        let remaining = self.0.access_remaining.get();
        assert!(remaining > 0, "access resumed without a fresh poll quantum");
        self.0.access_remaining.set(remaining - 1);
        Ok(self.borrow_vm())
    }
}
impl<'fx> VmAccess<'fx> {
    /// Scoped escape hatch for helpers not yet expressed on the rooted view facade.
    pub fn with<R>(&mut self, f: impl for<'gc> FnOnce(&mut Region<'gc, 'gc>) -> R) -> R {
        self.fx.enter(f)
    }
    pub fn string<'view>(&'view mut self, source: &'view StringRoot) -> VmString<'view, 'fx> {
        VmString {
            access: self,
            source,
        }
    }
    pub fn new_string(&mut self, chars: Vec<u16>) -> Stashed {
        self.with(|cx| cx.string(chars).0)
    }
    pub fn stop_requested(&mut self) -> bool {
        self.with(|cx| cx.requested())
    }
    pub fn static_action(&mut self, ty: usize) -> StaticAction {
        self.with(|cx| cx.host().static_action(ty))
    }
    pub fn array_at(&mut self, owner: &ArrayRoot, index: usize) -> Outcome {
        self.with(|cx| {
            let value = {
                let object = owner.get(cx);
                let object = object.borrow();
                let ObjectData::Array(values) = &*object else {
                    return Err(Failure::Managed("ArgumentException"));
                };
                *values
                    .get(index)
                    .ok_or(Failure::Managed("IndexOutOfRangeException"))?
            };
            Ok(cx.stash(value))
        })
    }
}
/// A rooted proxy, not a borrowed UTF-16 slice or ObjectRef. No Deref implementation.
pub struct VmString<'view, 'fx> {
    access: &'view mut VmAccess<'fx>,
    source: &'view StringRoot,
}
impl VmString<'_, '_> {
    pub fn substring_len(&mut self, start: i32, length: Option<i32>) -> Result<usize, Failure> {
        self.access.with(|cx| {
            let source = self.source.get(cx);
            let source = source.borrow();
            let ObjectData::String(chars) = &*source else {
                return Err(Failure::Managed("ArgumentException"));
            };
            if start < 0 || start as usize > chars.len() {
                return Err(Failure::Managed("ArgumentOutOfRangeException:startIndex"));
            }
            let count = length.unwrap_or((chars.len() - start as usize) as i32);
            if count < 0 || count as usize > chars.len() - start as usize {
                return Err(Failure::Managed("ArgumentOutOfRangeException:length"));
            }
            Ok(count as usize)
        })
    }
    pub fn copy_range(
        &mut self,
        start: usize,
        end: usize,
        output: &mut Vec<u16>,
    ) -> Result<(), Failure> {
        self.access.with(|cx| {
            let source = self.source.get(cx);
            {
                let source = source.borrow();
                let ObjectData::String(chars) = &*source else {
                    return Err(Failure::Managed("ArgumentException"));
                };
                let chunk = chars
                    .get(start..end)
                    .ok_or(Failure::Managed("ArgumentOutOfRangeException"))?;
                output.extend_from_slice(chunk);
            }
            cx.host().copied(end - start);
            Ok(())
        })
    }
}
