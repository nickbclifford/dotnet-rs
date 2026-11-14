//! Intrinsics for managed resource loading (`System.Resources`).

use crate::{ReflectionIntrinsicHost, types::string_from_heap_obj};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    error::{ExecutionError, VmError},
    generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_value::StackValue;
use dotnet_vm_data::StepResult;
use dotnetdll::prelude::{MethodMemberIndex, resource::Implementation};

/// `UnmanagedMemoryStream(byte* pointer, long length)`: the only two-parameter constructor.
fn find_unmanaged_memory_stream_ctor(
    memory_stream_type: &TypeDescription,
) -> Option<MethodDescription> {
    let lookup = GenericLookup::default();
    memory_stream_type
        .definition()
        .methods
        .iter()
        .enumerate()
        .filter(|(_, method)| method.name == ".ctor")
        .map(|(idx, _)| {
            MethodDescription::new(
                memory_stream_type.clone(),
                lookup.clone(),
                memory_stream_type.resolution.clone(),
                MethodMemberIndex::Method(idx),
            )
        })
        .find(|candidate| {
            let signature = candidate.signature();
            signature.instance && signature.parameters.len() == 2
        })
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

    // Match the CLR: the stream is an UnmanagedMemoryStream over the metadata bytes, so that
    // ResourceReader reads its name-hash and name-position tables in place instead of one
    // BinaryReader.ReadInt32 (about seven managed calls) per entry. The metadata arena that owns
    // `resource_bytes` outlives every resolution, hence the VM, and the stream is read-only.
    let pointer = resource_bytes.as_ptr().expose_provenance() as isize;
    let length = resource_bytes.len() as i64;

    let stream_type =
        dotnet_vm_ops::vm_try!(ctx.loader().corlib_type("System.IO.UnmanagedMemoryStream"));
    let instance = dotnet_vm_ops::vm_try!(ctx.new_object(stream_type.clone()));

    let lookup = GenericLookup::default();
    let Some(constructor) = find_unmanaged_memory_stream_ctor(&stream_type) else {
        return StepResult::Error(VmError::Execution(ExecutionError::InternalError(
            "Could not find System.IO.UnmanagedMemoryStream::.ctor(byte*, long)".into(),
        )));
    };

    let method_info = dotnet_vm_ops::vm_try!(ctx.reflection_method_info(constructor, &lookup));

    ctx.push(StackValue::NativeInt(pointer));
    ctx.push(StackValue::Int64(length));
    dotnet_vm_ops::vm_try!(ctx.reflection_constructor_frame(instance, method_info, lookup));
    StepResult::FramePushed
}
