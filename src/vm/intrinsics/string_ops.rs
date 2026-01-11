use crate::{
    pop_args,
    types::{
        generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
    },
    value::{
        object::{HeapStorage, Object, ObjectRef},
        pointer::ManagedPtrOwner,
        string::{with_string, with_string_mut, CLRString},
        StackValue,
    },
    vm::{intrinsics::span::span_to_slice, CallStack, GCHandle, StepResult},
    vm_pop, vm_push,
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

    let res = match (a_val, b_val) {
        (StackValue::ObjectRef(ObjectRef(None)), StackValue::ObjectRef(ObjectRef(None))) => true,
        (StackValue::ObjectRef(ObjectRef(None)), _)
        | (_, StackValue::ObjectRef(ObjectRef(None))) => false,
        (
            StackValue::ObjectRef(ObjectRef(Some(a_handle))),
            StackValue::ObjectRef(ObjectRef(Some(b_handle))),
        ) => {
            if unsafe { std::ptr::eq(a_handle.as_ptr(), b_handle.as_ptr()) } {
                true
            } else {
                let a_heap = a_handle.borrow();
                let b_heap = b_handle.borrow();
                match (&a_heap.storage, &b_heap.storage) {
                    (HeapStorage::Str(a), HeapStorage::Str(b)) => a == b,
                    _ => false,
                }
            }
        }
        _ => false,
    };

    vm_push!(stack, gc, Int32(if res { 1 } else { 0 }));
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
        pop_args!(stack, [Int32(i)]);
        if i < 0 {
            return stack.throw_by_name(gc, "System.OverflowException");
        }
        i as usize
    } else {
        // Overload with MethodTable* as first param
        pop_args!(stack, [NativeInt(i)]);
        vm_pop!(stack); // pop method table pointer
        if i < 0 {
            return stack.throw_by_name(gc, "System.OverflowException");
        }
        i as usize
    };

    // Defensive check: limit string size to 512MB characters
    if len > 0x2000_0000 {
        return stack.throw_by_name(gc, "System.OutOfMemoryException");
    }

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
    pop_args!(
        stack,
        [ValueType(span2), ValueType(span1), ValueType(span0),]
    );

    fn char_span_into_str(span: Object) -> Vec<u16> {
        span_to_slice(span, 2)
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
    let owner = obj_h.map(ManagedPtrOwner::Heap);
    let value =
        StackValue::managed_ptr(ptr, stack.loader().corlib_type("System.Char"), owner, false);
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
        pop_args!(stack, [Int32(c)]);
        (c as u16, 0usize)
    } else {
        pop_args!(stack, [Int32(start_at), Int32(c)]);
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
    pop_args!(stack, [Int32(start_at)]);
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

pub fn intrinsic_field_string_empty<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _field: FieldDescription,
    _type_generics: Vec<ConcreteType>,
) {
    vm_push!(stack, gc, string(gc, CLRString::new(vec![])));
}

/// System.String::CopyStringContent(string, int, string)
pub fn intrinsic_string_copy_string_content<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(
        stack,
        [ObjectRef(src_val), Int32(dest_pos), ObjectRef(dest_val)]
    );
    let src = with_string!(stack, gc, StackValue::ObjectRef(src_val), |s| s.to_vec());
    with_string_mut!(stack, gc, StackValue::ObjectRef(dest_val), |dest| {
        let dest_pos = dest_pos as usize;
        let len = src.len();
        dest.as_mut_slice()[dest_pos..dest_pos + len].copy_from_slice(&src);
    });
    StepResult::InstructionStepped
}
