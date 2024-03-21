use gc_arena::{Collect, unsafe_empty_collect};

#[derive(Debug, Clone)]
pub struct CLRString(Vec<u16>);
unsafe_empty_collect!(CLRString);

impl CLRString {
    pub fn new(chars: Vec<u16>) -> Self {
        Self(chars)
    }
}

// TODO: internal calls
