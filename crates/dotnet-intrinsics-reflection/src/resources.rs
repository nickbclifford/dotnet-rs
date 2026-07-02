//! Intrinsics for managed resource loading (`System.Resources`).

use crate::{ReflectionIntrinsicHost, types::string_from_heap_obj};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    error::{ExecutionError, VmError},
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    runtime::RuntimeType,
};
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
};
use dotnet_vm_ops::StepResult;
use dotnetdll::prelude::{MethodMemberIndex, ParameterType, resource::Implementation};

fn find_memory_stream_byte_array_ctor<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    memory_stream_type: &TypeDescription,
) -> Option<MethodDescription> {
    let lookup = GenericLookup::default();

    for (idx, method) in memory_stream_type.definition().methods.iter().enumerate() {
        if method.name != ".ctor" {
            continue;
        }

        let candidate = MethodDescription::new(
            memory_stream_type.clone(),
            lookup.clone(),
            memory_stream_type.resolution.clone(),
            MethodMemberIndex::Method(idx),
        );
        let signature = candidate.signature();

        if !signature.instance || signature.parameters.len() != 1 {
            continue;
        }

        let ParameterType::Value(parameter_type) = &signature.parameters[0].1 else {
            continue;
        };

        let runtime_type =
            ctx.reflection_make_runtime_type_for_method(candidate.clone(), &lookup, parameter_type);
        if matches!(runtime_type, RuntimeType::Vector(inner) if *inner == RuntimeType::UInt8) {
            return Some(candidate);
        }
    }

    None
}

#[dotnet_intrinsic(
    "System.IO.Stream System.Reflection.RuntimeAssembly::GetManifestResourceStream(string)"
)]
pub fn intrinsic_runtime_assembly_get_manifest_resource_stream<
    'gc,
    T: ReflectionIntrinsicHost<'gc>,
>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Instance call, stack top-down: [name, this].
    let name_obj = ctx.pop_obj();
    let runtime_assembly_obj = ctx.pop_obj();

    let resource_name = dotnet_vm_ops::vm_try!(string_from_heap_obj(name_obj));

    let Some(resolution) = ctx.reflection_runtime_assembly_resolution(runtime_assembly_obj) else {
        ctx.push(StackValue::null());
        return StepResult::Continue;
    };

    let Some(resource_bytes) =
        resolution
            .definition()
            .manifest_resources
            .iter()
            .find_map(|entry| {
                if entry.name.as_ref() != resource_name {
                    return None;
                }

                match &entry.implementation {
                    Implementation::CurrentFile(bytes) => Some(bytes.as_ref()),
                    Implementation::File { .. } | Implementation::Assembly { .. } => None,
                }
            })
    else {
        ctx.push(StackValue::null());
        return StepResult::Continue;
    };

    let byte_type = dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.Byte"));
    let mut byte_array =
        dotnet_vm_ops::vm_try!(ctx.new_vector(ConcreteType::from(byte_type), resource_bytes.len()));
    byte_array.get_mut().copy_from_slice(resource_bytes);

    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let byte_array_obj = ObjectRef::new(gc, HeapStorage::Vec(Box::new(byte_array)));
    ctx.register_new_object(&byte_array_obj);

    let memory_stream_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.IO.MemoryStream"));
    let instance = dotnet_vm_ops::vm_try!(ctx.new_object(memory_stream_type.clone()));

    let lookup = GenericLookup::default();
    let Some(constructor) = find_memory_stream_byte_array_ctor(ctx, &memory_stream_type) else {
        return StepResult::Error(VmError::Execution(ExecutionError::InternalError(
            "Could not find System.IO.MemoryStream::.ctor(byte[])".into(),
        )));
    };

    let method_info = dotnet_vm_ops::vm_try!(ctx.reflection_method_info(constructor, &lookup));

    ctx.push_obj(byte_array_obj);
    dotnet_vm_ops::vm_try!(ctx.reflection_constructor_frame(instance, method_info, lookup));
    StepResult::FramePushed
}
