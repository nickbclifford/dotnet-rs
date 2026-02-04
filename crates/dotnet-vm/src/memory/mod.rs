pub mod access;
pub mod heap;

pub use access::{RawMemoryAccess, check_read_safety, has_ref_at};
pub use heap::HeapManager;
