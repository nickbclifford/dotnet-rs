//! # dotnet-value
//!
//! Representation of .NET values, objects, and pointers.
//! This crate defines the fundamental types used by the VM's evaluation stack and heap.
//!
//! ## Core Types
//!
//! - **[`StackValue`]**: Unified representation of values on the execution stack.
//! - **[`ObjectRef`]**: GC-managed reference to a heap object.
//! - **[`ManagedPtr`]**: Pointer to a location within a managed object or the stack.
//! - **[`Object`]**: Internal representation of an object's instance data and layout.
//! - **[`CLRString`]**: Specialized representation for .NET strings.
#![allow(clippy::mutable_key_type)]
#[cfg(test)]
mod atomic_tests;
pub mod layout;
pub mod object;
#[cfg(test)]
mod object_tests;
pub mod pointer;
pub mod ptr_common;
pub mod stack_value;
pub mod storage;
pub mod string;
pub mod validation;
#[cfg(test)]
mod validation_tests;

pub use crate::{
    object::{HeapStorage, Object, ObjectHandle, ObjectPtr, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin, UnmanagedPtr},
    stack_value::{ManagedExceptionError, StackValue},
    string::CLRString,
    validation::ValidationTag,
};
pub use dotnet_utils::{
    ArenaId, ArgumentIndex, BorrowScopeOps, ByteOffset, FieldIndex, GcReadyToken, GcScopeGuard,
    LocalIndex, StackSlotIndex,
};
pub use dotnetdll::prelude::{LoadType, NumberSign, StoreType};
