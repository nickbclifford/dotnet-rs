use thiserror::Error;

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

#[derive(Debug, Error, Clone, PartialEq)]
#[cfg_attr(feature = "fuzzing", derive(Arbitrary))]
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

#[derive(Debug, Error, Clone, PartialEq)]
#[cfg_attr(feature = "fuzzing", derive(Arbitrary))]
pub enum AssemblyLoadError {
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
    #[error("IO error: {0}")]
    Io(String),
}

impl From<std::io::Error> for AssemblyLoadError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
#[cfg_attr(feature = "fuzzing", derive(Arbitrary))]
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

    #[error("Internal error: {0}")]
    InternalError(String),

    #[error("Fuzzing instruction budget exceeded")]
    FuzzBudgetExceeded,

    #[error("Execution aborted: {0}")]
    Aborted(String),
}

#[derive(Debug, Error, Clone, PartialEq)]
#[cfg_attr(feature = "fuzzing", derive(Arbitrary))]
pub enum MemoryError {
    #[error("Access violation at {0:x}")]
    AccessViolation(usize),
    #[error("Out of memory")]
    OutOfMemory,
}

#[derive(Debug, Error, Clone, PartialEq)]
#[cfg_attr(feature = "fuzzing", derive(Arbitrary))]
pub enum PInvokeError {
    #[error("Unable to find library '{0}'")]
    LibraryNotFound(String),
    #[error("Unable to find entry point '{1}' in library '{0}'")]
    SymbolNotFound(String, String),
    #[error("Failed to load library '{0}': {1}")]
    LoadError(String, String),
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum PointerDeserializationError {
    #[error("Unknown tag: {0}")]
    UnknownTag(usize),
    #[error("Unknown subtag: {0}")]
    UnknownSubtag(usize),
    #[error("Invalid static id: {0}")]
    InvalidStaticId(u32),
    #[error("Checksum mismatch")]
    ChecksumMismatch,
}

#[cfg(feature = "fuzzing")]
impl Arbitrary<'_> for PointerDeserializationError {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let tag = u.arbitrary::<usize>()? % 4;
        match tag {
            0 => Ok(PointerDeserializationError::UnknownTag(u.arbitrary()?)),
            1 => Ok(PointerDeserializationError::UnknownSubtag(u.arbitrary()?)),
            2 => Ok(PointerDeserializationError::InvalidStaticId(u.arbitrary()?)),
            _ => Ok(PointerDeserializationError::ChecksumMismatch),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
#[cfg_attr(feature = "fuzzing", derive(Arbitrary))]
pub enum MemoryAccessError {
    #[error("Access out of bounds: offset={offset}, size={size}, len={len}")]
    BoundsCheck {
        offset: usize,
        size: usize,
        len: usize,
    },
    #[error("Null pointer access: {0}")]
    NullPointer(String),
    #[error("Unaligned access at address {0:x}")]
    UnalignedAccess(usize),
    #[error("Type mismatch: {0}")]
    TypeMismatch(String),
    #[error("Invalid pointer origin")]
    InvalidOrigin,
    #[error("Cross-arena violation")]
    CrossArenaViolation,
}

#[derive(Debug, Error, Clone, PartialEq)]
#[cfg_attr(feature = "fuzzing", derive(Arbitrary))]
pub enum IntrinsicError {
    #[error("Intrinsic error: {0}")]
    Message(String),
    #[error("Memory error: {0}")]
    Memory(#[from] MemoryAccessError),
}

#[derive(Debug, Error, Clone, PartialEq)]
#[cfg_attr(feature = "fuzzing", derive(Arbitrary))]
pub enum VmError {
    #[error("Assembly loading failed: {0}")]
    AssemblyLoad(#[from] AssemblyLoadError),

    #[error("Type resolution failed: {0}")]
    TypeResolution(#[from] TypeResolutionError),

    #[error("Method execution failed: {0}")]
    Execution(#[from] ExecutionError),

    #[error("Memory access violation: {0}")]
    Memory(#[from] MemoryError),

    #[error("Memory access error: {0}")]
    MemoryAccess(#[from] MemoryAccessError),

    #[error("Intrinsic error: {0}")]
    Intrinsic(#[from] IntrinsicError),

    #[error("PInvoke error: {0}")]
    PInvoke(#[from] PInvokeError),

    #[error("Pointer deserialization failed: {0}")]
    PointerDeserialization(#[from] PointerDeserializationError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversions() {
        let err = MemoryAccessError::BoundsCheck {
            offset: 10,
            size: 4,
            len: 8,
        };
        let vm_err: VmError = err.into();
        match vm_err {
            VmError::MemoryAccess(MemoryAccessError::BoundsCheck {
                offset: 10,
                size: 4,
                len: 8,
            }) => {}
            _ => panic!("Expected MemoryAccessError::BoundsCheck"),
        }
    }
}
