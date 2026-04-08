use crate::{
    ThreadingIntrinsicHost,
    atomic_dispatch::{
        VolatileAtomicTypeDispatch, resolve_atomic_ref_target_type, volatile_atomic_dispatch,
    },
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::sync::Ordering;
use dotnet_value::{StackValue, object::ObjectRef};
use dotnet_vm_ops::StepResult;
use dotnetdll::prelude::BaseType;
use gc_arena::Gc;

/// System.Threading.Volatile::Read<T>(ref T location)
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
    let _gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    debug_assert_eq!(
        ObjectRef::SIZE,
        std::mem::size_of::<usize>(),
        "ObjectRef must be pointer-sized for atomic pointer loads"
    );
    let target_type = crate::vm_try!(resolve_atomic_ref_target_type(
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
                ctx.threading_load_atomic(
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
                ctx.threading_load_atomic(
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
                ctx.threading_load_atomic(
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
                ctx.threading_load_atomic(
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
                ctx.threading_load_atomic(
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
                ctx.threading_load_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    ObjectRef::SIZE,
                    Ordering::Acquire,
                )
                .unwrap()
            };
            let obj = if val == 0 {
                ObjectRef(None)
            } else {
                ObjectRef(Some(unsafe { Gc::from_ptr(val as usize as *const _) }))
            };
            ctx.push_obj(obj);
        }
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
    let target_type = crate::vm_try!(resolve_atomic_ref_target_type(
        ctx,
        &method,
        generics,
        "intrinsic_volatile_write",
    ));

    match volatile_atomic_dispatch(target_type.get()) {
        VolatileAtomicTypeDispatch::Byte => {
            let val = match value {
                StackValue::Int32(i) => i as u64,
                _ => panic!("Expected Int32 for byte-sized Volatile.Write"),
            };
            // SAFETY: The write target came from a managed by-ref argument and this arm writes a
            // single byte, matching the resolved element width.
            unsafe {
                ctx.threading_store_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val,
                    1,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
        VolatileAtomicTypeDispatch::Int16 => {
            let val = match value {
                StackValue::Int32(i) => i as u64,
                _ => panic!("Expected Int32 for 16-bit Volatile.Write"),
            };
            // SAFETY: The dispatch guarantees 16-bit storage and the source value is normalized to
            // the corresponding 2-byte representation before the atomic store.
            unsafe {
                ctx.threading_store_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val,
                    2,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
        VolatileAtomicTypeDispatch::Word32 => {
            let val = match value {
                StackValue::Int32(i) => i as u32 as u64,
                StackValue::NativeFloat(f) => (f as f32).to_bits() as u64,
                _ => panic!("Expected Int32 or Float for 32-bit Volatile.Write"),
            };
            // SAFETY: This branch is only for 32-bit payloads and stores exactly 4 bytes into the
            // managed by-ref location selected by the intrinsic dispatch.
            unsafe {
                ctx.threading_store_atomic(
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
                _ => panic!("Expected Int64 or Float for 64-bit Volatile.Write"),
            };
            // SAFETY: This arm handles only 64-bit payloads and performs an 8-byte atomic write
            // against a managed by-ref pointer validated by the VM.
            unsafe {
                ctx.threading_store_atomic(
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
                _ => panic!("Expected NativeInt for Volatile.Write"),
            };
            let size = ObjectRef::SIZE;
            // SAFETY: Pointer-sized values are written using the runtime pointer width and the
            // destination pointer originates from a managed by-ref argument.
            unsafe {
                ctx.threading_store_atomic(
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
                StackValue::ObjectRef(ObjectRef(Some(ptr))) => Gc::as_ptr(ptr) as usize as u64,
                StackValue::ObjectRef(ObjectRef(None)) => 0,
                StackValue::NativeInt(i) => i as u64,
                _ => panic!("Expected ObjectRef or NativeInt for Volatile.Write"),
            };
            let size = ObjectRef::SIZE;
            // SAFETY: This branch stores one object-reference-sized slot atomically; source values
            // are converted to raw pointer-sized integers before the write.
            unsafe {
                ctx.threading_store_atomic(
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
