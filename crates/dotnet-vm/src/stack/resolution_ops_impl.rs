use crate::{
    ResolutionContext,
    stack::{context::VesContext, ops::ResolutionOps},
};
use dotnet_types::generics::GenericLookup;

impl<'a, 'gc> ResolutionOps<'gc> for VesContext<'a, 'gc> {
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
