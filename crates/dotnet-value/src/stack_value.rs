#[cfg(feature = "multithreading")]
use crate::object::ObjectPtr;
#[cfg(feature = "multithreading")]
use dotnet_utils::{ArenaId, gc::record_cross_arena_ref};

use crate::{
    object::{self, HeapStorage, Object, ObjectHandle, ObjectRef},
    pointer::{self, ManagedPtr, PointerOrigin, UnmanagedPtr},
    string::CLRString,
};
use dotnet_types::{TypeDescription, error::ExecutionError};
use dotnet_utils::{
    ByteOffset, StackSlotIndex,
    atomic::{AtomicAccess, StandardAtomicAccess},
    gc::{GCHandle, ThreadSafeLock},
    sync::Ordering as AtomicOrdering,
    validate_alignment,
};
use dotnetdll::prelude::*;
use gc_arena::{Collect, Gc, collect::Trace};
use std::{
    cmp::Ordering,
    fmt::Debug,
    ops::{Add, BitAnd, BitOr, BitXor, Deref, DerefMut, Mul, Neg, Not, Shl, Sub},
    ptr::NonNull,
    sync::Arc,
};
use thiserror::Error;

#[cfg(feature = "fuzzing")]
use arbitrary::Arbitrary;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{exception_type}: {message}")]
pub struct ManagedExceptionError {
    pub exception_type: &'static str,
    pub message: &'static str,
}

impl ManagedExceptionError {
    pub const fn overflow() -> Self {
        Self {
            exception_type: "System.OverflowException",
            message: "Arithmetic operation resulted in an overflow.",
        }
    }

