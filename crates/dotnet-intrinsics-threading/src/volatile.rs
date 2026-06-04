use crate::{
    ThreadingIntrinsicHost,
    atomic_dispatch::{
        VolatileAtomicTypeDispatch, resolve_atomic_ref_target_type, volatile_atomic_dispatch,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    error::{ExecutionError, VmError},
    generics::GenericLookup,
    members::MethodDescription,
};
use dotnet_utils::sync::Ordering;
use dotnet_value::{
    StackValue,
    object::ObjectRef,
    pointer::ManagedPtr,
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::ops::RawMemoryOps;
use dotnetdll::prelude::BaseType;

/// `System.Threading.Volatile::Read<T>(ref T location)`
#[dotnet_intrinsic("static T System.Threading.Volatile::Read<T>(T&)")]
#[dotnet_intrinsic("static bool System.Threading.Volatile::Read(bool&)")]
#[dotnet_intrinsic("static sbyte System.Threading.Volatile::Read(sbyte&)")]
#[dotnet_intrinsic("static byte System.Threading.Volatile::Read(byte&)")]
#[dotnet_intrinsic("static short System.Threading.Volatile::Read(short&)")]
#[dotnet_intrinsic("static ushort System.Threading.Volatile::Read(ushort&)")]
#[dotnet_intrinsic("static int System.Threading.Volatile::Read(int&)")]
#[dotnet_intrinsic("static uint System.Threading.Volatile::Read(uint&)")]
#[dotnet_intrinsic("static long System.Threading.Volatile::Read(long&)")]
#[dotnet_intrinsic("static ulong System.Threading.Volatile::Read(ulong&)")]
#[dotnet_intrinsic("static IntPtr System.Threading.Volatile::Read(IntPtr&)")]
#[dotnet_intrinsic("static UIntPtr System.Threading.Volatile::Read(UIntPtr&)")]
#[dotnet_intrinsic("static float System.Threading.Volatile::Read(float&)")]
#[dotnet_intrinsic("static double System.Threading.Volatile::Read(double&)")]
pub fn intrinsic_volatile_read<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    debug_assert_eq!(
        ObjectRef::SIZE,
        std::mem::size_of::<usize>(),
        "ObjectRef must be pointer-sized for atomic pointer loads"
    );
    let target_type = dotnet_vm_ops::vm_try!(resolve_atomic_ref_target_type(
        ctx,
        &method,
        generics,
        "intrinsic_volatile_read",
    ));

    match volatile_atomic_dispatch(target_type.get()) {
        VolatileAtomicTypeDispatch::Byte => {
            let target_ptr = ctx.pop_managed_ptr();
            // SAFETY: The intrinsic resolves `target_ptr` from a by-ref CLR argument and this
            // branch enforces a 1-byte atomic width that matches the target type dispatch.
            let val = unsafe {
                RawMemoryOps::load_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    1,
                    Ordering::Acquire,
                )
                .unwrap()
            };
            ctx.push_i32(val as i32);
        }
        VolatileAtomicTypeDispatch::Int16 => {
            let target_ptr = ctx.pop_managed_ptr();
            // SAFETY: The pointer came from managed by-ref argument materialization and this branch
            // performs a 2-byte atomic read consistent with the dispatch-selected type width.
            let val = unsafe {
                RawMemoryOps::load_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    2,
                    Ordering::Acquire,
                )
                .unwrap()
            };
            ctx.push_i32(val as i32);
        }
        VolatileAtomicTypeDispatch::Word32 => {
            let target_ptr = ctx.pop_managed_ptr();
            // SAFETY: `target_ptr` is a managed by-ref location and this dispatch arm guarantees a
            // 4-byte atomic read for 32-bit scalar/float payloads.
            let val = unsafe {
                RawMemoryOps::load_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    4,
                    Ordering::Acquire,
                )
                .unwrap()
            };
            if matches!(target_type.get(), BaseType::Float32) {
                ctx.push_f64(f32::from_bits(val as u32) as f64);
            } else {
                ctx.push_i32(val as i32);
            }
        }
        VolatileAtomicTypeDispatch::Word64 => {
            let target_ptr = ctx.pop_managed_ptr();
            // SAFETY: `target_ptr` is validated by the VM and this arm uses an 8-byte atomic read,
            // matching the resolved 64-bit payload kind.
            let val = unsafe {
                RawMemoryOps::load_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    8,
                    Ordering::Acquire,
                )
                .unwrap()
            };
            if matches!(target_type.get(), BaseType::Float64) {
                ctx.push_f64(f64::from_bits(val));
            } else {
                ctx.push_i64(val as i64);
            }
        }
        VolatileAtomicTypeDispatch::PointerSized => {
            let target_ptr = ctx.pop_managed_ptr();
            let size = ObjectRef::SIZE;
            // SAFETY: Pointer-sized volatile reads use the runtime's object-reference width and the
            // by-ref target pointer is sourced from managed stack state.
            let val = unsafe {
                RawMemoryOps::load_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    size,
                    Ordering::Acquire,
                )
                .unwrap()
            };
            ctx.push_isize(val as isize);
        }
        VolatileAtomicTypeDispatch::ObjectRef => {
            // Assume ObjectRef
            let target_ptr = ctx.pop_managed_ptr();
            // SAFETY: This path is selected only for object-reference targets and reads exactly
            // one pointer-sized slot atomically from managed memory.
            let val = unsafe {
                RawMemoryOps::load_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    ObjectRef::SIZE,
                    Ordering::Acquire,
                )
                .unwrap()
            };
            // SAFETY: `val` was read atomically from an object-reference slot and `gc`
            // brands the reconstructed handle to the current arena lifetime.
            let obj = unsafe { ObjectRef::read_branded(&val.to_ne_bytes(), &gc) };
            ctx.push_obj(obj);
        }
    }

    StepResult::Continue
}

