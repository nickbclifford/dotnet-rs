use crate::{NULL_REF_MSG, ReflectionIntrinsicHost};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_value::{
    StackValue,
    layout::{LayoutManager, Scalar},
    object::{HeapStorage, ObjectRef},
};
use dotnet_vm_ops::{
    StepResult,
    ops::{LoaderOps, MemoryOps, TypedStackOps},
};
use dotnetdll::prelude::{BaseType, MemberType, MethodMemberIndex, MethodType, TypeSource};

pub mod type_construction;
pub mod type_members;
pub mod type_queries;

pub use type_construction::*;
pub use type_members::*;
pub use type_queries::*;

#[dotnet_intrinsic("static System.Type System.Type::GetType(string)")]
pub fn intrinsic_type_get_type<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let name_obj = ctx.pop_obj();
    if name_obj.0.is_none() {
        ctx.push(StackValue::null());
        return StepResult::Continue;
    }

    let ObjectRef(Some(name_handle)) = name_obj else {
        ctx.push(StackValue::null());
        return StepResult::Continue;
    };
    let name = {
        let name_storage = name_handle.borrow();
        let HeapStorage::Str(s) = &name_storage.storage else {
            return ctx.throw_by_name_with_message(
                "System.InvalidCastException",
                "Type.GetType requires a string argument.",
            );
        };
        s.as_string()
    };

    match ctx.loader().corlib_type(&name) {
        Ok(td) => {
            let rt_obj = crate::common::get_runtime_type(ctx, RuntimeType::Type(td));
            ctx.push_obj(rt_obj);
        }
        Err(_) => ctx.push(StackValue::null()),
    }

    StepResult::Continue
}