    pub const fn divide_by_zero() -> Self {
        Self {
            exception_type: "System.DivideByZeroException",
            message: "Attempted to divide by zero.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum StackValueError {
    #[error("{0}")]
    ManagedException(#[from] ManagedExceptionError),
    #[error(transparent)]
    Execution(#[from] ExecutionError),
}

#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub struct StackManagedPtr<'gc>(Box<ManagedPtr<'gc>>);

impl<'gc> StackManagedPtr<'gc> {
    pub fn new(ptr: ManagedPtr<'gc>) -> Self {
        Self(Box::new(ptr))
    }

    pub fn into_inner(self) -> ManagedPtr<'gc> {
        *self.0
    }
}

impl<'gc> From<ManagedPtr<'gc>> for StackManagedPtr<'gc> {
    fn from(value: ManagedPtr<'gc>) -> Self {
        Self::new(value)
    }
}

impl<'gc> Deref for StackManagedPtr<'gc> {
    type Target = ManagedPtr<'gc>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'gc> DerefMut for StackManagedPtr<'gc> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

// SAFETY: StackManagedPtr contains a boxed ManagedPtr and delegates tracing to it.
unsafe impl<'gc> Collect<'gc> for StackManagedPtr<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        self.0.trace(cc);
    }
}

#[derive(Clone, Debug)]
pub enum StackValue<'gc> {
    Int32(i32),
    Int64(i64),
    NativeInt(isize),
    NativeFloat(f64),
    ObjectRef(ObjectRef<'gc>),
    UnmanagedPtr(UnmanagedPtr),
    ManagedPtr(StackManagedPtr<'gc>),
    ValueType(Object<'gc>),
    TypedRef(StackManagedPtr<'gc>, Arc<TypeDescription>),
    /// Reference to an object in another thread's arena.
    /// (ObjectPtr, OwningThreadID)
    #[cfg(feature = "multithreading")]
    CrossArenaObjectRef(ObjectPtr, ArenaId),
}

#[cfg(feature = "fuzzing")]
impl<'a, 'gc> Arbitrary<'a> for StackValue<'gc> {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let variant = u.int_in_range(0..=9)?;
        match variant {
            0 => Ok(StackValue::Int32(u.arbitrary()?)),
            1 => Ok(StackValue::Int64(u.arbitrary()?)),
            2 => Ok(StackValue::NativeInt(u.arbitrary()?)),
            3 => Ok(StackValue::NativeFloat(u.arbitrary()?)),
            4 => Ok(StackValue::ObjectRef(u.arbitrary()?)),
            5 => Ok(StackValue::UnmanagedPtr(u.arbitrary()?)),
            6 => Ok(StackValue::ManagedPtr(StackManagedPtr::new(u.arbitrary()?))),
            7 => Ok(StackValue::ValueType(u.arbitrary()?)),
            8 => Ok(StackValue::TypedRef(
                StackManagedPtr::new(u.arbitrary()?),
                Arc::new(u.arbitrary()?),
            )),
            #[cfg(feature = "multithreading")]
            9 => Ok(StackValue::CrossArenaObjectRef(
                u.arbitrary()?,
                u.arbitrary()?,
            )),
            #[cfg(not(feature = "multithreading"))]
            9 => Ok(StackValue::Int32(u.arbitrary()?)),
            _ => unreachable!(),
        }
    }
}

// SAFETY: StackValue contains several variants that hold GC-managed references.
// We manually implement trace to ensure all such references (ObjectRef, ManagedPtr, ValueType)
// are correctly visited by the GC. Cross-arena references are recorded for coordinated GC.
unsafe impl<'gc> Collect<'gc> for StackValue<'gc> {
    fn trace<Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        match self {
            Self::ObjectRef(o) => o.trace(cc),
            Self::ManagedPtr(m) => m.trace(cc),
            Self::TypedRef(m, _) => m.trace(cc),
            Self::ValueType(v) => {
                v.trace(cc);
            }
            #[cfg(feature = "multithreading")]
            Self::CrossArenaObjectRef(ptr, tid) => {
                record_cross_arena_ref(*tid, ptr.as_ptr() as usize);
            }
            _ => {}
        }
    }
}

#[inline]
pub fn stack_value_kind(v: &StackValue<'_>) -> &'static str {
    match v {
        StackValue::Int32(_) => "Int32",
        StackValue::Int64(_) => "Int64",
        StackValue::NativeInt(_) => "NativeInt",
        StackValue::NativeFloat(_) => "NativeFloat",
        StackValue::ObjectRef(_) => "ObjectRef",
        StackValue::UnmanagedPtr(_) => "UnmanagedPtr",
        StackValue::ManagedPtr(_) => "ManagedPtr",
        StackValue::ValueType(_) => "ValueType",
        StackValue::TypedRef(_, _) => "TypedRef",
        #[cfg(feature = "multithreading")]
        StackValue::CrossArenaObjectRef(_, _) => "CrossArenaObjectRef",
    }
}

#[inline]
fn extract_shift_amount(v: StackValue<'_>) -> Result<u32, ExecutionError> {
    match v {
        StackValue::Int32(i) => Ok(i as u32),
        StackValue::NativeInt(i) => Ok(i as u32),
        v => Err(ExecutionError::TypeMismatch {
            expected: "shift amount (Int32 or NativeInt)".to_string(),
            actual: stack_value_kind(&v).to_string(),
        }),
    }
}

#[inline]
fn managed_ptr_to_raw<'gc>(m: &StackManagedPtr<'gc>) -> *mut u8 {
    if let Some(owner) = m.owner() {
        unsafe {
            owner
                .0
                .map(|h: ObjectHandle<'gc>| h.borrow().storage.raw_data_ptr().add(m.offset.as_usize()))
                .unwrap_or(std::ptr::null_mut())
        }
    } else {
        m.offset.as_usize() as *mut u8
    }
}

macro_rules! checked_arithmetic_op {
    ($l:expr, $r:expr, $sgn:expr, $op:ident) => {
        match ($l, $r, $sgn) {
            (StackValue::Int32(l), StackValue::Int32(r), NumberSign::Signed) => l
                .$op(r)
                .map(StackValue::Int32)
                .ok_or_else(|| ManagedExceptionError::overflow().into()),
            (StackValue::Int32(l), StackValue::Int32(r), NumberSign::Unsigned) => (l as u32)
                .$op(r as u32)
                .map(|v| StackValue::Int32(v as i32))
                .ok_or_else(|| ManagedExceptionError::overflow().into()),
            (StackValue::Int64(l), StackValue::Int64(r), NumberSign::Signed) => l
                .$op(r)
                .map(StackValue::Int64)
                .ok_or_else(|| ManagedExceptionError::overflow().into()),
            (StackValue::Int64(l), StackValue::Int64(r), NumberSign::Unsigned) => (l as u64)
                .$op(r as u64)
                .map(|v| StackValue::Int64(v as i64))
                .ok_or_else(|| ManagedExceptionError::overflow().into()),
            (StackValue::NativeInt(l), StackValue::NativeInt(r), NumberSign::Signed) => l
                .$op(r)
                .map(StackValue::NativeInt)
                .ok_or_else(|| ManagedExceptionError::overflow().into()),
            (StackValue::NativeInt(l), StackValue::NativeInt(r), NumberSign::Unsigned) => (l
                as usize)
                .$op(r as usize)
                .map(|v| StackValue::NativeInt(v as isize))
                .ok_or_else(|| ManagedExceptionError::overflow().into()),
            // Mixed types (Int32 <-> NativeInt)
            (StackValue::NativeInt(l), StackValue::Int32(r), NumberSign::Signed) => l
                .$op(r as isize)
                .map(StackValue::NativeInt)
                .ok_or_else(|| ManagedExceptionError::overflow().into()),
            (StackValue::NativeInt(l), StackValue::Int32(r), NumberSign::Unsigned) => (l as usize)
                .$op((r as u32) as usize)
                .map(|v| StackValue::NativeInt(v as isize))
                .ok_or_else(|| ManagedExceptionError::overflow().into()),
            (StackValue::Int32(l), StackValue::NativeInt(r), NumberSign::Signed) => (l as isize)
                .$op(r)
                .map(StackValue::NativeInt)
                .ok_or_else(|| ManagedExceptionError::overflow().into()),
            (StackValue::Int32(l), StackValue::NativeInt(r), NumberSign::Unsigned) => ((l as u32)
                as usize)
                .$op(r as usize)
                .map(|v| StackValue::NativeInt(v as isize))
                .ok_or_else(|| ManagedExceptionError::overflow().into()),
            (l, r, _) => Err(ExecutionError::TypeMismatch {
                expected: "checked arithmetic operands (Int32, Int64, NativeInt)".to_string(),
                actual: format!("{}, {}", stack_value_kind(&l), stack_value_kind(&r)),
            }
            .into()),
        }
    };
}

macro_rules! arithmetic_op {
    ($l:expr, $r:expr, $sgn:expr, $op:tt) => {
        match ($l, $r, $sgn) {
            (StackValue::Int32(l), StackValue::Int32(r), NumberSign::Signed) => {
                Ok(StackValue::Int32(l $op r))
            }
            (StackValue::Int32(l), StackValue::Int32(r), NumberSign::Unsigned) => {
                Ok(StackValue::Int32(((l as u32) $op (r as u32)) as i32))
            }
            (StackValue::Int64(l), StackValue::Int64(r), NumberSign::Signed) => {
                Ok(StackValue::Int64(l $op r))
            }
            (StackValue::Int64(l), StackValue::Int64(r), NumberSign::Unsigned) => {
                Ok(StackValue::Int64(((l as u64) $op (r as u64)) as i64))
            }
            (StackValue::NativeInt(l), StackValue::NativeInt(r), NumberSign::Signed) => {
                Ok(StackValue::NativeInt(l $op r))
            }
            (StackValue::NativeInt(l), StackValue::NativeInt(r), NumberSign::Unsigned) => {
                Ok(StackValue::NativeInt(((l as usize) $op (r as usize)) as isize))
            }
            (StackValue::NativeFloat(l), StackValue::NativeFloat(r), _) => {
                Ok(StackValue::NativeFloat(l $op r))
            }
            // Mixed types (Int32 <-> NativeInt)
            (StackValue::NativeInt(l), StackValue::Int32(r), NumberSign::Signed) => {
                Ok(StackValue::NativeInt(l $op (r as isize)))
            }
            (StackValue::NativeInt(l), StackValue::Int32(r), NumberSign::Unsigned) => {
                Ok(StackValue::NativeInt(
                    ((l as usize) $op ((r as u32) as usize)) as isize,
                ))
            }
            (StackValue::Int32(l), StackValue::NativeInt(r), NumberSign::Signed) => {
                Ok(StackValue::NativeInt((l as isize) $op r))
            }
            (StackValue::Int32(l), StackValue::NativeInt(r), NumberSign::Unsigned) => {
                Ok(StackValue::NativeInt(
                    (((l as u32) as usize) $op (r as usize)) as isize,
                ))
            }
            (l, r, _) => Err(ExecutionError::TypeMismatch {
                expected: "arithmetic operands (Int32, Int64, NativeInt, NativeFloat)".to_string(),
                actual: format!("{}, {}", stack_value_kind(&l), stack_value_kind(&r)),
            }),
        }
    };
}

macro_rules! shift_op {
    ($target:expr, $amount:expr, $sgn:expr, $op:tt) => {
        match ($target, $sgn) {
            (StackValue::Int32(i), NumberSign::Signed) => Ok(StackValue::Int32(i $op $amount)),
            (StackValue::Int32(i), NumberSign::Unsigned) => {
                Ok(StackValue::Int32(((i as u32) $op $amount) as i32))
            }
            (StackValue::Int64(i), NumberSign::Signed) => Ok(StackValue::Int64(i $op $amount)),
            (StackValue::Int64(i), NumberSign::Unsigned) => {
                Ok(StackValue::Int64(((i as u64) $op $amount) as i64))
            }
            (StackValue::NativeInt(i), NumberSign::Signed) => {
                Ok(StackValue::NativeInt(i $op $amount))
            }
            (StackValue::NativeInt(i), NumberSign::Unsigned) => {
                Ok(StackValue::NativeInt(((i as usize) $op $amount) as isize))
            }
            (v, _) => Err(ExecutionError::TypeMismatch {
                expected: "shift target (Int32, Int64, NativeInt)".to_string(),
                actual: stack_value_kind(&v).to_string(),
            }),
        }
    };
}

macro_rules! bitwise_op {
    ($l:expr, $r:expr, $op:tt) => {
        match ($l, $r) {
            (StackValue::Int32(l), StackValue::Int32(r)) => Ok(StackValue::Int32(l $op r)),
            (StackValue::Int32(l), StackValue::NativeInt(r)) => {
                Ok(StackValue::NativeInt((l as isize) $op r))
            }
            (StackValue::Int64(l), StackValue::Int64(r)) => Ok(StackValue::Int64(l $op r)),
            (StackValue::Int64(l), StackValue::NativeInt(r)) => {
                Ok(StackValue::Int64(l $op (r as i64)))
            }
            (StackValue::NativeInt(l), StackValue::Int32(r)) => {
                Ok(StackValue::NativeInt(l $op (r as isize)))
            }
            (StackValue::NativeInt(l), StackValue::Int64(r)) => Ok(StackValue::Int64((l as i64) $op r)),
            (StackValue::NativeInt(l), StackValue::NativeInt(r)) => Ok(StackValue::NativeInt(l $op r)),
            (l, r) => Err(ExecutionError::TypeMismatch {
                expected: "bitwise operands (Int32, Int64, NativeInt)".to_string(),
                actual: format!("{}, {}", stack_value_kind(&l), stack_value_kind(&r)),
            }),
        }
    };
}

macro_rules! wrapping_arithmetic_op {
    ($l:expr, $r:expr, $op:ident, $float_op:tt) => {
        match ($l, $r) {
            (StackValue::Int32(l), StackValue::Int32(r)) => Ok(StackValue::Int32(l.$op(r))),
            (StackValue::Int32(l), StackValue::NativeInt(r)) => {
                Ok(StackValue::NativeInt((l as isize).$op(r)))
            }
            (StackValue::Int64(l), StackValue::Int64(r)) => Ok(StackValue::Int64(l.$op(r))),
            (StackValue::Int64(l), StackValue::NativeInt(r)) => {
                Ok(StackValue::Int64(l.$op(r as i64)))
            }
            (StackValue::NativeInt(l), StackValue::Int32(r)) => {
                Ok(StackValue::NativeInt(l.$op(r as isize)))
            }
            (StackValue::NativeInt(l), StackValue::Int64(r)) => {
                Ok(StackValue::Int64((l as i64).$op(r)))
            }
            (StackValue::NativeInt(l), StackValue::NativeInt(r)) => {
                Ok(StackValue::NativeInt(l.$op(r)))
            }
            (StackValue::NativeFloat(l), StackValue::NativeFloat(r)) => {
                Ok(StackValue::NativeFloat(l $float_op r))
            }
            (StackValue::ObjectRef(l), StackValue::NativeInt(r)) => {
                let ptr = l.0.map(|p| Gc::as_ptr(p) as isize).unwrap_or(0);
                Ok(StackValue::NativeInt(ptr.$op(r)))
            }
            (StackValue::ObjectRef(l), StackValue::Int32(r)) => {
                let ptr = l.0.map(|p| Gc::as_ptr(p) as isize).unwrap_or(0);
                Ok(StackValue::NativeInt(ptr.$op(r as isize)))
            }
            (StackValue::ObjectRef(l), StackValue::ObjectRef(r)) => {
                let ptr_l = l.0.map(|p| Gc::as_ptr(p) as isize).unwrap_or(0);
                let ptr_r = r.0.map(|p| Gc::as_ptr(p) as isize).unwrap_or(0);
                Ok(StackValue::NativeInt(ptr_l.$op(ptr_r)))
            }
            (l, r) => Err(ExecutionError::TypeMismatch {
                expected: "arithmetic operands (Int32, Int64, NativeInt, NativeFloat, ObjectRef+nint)"
                    .to_string(),
                actual: format!("{}, {}", stack_value_kind(&l), stack_value_kind(&r)),
            }),
        }
    };
}

impl<'gc> StackValue<'gc> {
    pub fn unmanaged_ptr(ptr: *mut u8) -> Self {
        match NonNull::new(ptr) {
            Some(p) => Self::UnmanagedPtr(UnmanagedPtr(p)),
            None => Self::NativeInt(0),
        }
    }
    pub fn managed_ptr(
        ptr: *mut u8,
        target_type: TypeDescription,
        pinned: bool,
        offset: Option<ByteOffset>,
    ) -> Self {
        Self::managed_ptr_with_owner(ptr, target_type, None, pinned, offset)
    }

    pub fn managed_ptr_with_owner(
        ptr: *mut u8,
        target_type: TypeDescription,
        owner: Option<ObjectRef<'gc>>,
        pinned: bool,
        offset: Option<ByteOffset>,
    ) -> Self {
        Self::ManagedPtr(StackManagedPtr::new(ManagedPtr::new(
            NonNull::new(ptr),
            target_type,
            owner,
            pinned,
            offset,
        )))
    }
    pub fn managed_stack_ptr(
        index: StackSlotIndex,
        offset: ByteOffset,
        ptr: *mut u8,
        target_type: TypeDescription,
        pinned: bool,
    ) -> Self {
        let mut m = ManagedPtr::new(NonNull::new(ptr), target_type, None, pinned, Some(offset));
        m.origin = PointerOrigin::Stack(index);
        Self::ManagedPtr(StackManagedPtr::new(m))
    }
    pub fn null() -> Self {
        Self::ObjectRef(ObjectRef(None))
    }

    pub fn size_bytes(&self) -> usize {
        match self {
            Self::ValueType(v) => v.size_bytes(),
            _ => size_of::<StackValue>(),
        }
    }

    pub fn string(gc: GCHandle<'gc>, s: CLRString) -> Self {
        Self::ObjectRef(ObjectRef::new(gc, HeapStorage::Str(s)))
    }

    pub fn data_location(&self) -> NonNull<u8> {
        fn ref_to_ptr<T>(r: &T) -> NonNull<u8> {
            NonNull::from(r).cast()
        }

        match self {
            Self::Int32(i) => ref_to_ptr(i),
            Self::Int64(i) => ref_to_ptr(i),
            Self::NativeInt(i) => ref_to_ptr(i),
            Self::NativeFloat(f) => ref_to_ptr(f),
            Self::ObjectRef(o) => ref_to_ptr(o),
            Self::UnmanagedPtr(u) => ref_to_ptr(u),
            Self::ManagedPtr(m) => ref_to_ptr(m.deref()),
            Self::TypedRef(m, _) => ref_to_ptr(m.deref()),
            Self::ValueType(o) => {
                // SAFETY: Returning a pointer to the internal buffer.
                // This is used by ldloca/ldarga to get a byref to the value type.
                let ptr =
                    unsafe { o.instance_storage.raw_data_unsynchronized().as_ptr() as *mut u8 };
                NonNull::new(ptr).unwrap()
            }
            #[cfg(feature = "multithreading")]
            Self::CrossArenaObjectRef(p, _) => ref_to_ptr(p),
        }
    }

    pub fn as_ptr(&self) -> *mut u8 {
        match self {
            Self::NativeInt(i) => std::ptr::with_exposed_provenance_mut(*i as usize),
            Self::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
            Self::ManagedPtr(m) | Self::TypedRef(m, _) => {
                if m.is_null() {
                    std::ptr::null_mut()
                } else {
                    unsafe { m.with_data(0, |data| data.as_ptr() as *mut u8) }
                }
            }
            v => panic!("expected pointer on stack, received {:?}", v),
        }
    }

    pub fn try_as_i32(&self) -> Result<i32, ExecutionError> {
        match self {
            Self::Int32(i) => Ok(*i),
            Self::NativeInt(i) => Ok(*i as i32),
            v => Err(ExecutionError::TypeMismatch {
                expected: "Int32".to_string(),
                actual: stack_value_kind(v).to_string(),
            }),
        }
    }

    pub fn as_i32(&self) -> i32 {
        match self {
            Self::Int32(i) => *i,
            Self::NativeInt(i) => *i as i32,
            v => panic!("expected Int32, received {:?}", v),
        }
    }

    pub fn try_as_i64(&self) -> Result<i64, ExecutionError> {
        match self {
            Self::Int64(i) => Ok(*i),
            v => Err(ExecutionError::TypeMismatch {
                expected: "Int64".to_string(),
                actual: stack_value_kind(v).to_string(),
            }),
        }
    }

    pub fn as_i64(&self) -> i64 {
        match self {
            Self::Int64(i) => *i,
            v => panic!("expected Int64, received {:?}", v),
        }
    }

    pub fn try_as_f64(&self) -> Result<f64, ExecutionError> {
        match self {
            Self::NativeFloat(f) => Ok(*f),
            v => Err(ExecutionError::TypeMismatch {
                expected: "NativeFloat".to_string(),
                actual: stack_value_kind(v).to_string(),
            }),
        }
    }

    pub fn as_f64(&self) -> f64 {
        match self {
            Self::NativeFloat(f) => *f,
            v => panic!("expected NativeFloat, received {:?}", v),
        }
    }

    pub fn try_as_isize(&self) -> Result<isize, ExecutionError> {
        match self {
            Self::NativeInt(i) => Ok(*i),
            Self::Int32(i) => Ok(*i as isize),
            v => Err(ExecutionError::TypeMismatch {
                expected: "NativeInt".to_string(),
                actual: stack_value_kind(v).to_string(),
            }),
        }
    }

    pub fn as_isize(&self) -> isize {
        match self {
            Self::NativeInt(i) => *i,
            Self::Int32(i) => *i as isize,
            v => panic!("expected NativeInt, received {:?}", v),
        }
    }

    pub fn try_as_object_ref(&self) -> Result<ObjectRef<'gc>, ExecutionError> {
        match self {
            Self::ObjectRef(o) => Ok(*o),
            v => Err(ExecutionError::TypeMismatch {
                expected: "ObjectRef".to_string(),
                actual: stack_value_kind(v).to_string(),
            }),
        }
    }

    pub fn as_object_ref(&self) -> ObjectRef<'gc> {
        match self {
            Self::ObjectRef(o) => *o,
            v => panic!("expected ObjectRef, received {:?}", v),
        }
    }