fn store_int_volatile<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    target_ptr: &ManagedPtr<'gc>,
    value: &StackValue<'gc>,
    width: usize,
) -> StepResult {
    let val = match value {
        StackValue::Int32(i) => *i as u64,
        actual => {
            return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                expected: "Int32".to_string(),
                actual: format!("{actual:?}"),
            }));
        }
    };

    // SAFETY: Byte and Int16 volatile write paths both convert from Int32 and perform
    // an atomic store using the dispatch-selected integer width.
    unsafe {
        RawMemoryOps::store_atomic(
            ctx,
            target_ptr.origin().clone(),
            target_ptr.byte_offset(),
            val,
            width,
            Ordering::Release,
        )
        .unwrap();
    }

    StepResult::Continue
}

/// System.Threading.Volatile::Write(ref T location, T value)
#[dotnet_intrinsic("static void System.Threading.Volatile::Write<T>(T&, T)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(bool&, bool)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(sbyte&, sbyte)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(byte&, byte)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(short&, short)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(ushort&, ushort)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(int&, int)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(uint&, uint)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(long&, long)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(ulong&, ulong)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(IntPtr&, IntPtr)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(UIntPtr&, UIntPtr)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(float&, float)")]
#[dotnet_intrinsic("static void System.Threading.Volatile::Write(double&, double)")]
pub fn intrinsic_volatile_write<'gc, T: ThreadingIntrinsicHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    debug_assert_eq!(
        ObjectRef::SIZE,
        std::mem::size_of::<usize>(),
        "ObjectRef must be pointer-sized for atomic pointer stores"
    );
    let value = ctx.pop();
    let target_ptr = ctx.pop_managed_ptr();
    let target_type = dotnet_vm_ops::vm_try!(resolve_atomic_ref_target_type(
        ctx,
        &method,
        generics,
        "intrinsic_volatile_write",
    ));

    match volatile_atomic_dispatch(target_type.get()) {
        VolatileAtomicTypeDispatch::Byte => {
            match store_int_volatile(ctx, &target_ptr, &value, 1) {
                StepResult::Continue => {}
                other => return other,
            }
        }
        VolatileAtomicTypeDispatch::Int16 => {
            match store_int_volatile(ctx, &target_ptr, &value, 2) {
                StepResult::Continue => {}
                other => return other,
            }
        }
        VolatileAtomicTypeDispatch::Word32 => {
            let val = match value {
                StackValue::Int32(i) => i as u32 as u64,
                StackValue::NativeFloat(f) => (f as f32).to_bits() as u64,
                actual => {
                    return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                        expected: "Int32 or NativeFloat".to_string(),
                        actual: format!("{actual:?}"),
                    }));
                }
            };
            // SAFETY: This branch is only for 32-bit payloads and stores exactly 4 bytes into the
            // managed by-ref location selected by the intrinsic dispatch.
            unsafe {
                RawMemoryOps::store_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val,
                    4,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
        VolatileAtomicTypeDispatch::Word64 => {
            let val = match value {
                StackValue::Int64(i) => i as u64,
                StackValue::NativeFloat(f) => f.to_bits(),
                actual => {
                    return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                        expected: "Int64 or NativeFloat".to_string(),
                        actual: format!("{actual:?}"),
                    }));
                }
            };
            // SAFETY: This arm handles only 64-bit payloads and performs an 8-byte atomic write
            // against a managed by-ref pointer validated by the VM.
            unsafe {
                RawMemoryOps::store_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val,
                    8,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
        VolatileAtomicTypeDispatch::PointerSized => {
            let val = match value {
                StackValue::NativeInt(i) => i as u64,
                actual => {
                    return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                        expected: "NativeInt".to_string(),
                        actual: format!("{actual:?}"),
                    }));
                }
            };
            let size = ObjectRef::SIZE;
            // SAFETY: Pointer-sized values are written using the runtime pointer width and the
            // destination pointer originates from a managed by-ref argument.
            unsafe {
                RawMemoryOps::store_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val,
                    size,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
        VolatileAtomicTypeDispatch::ObjectRef => {
            // Assume ObjectRef
            let val_raw = match value {
                StackValue::ObjectRef(obj_ref) => {
                    let mut encoded = [0u8; ObjectRef::SIZE];
                    obj_ref.write(&mut encoded);
                    usize::from_ne_bytes(encoded) as u64
                }
                StackValue::NativeInt(i) => i as u64,
                actual => {
                    return StepResult::Error(VmError::Execution(ExecutionError::TypeMismatch {
                        expected: "ObjectRef or NativeInt".to_string(),
                        actual: format!("{actual:?}"),
                    }));
                }
            };
            let size = ObjectRef::SIZE;
            // SAFETY: This branch stores one object-reference-sized slot atomically; ObjectRef values
            // are serialized through `ObjectRef::write` to preserve any runtime tagging protocol.
            unsafe {
                RawMemoryOps::store_atomic(
                    ctx,
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val_raw,
                    size,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
    }

    StepResult::Continue
}
