use crate::{
    ResolutionContext,
    stack::{context::VesContext, ops::VmResolutionOps},
};
use dotnet_types::generics::GenericLookup;

impl<'a, 'gc> VmResolutionOps<'gc> for VesContext<'a, 'gc> {
    /// Builds the current frame's [`ResolutionContext`] as a short-lived view.
    ///
    /// This helper is used frequently by resolution/layout/value-type queries while
    /// dispatching IL. We intentionally keep it as an ephemeral stack value instead
    /// of caching it on `VesContext`: after the first call, `resolution_shared()` is
    /// a cheap `OnceLock` read + `Arc` clone, and the owner/resolution `Arc` clones
    /// are also cheap pointer bumps. Caching would require frame-change invalidation
    /// logic and adds statefulness to a currently stateless accessor.
    ///
    /// If profiling on representative workloads ever shows this method as a real
    /// hot spot, revisit by adding a per-frame cache with explicit invalidation.
    #[inline]
    fn current_context(&self) -> ResolutionContext<'_> {
        if !self.frame_stack.is_empty() {
            let f = self.frame_stack.current_frame();
            ResolutionContext {
                generics: &f.generic_inst,
                state: self.shared.resolution_shared(),
                resolution: f.source_resolution.clone(),
                type_owner: Some(f.state.info_handle.source.parent.clone()),
                method_owner: Some(f.state.info_handle.source.clone()),
            }
        } else {
            ResolutionContext {
                generics: &self.shared.empty_generics,
                state: self.shared.resolution_shared(),
                resolution: self
                    .shared
                    .loader
                    .corlib_type("System.Object")
                    .expect("System.Object must exist in corlib")
                    .resolution,
                type_owner: None,
                method_owner: None,
            }
        }
    }

    /// Creates a resolution context with replacement generics for VM-initiated lookups.
    ///
    /// Unlike [`ResolutionContext::with_generics`], this intentionally drops
    /// `type_owner` and `method_owner` (`None` for both). This is used for call paths
    /// such as static field initialization where we want the frame's source resolution
    /// but must not carry owner-specific context from the current frame.
    #[inline]
    fn with_generics<'b>(&self, lookup: &'b GenericLookup) -> ResolutionContext<'b> {
        let frame = self.frame_stack.current_frame();
        ResolutionContext {
            generics: lookup,
            state: self.shared.resolution_shared(),
            resolution: frame.source_resolution.clone(),
            type_owner: None,
            method_owner: None,
        }
    }
}