    pub fn try_as_managed_ptr(&self) -> Result<ManagedPtr<'gc>, ExecutionError> {
        match self {
            Self::ManagedPtr(m) => Ok(m.deref().clone()),
            v => Err(ExecutionError::TypeMismatch {
                expected: "ManagedPtr".to_string(),
                actual: stack_value_kind(v).to_string(),
            }),
        }
    }

    pub fn as_managed_ptr(&self) -> ManagedPtr<'gc> {
        match self {
            Self::ManagedPtr(m) => m.deref().clone(),
            v => panic!("expected ManagedPtr, received {:?}", v),
        }
    }

    pub fn is_managed_ptr(&self) -> bool {
        matches!(self, Self::ManagedPtr(_))
    }

    pub fn try_as_value_type(&self) -> Result<Object<'gc>, ExecutionError> {
        match self {
            Self::ValueType(v) => Ok(v.clone()),
            v => Err(ExecutionError::TypeMismatch {
                expected: "ValueType".to_string(),
                actual: stack_value_kind(v).to_string(),
            }),
        }
    }

    pub fn as_value_type(&self) -> Object<'gc> {
        match self {
            Self::ValueType(v) => v.clone(),
            v => panic!("expected ValueType, received {:?}", v),
        }
    }

    pub fn is_zero(&self) -> bool {
        matches!(self, Self::Int32(0) | Self::Int64(0) | Self::NativeInt(0))
    }

    pub fn is_null(&self) -> bool {
        match self {
            Self::ObjectRef(o) => o.0.is_none(),
            Self::ManagedPtr(m) => m.is_null(),
            Self::UnmanagedPtr(_) => false,
            _ => self.is_zero(),
        }
    }

    pub fn shr(self, other: Self, sgn: NumberSign) -> Result<Self, ExecutionError> {
        let amount = extract_shift_amount(other)?;
        shift_op!(self, amount, sgn, >>)
    }

    pub fn div(self, other: Self, sgn: NumberSign) -> Result<Self, StackValueError> {
        if other.is_zero() {
            return Err(ManagedExceptionError::divide_by_zero().into());
        }
        arithmetic_op!(self, other, sgn, /).map_err(StackValueError::from)
    }

    pub fn rem(self, other: Self, sgn: NumberSign) -> Result<Self, StackValueError> {
        if other.is_zero() {
            return Err(ManagedExceptionError::divide_by_zero().into());
        }
        arithmetic_op!(self, other, sgn, %).map_err(StackValueError::from)
    }

    pub fn compare(&self, other: &Self, sgn: NumberSign) -> Option<Ordering> {
        use NumberSign::*;
        use StackValue::*;
        match (self, other, sgn) {
            (Int32(l), Int32(r), Unsigned) => (*l as u32).partial_cmp(&(*r as u32)),
            (Int32(l), NativeInt(r), Unsigned) => (*l as u32 as usize).partial_cmp(&(*r as usize)),
            (NativeInt(l), Int32(r), Unsigned) => (*l as usize).partial_cmp(&(*r as u32 as usize)),
            (NativeInt(l), NativeInt(r), Unsigned) => (*l as usize).partial_cmp(&(*r as usize)),
            (Int64(l), Int64(r), Unsigned) => (*l as u64).partial_cmp(&(*r as u64)),
            _ => self.partial_cmp(other),
        }
    }

    pub fn checked_add(self, other: Self, sgn: NumberSign) -> Result<Self, StackValueError> {
        checked_arithmetic_op!(self, other, sgn, checked_add)
    }

    pub fn checked_sub(self, other: Self, sgn: NumberSign) -> Result<Self, StackValueError> {
        checked_arithmetic_op!(self, other, sgn, checked_sub)
    }

    pub fn checked_mul(self, other: Self, sgn: NumberSign) -> Result<Self, StackValueError> {
        checked_arithmetic_op!(self, other, sgn, checked_mul)
    }

    /// # Safety
    /// `ptr` must be a valid, aligned pointer to a value of the type specified by `t`.
    pub unsafe fn load(ptr: *const u8, t: LoadType) -> Self {
        unsafe { Self::load_atomic(ptr, t, AtomicOrdering::Relaxed) }
    }

    /// # Safety
    /// `ptr` must be a valid, aligned pointer to a value of the type specified by `t`.
    /// Loads a value from a raw pointer using atomic operations.
    /// Note: This uses `AtomicT::from_ptr` which is supported in recent Rust versions.
    /// Also, it does not ensure that the appropriate locks are held for the memory being accessed.
    pub unsafe fn load_atomic(ptr: *const u8, t: LoadType, ordering: AtomicOrdering) -> Self {
        debug_assert!(!ptr.is_null(), "Attempted to load from a null pointer");
        let alignment = load_type_alignment(t);
        validate_alignment(ptr, alignment);

        let size = match t {
            LoadType::Int8 | LoadType::UInt8 => 1,
            LoadType::Int16 | LoadType::UInt16 => 2,
            LoadType::Int32 | LoadType::UInt32 | LoadType::Float32 => 4,
            LoadType::Int64 | LoadType::Float64 => 8,
            LoadType::IntPtr => size_of::<isize>(),
            LoadType::Object => ObjectRef::SIZE,
        };

        let val = unsafe { StandardAtomicAccess::load_atomic(ptr, size, ordering) };

        match t {
            LoadType::Int8 => Self::Int32(val as i8 as i32),
            LoadType::UInt8 => Self::Int32(val as u8 as i32),
            LoadType::Int16 => Self::Int32(val as i16 as i32),
            LoadType::UInt16 => Self::Int32(val as u16 as i32),
            LoadType::Int32 | LoadType::UInt32 => Self::Int32(val as i32),
            LoadType::Int64 => Self::Int64(val as i64),
            LoadType::Float32 => Self::NativeFloat(f32::from_bits(val as u32) as f64),
            LoadType::Float64 => Self::NativeFloat(f64::from_bits(val)),
            LoadType::IntPtr => Self::NativeInt(val as isize),
            LoadType::Object => {
                let ptr = std::ptr::with_exposed_provenance::<ThreadSafeLock<object::ObjectInner<'gc>>>(
                    val as usize,
                );
                let obj = if ptr.is_null() {
                    None
                } else {
                    Some(unsafe { Gc::from_ptr(ptr) })
                };
                Self::ObjectRef(ObjectRef(obj))
            }
        }
    }

    /// # Safety
    /// `ptr` must be a valid, aligned pointer to a location with sufficient space for the type specified by `t`.
    pub unsafe fn store(self, ptr: *mut u8, t: StoreType) {
        unsafe {
            self.store_atomic(ptr, t, AtomicOrdering::Relaxed);
        }
    }

    /// # Safety
    /// `ptr` must be a valid, aligned pointer to a location with sufficient space for the type specified by `t`.
    /// Stores a value to a raw pointer using atomic operations.
    /// Note: This uses `AtomicT::from_ptr` which is supported in recent Rust versions.
    /// Also, it does not ensure that the appropriate locks are held for the memory being accessed.
    pub unsafe fn store_atomic(self, ptr: *mut u8, t: StoreType, ordering: AtomicOrdering) {
        debug_assert!(!ptr.is_null(), "Attempted to store to a null pointer");
        let alignment = store_type_alignment(t);
        validate_alignment(ptr, alignment);

        let (val, size) = match t {
            StoreType::Int8 => (self.as_i32() as u64, 1),
            StoreType::Int16 => (self.as_i32() as u64, 2),
            StoreType::Int32 => (self.as_i32() as u64, 4),
            StoreType::Int64 => (self.as_i64() as u64, 8),
            StoreType::Float32 => ((self.as_f64() as f32).to_bits() as u64, 4),
            StoreType::Float64 => (self.as_f64().to_bits(), 8),
            StoreType::IntPtr => (self.as_isize() as u64, size_of::<isize>()),
            StoreType::Object => {
                let obj = self.as_object_ref();
                let val = match obj.0 {
                    Some(h) => Gc::as_ptr(h).expose_provenance(),
                    None => 0,
                };
                (val as u64, ObjectRef::SIZE)
            }
        };

        unsafe {
            StandardAtomicAccess::store_atomic(ptr, size, val, ordering);
        }
    }
}

