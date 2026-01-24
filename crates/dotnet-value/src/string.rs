use gc_arena::{unsafe_empty_collect, Collect};
use std::{
    fmt::{Debug, Formatter},
    ops::Deref,
};

#[macro_export]
macro_rules! with_string {
    ($stack:expr, $gc:expr, $value:expr, |$s:ident| $code:expr) => {{
        $crate::vm_expect_stack!(let ObjectRef(obj) = $value);
        let $crate::object::ObjectRef(Some(obj)) = obj else {
            return $stack.throw_by_name($gc, "System.NullReferenceException");
        };
        let heap = obj.borrow();
        let $crate::object::HeapStorage::Str($s) = &heap.storage else {
            panic!("invalid type on stack, expected string, received {:?}", heap.storage)
        };
        $code
    }};
}

#[macro_export]
macro_rules! with_string_mut {
    ($stack:expr, $gc:expr, $value:expr, |$s:ident| $code:expr) => {{
        $crate::vm_expect_stack!(let ObjectRef(obj) = $value);
        let $crate::object::ObjectRef(Some(obj)) = obj else {
            return $stack.throw_by_name($gc, "System.NullReferenceException");
        };
        let mut heap = obj.borrow_mut($gc);
        let $crate::object::HeapStorage::Str($s) = &mut heap.storage else {
            panic!("invalid type on stack, expected string, received {:?}", heap.storage)
        };
        $code
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
