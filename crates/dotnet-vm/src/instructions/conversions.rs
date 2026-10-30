use crate::{
    StepResult,
    stack::ops::{EvalStackOps, ExceptionOps},
};

const INVALID_PROGRAM_MSG: &str = "Common Language Runtime detected an invalid program.";
const OVERFLOW_MSG: &str = "Arithmetic operation resulted in an overflow.";
use dotnet_macros::dotnet_instruction;
use dotnet_value::{StackValue, pointer::UnmanagedPtr};
use dotnetdll::prelude::*;

#[dotnet_instruction(Convert(t))]
pub fn conv<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    t: ConversionType,
) -> StepResult {
    let value = vm_pop!(ctx).coerce_enum_to_underlying();

    macro_rules! managed_address {
        ($m:expr) => {
            match $m.try_observe_address() {
                Ok(address) => address.map_or(0, |address| address.as_ptr().expose_provenance()),
                Err(_) => {
                    return StepResult::not_implemented(
                        "Cannot convert a Transient managed pointer to an integer",
                    );
                }
            }
        };
    }

    macro_rules! simple_cast {
        ($t:ty) => {
            match value {
                StackValue::Int32(i) => i as $t,
                StackValue::Int64(i) => i as $t,
                StackValue::NativeInt(i) => i as $t,
                StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr().expose_provenance() as $t,
                StackValue::NativeFloat(f) => f as $t,
                _ => {
                    return ctx.throw_by_name_with_message(
                        "System.InvalidProgramException",
                        INVALID_PROGRAM_MSG,
                    );
                }
            }
        };
    }

    match t {
        ConversionType::Int8 => {
            let i = simple_cast!(i8);
            ctx.push(StackValue::Int32(i as i32));
        }
        ConversionType::UInt8 => {
            let i = simple_cast!(u8);
            ctx.push(StackValue::Int32(i as i32));
        }
        ConversionType::Int16 => {
            let i = simple_cast!(i16);
            ctx.push(StackValue::Int32(i as i32));
        }
        ConversionType::UInt16 => {
            let i = simple_cast!(u16);
            ctx.push(StackValue::Int32(i as i32));
        }
        ConversionType::Int32 => {
            let i = simple_cast!(i32);
            ctx.push(StackValue::Int32(i));
        }
        ConversionType::UInt32 => {
            let i = simple_cast!(u32);
            ctx.push(StackValue::Int32(i as i32));
        }
        ConversionType::Int64 => {
            let i = match value {
                // Observation and exposure do not retain an owner or grant access.
                StackValue::ManagedPtr(m) => managed_address!(m) as i64,
                _ => simple_cast!(i64),
            };
            ctx.push(StackValue::Int64(i));
        }
        ConversionType::UInt64 => {
            let i = match value {
                // all Rust casts from signed types will sign extend
                // so first we have to make them unsigned so they'll properly zero extend
                StackValue::Int32(i) => (i as u32) as u64,
                StackValue::Int64(i) => i as u64,
                StackValue::NativeInt(i) => i as usize as u64,
                StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr().expose_provenance() as u64,
                // Heap address observation uses wrapping address formation, not a payload
                // slice or access check. Exposing provenance supports an integer-address
                // round trip only; the integer retains no owner/root or borrow/lock and grants
                // no access right.
                StackValue::ManagedPtr(m) => managed_address!(m) as u64,
                StackValue::NativeFloat(f) => f as u64,
                _ => {
                    return ctx.throw_by_name_with_message(
                        "System.InvalidProgramException",
                        INVALID_PROGRAM_MSG,
                    );
                }
            };
            ctx.push(StackValue::Int64(i as i64));
        }
        ConversionType::IntPtr => {
            let i = match value {
                // Observation and exposure do not retain an owner or grant access.
                StackValue::ManagedPtr(m) => managed_address!(m) as isize,
                _ => simple_cast!(isize),
            };
            ctx.push(StackValue::NativeInt(i));
        }
        ConversionType::UIntPtr => {
            let i = match value {
                StackValue::Int32(i) => (i as u32) as usize,
                StackValue::Int64(i) => i as u64 as usize,
                StackValue::NativeInt(i) => i as usize,
                StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr().expose_provenance(),
                // Heap address observation uses wrapping address formation, not a payload
                // slice or access check. Exposing provenance supports an integer-address
                // round trip only; the integer retains no owner/root or borrow/lock and grants
                // no access right.
                StackValue::ManagedPtr(m) => managed_address!(m),
                StackValue::NativeFloat(f) => f as usize,
                _ => {
                    return ctx.throw_by_name_with_message(
                        "System.InvalidProgramException",
                        INVALID_PROGRAM_MSG,
                    );
                }
            };
            ctx.push(StackValue::NativeInt(i as isize));
        }
    }
    StepResult::Continue
}

