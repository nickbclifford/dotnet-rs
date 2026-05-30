//! Memory runtime services shared by VM and intrinsic execution.
pub mod access;
pub mod heap;
pub mod host;
pub mod ops;
pub mod validation;
pub(crate) mod write_barrier;

pub use access::RawMemoryAccess;
pub use heap::HeapManager;
pub use host::{MemoryOrderingHost, MemorySharedStateHost};
pub use validation::check_read_safety;
pub use write_barrier::{HeapWriteTarget, MemoryOwner, flush_write_barrier_buffer};
