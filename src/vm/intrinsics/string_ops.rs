use crate::{
    pop_args,
    types::{generics::GenericLookup, members::MethodDescription},
    value::{
        object::{Object, ObjectRef},
        string::{with_string, CLRString},
        StackValue,
    },
    vm::{intrinsics::span::span_to_slice, CallStack, GCHandle, StepResult},
    vm_expect_stack, vm_pop, vm_push,
};
use std::hash::{DefaultHasher, Hash, Hasher};

/// System.String::Equals(string, string)
/// System.String::Equals(string)
pub fn intrinsic_string_equals<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let b_val = vm_pop!(stack);
    let a_val = vm_pop!(stack);
    let b = with_string!(stack, gc, b_val, |b| b.to_vec());
    let a = with_string!(stack, gc, a_val, |a| a.to_vec());

    vm_push!(stack, gc, Int32(if a == b { 1 } else { 0 }));
    StepResult::InstructionStepped
}

/// System.String::FastAllocateString(int)
pub fn intrinsic_string_fast_allocate_string<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let len = if method.method.signature.parameters.len() == 1 {
        vm_expect_stack!(let Int32(i) = vm_pop!(stack));
        i as usize
    } else {
        // Overload with MethodTable* as first param
        vm_expect_stack!(let NativeInt(i) = vm_pop!(stack));
        vm_pop!(stack); // pop method table pointer
        i as usize
    };

    // Check GC safe point before allocating large strings
    const LARGE_STRING_THRESHOLD: usize = 1024;
    if len > LARGE_STRING_THRESHOLD {
        stack.check_gc_safe_point();
    }

    let value = CLRString::new(vec![0u16; len]);
    vm_push!(stack, gc, string(gc, value));
    StepResult::InstructionStepped
}

/// System.String::get_Chars(int)
pub fn intrinsic_string_get_chars<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, [Int32(index)]);
    let val = vm_pop!(stack);
    let value = with_string!(stack, gc, val, |s| s[index as usize]);
    vm_push!(stack, gc, Int32(value as i32));
    StepResult::InstructionStepped
}

/// System.String::get_Length()
pub fn intrinsic_string_get_length<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = vm_pop!(stack);
    let len = with_string!(stack, gc, val, |s| s.len());
    vm_push!(stack, gc, Int32(len as i32));
    StepResult::InstructionStepped
}

/// System.String::Concat(ReadOnlySpan<char>, ReadOnlySpan<char>, ReadOnlySpan<char>)
pub fn intrinsic_string_concat_three_spans<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    vm_expect_stack!(let ValueType(span2) = vm_pop!(stack));
    vm_expect_stack!(let ValueType(span1) = vm_pop!(stack));
    vm_expect_stack!(let ValueType(span0) = vm_pop!(stack));

    fn char_span_into_str(span: Object) -> Vec<u16> {
        span_to_slice(span)
            .chunks_exact(2)
            .map(|c| u16::from_ne_bytes([c[0], c[1]]))
            .collect::<Vec<_>>()
    }

    let data0 = char_span_into_str(*span0);
    let data1 = char_span_into_str(*span1);
    let data2 = char_span_into_str(*span2);

    let total_length = data0.len() + data1.len() + data2.len();
    const LARGE_STRING_CONCAT_THRESHOLD: usize = 1024;
    if total_length > LARGE_STRING_CONCAT_THRESHOLD {
        stack.check_gc_safe_point();
    }

    let value = CLRString::new(data0.into_iter().chain(data1).chain(data2).collect());
    vm_push!(stack, gc, string(gc, value));
    StepResult::InstructionStepped
}

/// System.String::GetHashCodeOrdinalIgnoreCase()
pub fn intrinsic_string_get_hash_code_ordinal_ignore_case<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = vm_pop!(stack);
    let mut h = DefaultHasher::new();
    let value = with_string!(stack, gc, val, |s| String::from_utf16_lossy(s)
        .to_uppercase()
        .into_bytes());
    value.hash(&mut h);
    let code = h.finish();

    vm_push!(stack, gc, Int32(code as i32));
    StepResult::InstructionStepped
}

/// System.String::GetPinnableReference()
/// System.String::GetRawStringData()
pub fn intrinsic_string_get_raw_data<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = vm_pop!(stack);
    let obj_h = if let StackValue::ObjectRef(ObjectRef(Some(h))) = &val {
        Some(*h)
    } else {
        None
    };
    let ptr = with_string!(stack, gc, val, |s| s.as_ptr() as *mut u8);
    let value =
        StackValue::managed_ptr(ptr, stack.loader().corlib_type("System.Char"), obj_h, false);
    vm_push!(stack, gc, value);
    StepResult::InstructionStepped
}

/// System.String::IndexOf(char)
/// System.String::IndexOf(char, int)
pub fn intrinsic_string_index_of<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (c, start_at) = if method.method.signature.parameters.len() == 1 {
        vm_expect_stack!(let Int32(c) = vm_pop!(stack));
        (c as u16, 0usize)
    } else {
        vm_expect_stack!(let Int32(start_at) = vm_pop!(stack));
        vm_expect_stack!(let Int32(c) = vm_pop!(stack));
        (c as u16, start_at as usize)
    };

    let val = vm_pop!(stack);
    let index = with_string!(stack, gc, val, |s| s
        .iter()
        .skip(start_at)
        .position(|x| *x == c));

    vm_push!(
        stack,
        gc,
        Int32(match index {
            None => -1,
            Some(i) => (i + start_at) as i32,
        })
    );
    StepResult::InstructionStepped
}

/// System.String::Substring(int)
pub fn intrinsic_string_substring<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    vm_expect_stack!(let Int32(start_at) = vm_pop!(stack));
    let val = vm_pop!(stack);
    let value = with_string!(stack, gc, val, |s| s.split_at(start_at as usize).0.to_vec());
    vm_push!(stack, gc, string(gc, CLRString::new(value)));
    StepResult::InstructionStepped
}

/// System.String::IsNullOrEmpty(string)
pub fn intrinsic_string_is_null_or_empty<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let str_val = vm_pop!(stack);
    let is_null_or_empty = match &str_val {
        StackValue::ObjectRef(ObjectRef(None)) => true,
        _ => with_string!(stack, gc, str_val, |s| s.is_empty()),
    };
    vm_push!(stack, gc, Int32(is_null_or_empty as i32));
    StepResult::InstructionStepped
}

/// Dispatcher for string intrinsics that use string matching
/// (for methods not in metadata)
pub fn handle_string_intrinsic<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    use crate::match_method;

    if match_method!(method, System.String::get_Length()) {
        return intrinsic_string_get_length(gc, stack, method, generics);
    }

    if match_method!(method, System.String::get_Chars(int)) {
        return intrinsic_string_get_chars(gc, stack, method, generics);
    }

    panic!("unsupported string intrinsic: {:?}", method);
}
