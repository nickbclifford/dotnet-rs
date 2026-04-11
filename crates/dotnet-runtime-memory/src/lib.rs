//! Memory runtime services shared by VM and intrinsic execution.
pub mod access;
pub mod heap;
pub mod host;
pub mod ops;
pub mod validation;

pub use access::{MemoryOwner, RawMemoryAccess};
pub use heap::HeapManager;
pub use host::{MemoryOrderingHost, MemorySharedStateHost};
pub use validation::{check_read_safety, has_ref_at};