#[dotnet_intrinsic("string System.Type::get_Name()")]
#[dotnet_intrinsic("string System.Type::get_Namespace()")]
#[dotnet_intrinsic("System.Reflection.Assembly System.Type::get_Assembly()")]
#[dotnet_intrinsic("System.Type System.Type::get_BaseType()")]
#[dotnet_intrinsic("bool System.Type::get_IsGenericType()")]
#[dotnet_intrinsic("System.Type System.Type::GetGenericTypeDefinition()")]
#[dotnet_intrinsic("System.Type[] System.Type::GetGenericArguments()")]
#[dotnet_intrinsic("System.RuntimeTypeHandle System.Type::get_TypeHandle()")]
#[dotnet_intrinsic("System.Type System.Type::MakeGenericType(System.Type[])")]
#[dotnet_intrinsic("string System.RuntimeType::get_Name()")]
#[dotnet_intrinsic("string System.RuntimeType::GetName()")]
#[dotnet_intrinsic("string System.RuntimeType::get_Namespace()")]
#[dotnet_intrinsic("string System.RuntimeType::GetNamespace()")]
#[dotnet_intrinsic("System.Reflection.Assembly System.RuntimeType::get_Assembly()")]
#[dotnet_intrinsic("System.Reflection.Assembly System.RuntimeType::GetAssembly()")]
#[dotnet_intrinsic("System.Type System.RuntimeType::get_BaseType()")]
#[dotnet_intrinsic("System.Type System.RuntimeType::GetBaseType()")]
#[dotnet_intrinsic("bool System.RuntimeType::get_IsGenericType()")]
#[dotnet_intrinsic("bool System.RuntimeType::GetIsGenericType()")]
#[dotnet_intrinsic("System.Type System.RuntimeType::get_GenericTypeDefinition()")]
#[dotnet_intrinsic("System.Type System.RuntimeType::GetGenericTypeDefinition()")]
#[dotnet_intrinsic("System.Type[] System.RuntimeType::GetGenericArguments()")]
#[dotnet_intrinsic("System.RuntimeTypeHandle System.RuntimeType::get_TypeHandle()")]
#[dotnet_intrinsic("System.Type System.RuntimeType::MakeGenericType(System.Type[])")]
#[dotnet_intrinsic(
    "System.Reflection.MethodInfo[] System.RuntimeType::GetMethods(System.Reflection.BindingFlags)"
)]
#[dotnet_intrinsic(
    "System.Reflection.MethodInfo System.RuntimeType::GetMethodImpl(string, System.Reflection.BindingFlags, System.Reflection.Binder, System.Reflection.CallingConventions, System.Type[], System.Reflection.ParameterModifier[])"
)]
#[dotnet_intrinsic(
    "System.Reflection.ConstructorInfo[] System.RuntimeType::GetConstructors(System.Reflection.BindingFlags)"
)]
#[dotnet_intrinsic(
    "System.Reflection.ConstructorInfo System.RuntimeType::GetConstructorImpl(System.Reflection.BindingFlags, System.Reflection.Binder, System.Reflection.CallingConventions, System.Type[], System.Reflection.ParameterModifier[])"
)]
#[dotnet_intrinsic(
    "System.Reflection.FieldInfo[] System.RuntimeType::GetFields(System.Reflection.BindingFlags)"
)]
#[dotnet_intrinsic(
    "System.Reflection.FieldInfo System.RuntimeType::GetField(string, System.Reflection.BindingFlags)"
)]
#[dotnet_intrinsic(
    "System.Reflection.PropertyInfo[] System.RuntimeType::GetProperties(System.Reflection.BindingFlags)"
)]
#[dotnet_intrinsic(
    "System.Reflection.PropertyInfo System.RuntimeType::GetPropertyImpl(string, System.Reflection.BindingFlags, System.Reflection.Binder, System.Type, System.Type[], System.Reflection.ParameterModifier[])"
)]
#[dotnet_intrinsic("bool System.RuntimeType::IsPrimitiveImpl()")]
#[dotnet_intrinsic("bool System.RuntimeType::IsArrayImpl()")]
#[dotnet_intrinsic("bool System.RuntimeType::IsByRefImpl()")]
#[dotnet_intrinsic("bool System.RuntimeType::IsPointerImpl()")]
#[dotnet_intrinsic("System.Type[] System.RuntimeType::GetInterfaces()")]
#[dotnet_intrinsic("System.Type System.RuntimeType::GetInterface(string, bool)")]
#[dotnet_intrinsic("System.Type System.RuntimeType::GetElementType()")]
#[dotnet_intrinsic("System.Reflection.TypeAttributes System.RuntimeType::GetAttributeFlagsImpl()")]
pub fn runtime_type_intrinsic_call<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let method_name = &*method.method().name;
    let param_count = method.method().signature.parameters.len();

    match (method_name, param_count) {
        ("CreateInstanceCheckThis", 0) => handle_create_instance_check_this(ctx, generics),
        ("GetAssembly" | "get_Assembly", 0) => handle_get_assembly(ctx, generics),
        ("GetNamespace" | "get_Namespace", 0) => handle_get_namespace(ctx, generics),
        ("GetMethods", 1) => handle_get_methods(ctx, generics),
        ("GetMethodImpl", 6) => handle_get_method_impl(ctx, generics),
        ("GetConstructors", 1) => handle_get_constructors(ctx, generics),
        ("GetConstructorImpl", 5) => handle_get_constructor_impl(ctx, generics),
        ("GetMembers", 1) => handle_get_members(ctx, generics),
        ("GetNestedTypes", 1) => handle_get_nested_types(ctx, generics),
        ("InvokeMember", 8) => handle_invoke_member(ctx, generics),
        ("GetName" | "get_Name", 0) => handle_get_name(ctx, generics),
        ("GetBaseType" | "get_BaseType", 0) => handle_get_base_type(ctx, generics),
        ("GetIsGenericType" | "get_IsGenericType", 0) => handle_get_is_generic_type(ctx, generics),
        ("GetGenericTypeDefinition" | "get_GenericTypeDefinition", 0) => {
            handle_get_generic_type_definition(ctx, generics)
        }
        ("GetGenericArguments" | "get_GenericArguments", 0) => {
            handle_get_generic_arguments(ctx, generics)
        }
        ("GetTypeHandle" | "get_TypeHandle", 0) => handle_get_type_handle(ctx, generics),
        ("MakeGenericType", 1) => handle_make_generic_type(ctx, generics),
        ("CreateInstanceDefaultCtor", 2) => handle_create_instance_default_ctor(ctx, generics),
        ("GetFields", 1) => handle_get_fields(ctx, generics),
        ("GetField", 2) => handle_get_field(ctx, generics),
        ("GetProperties", 1) => handle_get_properties(ctx, generics),
        ("GetPropertyImpl", 6) => handle_get_property_impl(ctx, generics),
        ("IsPrimitiveImpl", 0) => handle_is_primitive_impl(ctx, generics),
        ("IsArrayImpl", 0) => handle_is_array_impl(ctx, generics),
        ("IsByRefImpl", 0) => handle_is_by_ref_impl(ctx, generics),
        ("IsPointerImpl", 0) => handle_is_pointer_impl(ctx, generics),
        ("GetInterfaces", 0) => handle_get_interfaces(ctx, generics),
        ("GetInterface", 2) => handle_get_interface(ctx, generics),
        ("GetElementType", 0) => handle_get_element_type(ctx, generics),
        ("GetAttributeFlagsImpl", 0) => handle_get_attribute_flags_impl(ctx, generics),
        _ => {
            let message = format!(
                "Unsupported System.RuntimeType intrinsic call: {}.{} with {} parameter(s).",
                method.parent.type_name(),
                method_name,
                param_count
            );
            ctx.throw_by_name_with_message("System.MissingMethodException", &message)
        }
    }
}

