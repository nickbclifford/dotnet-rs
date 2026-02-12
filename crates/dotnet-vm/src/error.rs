use dotnet_assemblies::error::AssemblyLoadError;
use dotnet_types::error::TypeResolutionError;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum VmError {
    #[error("Assembly loading failed: {0}")]
    AssemblyLoad(#[from] AssemblyLoadError),

    #[error("Type resolution failed: {0}")]
    TypeResolution(#[from] TypeResolutionError),

    #[error("Method execution failed: {0}")]
    Execution(#[from] ExecutionError),

    #[error("Memory access violation: {0}")]
    Memory(#[from] MemoryError),
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ExecutionError {
    #[error("Stack underflow")]
    StackUnderflow,

    #[error("Invalid instruction pointer: {0}")]
    InvalidIP(usize),

    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    #[error("Null reference")]
    NullReference,

    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum MemoryError {
    #[error("Access violation at {0:x}")]
    AccessViolation(usize),
    #[error("Out of memory")]
    OutOfMemory,
}