const fn load_type_alignment(t: LoadType) -> usize {
    match t {
        LoadType::Int8 | LoadType::UInt8 => 1,
        LoadType::Int16 | LoadType::UInt16 => align_of::<i16>(),
        LoadType::Int32 | LoadType::UInt32 => align_of::<i32>(),
        LoadType::Int64 => align_of::<i64>(),
        LoadType::Float32 => align_of::<f32>(),
        LoadType::Float64 => align_of::<f64>(),
        LoadType::IntPtr => align_of::<isize>(),
        LoadType::Object => align_of::<ObjectRef>(),
    }
}

const fn store_type_alignment(t: StoreType) -> usize {
    match t {
        StoreType::Int8 => 1,
        StoreType::Int16 => align_of::<i16>(),
        StoreType::Int32 => align_of::<i32>(),
        StoreType::Int64 => align_of::<i64>(),
        StoreType::Float32 => align_of::<f32>(),
        StoreType::Float64 => align_of::<f64>(),
        StoreType::IntPtr => align_of::<isize>(),
        StoreType::Object => align_of::<ObjectRef>(),
    }
}
impl Default for StackValue<'_> {
    fn default() -> Self {
        Self::null()
    }
}
impl<'gc> PartialEq for StackValue<'gc> {
    fn eq(&self, other: &Self) -> bool {
        use StackValue::*;
        match (self, other) {
            (Int32(l), Int32(r)) => l == r,
            (Int32(l), NativeInt(r)) => (*l as isize) == *r,
            (Int64(l), Int64(r)) => l == r,
            (NativeInt(l), Int32(r)) => *l == (*r as isize),
            (NativeInt(l), NativeInt(r)) => l == r,
            (NativeFloat(l), NativeFloat(r)) => l == r,
            (ManagedPtr(l), ManagedPtr(r)) => l == r,
            (ManagedPtr(l), NativeInt(r)) => {
                let l_ptr = managed_ptr_to_raw(l);
                (l_ptr as isize) == *r
            }
            (NativeInt(l), ManagedPtr(r)) => {
                let r_ptr = managed_ptr_to_raw(r);
                *l == (r_ptr as isize)
            }
            (ObjectRef(l), ObjectRef(r)) => l == r,
            (ValueType(l), ValueType(r)) => l == r,
            _ => false,
        }
    }
}