pub(crate) fn member_to_method_type(src: &TypeSource<MemberType>) -> MethodType {
    match src {
        TypeSource::User(h) => MethodType::Base(Box::new(BaseType::Type {
            source: TypeSource::User(*h),
            value_kind: None,
        })),
        TypeSource::Generic { base, parameters } => MethodType::Base(Box::new(BaseType::Type {
            source: TypeSource::Generic {
                base: *base,
                parameters: parameters.iter().cloned().map(MethodType::from).collect(),
            },
            value_kind: None,
        })),
    }
}

pub(crate) fn build_generic_lookup_from_runtime_type<T: LoaderOps>(
    ctx: &mut T,
    target_type: &RuntimeType,
) -> GenericLookup {
    let mut lookup = GenericLookup::default();
    if let RuntimeType::Generic(_, args) = target_type {
        lookup.type_generics = args
            .iter()
            .map(|a| a.to_concrete(ctx.loader().as_ref()))
            .collect::<Vec<_>>()
            .into();
    }
    lookup
}

pub(crate) fn populate_reflection_array<'gc, T: MemoryOps<'gc> + TypedStackOps<'gc>>(
    ctx: &mut T,
    items: Vec<ObjectRef<'gc>>,
    item_type: ConcreteType,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let mut vector = match ctx.new_vector(item_type, items.len()) {
        Ok(v) => v,
        Err(e) => return StepResult::Error(e.into()),
    };
    for (i, item) in items.into_iter().enumerate() {
        item.write(&mut vector.get_mut()[i * ObjectRef::SIZE..(i + 1) * ObjectRef::SIZE]);
    }
    ctx.push_obj(ObjectRef::new(gc, HeapStorage::Vec(vector)));
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static void System.RuntimeTypeHandle::GetActivationInfo(System.RuntimeTypeHandle, System.IntPtr&, System.IntPtr&, System.IntPtr&, bool&)"
)]
pub fn runtime_type_handle_intrinsic_call<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let method_name = &*method.method().name;
    let param_count = method.method().signature.parameters.len();

    match (method_name, param_count) {
        ("GetActivationInfo", 5) => {
            let ctor_is_public = ctx.pop_managed_ptr();
            let pfn_ctor = ctx.pop_managed_ptr();
            let allocator_first_arg = ctx.pop_managed_ptr();
            let pfn_allocator = ctx.pop_managed_ptr();

            let rt_obj = match ctx.pop() {
                StackValue::ValueType(rth_handle) => rth_handle
                    .instance_storage
                    .field::<ObjectRef<'gc>>(rth_handle.description, "_value")
                    .unwrap()
                    .read(),
                StackValue::ObjectRef(rt_obj) => rt_obj,
                v => {
                    let message = format!(
                        "Invalid type on stack ({:?}); expected RuntimeTypeHandle or RuntimeType.",
                        v
                    );
                    return ctx.throw_by_name_with_message("System.InvalidCastException", &message);
                }
            };

            let rt = crate::common::resolve_runtime_type(ctx, rt_obj);
            let td = match &rt {
                RuntimeType::Type(td) => td.clone(),
                RuntimeType::Generic(td, _) => td.clone(),
                _ => {
                    return ctx.throw_by_name_with_message(
                        "System.ArgumentException",
                        "GetActivationInfo requires a RuntimeType that represents a type.",
                    );
                }
            };

            let val = StackValue::NativeInt(0);
            let layout = LayoutManager::Scalar(Scalar::NativeInt);
            unsafe {
                vm_try!(ctx.write_unaligned(
                    pfn_allocator.origin().clone(),
                    pfn_allocator.byte_offset(),
                    val,
                    &layout,
                ));
            }

            let val = StackValue::NativeInt(0);
            unsafe {
                vm_try!(ctx.write_unaligned(
                    allocator_first_arg.origin().clone(),
                    allocator_first_arg.byte_offset(),
                    val,
                    &layout,
                ));
            }

            let mut found_ctor = false;
            for (idx, m) in td.definition().methods.iter().enumerate() {
                if m.name == ".ctor" && m.signature.instance && m.signature.parameters.is_empty() {
                    let lookup = if let RuntimeType::Generic(_, type_generics) = rt {
                        GenericLookup::new(
                            type_generics
                                .iter()
                                .map(|t| t.to_concrete(ctx.loader().as_ref()))
                                .collect(),
                        )
                    } else {
                        ctx.reflection_empty_generics()
                    };
                    let method_idx = crate::common::get_runtime_method_index(
                        ctx,
                        MethodDescription::new(
                            td.clone(),
                            lookup.clone(),
                            td.resolution.clone(),
                            MethodMemberIndex::Method(idx),
                        ),
                        lookup,
                    );

                    let method_val = StackValue::NativeInt(method_idx as isize);
                    let layout = LayoutManager::Scalar(Scalar::NativeInt);
                    unsafe {
                        vm_try!(ctx.write_unaligned(
                            pfn_ctor.origin().clone(),
                            pfn_ctor.byte_offset(),
                            method_val,
                            &layout,
                        ));
                    }

                    let public_val = StackValue::Int32(1);
                    let byte_layout = LayoutManager::Scalar(Scalar::Int8);
                    unsafe {
                        vm_try!(ctx.write_unaligned(
                            ctor_is_public.origin().clone(),
                            ctor_is_public.byte_offset(),
                            public_val,
                            &byte_layout,
                        ));
                    }
                    found_ctor = true;
                    break;
                }
            }

            if !found_ctor {
                let message = format!(
                    "Could not find parameterless constructor for {}.",
                    td.type_name()
                );
                return ctx.throw_by_name_with_message("System.MissingMethodException", &message);
            }

            StepResult::Continue
        }
        _ => {
            let message = format!(
                "Unsupported System.RuntimeTypeHandle intrinsic call: {}.{} with {} parameter(s).",
                method.parent.type_name(),
                method_name,
                param_count
            );
            ctx.throw_by_name_with_message("System.MissingMethodException", &message)
        }
    }
}