#[dotnet_instruction(ConvertOverflow(t, sgn))]
pub fn conv_ovf<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc>>(
    ctx: &mut T,
    t: ConversionType,
    sgn: NumberSign,
) -> StepResult {
    let value = vm_pop!(ctx);

    macro_rules! do_conv {
        ($target:ty, $stack_variant:ident) => {{
            let result = match (value.clone(), sgn) {
                (StackValue::Int32(i), NumberSign::Signed) => {
                    <$target>::try_from(i).map_err(|_| ())
                }
                (StackValue::Int32(i), NumberSign::Unsigned) => {
                    <$target>::try_from(i as u32).map_err(|_| ())
                }
                (StackValue::Int64(i), NumberSign::Signed) => {
                    <$target>::try_from(i).map_err(|_| ())
                }
                (StackValue::Int64(i), NumberSign::Unsigned) => {
                    <$target>::try_from(i as u64).map_err(|_| ())
                }
                (StackValue::NativeInt(i), NumberSign::Signed) => {
                    <$target>::try_from(i).map_err(|_| ())
                }
                (StackValue::NativeInt(i), NumberSign::Unsigned) => {
                    <$target>::try_from(i as usize).map_err(|_| ())
                }
                (StackValue::NativeFloat(f), _) => {
                    // ECMA-335: truncation towards zero
                    if f.is_nan() || f < (<$target>::MIN as f64) || f > (<$target>::MAX as f64) {
                        Err(())
                    } else {
                        Ok(f as $target)
                    }
                }
                _ => {
                    return ctx.throw_by_name_with_message(
                        "System.InvalidProgramException",
                        INVALID_PROGRAM_MSG,
                    );
                }
            };

            match result {
                Ok(v) => {
                    ctx.push(StackValue::$stack_variant(v as _));
                    StepResult::Continue
                }
                Err(_) => ctx.throw_by_name_with_message("System.OverflowException", OVERFLOW_MSG),
            }
        }};
    }

    match t {
        ConversionType::Int8 => do_conv!(i8, Int32),
        ConversionType::UInt8 => do_conv!(u8, Int32),
        ConversionType::Int16 => do_conv!(i16, Int32),
        ConversionType::UInt16 => do_conv!(u16, Int32),
        ConversionType::Int32 => do_conv!(i32, Int32),
        ConversionType::UInt32 => do_conv!(u32, Int32),
        ConversionType::Int64 => do_conv!(i64, Int64),
        ConversionType::UInt64 => do_conv!(u64, Int64),
        ConversionType::IntPtr => do_conv!(isize, NativeInt),
        ConversionType::UIntPtr => do_conv!(usize, NativeInt),
    }
}

#[dotnet_instruction(ConvertFloat32)]
pub fn conv_r4<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc>>(ctx: &mut T) -> StepResult {
    let v = match vm_pop!(ctx) {
        StackValue::Int32(i) => i as f32,
        StackValue::Int64(i) => i as f32,
        StackValue::NativeInt(i) => i as f32,
        StackValue::NativeFloat(i) => i as f32,
        _ => {
            return ctx
                .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
        }
    };
    ctx.push(StackValue::NativeFloat(v as f64));
    StepResult::Continue
}

#[dotnet_instruction(ConvertFloat64)]
pub fn conv_r8<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc>>(ctx: &mut T) -> StepResult {
    let v = match vm_pop!(ctx) {
        StackValue::Int32(i) => i as f64,
        StackValue::Int64(i) => i as f64,
        StackValue::NativeInt(i) => i as f64,
        StackValue::NativeFloat(i) => i,
        _ => {
            return ctx
                .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
        }
    };
    ctx.push(StackValue::NativeFloat(v));
    StepResult::Continue
}

