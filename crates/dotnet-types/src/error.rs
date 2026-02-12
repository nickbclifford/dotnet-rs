use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum TypeResolutionError {
    #[error("Type not found: {0}")]
    TypeNotFound(String),
    #[error("Method not found: {0}")]
    MethodNotFound(String),
    #[error("Field not found: {0}")]
    FieldNotFound(String),
    #[error("Invalid type handle")]
    InvalidHandle,
    #[error("Assembly load error: {0}")]
    AssemblyLoad(String),
    #[error("Massive allocation: {0}")]
    MassiveAllocation(String),
    #[error("Invalid layout: {0}")]
    InvalidLayout(String),
    #[error("Generic index {index} out of bounds (length {length})")]
    GenericIndexOutOfBounds { index: usize, length: usize },
}
