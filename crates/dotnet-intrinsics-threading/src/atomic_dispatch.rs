use crate::ThreadingIntrinsicHost;
use dotnet_types::{
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnetdll::prelude::{BaseType, Parameter, ParameterType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InterlockedAtomicTypeDispatch {
    Int32,
    Int64,
    PointerSized,
    ObjectRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VolatileAtomicTypeDispatch {
    Byte,
    Int16,
    Word32,
    Word64,
    PointerSized,
    ObjectRef,
}

pub(crate) fn resolve_atomic_ref_target_type<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: &MethodDescription,
    generics: &GenericLookup,
    intrinsic_name: &str,
) -> Result<ConcreteType, TypeResolutionError> {
    let params = &method.method().signature.parameters;
    let Parameter(_, first_param_type) = &params[0];

    if let ParameterType::Ref(inner) = first_param_type {
        generics.make_concrete(method.resolution(), inner.clone(), ctx.loader().as_ref())
    } else {
        panic!(
            "{intrinsic_name}: First parameter must be Ref, found {:?}",
            first_param_type
        );
    }
}

pub(crate) fn interlocked_atomic_dispatch<EnclosingType>(
    base_type: &BaseType<EnclosingType>,
) -> InterlockedAtomicTypeDispatch {
    match base_type {
        BaseType::Int32 => InterlockedAtomicTypeDispatch::Int32,
        BaseType::Int64 => InterlockedAtomicTypeDispatch::Int64,
        BaseType::IntPtr | BaseType::UIntPtr => InterlockedAtomicTypeDispatch::PointerSized,
        _ => InterlockedAtomicTypeDispatch::ObjectRef,
    }
}

pub(crate) fn volatile_atomic_dispatch<EnclosingType>(
    base_type: &BaseType<EnclosingType>,
) -> VolatileAtomicTypeDispatch {
    match base_type {
        BaseType::Boolean | BaseType::Int8 | BaseType::UInt8 => VolatileAtomicTypeDispatch::Byte,
        BaseType::Int16 | BaseType::UInt16 => VolatileAtomicTypeDispatch::Int16,
        BaseType::Int32 | BaseType::UInt32 | BaseType::Float32 => {
            VolatileAtomicTypeDispatch::Word32
        }
        BaseType::Int64 | BaseType::UInt64 | BaseType::Float64 => {
            VolatileAtomicTypeDispatch::Word64
        }
        BaseType::IntPtr | BaseType::UIntPtr => VolatileAtomicTypeDispatch::PointerSized,
        _ => VolatileAtomicTypeDispatch::ObjectRef,
    }
}
