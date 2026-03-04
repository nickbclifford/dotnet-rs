use crate::intrinsics::{FieldIntrinsicId, MethodIntrinsicId};
use dotnet_types::members::MethodDescription;

pub type SignatureFilter = fn(&MethodDescription) -> bool;

#[derive(Copy, Clone, Debug)]
#[allow(dead_code)] // start and len are used in generated INTRINSIC_LOOKUP, but may appear dead to some analyses
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
    // type_name, member_name, and arity are for metadata and debugging, not used in runtime dispatch
    pub type_name: &'static str,
    #[allow(dead_code)]
    pub member_name: &'static str,
    #[allow(dead_code)]
    pub arity: u16,
    pub is_static: bool,
    pub handler: StaticIntrinsicHandler,
    pub filter: Option<SignatureFilter>,
}
