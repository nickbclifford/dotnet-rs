use crate::{
    StepResult,
    stack::ops::{
        ExceptionOps, LoaderOps, MemoryOps, RawMemoryOps, ReflectionOps, ResolutionOps, StackOps,
        ThreadOps,
    },
    sync::Ordering,
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_value::{StackValue, object::ObjectRef};
use dotnetdll::prelude::{BaseType, Parameter, ParameterType};
use gc_arena::Gc;

#[allow(dead_code)]
const NULL_REF_MSG: &str = "Object reference not set to an instance of an object.";

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
pub fn intrinsic_volatile_read<
    'gc,
    T: StackOps<'gc>
        + ThreadOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new());
    let params = &method.method().signature.parameters;
    let Parameter(_, first_param_type) = &params[0];

    let target_type = if let ParameterType::Ref(inner) = first_param_type {
        vm_try!(generics.make_concrete(method.resolution(), inner.clone()))
    } else {
        panic!(
            "intrinsic_volatile_read: First parameter must be Ref, found {:?}",
            first_param_type
        );
    };

    match target_type.get() {
        BaseType::Boolean | BaseType::Int8 | BaseType::UInt8 => {
            let target_ptr = ctx.pop_managed_ptr();
            let val = unsafe {
                ctx.load_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    1,
                    Ordering::Acquire,
                )
                .unwrap()
            };
            ctx.push_i32(val as i32);
        }
        BaseType::Int16 | BaseType::UInt16 => {
            let target_ptr = ctx.pop_managed_ptr();
            let val = unsafe {
                ctx.load_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    2,
                    Ordering::Acquire,
                )
                .unwrap()
            };
            ctx.push_i32(val as i32);
        }
        BaseType::Int32 | BaseType::UInt32 | BaseType::Float32 => {
            let target_ptr = ctx.pop_managed_ptr();
            let val = unsafe {
                ctx.load_atomic(
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
        BaseType::Int64 | BaseType::UInt64 | BaseType::Float64 => {
            let target_ptr = ctx.pop_managed_ptr();
            let val = unsafe {
                ctx.load_atomic(
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
        BaseType::IntPtr | BaseType::UIntPtr => {
            let target_ptr = ctx.pop_managed_ptr();
            let size = ObjectRef::SIZE;
            let val = unsafe {
                ctx.load_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    size,
                    Ordering::Acquire,
                )
                .unwrap()
            };
            ctx.push_isize(val as isize);
        }
        _ => {
            // Assume ObjectRef
            let target_ptr = ctx.pop_managed_ptr();
            let val = unsafe {
                ctx.load_atomic(
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
pub fn intrinsic_volatile_write<
    'gc,
    T: StackOps<'gc>
        + ThreadOps
        + MemoryOps<'gc>
        + RawMemoryOps<'gc>
        + ExceptionOps<'gc>
        + LoaderOps
        + ResolutionOps<'gc>
        + ReflectionOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let _gc = ctx.gc_with_token(&dotnet_utils::NoActiveBorrows::new());
    let value = ctx.pop();
    let target_ptr = ctx.pop_managed_ptr();

    let params = &method.method().signature.parameters;
    let Parameter(_, target_ref_type) = &params[0];

    let target_type = if let ParameterType::Ref(inner) = target_ref_type {
        vm_try!(generics.make_concrete(method.resolution(), inner.clone()))
    } else {
        panic!(
            "intrinsic_volatile_write: First parameter must be Ref, found {:?}",
            target_ref_type
        );
    };

    match target_type.get() {
        BaseType::Boolean | BaseType::Int8 | BaseType::UInt8 => {
            let val = match value {
                StackValue::Int32(i) => i as u64,
                _ => panic!("Expected Int32 for byte-sized Volatile.Write"),
            };
            unsafe {
                ctx.store_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val,
                    1,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
        BaseType::Int16 | BaseType::UInt16 => {
            let val = match value {
                StackValue::Int32(i) => i as u64,
                _ => panic!("Expected Int32 for 16-bit Volatile.Write"),
            };
            unsafe {
                ctx.store_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val,
                    2,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
        BaseType::Int32 | BaseType::UInt32 | BaseType::Float32 => {
            let val = match value {
                StackValue::Int32(i) => i as u32 as u64,
                StackValue::NativeFloat(f) => (f as f32).to_bits() as u64,
                _ => panic!("Expected Int32 or Float for 32-bit Volatile.Write"),
            };
            unsafe {
                ctx.store_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val,
                    4,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
        BaseType::Int64 | BaseType::UInt64 | BaseType::Float64 => {
            let val = match value {
                StackValue::Int64(i) => i as u64,
                StackValue::NativeFloat(f) => f.to_bits(),
                _ => panic!("Expected Int64 or Float for 64-bit Volatile.Write"),
            };
            unsafe {
                ctx.store_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val,
                    8,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
        BaseType::IntPtr | BaseType::UIntPtr => {
            let val = match value {
                StackValue::NativeInt(i) => i as u64,
                _ => panic!("Expected NativeInt for Volatile.Write"),
            };
            let size = ObjectRef::SIZE;
            unsafe {
                ctx.store_atomic(
                    target_ptr.origin().clone(),
                    target_ptr.byte_offset(),
                    val,
                    size,
                    Ordering::Release,
                )
                .unwrap();
            }
        }
        _ => {
            // Assume ObjectRef
            let val_raw = match value {
                StackValue::ObjectRef(ObjectRef(Some(ptr))) => Gc::as_ptr(ptr) as usize as u64,
                StackValue::ObjectRef(ObjectRef(None)) => 0,
                StackValue::NativeInt(i) => i as u64,
                _ => panic!("Expected ObjectRef or NativeInt for Volatile.Write"),
            };
            let size = ObjectRef::SIZE;
            unsafe {
                ctx.store_atomic(
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
