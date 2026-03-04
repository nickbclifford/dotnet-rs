use crate::{
    MethodType, ResolutionContext,
    stack::{
        context::VesContext,
        ops::{LoaderOps, ResolutionOps, StackOps},
    },
};
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
};
use dotnet_value::StackValue;

impl<'a, 'gc> ResolutionOps<'gc> for VesContext<'a, 'gc> {
    #[inline]
    fn stack_value_type(
        &self,
        val: &StackValue<'gc>,
    ) -> Result<TypeDescription, TypeResolutionError> {
        self.resolver().stack_value_type(val)
    }

    #[inline]
    fn make_concrete(&self, t: &MethodType) -> Result<ConcreteType, TypeResolutionError> {
        let f = self.current_frame();
        self.resolver()
            .make_concrete(f.source_resolution, &f.generic_inst, t)
    }

    #[inline]
    fn current_context(&self) -> ResolutionContext<'_> {
        if !self.frame_stack.is_empty() {
            let f = self.frame_stack.current_frame();
            ResolutionContext {
                generics: &f.generic_inst,
                loader: self.shared.loader.clone(),
                resolution: f.source_resolution,
                type_owner: Some(f.state.info_handle.source.parent),
                method_owner: Some(f.state.info_handle.source),
                caches: self.shared.caches.clone(),
                shared: Some(self.shared.clone()),
            }
        } else {
            ResolutionContext {
                generics: &self.shared.empty_generics,
                loader: self.shared.loader.clone(),
                resolution: self
                    .shared
                    .loader
                    .corlib_type("System.Object")
                    .expect("System.Object must exist in corlib")
                    .resolution,
                type_owner: None,
                method_owner: None,
                caches: self.shared.caches.clone(),
                shared: Some(self.shared.clone()),
            }
        }
    }

    #[inline]
    fn with_generics<'b>(&self, lookup: &'b GenericLookup) -> ResolutionContext<'b> {
        let frame = self.frame_stack.current_frame();
        ResolutionContext {
            loader: self.shared.loader.clone(),
            resolution: frame.source_resolution,
            generics: lookup,
            caches: self.shared.caches.clone(),
            type_owner: None,
            method_owner: None,
            shared: Some(self.shared.clone()),
        }
    }
}
