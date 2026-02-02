use crate::{CallStack, StepResult};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{pointer::UnmanagedPtr, StackValue};
use dotnetdll::prelude::*;
use dotnet_macros::dotnet_instruction;

#[dotnet_instruction(Convert)]
pub fn conv<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    t: ConversionType,
) -> StepResult {
    let value = stack.pop(gc);

    macro_rules! simple_cast {
        ($t:ty) => {
            match value {
                StackValue::Int32(i) => i as $t,
                StackValue::Int64(i) => i as $t,
                StackValue::NativeInt(i) => i as $t,
                StackValue::NativeFloat(f) => f as $t,
                v => panic!(
                    "invalid type on stack ({:?}) for conversion to {}",
                    v,
                    stringify!($t)
                ),
            }
        };
    }

    match t {
        ConversionType::Int8 => {
            let i = simple_cast!(i8);
            stack.push(gc, StackValue::Int32(i as i32));
        }
        ConversionType::UInt8 => {
            let i = simple_cast!(u8);
            stack.push(gc, StackValue::Int32(i as i32));
        }
        ConversionType::Int16 => {
            let i = simple_cast!(i16);
            stack.push(gc, StackValue::Int32(i as i32));
        }
        ConversionType::UInt16 => {
            let i = simple_cast!(u16);
            stack.push(gc, StackValue::Int32(i as i32));
        }
        ConversionType::Int32 => {
            let i = simple_cast!(i32);
            stack.push(gc, StackValue::Int32(i));
        }
        ConversionType::UInt32 => {
            let i = simple_cast!(u32);
            stack.push(gc, StackValue::Int32(i as i32));
        }
        ConversionType::Int64 => {
            let i = simple_cast!(i64);
            stack.push(gc, StackValue::Int64(i));
        }
        ConversionType::UInt64 => {
            let i = match value {
                // all Rust casts from signed types will sign extend
                // so first we have to make them unsigned so they'll properly zero extend
                StackValue::Int32(i) => (i as u32) as u64,
                StackValue::Int64(i) => i as u64,
                StackValue::NativeInt(i) => i as usize as u64,
                StackValue::UnmanagedPtr(UnmanagedPtr(p)) => (p.as_ptr() as usize) as u64,
                StackValue::ManagedPtr(m) => {
                    (m.pointer().map_or(0, |ptr| ptr.as_ptr() as usize)) as u64
                }
                StackValue::NativeFloat(f) => {
                    todo!("truncate {} towards zero for conversion to u64", f)
                }
                v => panic!("invalid type on stack ({:?}) for conversion to u64", v),
            };
            stack.push(gc, StackValue::Int64(i as i64));
        }
        ConversionType::IntPtr => {
            let i = simple_cast!(isize);
            stack.push(gc, StackValue::NativeInt(i));
        }
        ConversionType::UIntPtr => {
             let i = match value {
                StackValue::Int32(i) => (i as u32) as usize,
                StackValue::Int64(i) => i as u64 as usize,
                StackValue::NativeInt(i) => i as usize,
                StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr() as usize ,
                StackValue::ManagedPtr(m) => {
                    m.pointer().map_or(0, |ptr| ptr.as_ptr() as usize)
                }
                StackValue::NativeFloat(f) => {
                    todo!("truncate {} towards zero for conversion to usize", f)
                }
                v => panic!("invalid type on stack ({:?}) for conversion to usize", v),
            };
            stack.push(gc, StackValue::NativeInt(i as isize));
        }
    }
    StepResult::InstructionStepped
}

#[dotnet_instruction(ConvertOverflow)]
pub fn conv_ovf<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    t: ConversionType,
    sgn: NumberSign,
) -> StepResult {
    let value = stack.pop(gc);
    todo!(
        "{:?} conversion to {:?} with overflow detection ({:?})",
        t,
        sgn,
        value
    )
}

#[dotnet_instruction(ConvertFloat32)]
pub fn conv_r4<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v = match stack.pop(gc) {
        StackValue::Int32(i) => i as f32,
        StackValue::Int64(i) => i as f32,
        StackValue::NativeInt(i) => i as f32,
        StackValue::NativeFloat(i) => i as f32,
        rest => panic!(
            "invalid type on stack ({:?}) for conversion to float32",
            rest
        ),
    };
    stack.push(gc, StackValue::NativeFloat(v as f64));
    StepResult::InstructionStepped
}

#[dotnet_instruction(ConvertFloat64)]
pub fn conv_r8<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let v = match stack.pop(gc) {
        StackValue::Int32(i) => i as f64,
        StackValue::Int64(i) => i as f64,
        StackValue::NativeInt(i) => i as f64,
        StackValue::NativeFloat(i) => i,
        rest => panic!(
            "invalid type on stack ({:?}) for conversion to float64",
            rest
        ),
    };
    stack.push(gc, StackValue::NativeFloat(v));
    StepResult::InstructionStepped
}

#[dotnet_instruction(ConvertUnsignedToFloat)]
pub fn conv_r_un<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
) -> StepResult {
    let value = stack.pop(gc);
    todo!("conv.r.un({:?})", value)
}
