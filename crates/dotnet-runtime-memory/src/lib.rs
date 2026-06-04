//! Memory runtime services shared by VM execution and intrinsic handlers.
//!
//! [`RawMemoryAccess`] is the core memory-safety abstraction for managed reads and writes
//! (implemented as a large, ~1k-line access layer). It centralizes bounds validation,
//! layout-aware field safety checks, atomic-access constraints, and write-barrier recording
//! so higher-level opcode and intrinsic code paths can perform memory operations through one API.
//!
//! [`HeapManager`] owns heap-side coordination concerns: object registration for diagnostics,
//! finalization queues, handle tracking, pinning, and GC-related bookkeeping used during
//! mark/finalization flows.
//!
//! [`MemoryOwner`] routes access paths between local arena objects and (when multithreading is
//! enabled) cross-arena references. The host traits [`MemoryOrderingHost`] and
//! [`MemorySharedStateHost`] provide the threading/observability seams used by memory and
//! finalization code without coupling this crate to VM-global runtime state types.
//!
//! The write-barrier subsystem (including [`HeapWriteTarget`] and
//! [`flush_write_barrier_buffer`]) records cross-arena reference writes and flushes buffered
//! entries at safepoints (and panic unwind) so GC marking sees a complete inter-arena
//! reference graph. See `docs/GC_AND_MEMORY_SAFETY.md` for the end-to-end design.
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
