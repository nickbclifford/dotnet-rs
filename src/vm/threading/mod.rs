#[cfg(feature = "multithreading")]
mod basic;
#[cfg(feature = "multithreading")]
#[cfg(feature = "multithreaded-gc")]
mod gc_coord;
#[cfg(not(feature = "multithreading"))]
mod stub;

#[cfg(feature = "multithreading")]
pub use basic::*;

#[cfg(feature = "multithreading")]
#[cfg(feature = "multithreaded-gc")]
pub use gc_coord::*;

#[cfg(not(feature = "multithreading"))]
pub use stub::*;
