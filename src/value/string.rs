use std::fmt::{Debug, Formatter};
use gc_arena::{Collect, unsafe_empty_collect};

#[derive(Clone)]
pub struct CLRString(Vec<u16>);
unsafe_empty_collect!(CLRString);

impl CLRString {
    pub fn new(chars: Vec<u16>) -> Self {
        Self(chars)
    }
}

impl Debug for CLRString {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", String::from_utf16(&self.0).unwrap())
    }
}

// TODO: internal calls
