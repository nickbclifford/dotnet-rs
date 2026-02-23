use gc_arena::{Collect, unsafe_empty_collect};
use std::{
    fmt::{Debug, Formatter},
    ops::Deref,
};

#[macro_export]
macro_rules! with_string {
    ($stack:expr, $value:expr, |$s:ident| $code:expr) => {{
        let value = $value;
        let _guard = $crate::BorrowGuard::new($stack);
        let obj = value.as_object_ref();
        if let Some(handle) = obj.0 {
            let heap = handle.borrow();
            if let $crate::object::HeapStorage::Str(ref $s) = heap.storage {
                $code
            } else {
                panic!(
                    "invalid type on stack, expected string, received {:?}",
                    heap.storage
                )
            }
        } else {
            return $stack.throw_by_name_with_message(
                "System.NullReferenceException",
                "Object reference not set to an instance of an object.",
            );
        }
    }};
}

#[macro_export]
macro_rules! with_string_mut {
    ($stack:expr, $value:expr, |$s:ident| $code:expr) => {{
        let value = $value;
        let _guard = $crate::BorrowGuard::new($stack);
        let obj = value.as_object_ref();
        if let Some(handle) = obj.0 {
            let mut heap = handle.borrow_mut(&$stack.gc());
            if let $crate::object::HeapStorage::Str(ref mut $s) = heap.storage {
                $code
            } else {
                panic!(
                    "invalid type on stack, expected string, received {:?}",
                    heap.storage
                )
            }
        } else {
            return $stack.throw_by_name_with_message(
                "System.NullReferenceException",
                "Object reference not set to an instance of an object.",
            );
        }
    }};
}

#[derive(Clone, PartialEq)]
pub struct CLRString(Vec<u16>);
unsafe_empty_collect!(CLRString);

impl CLRString {
    pub fn new(chars: Vec<u16>) -> Self {
        Self(chars)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn size_bytes(&self) -> usize {
        size_of::<CLRString>() + self.0.len() * 2
    }

    pub fn as_string(&self) -> String {
        String::from_utf16(&self.0).unwrap()
    }

    pub fn as_mut_slice(&mut self) -> &mut [u16] {
        &mut self.0
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
