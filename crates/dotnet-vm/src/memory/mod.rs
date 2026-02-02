pub mod access;
pub mod heap;

pub use access::{check_read_safety, has_ref_at, RawMemoryAccess};
pub use heap::HeapManager;
