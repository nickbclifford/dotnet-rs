use crate::{
    types::{generics::GenericLookup, members::MethodDescription},
    value::{
        object::{HeapStorage, Object, ObjectRef},
        StackValue,
    },
    vm::{context::ResolutionContext, CallStack, GCHandle, StepResult},
    vm_pop, vm_push,
};

pub fn intrinsic_vector_is_hardware_accelerated<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    vm_push!(stack, gc, Int32(0));
    StepResult::InstructionStepped
}

pub fn intrinsic_equality_comparer_get_default<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_type = generics.type_generics[0].clone();
    let comparer_type_name = "System.Collections.Generic.GenericEqualityComparer`1";
    let comparer_td = stack.loader().corlib_type(comparer_type_name);

    let new_lookup = GenericLookup::new(vec![target_type]);
    let ctx = ResolutionContext::for_method(method, stack.loader(), generics, stack.shared.clone())
        .with_generics(&new_lookup);
    let instance = ObjectRef::new(gc, HeapStorage::Obj(Object::new(comparer_td, &ctx)));

    vm_push!(stack, gc, ObjectRef(instance));
    StepResult::InstructionStepped
}

pub fn intrinsic_numeric_create_truncating<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = vm_pop!(stack);
    let target_type = method.parent.definition().type_name();

    macro_rules! convert {
        ($val:expr) => {
            match target_type.as_str() {
                "Byte" => vm_push!(stack, gc, Int32(($val as u8) as i32)),
                "SByte" => vm_push!(stack, gc, Int32(($val as i8) as i32)),
                "UInt16" => vm_push!(stack, gc, Int32(($val as u16) as i32)),
                "Int16" => vm_push!(stack, gc, Int32(($val as i16) as i32)),
                "UInt32" => vm_push!(stack, gc, Int32(($val as u32) as i32)),
                "Int32" => vm_push!(stack, gc, Int32($val as i32)),
                "UInt64" => vm_push!(stack, gc, Int64(($val as u64) as i64)),
                "Int64" => vm_push!(stack, gc, Int64($val as i64)),
                "UIntPtr" => vm_push!(stack, gc, NativeInt(($val as usize) as isize)),
                "IntPtr" => vm_push!(stack, gc, NativeInt($val as isize)),
                _ => panic!("unsupported CreateTruncating target type: {}", target_type),
            }
        };
    }

    match value {
        StackValue::Int32(v) => convert!(v),
        StackValue::Int64(v) => convert!(v),
        StackValue::NativeInt(v) => convert!(v),
        _ => panic!("unsupported CreateTruncating source type: {:?}", value),
    }

    StepResult::InstructionStepped
}

pub fn intrinsic_math_min_double<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b = vm_pop!(stack);
    let a = vm_pop!(stack);
    match (&a, &b) {
        (StackValue::NativeFloat(av), StackValue::NativeFloat(bv)) => {
            vm_push!(stack, gc, NativeFloat(av.min(*bv)));
        }
        _ => panic!("Math.Min(double, double) called with non-double arguments: {:?}, {:?}", a, b),
    }
    StepResult::InstructionStepped
}
