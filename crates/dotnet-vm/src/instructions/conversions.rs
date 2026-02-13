use crate::{
    StepResult,
    stack::ops::{ExceptionOps, StackOps},
};
use dotnet_macros::dotnet_instruction;
use dotnet_value::{StackValue, pointer::UnmanagedPtr};
use dotnetdll::prelude::*;

#[dotnet_instruction(Convert(t))]
pub fn conv<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    t: ConversionType,
) -> StepResult {
    let value = ctx.pop();

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
            let i = simple_cast!(i64);
            ctx.push(StackValue::Int64(i));
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
                    #[allow(deprecated)]
                    {
                        m.pointer().map_or(0, |ptr| ptr.as_ptr() as usize) as u64
                    }
                }
                StackValue::NativeFloat(f) => f as u64,
                v => panic!("invalid type on stack ({:?}) for conversion to u64", v),
            };
            ctx.push(StackValue::Int64(i as i64));
        }
        ConversionType::IntPtr => {
            let i = simple_cast!(isize);
            ctx.push(StackValue::NativeInt(i));
        }
        ConversionType::UIntPtr => {
            let i = match value {
                StackValue::Int32(i) => (i as u32) as usize,
                StackValue::Int64(i) => i as u64 as usize,
                StackValue::NativeInt(i) => i as usize,
                StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr() as usize,
                StackValue::ManagedPtr(m) => {
                    #[allow(deprecated)]
                    {
                        m.pointer().map_or(0, |ptr| ptr.as_ptr() as usize)
                    }
                }
                StackValue::NativeFloat(f) => f as usize,
                v => panic!("invalid type on stack ({:?}) for conversion to usize", v),
            };
            ctx.push(StackValue::NativeInt(i as isize));
        }
    }
    StepResult::Continue
}

#[dotnet_instruction(ConvertOverflow(t, sgn))]
pub fn conv_ovf<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    t: ConversionType,
    sgn: NumberSign,
) -> StepResult {
    let value = ctx.pop();

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
                _ => panic!("invalid type on stack ({:?}) for checked conversion", value),
            };

            match result {
                Ok(v) => {
                    ctx.push(StackValue::$stack_variant(v as _));
                    StepResult::Continue
                }
                Err(_) => ctx.throw_by_name("System.OverflowException"),
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
pub fn conv_r4<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(ctx: &mut T) -> StepResult {
    let v = match ctx.pop() {
        StackValue::Int32(i) => i as f32,
        StackValue::Int64(i) => i as f32,
        StackValue::NativeInt(i) => i as f32,
        StackValue::NativeFloat(i) => i as f32,
        rest => panic!(
            "invalid type on stack ({:?}) for conversion to float32",
            rest
        ),
    };
    ctx.push(StackValue::NativeFloat(v as f64));
    StepResult::Continue
}

#[dotnet_instruction(ConvertFloat64)]
pub fn conv_r8<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(ctx: &mut T) -> StepResult {
    let v = match ctx.pop() {
        StackValue::Int32(i) => i as f64,
        StackValue::Int64(i) => i as f64,
        StackValue::NativeInt(i) => i as f64,
        StackValue::NativeFloat(i) => i,
        rest => panic!(
            "invalid type on stack ({:?}) for conversion to float64",
            rest
        ),
    };
    ctx.push(StackValue::NativeFloat(v));
    StepResult::Continue
}

#[dotnet_instruction(ConvertUnsignedToFloat)]
pub fn conv_r_un<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(ctx: &mut T) -> StepResult {
    let value = ctx.pop();
    let f = match value {
        StackValue::Int32(i) => (i as u32) as f64,
        StackValue::Int64(i) => (i as u64) as f64,
        StackValue::NativeInt(i) => (i as usize) as f64,
        v => panic!("invalid type on stack ({:?}) for conv.r.un", v),
    };
    ctx.push(StackValue::NativeFloat(f));
    StepResult::Continue
}
