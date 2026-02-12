use super::{context::VesContext, ops::{ResolutionOps, LoaderOps, StackOps}};
use crate::{MethodType, ResolutionContext};
use dotnet_types::{TypeDescription, generics::{ConcreteType, GenericLookup}};
use dotnet_value::StackValue;

impl<'a, 'gc, 'm: 'gc> ResolutionOps<'gc, 'm> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn stack_value_type(&self, val: &StackValue<'gc>) -> TypeDescription {
        self.resolver().stack_value_type(val)
    }

    #[inline]
    fn make_concrete(&self, t: &MethodType) -> ConcreteType {
        let f = self.current_frame();
        self.resolver()
            .make_concrete(f.source_resolution, &f.generic_inst, t)
    }

    #[inline]
    fn current_context(&self) -> ResolutionContext<'_, 'm> {
        if !self.frame_stack.is_empty() {
            let f = self.frame_stack.current_frame();
            ResolutionContext {
                generics: &f.generic_inst,
                loader: self.shared.loader,
                resolution: f.source_resolution,
                type_owner: Some(f.state.info_handle.source.parent),
                method_owner: Some(f.state.info_handle.source),
                caches: self.shared.caches.clone(),
                shared: Some(self.shared.clone()),
            }
        } else {
            ResolutionContext {
                generics: &self.shared.empty_generics,
                loader: self.shared.loader,
                resolution: self.shared.loader.corlib_type("System.Object").resolution,
                type_owner: None,
                method_owner: None,
                caches: self.shared.caches.clone(),
                shared: Some(self.shared.clone()),
            }
        }
    }

    #[inline]
    fn with_generics<'b>(&self, lookup: &'b GenericLookup) -> ResolutionContext<'b, 'm> {
        let frame = self.frame_stack.current_frame();
        ResolutionContext {
            loader: self.shared.loader,
            resolution: frame.source_resolution,
            generics: lookup,
            caches: self.shared.caches.clone(),
            type_owner: None,
            method_owner: None,
            shared: Some(self.shared.clone()),
        }
    }
}
