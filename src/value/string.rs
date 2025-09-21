use crate::{
    value::{GenericLookup, HeapStorage, MethodDescription, Object, ObjectRef, StackValue},
    vm::{
        intrinsics::{expect_stack, span_to_slice},
        CallStack, GCHandle,
    },
};

use gc_arena::{unsafe_empty_collect, Collect};
use std::{
    fmt::{Debug, Formatter},
    ops::Deref,
};

macro_rules! with_string {
    ($value:expr, |$s:ident| $code:expr) => {{
        expect_stack!(let ObjectRef(obj) = $value);
        let ObjectRef(Some(obj)) = obj else { todo!("NullPointerException") };
        let heap = obj.borrow();
        let HeapStorage::Str($s) = &*heap else {
            todo!("invalid type on stack, expected string")
        };
        $code
    }};
}
pub(crate) use with_string;

#[derive(Clone)]
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
) {
    macro_rules! pop {
        () => {
            stack.pop_stack()
        };
    }
    macro_rules! push {
        ($value:expr) => {
            stack.push_stack(gc, $value)
        };
    }

    // TODO: real signature checking
    match format!("{:?}", method).as_str() {
        "static bool System.String::Equals(string, string)" => {
            let b = with_string!(pop!(), |b| b.to_vec());
            let a = with_string!(pop!(), |a| a.to_vec());

            push!(StackValue::Int32(if a == b { 1 } else { 0 }));
        }
        "static string System.String::FastAllocateString(int)" => {
            expect_stack!(let Int32(len) = pop!());
            let value = CLRString::new(vec![0u16; len as usize]);
            push!(StackValue::string(gc, value));
        }
        "char System.String::get_Chars(int)" => {
            expect_stack!(let Int32(index) = pop!());
            let value = with_string!(pop!(), |s| s[index as usize]);
            push!(StackValue::Int32(value as i32));
        }
        "static string System.String::Concat(valuetype System.ReadOnlySpan`1<char>, valuetype System.ReadOnlySpan`1<char>, valuetype System.ReadOnlySpan`1<char>)" => {
            expect_stack!(let ValueType(span2) = pop!());
            expect_stack!(let ValueType(span1) = pop!());
            expect_stack!(let ValueType(span0) = pop!());

            fn char_span_into_str(span: Object) -> Vec<u16> {
                span_to_slice(span).chunks_exact(2).map(|c| u16::from_ne_bytes([c[0], c[1]])).collect::<Vec<_>>()
            }

            let data0 = char_span_into_str(*span0);
            let data1 = char_span_into_str(*span1);
            let data2 = char_span_into_str(*span2);

            let value = CLRString::new(data0.into_iter().chain(data1).chain(data2).collect());
            push!(StackValue::string(gc, value));
        }
        "int System.String::GetHashCodeOrdinalIgnoreCase()" => {
            use std::hash::*;

            let mut h = DefaultHasher::new();
            let value = with_string!(pop!(), |s| String::from_utf16_lossy(s).to_uppercase().into_bytes());
            value.hash(&mut h);
            let code = h.finish();

            push!(StackValue::Int32(code as i32));
        }
        "<req System.Runtime.InteropServices.InAttribute> ref char System.String::GetPinnableReference()" |
        "ref char System.String::GetRawStringData()" => {
            let ptr = with_string!(pop!(), |s| s.as_ptr() as *mut u8);
            let value = StackValue::managed_ptr(ptr, stack.assemblies.corlib_type("System.Char"));
            push!(value);
        }
        "int System.String::get_Length()" => {
            let len = with_string!(pop!(), |s| s.len());
            push!(StackValue::Int32(len as i32));
        }
        "int System.String::IndexOf(char)" => {
            expect_stack!(let Int32(c) = pop!());
            let c = c as u16;
            let index = with_string!(pop!(), |s| s.iter().position(|x| *x == c));

            push!(StackValue::Int32(match index {
                None => -1,
                Some(i) => i as i32,
            }));
        }
        "int System.String::IndexOf(char, int)" => {
            expect_stack!(let Int32(start_at) = pop!());
            expect_stack!(let Int32(c) = pop!());
            let c = c as u16;
            let index = with_string!(pop!(), |s| s.iter().skip(start_at as usize).position(|x| *x == c));

            push!(StackValue::Int32(match index {
                None => -1,
                Some(i) => i as i32,
            }));
        }
        "string System.String::Substring(int)" => {
            expect_stack!(let Int32(start_at) = pop!());
            let value = with_string!(pop!(), |s| s.split_at(start_at as usize).0.to_vec());
            push!(StackValue::string(gc, CLRString::new(value)));
        }
        x => panic!("unsupported intrinsic call to {}", x),
    }
    stack.increment_ip();
}