impl<'gc> PartialOrd for StackValue<'gc> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        use StackValue::*;
        match (self, other) {
            (Int32(l), Int32(r)) => l.partial_cmp(r),
            (Int32(l), NativeInt(r)) => (*l as isize).partial_cmp(r),
            (Int64(l), Int64(r)) => l.partial_cmp(r),
            (NativeInt(l), Int32(r)) => l.partial_cmp(&(*r as isize)),
            (NativeInt(l), NativeInt(r)) => l.partial_cmp(r),
            (NativeFloat(l), NativeFloat(r)) => l.partial_cmp(r),
            (ManagedPtr(l), ManagedPtr(r)) => l.partial_cmp(r),
            (ManagedPtr(l), NativeInt(r)) => {
                let l_ptr = managed_ptr_to_raw(l);
                (l_ptr as isize).partial_cmp(r)
            }
            (NativeInt(l), ManagedPtr(r)) => {
                let r_ptr = managed_ptr_to_raw(r);
                l.partial_cmp(&(r_ptr as isize))
            }
            (ObjectRef(l), ObjectRef(r)) => l.partial_cmp(r),
            _ => None,
        }
    }
}

impl<'gc> Add for StackValue<'gc> {
    type Output = Result<Self, ExecutionError>;
    fn add(self, rhs: Self) -> Self::Output {
        use StackValue::*;
        match (self, rhs) {
            (Int32(i), ManagedPtr(m)) | (ManagedPtr(m), Int32(i)) => {
                // SAFETY: Pointer arithmetic is performed within the bounds of the managed object
                // or stack slot it points to. The VM ensures that pointers stay within allocated regions.
                // ManagedPtr::offset is an unsafe method that requires the resulting pointer to be within bounds.
                Ok(unsafe { ManagedPtr(StackManagedPtr::new(m.into_inner().offset(i as isize))) })
            }
            (NativeInt(i), ManagedPtr(m)) | (ManagedPtr(m), NativeInt(i)) => {
                // SAFETY: Pointer arithmetic is performed within the bounds of the managed object.
                // ManagedPtr::offset is an unsafe method that requires the resulting pointer to be within bounds.
                Ok(unsafe { ManagedPtr(StackManagedPtr::new(m.into_inner().offset(i))) })
            }
            (Int32(i), UnmanagedPtr(u)) | (UnmanagedPtr(u), Int32(i)) => {
                Ok(UnmanagedPtr(pointer::UnmanagedPtr(unsafe {
                    NonNull::new_unchecked(u.0.as_ptr().offset(i as isize))
                })))
            }
            (NativeInt(i), UnmanagedPtr(u)) | (UnmanagedPtr(u), NativeInt(i)) => {
                Ok(UnmanagedPtr(pointer::UnmanagedPtr(unsafe {
                    NonNull::new_unchecked(u.0.as_ptr().offset(i))
                })))
            }
            (l, r) => wrapping_arithmetic_op!(l, r, wrapping_add, +),
        }
    }
}

