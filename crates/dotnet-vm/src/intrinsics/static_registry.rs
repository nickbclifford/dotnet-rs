use crate::intrinsics::{MethodIntrinsicId, FieldIntrinsicId};
use dotnet_types::members::MethodDescription;

pub type SignatureFilter = fn(&MethodDescription) -> bool;

#[derive(Copy, Clone, Debug)]
#[allow(dead_code)]
pub struct Range {
    pub start: usize,
    pub len: usize,
}

pub enum StaticIntrinsicHandler {
    Method(MethodIntrinsicId),
    Field(FieldIntrinsicId),
}

pub struct StaticIntrinsicEntry {
    #[allow(dead_code)]
    pub type_name: &'static str,
    #[allow(dead_code)]
    pub member_name: &'static str,
    #[allow(dead_code)]
    pub arity: u16,
    pub is_static: bool,
    pub handler: StaticIntrinsicHandler,
    pub filter: Option<SignatureFilter>,
}
