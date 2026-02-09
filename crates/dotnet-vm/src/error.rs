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
pub enum AssemblyLoadError {
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
    #[error("IO error: {0}")]
    Io(String),
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum TypeResolutionError {
    #[error("Type not found: {0}")]
    NotFound(String),
    #[error("Invalid type handle")]
    InvalidHandle,
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