impl<'gc> Sub for StackValue<'gc> {
    type Output = Result<Self, ExecutionError>;
    fn sub(self, rhs: Self) -> Self::Output {
        use StackValue::*;
        match (self, rhs) {
            (ManagedPtr(m), Int32(i)) => {
                // SAFETY: Pointer arithmetic is performed within the bounds of the managed object.
                // ManagedPtr::offset is an unsafe method that requires the resulting pointer to be within bounds.
                Ok(unsafe {
                    ManagedPtr(StackManagedPtr::new(m.into_inner().offset(-(i as isize))))
                })
            }
            (ManagedPtr(m), NativeInt(i)) => {
                // SAFETY: Pointer arithmetic is performed within the bounds of the managed object.
                // ManagedPtr::offset is an unsafe method that requires the resulting pointer to be within bounds.
                Ok(unsafe { ManagedPtr(StackManagedPtr::new(m.into_inner().offset(-i))) })
            }
            (ManagedPtr(m), Int64(i)) => {
                // SAFETY: Pointer arithmetic is performed within the bounds of the managed object.
                // ManagedPtr::offset is an unsafe method that requires the resulting pointer to be within bounds.
                Ok(unsafe {
                    ManagedPtr(StackManagedPtr::new(m.into_inner().offset(-(i as isize))))
                })
            }
            (UnmanagedPtr(u), Int32(i)) => Ok(UnmanagedPtr(pointer::UnmanagedPtr(unsafe {
                NonNull::new_unchecked(u.0.as_ptr().offset(-(i as isize)))
            }))),
            (UnmanagedPtr(u), NativeInt(i)) => Ok(UnmanagedPtr(pointer::UnmanagedPtr(unsafe {
                NonNull::new_unchecked(u.0.as_ptr().offset(-i))
            }))),
            (ManagedPtr(m1), ManagedPtr(m2)) => {
                let v1 = managed_ptr_to_raw(&m1);
                let v2 = managed_ptr_to_raw(&m2);
                Ok(NativeInt((v1 as isize) - (v2 as isize)))
            }
            (l, r) => wrapping_arithmetic_op!(l, r, wrapping_sub, -),
        }
    }
}