#[dotnet_intrinsic(
    "static System.RuntimeTypeHandle System.Runtime.CompilerServices.RuntimeHelpers::GetMethodTable(System.RuntimeTypeHandle)"
)]
pub fn intrinsic_runtime_helpers_get_method_table<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let obj = ctx.pop();
    let object_type = match obj {
        StackValue::ObjectRef(o @ ObjectRef(Some(_))) => ctx.get_heap_description(o),
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
        value => {
            let message = format!(
                "Invalid type on stack ({:?}); expected object reference.",
                value
            );
            return ctx.throw_by_name_with_message("System.InvalidCastException", &message);
        }
    };

    let mt_ptr = vm_try!(object_type).definition() as *const _;
    ctx.push_isize(mt_ptr as isize);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static bool System.Runtime.CompilerServices.RuntimeHelpers::IsBitwiseEquatable<T>()"
)]
pub fn intrinsic_runtime_helpers_is_bitwise_equatable<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = vm_try!(ctx.reflection_type_layout(target.clone()));
    let value = match &*layout {
        LayoutManager::Scalar(Scalar::ObjectRef) => false,
        LayoutManager::Scalar(_) => true,
        _ => false,
    };
    ctx.push_i32(value as i32);
    StepResult::Continue
}

#[dotnet_intrinsic(
    "static bool System.Runtime.CompilerServices.RuntimeHelpers::IsReferenceOrContainsReferences()"
)]
pub fn intrinsic_runtime_helpers_is_reference_or_contains_references<
    'gc,
    T: ReflectionIntrinsicHost<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let layout = vm_try!(ctx.reflection_type_layout(target.clone()));
    let res = layout.is_or_contains_refs();

    ctx.push_i32(res as i32);
    StepResult::Continue
}
