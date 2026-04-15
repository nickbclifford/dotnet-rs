#![allow(clippy::mutable_key_type)]
//! Reflection intrinsic handlers and runtime reflection host interfaces.
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
    runtime::RuntimeType,
};
use dotnet_value::{
    layout::LayoutManager,
    object::{Object, ObjectRef},
};
use dotnet_vm_ops::{MethodInfo, ops::ReflectionIntrinsicHost as VmReflectionIntrinsicHost};
use dotnetdll::prelude::{MethodType, UserType};
use std::sync::Arc;

pub mod common;
pub mod fields;
pub mod methods;
pub mod parameters;
pub mod types;

pub trait RuntimeTypeContext {
    fn reflection_locate_type(
        &self,
        handle: UserType,
    ) -> Result<TypeDescription, TypeResolutionError>;

    fn reflection_type_owner(&self) -> Option<TypeDescription>;

    fn reflection_method_owner(&self) -> Option<MethodDescription>;
}

pub trait ResolutionContextHost<'gc> {
    fn reflection_make_runtime_type_with_lookup(
        &self,
        source: &MethodType,
        lookup: &GenericLookup,
    ) -> RuntimeType;

    fn reflection_make_runtime_type_for_method(
        &self,
        method: MethodDescription,
        lookup: &GenericLookup,
        source: &MethodType,
    ) -> RuntimeType;

    fn reflection_is_value_type_with_lookup(
        &self,
        td: TypeDescription,
        lookup: &GenericLookup,
    ) -> Result<bool, TypeResolutionError>;

    fn reflection_new_object_with_lookup(
        &self,
        td: TypeDescription,
        lookup: &GenericLookup,
    ) -> Result<Object<'gc>, TypeResolutionError>;

    fn reflection_method_info(
        &self,
        method: MethodDescription,
        lookup: &GenericLookup,
    ) -> Result<MethodInfo<'static>, TypeResolutionError>;

    fn reflection_empty_generics(&self) -> GenericLookup;

    fn reflection_dispatch_method(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> dotnet_vm_ops::StepResult;

    fn reflection_constructor_frame(
        &mut self,
        instance: Object<'gc>,
        method: MethodInfo<'static>,
        generic_inst: GenericLookup,
    ) -> Result<(), TypeResolutionError>;
}

pub trait LayoutQueryHost {
    fn reflection_type_layout(
        &self,
        t: ConcreteType,
    ) -> Result<Arc<LayoutManager>, TypeResolutionError>;
}

pub trait ReflectionRegistryHost<'gc> {
    fn reflection_cached_runtime_assembly(&self, resolution: ResolutionS)
    -> Option<ObjectRef<'gc>>;

    fn reflection_cache_runtime_assembly(&self, resolution: ResolutionS, object: ObjectRef<'gc>);

    fn reflection_cached_runtime_type(&self, target: &RuntimeType) -> Option<ObjectRef<'gc>>;

    fn reflection_cache_runtime_type(&self, target: RuntimeType, object: ObjectRef<'gc>);

    fn reflection_runtime_type_index_get_or_insert(&self, target: RuntimeType) -> usize;

    fn reflection_runtime_type_by_index(&self, index: usize) -> RuntimeType;

    fn reflection_runtime_method_index_get_or_insert(
        &self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> usize;

    fn reflection_runtime_method_by_index(
        &self,
        index: usize,
    ) -> (MethodDescription, GenericLookup);

    fn reflection_runtime_field_index_get_or_insert(
        &self,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> usize;

    fn reflection_runtime_field_by_index(&self, index: usize) -> (FieldDescription, GenericLookup);

    fn reflection_cached_runtime_method_obj(
        &self,
        method: &MethodDescription,
        lookup: &GenericLookup,
    ) -> Option<ObjectRef<'gc>>;

    fn reflection_cache_runtime_method_obj(
        &self,
        method: MethodDescription,
        lookup: GenericLookup,
        object: ObjectRef<'gc>,
    );

    fn reflection_cached_runtime_field_obj(
        &self,
        field: &FieldDescription,
        lookup: &GenericLookup,
    ) -> Option<ObjectRef<'gc>>;

    fn reflection_cache_runtime_field_obj(
        &self,
        field: FieldDescription,
        lookup: GenericLookup,
        object: ObjectRef<'gc>,
    );
}

dotnet_vm_ops::trait_alias! {
    pub trait ReflectionIntrinsicHost<'gc> =
        VmReflectionIntrinsicHost<'gc>
        + ResolutionContextHost<'gc>
        + LayoutQueryHost
        + ReflectionRegistryHost<'gc>;
}

pub(crate) const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";