impl<'gc> Mul for StackValue<'gc> {
    type Output = Result<Self, ExecutionError>;
    fn mul(self, rhs: Self) -> Self::Output {
        wrapping_arithmetic_op!(self, rhs, wrapping_mul, *)
    }
}

impl<'gc> BitAnd for StackValue<'gc> {
    type Output = Result<Self, ExecutionError>;
    fn bitand(self, rhs: Self) -> Self::Output {
        bitwise_op!(self, rhs, &)
    }
}

impl<'gc> BitOr for StackValue<'gc> {
    type Output = Result<Self, ExecutionError>;
    fn bitor(self, rhs: Self) -> Self::Output {
        bitwise_op!(self, rhs, |)
    }
}

impl<'gc> BitXor for StackValue<'gc> {
    type Output = Result<Self, ExecutionError>;
    fn bitxor(self, rhs: Self) -> Self::Output {
        bitwise_op!(self, rhs, ^)
    }
}

impl<'gc> Shl for StackValue<'gc> {
    type Output = Result<Self, ExecutionError>;
    fn shl(self, rhs: Self) -> Self::Output {
        let amount = extract_shift_amount(rhs)?;
        match self {
            StackValue::Int32(i) => Ok(StackValue::Int32(i << amount)),
            StackValue::Int64(i) => Ok(StackValue::Int64(i << amount)),
            StackValue::NativeInt(i) => Ok(StackValue::NativeInt(i << amount)),
            v => Err(ExecutionError::TypeMismatch {
                expected: "shift target (Int32, Int64, NativeInt)".to_string(),
                actual: stack_value_kind(&v).to_string(),
            }),
        }
    }
}