#[dotnet_instruction(ConvertUnsignedToFloat)]
pub fn conv_r_un<'gc, T: EvalStackOps<'gc> + ExceptionOps<'gc>>(ctx: &mut T) -> StepResult {
    let value = vm_pop!(ctx);
    let f = match value {
        StackValue::Int32(i) => (i as u32) as f64,
        StackValue::Int64(i) => (i as u64) as f64,
        StackValue::NativeInt(i) => (i as usize) as f64,
        _ => {
            return ctx
                .throw_by_name_with_message("System.InvalidProgramException", INVALID_PROGRAM_MSG);
        }
    };
    ctx.push(StackValue::NativeFloat(f));
    StepResult::Continue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        state::SharedGlobalState,
        sync::Arc,
        test_utils::{new_test_arena, test_gc_handle},
    };
    use dotnet_assemblies::AssemblyLoader;
    use dotnet_types::{TypeDescription, generics::ConcreteType, resolution::ResolutionS};
    use dotnet_utils::ByteOffset;
    use dotnet_value::{
        layout::{ArrayLayoutManager, LayoutManager, Scalar},
        object::{HeapStorage, ObjectRef, Vector},
        pointer::ManagedPtr,
    };

    #[test]
    #[allow(clippy::arc_with_non_send_sync)]
    fn transient_pointer_integer_conversions_and_as_pointer_are_rejected() {
        use dotnet_intrinsics_unsafe::unsafe_ptr::intrinsic_unsafe_as_pointer;
        use dotnet_types::{generics::GenericLookup, members::MethodDescription};
        use dotnet_value::{
            layout::{FieldLayoutManager, GcDesc},
            object::Object,
            storage::FieldStorage,
        };
        use dotnetdll::prelude::MethodMemberIndex;
        use std::ptr::NonNull;

        let loader = Arc::new(AssemblyLoader::new_bare("conv_transient_test".into()).unwrap());
        let shared = Arc::new(SharedGlobalState::new(loader));
        let mut arena = new_test_arena(&shared);
        arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — the arena root stays live throughout this mutation.
            let gc_handle = unsafe { test_gc_handle(gc, engine) };
            let mut ctx = engine.ves_context(gc_handle);
            let transient = || {
                let layout = Arc::new(FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 1,
                    alignment: 1,
                    gc_desc: GcDesc::default(),
                    has_ref_fields: false,
                });
                let object = Object::new(
                    TypeDescription::NULL,
                    GenericLookup::default(),
                    FieldStorage::new(layout, vec![0xA5]),
                );
                let address = object.with_data(|data| NonNull::new(data.as_ptr().cast_mut()));
                ManagedPtr::new_transient(
                    address,
                    TypeDescription::NULL,
                    object,
                    ByteOffset::new(0),
                )
            };
            for conversion in [
                ConversionType::IntPtr,
                ConversionType::UIntPtr,
                ConversionType::Int64,
                ConversionType::UInt64,
            ] {
                ctx.push(StackValue::ManagedPtr(transient().into()));
                assert_eq!(
                    conv(&mut ctx, conversion),
                    StepResult::not_implemented(
                        "Cannot convert a Transient managed pointer to an integer"
                    )
                );
            }
            ctx.push(StackValue::ManagedPtr(transient().into()));
            let method = MethodDescription::new(
                TypeDescription::NULL,
                GenericLookup::default(),
                ResolutionS::NULL,
                MethodMemberIndex::Method(0),
            );
            assert_eq!(
                intrinsic_unsafe_as_pointer(&mut ctx, method, &GenericLookup::default()),
                StepResult::not_implemented(
                    "Unsafe.AsPointer cannot convert a Transient managed pointer to an integer"
                )
            );
        });
    }

    #[test]
    #[allow(clippy::arc_with_non_send_sync)]
    fn pointer_conversions_expose_live_addresses_and_truncate_unmanaged() {
        let loader = Arc::new(AssemblyLoader::new_bare("conv_pointer_test".into()).unwrap());
        let shared = Arc::new(SharedGlobalState::new(loader));
        let mut arena = new_test_arena(&shared);
        arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — the arena root stays live throughout this mutation.
            let gc_handle = unsafe { test_gc_handle(gc, engine) };
            let vector = Vector::new(
                ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
                ArrayLayoutManager {
                    element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                    length: 2,
                },
                vec![0, 0xA5],
                vec![2, 1],
            );
            let owner = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
            let managed = ManagedPtr::new(
                owner.pointer(),
                TypeDescription::NULL,
                Some(owner),
                false,
                Some(ByteOffset::new(1)),
            );
            let ptr = managed.observe_address().unwrap();
            let addr = ptr.as_ptr().addr();
            let mut ctx = engine.ves_context(gc_handle);

            for input in [
                StackValue::ManagedPtr(managed.into()),
                StackValue::UnmanagedPtr(UnmanagedPtr(ptr)),
            ] {
                for conversion in [ConversionType::IntPtr, ConversionType::Int64] {
                    ctx.push(input.clone());
                    assert!(matches!(conv(&mut ctx, conversion), StepResult::Continue));
                    let result = match ctx.pop() {
                        StackValue::NativeInt(i) => i as usize,
                        StackValue::Int64(i) => i as usize,
                        other => panic!("unexpected converted value: {other:?}"),
                    };
                    // SAFETY: F10.RawMemoryAccessValid — the converted address refers to the
                    // second byte of the live, rooted vector, and conv exposed its provenance.
                    let byte = unsafe { *std::ptr::with_exposed_provenance::<u8>(result) };
                    assert_eq!(byte, 0xA5);
                }
            }

            for (conversion, expected) in [
                (ConversionType::Int8, addr as i8 as i32),
                (ConversionType::UInt8, addr as u8 as i32),
                (ConversionType::Int16, addr as i16 as i32),
                (ConversionType::UInt16, addr as u16 as i32),
                (ConversionType::Int32, addr as i32),
                (ConversionType::UInt32, addr as u32 as i32),
            ] {
                ctx.push(StackValue::UnmanagedPtr(UnmanagedPtr(ptr)));
                assert!(matches!(conv(&mut ctx, conversion), StepResult::Continue));
                assert!(matches!(ctx.pop(), StackValue::Int32(i) if i == expected));
            }
            // SAFETY: F10.RawMemoryAccessValid — ptr addresses the live vector's second byte.
            let byte = unsafe { *ptr.as_ptr() };
            assert_eq!(byte, 0xA5);
        });
    }
}
