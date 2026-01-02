use crate::{
    match_method,
    value::{GenericLookup, HeapStorage, MethodDescription, Object, ObjectRef, StackValue},
    vm::{intrinsics::span_to_slice, CallStack, GCHandle, StepResult},
};

use gc_arena::{unsafe_empty_collect, Collect};
use std::{
    fmt::{Debug, Formatter},
    ops::Deref,
};

macro_rules! with_string {
    ($stack:expr, $gc:expr, $value:expr, |$s:ident| $code:expr) => {{
        vm_expect_stack!(let ObjectRef(obj) = $value);
        let ObjectRef(Some(obj)) = obj else {
            return $stack.throw_by_name($gc, "System.NullReferenceException");
        };
        let heap = obj.borrow();
        let HeapStorage::Str($s) = &*heap else {
            panic!("invalid type on stack, expected string, received {:?}", heap)
        };
        $code
    }};
}
pub(crate) use with_string;

#[derive(Clone, PartialEq)]
pub struct CLRString(Vec<u16>);
unsafe_empty_collect!(CLRString);

impl CLRString {
    pub fn new(chars: Vec<u16>) -> Self {
        Self(chars)
    }

    pub fn as_string(&self) -> String {
        String::from_utf16(&self.0).unwrap()
    }
}

impl Deref for CLRString {
    type Target = [u16];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Debug for CLRString {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_string())
    }
}

impl<T: AsRef<str>> From<T> for CLRString {
    fn from(s: T) -> Self {
        Self::new(s.as_ref().encode_utf16().collect())
    }
}

pub fn string_intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: GenericLookup,
) -> StepResult {
    macro_rules! pop {
        () => {
            vm_pop!(stack)
        };
    }
    macro_rules! push {
        ($($args:tt)*) => {
            vm_push!(stack, gc, $($args)*)
        };
    }
    macro_rules! string_op {
        ($($args:tt)*) => {
            with_string!(stack, gc, $($args)*)
        }
    }

    match_method!(method, {
        [static System.String::Equals(string, string)] => {
            let b = string_op!(pop!(), |b| b.to_vec());
            let a = string_op!(pop!(), |a| a.to_vec());

            push!(Int32(if a == b { 1 } else { 0 }));
            Some(StepResult::InstructionStepped)
        },
        [static System.String::FastAllocateString(int)] => {
            let len = match pop!() {
                StackValue::Int32(i) => i as usize,
                rest => panic!("invalid length for FastAllocateString: {:?}", rest),
            };
            let value = CLRString::new(vec![0u16; len]);
            push!(string(gc, value));
            Some(StepResult::InstructionStepped)
        },
        [static System.String::FastAllocateString(
            * System.Runtime.CompilerServices.MethodTable,
            nint
        )] => {
            let len = match pop!() {
                StackValue::NativeInt(i) => i as usize,
                rest => panic!("invalid length for FastAllocateString: {:?}", rest),
            };
            pop!(); // pop method table pointer
            let value = CLRString::new(vec![0u16; len]);
            push!(string(gc, value));
            Some(StepResult::InstructionStepped)
        },
        [System.String::get_Chars(int)] => {
            vm_expect_stack!(let Int32(index) = pop!());
            let value = string_op!(pop!(), |s| s[index as usize]);
            push!(Int32(value as i32));
            Some(StepResult::InstructionStepped)
        },
        [static System.String::Concat(ReadOnlySpan<char>, ReadOnlySpan<char>, ReadOnlySpan<char>)] => {
            vm_expect_stack!(let ValueType(span2) = pop!());
            vm_expect_stack!(let ValueType(span1) = pop!());
            vm_expect_stack!(let ValueType(span0) = pop!());

            fn char_span_into_str(span: Object) -> Vec<u16> {
                span_to_slice(span)
                    .chunks_exact(2)
                    .map(|c| u16::from_ne_bytes([c[0], c[1]]))
                    .collect::<Vec<_>>()
            }

            let data0 = char_span_into_str(*span0);
            let data1 = char_span_into_str(*span1);
            let data2 = char_span_into_str(*span2);

            let value = CLRString::new(data0.into_iter().chain(data1).chain(data2).collect());
            push!(string(gc, value));
            Some(StepResult::InstructionStepped)
        },
        [System.String::GetHashCodeOrdinalIgnoreCase()] => {
            use std::hash::*;

            let mut h = DefaultHasher::new();
            let value = string_op!(pop!(), |s| String::from_utf16_lossy(s)
                .to_uppercase()
                .into_bytes());
            value.hash(&mut h);
            let code = h.finish();

            push!(Int32(code as i32));
            Some(StepResult::InstructionStepped)
        },
        [System.String::GetPinnableReference()]
        | [System.String::GetRawStringData()] => {
            let val = pop!();
            let obj_h = if let StackValue::ObjectRef(ObjectRef(Some(h))) = &val {
                Some(*h)
            } else {
                None
            };
            let ptr = with_string!(stack, gc, val, |s| s.as_ptr() as *mut u8);
            let value =
                StackValue::managed_ptr(ptr, stack.runtime.assemblies.corlib_type("System.Char"), obj_h, false);
            push!(value);
            Some(StepResult::InstructionStepped)
        },
        [System.String::get_Length()] => {
            let len = string_op!(pop!(), |s| s.len());
            push!(Int32(len as i32));
            Some(StepResult::InstructionStepped)
        },
        [System.String::IndexOf(char)] => {
            vm_expect_stack!(let Int32(c) = pop!());
            let c = c as u16;
            let index = string_op!(pop!(), |s| s.iter().position(|x| *x == c));

            push!(Int32(match index {
                None => -1,
                Some(i) => i as i32,
            }));
            Some(StepResult::InstructionStepped)
        },
        [System.String::IndexOf(char, int)] => {
            vm_expect_stack!(let Int32(start_at) = pop!());
            vm_expect_stack!(let Int32(c) = pop!());
            let c = c as u16;
            let index = string_op!(pop!(), |s| s
                .iter()
                .skip(start_at as usize)
                .position(|x| *x == c));

            push!(Int32(match index {
                None => -1,
                Some(i) => i as i32,
            }));
            Some(StepResult::InstructionStepped)
        },
        [System.String::Substring(int)] => {
            vm_expect_stack!(let Int32(start_at) = pop!());
            let value = string_op!(pop!(), |s| s.split_at(start_at as usize).0.to_vec());
            push!(string(gc, CLRString::new(value)));
            Some(StepResult::InstructionStepped)
        },
    }).expect("unsupported intrinsic call to String");

    StepResult::InstructionStepped
}