impl<'gc> Not for StackValue<'gc> {
    type Output = Result<Self, ExecutionError>;
    fn not(self) -> Self::Output {
        use StackValue::*;
        match self {
            Int32(i) => Ok(Int32(!i)),
            Int64(i) => Ok(Int64(!i)),
            NativeInt(i) => Ok(NativeInt(!i)),
            v => Err(ExecutionError::TypeMismatch {
                expected: "not operand (Int32, Int64, NativeInt)".to_string(),
                actual: stack_value_kind(&v).to_string(),
            }),
        }
    }
}

impl<'gc> Neg for StackValue<'gc> {
    type Output = Result<Self, ExecutionError>;
    fn neg(self) -> Self::Output {
        use StackValue::*;
        match self {
            Int32(i) => Ok(Int32(-i)),
            Int64(i) => Ok(Int64(-i)),
            NativeInt(i) => Ok(NativeInt(-i)),
            NativeFloat(f) => Ok(NativeFloat(-f)),
            v => Err(ExecutionError::TypeMismatch {
                expected: "neg operand (Int32, Int64, NativeInt, NativeFloat)".to_string(),
                actual: stack_value_kind(&v).to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stack_value_arithmetic() {
        let v1 = StackValue::Int32(10);
        let v2 = StackValue::Int32(20);

        let sum = (v1.clone() + v2.clone()).unwrap();
        assert_eq!(sum, StackValue::Int32(30));

        let v3 = StackValue::Int32(30);
        let v4 = StackValue::Int32(10);
        let diff = (v3 - v4).unwrap();
        assert_eq!(diff, StackValue::Int32(20));

        // Test division by zero
        let v_zero = StackValue::Int32(0);
        let res = v1.div(v_zero, NumberSign::Signed);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(
            err,
            StackValueError::ManagedException(ManagedExceptionError::divide_by_zero())
        );

        // Test checked overflow
        let v_max = StackValue::Int32(i32::MAX);
        let v_one = StackValue::Int32(1);
        let res = v_max.checked_add(v_one, NumberSign::Signed);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(
            err,
            StackValueError::ManagedException(ManagedExceptionError::overflow())
        );
    }
}
