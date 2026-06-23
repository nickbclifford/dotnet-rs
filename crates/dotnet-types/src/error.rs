use thiserror::Error;

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;
use std::{fmt, io, sync::Arc};

#[derive(Debug, Error, Clone, PartialEq)]
#[cfg_attr(feature = "fuzzing", derive(Arbitrary))]
pub enum TypeResolutionError {
    #[error("Type not found: {0}")]
    TypeNotFound(Box<str>),
    #[error("Method not found: {0}")]
    MethodNotFound(Box<str>),
    #[error("Field not found: {0}")]
    FieldNotFound(Box<str>),
    #[error("Invalid type handle")]
    InvalidHandle,
    #[error("Assembly load error: {0}")]
    AssemblyLoad(Box<str>),
    #[error("Massive allocation: {0}")]
    MassiveAllocation(Box<str>),
    #[error("Invalid layout: {0}")]
    InvalidLayout(Box<str>),
    #[error("Generic index {index} out of bounds (length {length})")]
    GenericIndexOutOfBounds { index: usize, length: usize },
    #[error("Generic constraint violation: {0}")]
    GenericConstraintViolation(Box<str>),
}

#[derive(Debug)]
pub struct CloneableIoError(Arc<io::Error>);

impl CloneableIoError {
    pub fn new(err: io::Error) -> Self {
        Self(Arc::new(err))
    }
}

