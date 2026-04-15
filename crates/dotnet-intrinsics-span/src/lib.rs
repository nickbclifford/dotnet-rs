//! Span and ReadOnlySpan intrinsic handlers plus span host abstractions.
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    runtime::RuntimeType,
};
use dotnet_utils::ByteOffset;
use dotnet_value::{
    StackValue,
    layout::LayoutManager,
    object::{Object, ObjectRef},
    pointer::PointerOrigin,
};
use dotnet_vm_ops::{StepResult, ops::SpanIntrinsicHost as VmSpanIntrinsicHost};
use std::sync::Arc;

pub mod conversions;
pub mod ctor;
pub mod equality;
pub mod helpers;

pub trait LayoutQueryHost {
    fn span_type_layout(&self, t: ConcreteType) -> Result<Arc<LayoutManager>, TypeResolutionError>;
}

pub trait SpanPointerIntrospectionHost<'gc> {
    fn span_ptr_info(
        &mut self,
        val: &StackValue<'gc>,
    ) -> Result<(PointerOrigin<'gc>, ByteOffset), StepResult>;
}

pub trait SpanObjectFactoryHost<'gc> {
    fn span_new_object_with_type_generics(
        &self,
        td: TypeDescription,
        type_generics: Vec<ConcreteType>,
    ) -> Result<Object<'gc>, TypeResolutionError>;
}

pub trait SpanRuntimeHost<'gc> {
    fn span_resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup);

    fn span_resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> RuntimeType;

    fn span_dispatch_method(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> StepResult;
}

dotnet_vm_ops::trait_alias! {
    pub trait SpanIntrinsicHost<'gc> =
        VmSpanIntrinsicHost<'gc>
        + LayoutQueryHost
        + SpanPointerIntrospectionHost<'gc>
        + SpanObjectFactoryHost<'gc>
        + SpanRuntimeHost<'gc>;
}