impl Clone for CloneableIoError {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl PartialEq for CloneableIoError {
    fn eq(&self, other: &Self) -> bool {
        self.0.kind() == other.0.kind()
            && self.0.raw_os_error() == other.0.raw_os_error()
            && self.0.to_string() == other.0.to_string()
    }
}

impl fmt::Display for CloneableIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CloneableIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum AssemblyLoadError {
    #[error("File not found: {0}")]
    FileNotFound(Box<str>),
    #[error("Invalid format: {0}")]
    InvalidFormat(Box<str>),
    #[error("IO error: {0}")]
    Io(#[source] CloneableIoError),
}

impl From<io::Error> for AssemblyLoadError {
    fn from(err: io::Error) -> Self {
        Self::Io(CloneableIoError::new(err))
    }
}

#[cfg(feature = "fuzzing")]
impl Arbitrary<'_> for AssemblyLoadError {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let tag = u.arbitrary::<u8>()? % 3;
        match tag {
            0 => Ok(Self::FileNotFound(
                u.arbitrary::<String>()?.into_boxed_str(),
            )),
            1 => Ok(Self::InvalidFormat(
                u.arbitrary::<String>()?.into_boxed_str(),
            )),
            _ => Ok(Self::Io(CloneableIoError::new(io::Error::new(
                io::ErrorKind::NotFound,
                "arbitrary io error",
            )))),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ExecutionError {
    #[error("Stack underflow")]
    StackUnderflow,

    #[error("Invalid instruction pointer: {0}")]
    InvalidIP(usize),

    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch {
        expected: &'static str,
        actual: Box<str>,
    },

    #[error("Null reference")]
    NullReference,

    #[error("Not implemented: {0}")]
    NotImplemented(Box<str>),

    #[error("Invalid CIL: {0}")]
    InvalidCil(Box<str>),

    #[error("Internal error: {0}")]
    InternalError(Box<str>),

    #[error("Fuzzing instruction budget exceeded")]
    FuzzBudgetExceeded,

    #[error("Execution aborted: {0}")]
    Aborted(Box<str>),
}

#[cfg(feature = "fuzzing")]
impl Arbitrary<'_> for ExecutionError {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let tag = u.arbitrary::<u8>()? % 9;
        match tag {
            0 => Ok(ExecutionError::StackUnderflow),
            1 => Ok(ExecutionError::InvalidIP(u.arbitrary()?)),
            2 => Ok(ExecutionError::TypeMismatch {
                expected: "arbitrary expected type",
                actual: u.arbitrary::<String>()?.into_boxed_str(),
            }),
            3 => Ok(ExecutionError::NullReference),
            4 => Ok(ExecutionError::NotImplemented(
                u.arbitrary::<String>()?.into_boxed_str(),
            )),
            5 => Ok(ExecutionError::InvalidCil(
                u.arbitrary::<String>()?.into_boxed_str(),
            )),
            6 => Ok(ExecutionError::InternalError(
                u.arbitrary::<String>()?.into_boxed_str(),
            )),
            7 => Ok(ExecutionError::FuzzBudgetExceeded),
            _ => Ok(ExecutionError::Aborted(
                u.arbitrary::<String>()?.into_boxed_str(),
            )),
        }
    }
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
    LibraryNotFound(Box<str>),
    #[error("Unable to find entry point '{1}' in library '{0}'")]
    SymbolNotFound(Box<str>, Box<str>),
    #[error("Failed to load library '{0}': {1}")]
    LoadError(Box<str>, Box<str>),
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
pub enum MemoryAccessError {
    #[error("Access out of bounds: offset={offset}, size={size}, len={len}")]
    BoundsCheck {
        offset: usize,
        size: usize,
        len: usize,
    },
    #[error("Null pointer access: {0}")]
    NullPointer(&'static str),
    #[error("Unaligned access at address {0:x}")]
    UnalignedAccess(usize),
    #[error("Type mismatch: {0}")]
    TypeMismatch(Box<str>),
    #[error("Invalid pointer origin")]
    InvalidOrigin,
    #[error("Cross-arena violation")]
    CrossArenaViolation,
}

#[cfg(feature = "fuzzing")]
impl Arbitrary<'_> for MemoryAccessError {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let tag = u.arbitrary::<u8>()? % 6;
        match tag {
            0 => Ok(Self::BoundsCheck {
                offset: u.arbitrary()?,
                size: u.arbitrary()?,
                len: u.arbitrary()?,
            }),
            1 => Ok(Self::NullPointer("arbitrary null pointer")),
            2 => Ok(Self::UnalignedAccess(u.arbitrary()?)),
            3 => Ok(Self::TypeMismatch(
                u.arbitrary::<String>()?.into_boxed_str(),
            )),
            4 => Ok(Self::InvalidOrigin),
            _ => Ok(Self::CrossArenaViolation),
        }
    }
}

/// Error type returned by compare-and-exchange atomic operations.
///
/// Distinguishes a CAS mismatch (the expected value did not match — the current
/// value is returned so the caller can retry or report it) from a hard
/// memory-access error (e.g. a bounds violation) that prevents the operation
/// from proceeding at all.
#[derive(Debug, Error, Clone, PartialEq)]
#[cfg_attr(feature = "fuzzing", derive(Arbitrary))]
pub enum CompareExchangeError {
    /// The expected value did not match; contains the actual current value.
    #[error("CAS mismatch: current value is {0}")]
    Mismatch(u64),
    /// A memory-access error occurred before the exchange could be attempted.
    #[error("Atomic bounds check failed: {0}")]
    Bounds(MemoryAccessError),
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum IntrinsicError {
    #[error("Intrinsic error: {0}")]
    Static(&'static str),
    #[error("Intrinsic error: {0}")]
    Message(Box<str>),
    #[error("Memory error: {0}")]
    Memory(#[from] MemoryAccessError),
}

#[cfg(feature = "fuzzing")]
impl Arbitrary<'_> for IntrinsicError {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        let tag = u.arbitrary::<u8>()? % 3;
        match tag {
            0 => Ok(Self::Static("arbitrary intrinsic error")),
            1 => Ok(Self::Message(u.arbitrary::<String>()?.into_boxed_str())),
            _ => Ok(Self::Memory(u.arbitrary()?)),
        }
    }
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
    use std::{error::Error as _, io};

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

    #[test]
    fn test_assembly_load_io_preserves_source() {
        let err = AssemblyLoadError::from(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "fixture denied",
        ));

        let cloneable_source = err
            .source()
            .expect("AssemblyLoadError::Io should expose CloneableIoError as source");
        assert_eq!(cloneable_source.to_string(), "fixture denied");

        let io_source = cloneable_source
            .source()
            .and_then(|source| source.downcast_ref::<io::Error>())
            .expect("CloneableIoError should expose the original io::Error as source");
        assert_eq!(io_source.kind(), io::ErrorKind::PermissionDenied);
    }
}
