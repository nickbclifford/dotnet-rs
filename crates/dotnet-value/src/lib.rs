#[cfg(feature = "multithreaded-gc")]
use dotnet_utils::gc::record_cross_arena_ref;

use dotnet_types::TypeDescription;
use dotnet_utils::{
    atomic::{AtomicAccess, StandardAtomicAccess},
    gc::{GCHandle, ThreadSafeLock},
    sync::Ordering as AtomicOrdering,
};
use dotnetdll::prelude::*;
use gc_arena::{Collect, Collection, Gc};
use std::{
    cmp::Ordering,
    fmt::Debug,
    ops::{Add, BitAnd, BitOr, BitXor, Mul, Neg, Not, Shl, Sub},
    ptr::NonNull,
};

#[cfg(test)]
mod atomic_tests;
pub mod layout;
pub mod object;
#[cfg(test)]
mod object_tests;
pub mod pointer;
pub mod storage;
pub mod string;

pub use object::{HeapStorage, Object, ObjectRef};
pub use pointer::{ManagedPtr, UnmanagedPtr};
pub use string::CLRString;

#[cfg(feature = "multithreaded-gc")]
use object::ObjectPtr;

#[derive(Clone, Debug)]
pub enum StackValue<'gc> {
    Int32(i32),
    Int64(i64),
    NativeInt(isize),
    NativeFloat(f64),
    ObjectRef(ObjectRef<'gc>),
    UnmanagedPtr(UnmanagedPtr),
    ManagedPtr(ManagedPtr<'gc>),
    ValueType(Object<'gc>),
    /// Reference to an object in another thread's arena.
    /// (ObjectPtr, OwningThreadID)
    #[cfg(feature = "multithreaded-gc")]
    CrossArenaObjectRef(ObjectPtr, u64),
}

unsafe impl<'gc> Collect for StackValue<'gc> {
    fn trace(&self, cc: &Collection) {
        match self {
            Self::ObjectRef(o) => o.trace(cc),
            Self::ManagedPtr(m) => m.trace(cc),
            Self::ValueType(v) => v.trace(cc),
            #[cfg(feature = "multithreaded-gc")]
            Self::CrossArenaObjectRef(ptr, tid) => {
                record_cross_arena_ref(*tid, ptr.as_ptr() as usize);
            }
            _ => {}
        }
    }
}
macro_rules! checked_arithmetic_op {
    ($l:expr, $r:expr, $sgn:expr, $op:ident) => {
        match ($l, $r, $sgn) {
            (StackValue::Int32(l), StackValue::Int32(r), NumberSign::Signed) => l
                .$op(r)
                .map(StackValue::Int32)
                .ok_or("System.OverflowException"),
            (StackValue::Int32(l), StackValue::Int32(r), NumberSign::Unsigned) => (l as u32)
                .$op(r as u32)
                .map(|v| StackValue::Int32(v as i32))
                .ok_or("System.OverflowException"),
            (StackValue::Int64(l), StackValue::Int64(r), NumberSign::Signed) => l
                .$op(r)
                .map(StackValue::Int64)
                .ok_or("System.OverflowException"),
            (StackValue::Int64(l), StackValue::Int64(r), NumberSign::Unsigned) => (l as u64)
                .$op(r as u64)
                .map(|v| StackValue::Int64(v as i64))
                .ok_or("System.OverflowException"),
            (StackValue::NativeInt(l), StackValue::NativeInt(r), NumberSign::Signed) => l
                .$op(r)
                .map(StackValue::NativeInt)
                .ok_or("System.OverflowException"),
            (StackValue::NativeInt(l), StackValue::NativeInt(r), NumberSign::Unsigned) => (l
                as usize)
                .$op(r as usize)
                .map(|v| StackValue::NativeInt(v as isize))
                .ok_or("System.OverflowException"),
            // Mixed types (Int32 <-> NativeInt)
            (StackValue::NativeInt(l), StackValue::Int32(r), NumberSign::Signed) => l
                .$op(r as isize)
                .map(StackValue::NativeInt)
                .ok_or("System.OverflowException"),
            (StackValue::NativeInt(l), StackValue::Int32(r), NumberSign::Unsigned) => (l as usize)
                .$op((r as u32) as usize)
                .map(|v| StackValue::NativeInt(v as isize))
                .ok_or("System.OverflowException"),
            (StackValue::Int32(l), StackValue::NativeInt(r), NumberSign::Signed) => (l as isize)
                .$op(r)
                .map(StackValue::NativeInt)
                .ok_or("System.OverflowException"),
            (StackValue::Int32(l), StackValue::NativeInt(r), NumberSign::Unsigned) => ((l as u32)
                as usize)
                .$op(r as usize)
                .map(|v| StackValue::NativeInt(v as isize))
                .ok_or("System.OverflowException"),
            (l, r, _) => panic!("invalid types for checked operation: {:?}, {:?}", l, r),
        }
    };
}

macro_rules! arithmetic_op {
    ($l:expr, $r:expr, $sgn:expr, $op:tt) => {
        match ($l, $r, $sgn) {
            (StackValue::Int32(l), StackValue::Int32(r), NumberSign::Signed) => StackValue::Int32(l $op r),
            (StackValue::Int32(l), StackValue::Int32(r), NumberSign::Unsigned) => StackValue::Int32(((l as u32) $op (r as u32)) as i32),
            (StackValue::Int64(l), StackValue::Int64(r), NumberSign::Signed) => StackValue::Int64(l $op r),
            (StackValue::Int64(l), StackValue::Int64(r), NumberSign::Unsigned) => StackValue::Int64(((l as u64) $op (r as u64)) as i64),
            (StackValue::NativeInt(l), StackValue::NativeInt(r), NumberSign::Signed) => StackValue::NativeInt(l $op r),
            (StackValue::NativeInt(l), StackValue::NativeInt(r), NumberSign::Unsigned) => StackValue::NativeInt(((l as usize) $op (r as usize)) as isize),
            (StackValue::NativeFloat(l), StackValue::NativeFloat(r), _) => StackValue::NativeFloat(l $op r),
            // Mixed types (Int32 <-> NativeInt)
            (StackValue::NativeInt(l), StackValue::Int32(r), NumberSign::Signed) => StackValue::NativeInt(l $op (r as isize)),
            (StackValue::NativeInt(l), StackValue::Int32(r), NumberSign::Unsigned) => StackValue::NativeInt(((l as usize) $op ((r as u32) as usize)) as isize),
            (StackValue::Int32(l), StackValue::NativeInt(r), NumberSign::Signed) => StackValue::NativeInt((l as isize) $op r),
            (StackValue::Int32(l), StackValue::NativeInt(r), NumberSign::Unsigned) => StackValue::NativeInt((((l as u32) as usize) $op (r as usize)) as isize),
            (l, r, _) => panic!("invalid types for arithmetic operation: {:?}, {:?}", l, r),
        }
    };
}

macro_rules! shift_op {
    ($target:expr, $amount:expr, $sgn:expr, $op:tt) => {
        match ($target, $sgn) {
            (StackValue::Int32(i), NumberSign::Signed) => StackValue::Int32(i $op $amount),
            (StackValue::Int32(i), NumberSign::Unsigned) => StackValue::Int32(((i as u32) $op $amount) as i32),
            (StackValue::Int64(i), NumberSign::Signed) => StackValue::Int64(i $op $amount),
            (StackValue::Int64(i), NumberSign::Unsigned) => StackValue::Int64(((i as u64) $op $amount) as i64),
            (StackValue::NativeInt(i), NumberSign::Signed) => StackValue::NativeInt(i $op $amount),
            (StackValue::NativeInt(i), NumberSign::Unsigned) => StackValue::NativeInt(((i as usize) $op $amount) as isize),
            (v, _) => panic!("invalid shift target: {:?}", v),
        }
    };
}

macro_rules! bitwise_op {
    ($l:expr, $r:expr, $op:tt) => {
        match ($l, $r) {
            (StackValue::Int32(l), StackValue::Int32(r)) => StackValue::Int32(l $op r),
            (StackValue::Int32(l), StackValue::NativeInt(r)) => StackValue::NativeInt((l as isize) $op r),
            (StackValue::Int64(l), StackValue::Int64(r)) => StackValue::Int64(l $op r),
            (StackValue::Int64(l), StackValue::NativeInt(r)) => StackValue::Int64(l $op (r as i64)),
            (StackValue::NativeInt(l), StackValue::Int32(r)) => StackValue::NativeInt(l $op (r as isize)),
            (StackValue::NativeInt(l), StackValue::Int64(r)) => StackValue::Int64((l as i64) $op r),
            (StackValue::NativeInt(l), StackValue::NativeInt(r)) => StackValue::NativeInt(l $op r),
            (l, r) => panic!("invalid types for bitwise operation: {:?}, {:?}", l, r),
        }
    };
}

macro_rules! wrapping_arithmetic_op {
    ($l:expr, $r:expr, $op:ident, $float_op:tt) => {
        match ($l, $r) {
            (StackValue::Int32(l), StackValue::Int32(r)) => StackValue::Int32(l.$op(r)),
            (StackValue::Int32(l), StackValue::NativeInt(r)) => StackValue::NativeInt((l as isize).$op(r)),
            (StackValue::Int64(l), StackValue::Int64(r)) => StackValue::Int64(l.$op(r)),
            (StackValue::Int64(l), StackValue::NativeInt(r)) => StackValue::Int64(l.$op(r as i64)),
            (StackValue::NativeInt(l), StackValue::Int32(r)) => StackValue::NativeInt(l.$op(r as isize)),
            (StackValue::NativeInt(l), StackValue::Int64(r)) => StackValue::Int64((l as i64).$op(r)),
            (StackValue::NativeInt(l), StackValue::NativeInt(r)) => StackValue::NativeInt(l.$op(r)),
            (StackValue::NativeFloat(l), StackValue::NativeFloat(r)) => StackValue::NativeFloat(l $float_op r),
            (StackValue::ObjectRef(l), StackValue::NativeInt(r)) => {
                 let ptr = l.0.map(|p| Gc::as_ptr(p) as isize).unwrap_or(0);
                 StackValue::NativeInt(ptr.$op(r))
            },
            (StackValue::ObjectRef(l), StackValue::Int32(r)) => {
                 let ptr = l.0.map(|p| Gc::as_ptr(p) as isize).unwrap_or(0);
                 StackValue::NativeInt(ptr.$op(r as isize))
            },
            (StackValue::ObjectRef(l), StackValue::ObjectRef(r)) => {
                 let ptr_l = l.0.map(|p| Gc::as_ptr(p) as isize).unwrap_or(0);
                 let ptr_r = r.0.map(|p| Gc::as_ptr(p) as isize).unwrap_or(0);
                 StackValue::NativeInt(ptr_l.$op(ptr_r))
            },
            (l, r) => panic!("invalid types for arithmetic operation: {:?}, {:?}", l, r),
        }
    };
}

impl<'gc> StackValue<'gc> {
    pub fn unmanaged_ptr(ptr: *mut u8) -> Self {
        if ptr.is_null() {
            Self::NativeInt(0)
        } else {
            Self::UnmanagedPtr(UnmanagedPtr(NonNull::new(ptr).unwrap()))
        }
    }
    pub fn managed_ptr(ptr: *mut u8, target_type: TypeDescription, pinned: bool) -> Self {
        Self::managed_ptr_with_owner(ptr, target_type, None, pinned)
    }

    pub fn managed_ptr_with_owner(
        ptr: *mut u8,
        target_type: TypeDescription,
        owner: Option<ObjectRef<'gc>>,
        pinned: bool,
    ) -> Self {
        Self::ManagedPtr(ManagedPtr::new(
            NonNull::new(ptr),
            target_type,
            owner,
            pinned,
        ))
    }
    pub fn managed_stack_ptr(
        index: usize,
        offset: usize,
        ptr: *mut u8,
        target_type: TypeDescription,
        pinned: bool,
    ) -> Self {
        let mut m = ManagedPtr::new(NonNull::new(ptr), target_type, None, pinned);
        m.stack_slot_origin = Some((index, offset));
        m.offset = offset;
        Self::ManagedPtr(m)
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
            Self::ManagedPtr(m) => ref_to_ptr(m),
            Self::ValueType(o) => {
                // SAFETY: Returning a pointer to the internal buffer. 
                // This is used by ldloca/ldarga to get a byref to the value type.
                let ptr = unsafe { o.instance_storage.raw_data_unsynchronized().as_ptr() as *mut u8 };
                NonNull::new(ptr).unwrap()
            }
            Self::CrossArenaObjectRef(p, _) => ref_to_ptr(p),
        }
    }

    pub fn as_ptr(&self) -> *mut u8 {
        match self {
            Self::NativeInt(i) => *i as *mut u8,
            Self::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
            Self::ManagedPtr(m) => m
                .pointer()
                .map(|p| p.as_ptr())
                .unwrap_or(std::ptr::null_mut()),
            v => panic!("expected pointer on stack, received {:?}", v),
        }
    }

    pub fn as_i32(&self) -> i32 {
        match self {
            Self::Int32(i) => *i,
            v => panic!("expected Int32, received {:?}", v),
        }
    }

    pub fn as_i64(&self) -> i64 {
        match self {
            Self::Int64(i) => *i,
            v => panic!("expected Int64, received {:?}", v),
        }
    }

    pub fn as_f64(&self) -> f64 {
        match self {
            Self::NativeFloat(f) => *f,
            v => panic!("expected NativeFloat, received {:?}", v),
        }
    }

    pub fn as_isize(&self) -> isize {
        match self {
            Self::NativeInt(i) => *i,
            v => panic!("expected NativeInt, received {:?}", v),
        }
    }

    pub fn as_object_ref(&self) -> ObjectRef<'gc> {
        match self {
            Self::ObjectRef(o) => *o,
            v => panic!("expected ObjectRef, received {:?}", v),
        }
    }

    pub fn as_managed_ptr(&self) -> ManagedPtr<'gc> {
        match self {
            Self::ManagedPtr(m) => *m,
            v => panic!("expected ManagedPtr, received {:?}", v),
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

    pub fn shr(self, other: Self, sgn: NumberSign) -> Self {
        let amount = match other {
            StackValue::Int32(i) => i as u32,
            StackValue::NativeInt(i) => i as u32,
            _ => panic!("invalid shift amount: {:?}", other),
        };
        shift_op!(self, amount, sgn, >>)
    }

    pub fn div(self, other: Self, sgn: NumberSign) -> Result<Self, &'static str> {
        if other.is_zero() {
            return Err("System.DivideByZeroException");
        }
        Ok(arithmetic_op!(self, other, sgn, /))
    }

    pub fn rem(self, other: Self, sgn: NumberSign) -> Result<Self, &'static str> {
        if other.is_zero() {
            return Err("System.DivideByZeroException");
        }
        Ok(arithmetic_op!(self, other, sgn, %))
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

    pub fn checked_add(self, other: Self, sgn: NumberSign) -> Result<Self, &'static str> {
        checked_arithmetic_op!(self, other, sgn, checked_add)
    }

    pub fn checked_sub(self, other: Self, sgn: NumberSign) -> Result<Self, &'static str> {
        checked_arithmetic_op!(self, other, sgn, checked_sub)
    }

    pub fn checked_mul(self, other: Self, sgn: NumberSign) -> Result<Self, &'static str> {
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
        unsafe {
            debug_assert!(!ptr.is_null(), "Attempted to load from a null pointer");
            let alignment = load_type_alignment(t);
            debug_assert!(
                (ptr as usize).is_multiple_of(alignment),
                "Attempted to load from an unaligned pointer {:?} for type {:?}",
                ptr,
                t
            );

            let size = match t {
                LoadType::Int8 | LoadType::UInt8 => 1,
                LoadType::Int16 | LoadType::UInt16 => 2,
                LoadType::Int32 | LoadType::UInt32 | LoadType::Float32 => 4,
                LoadType::Int64 | LoadType::Float64 => 8,
                LoadType::IntPtr => size_of::<isize>(),
                LoadType::Object => size_of::<usize>(),
            };

            let val = StandardAtomicAccess::load_atomic(ptr, size, ordering);

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
                    let ptr = val as usize as *const ThreadSafeLock<object::ObjectInner<'gc>>;
                    if ptr.is_null() {
                        Self::ObjectRef(ObjectRef(None))
                    } else {
                        Self::ObjectRef(ObjectRef(Some(Gc::from_ptr(ptr))))
                    }
                }
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
        unsafe {
            debug_assert!(!ptr.is_null(), "Attempted to store to a null pointer");
            let alignment = store_type_alignment(t);
            debug_assert!(
                (ptr as usize).is_multiple_of(alignment),
                "Attempted to store to an unaligned pointer {:?} for type {:?}",
                ptr,
                t
            );

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
                        Some(h) => Gc::as_ptr(h) as usize,
                        None => 0,
                    };
                    (val as u64, size_of::<usize>())
                }
            };

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
                (l.pointer().map(|p| p.as_ptr() as isize).unwrap_or(0)) == *r
            }
            (NativeInt(l), ManagedPtr(r)) => {
                *l == (r.pointer().map(|p| p.as_ptr() as isize).unwrap_or(0))
            }
            (ObjectRef(l), ObjectRef(r)) => l == r,
            (ValueType(l), ValueType(r)) => l == r,
            _ => false,
        }
    }
}

impl PartialOrd for StackValue<'_> {
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
            (ManagedPtr(l), NativeInt(r)) => l
                .pointer()
                .map(|p| p.as_ptr() as isize)
                .unwrap_or(0)
                .partial_cmp(r),
            (NativeInt(l), ManagedPtr(r)) => {
                l.partial_cmp(&(r.pointer().map(|p| p.as_ptr() as isize).unwrap_or(0)))
            }
            (ObjectRef(l), ObjectRef(r)) => l.partial_cmp(r),
            _ => None,
        }
    }
}

impl<'gc> Add for StackValue<'gc> {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        use StackValue::*;
        match (self, rhs) {
            (Int32(i), ManagedPtr(m)) | (ManagedPtr(m), Int32(i)) => {
                // SAFETY: Pointer arithmetic is performed within the bounds of the managed object
                // or stack slot it points to. The VM ensures that pointers stay within allocated regions.
                unsafe { ManagedPtr(m.offset(i as isize)) }
            }
            (NativeInt(i), ManagedPtr(m)) | (ManagedPtr(m), NativeInt(i)) => {
                // SAFETY: Pointer arithmetic is performed within the bounds of the managed object.
                unsafe { ManagedPtr(m.offset(i)) }
            }
            (l, r) => wrapping_arithmetic_op!(l, r, wrapping_add, +),
        }
    }
}

impl<'gc> Sub for StackValue<'gc> {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        use StackValue::*;
        match (self, rhs) {
            (ManagedPtr(m), Int32(i)) => {
                // SAFETY: Pointer arithmetic is performed within the bounds of the managed object.
                unsafe { ManagedPtr(m.offset(-(i as isize))) }
            }
            (ManagedPtr(m), NativeInt(i)) => {
                // SAFETY: Pointer arithmetic is performed within the bounds of the managed object.
                unsafe { ManagedPtr(m.offset(-i)) }
            }
            (ManagedPtr(m), Int64(i)) => {
                // SAFETY: Pointer arithmetic is performed within the bounds of the managed object.
                unsafe { ManagedPtr(m.offset(-(i as isize))) }
            }
            (ManagedPtr(m1), ManagedPtr(m2)) => {
                let v1 = m1.pointer().map(|p| p.as_ptr() as isize).unwrap_or(0);
                let v2 = m2.pointer().map(|p| p.as_ptr() as isize).unwrap_or(0);
                NativeInt(v1 - v2)
            }
            (l, r) => wrapping_arithmetic_op!(l, r, wrapping_sub, -),
        }
    }
}

impl<'gc> Mul for StackValue<'gc> {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self::Output {
        wrapping_arithmetic_op!(self, rhs, wrapping_mul, *)
    }
}

impl<'gc> BitAnd for StackValue<'gc> {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        bitwise_op!(self, rhs, &)
    }
}

impl<'gc> BitOr for StackValue<'gc> {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        bitwise_op!(self, rhs, |)
    }
}

impl<'gc> BitXor for StackValue<'gc> {
    type Output = Self;
    fn bitxor(self, rhs: Self) -> Self::Output {
        bitwise_op!(self, rhs, ^)
    }
}

impl<'gc> Shl for StackValue<'gc> {
    type Output = Self;
    fn shl(self, rhs: Self) -> Self::Output {
        let amount = match rhs {
            StackValue::Int32(i) => i as u32,
            StackValue::NativeInt(i) => i as u32,
            _ => panic!("invalid shift amount"),
        };
        match self {
            StackValue::Int32(i) => StackValue::Int32(i << amount),
            StackValue::Int64(i) => StackValue::Int64(i << amount),
            StackValue::NativeInt(i) => StackValue::NativeInt(i << amount),
            _ => panic!("invalid shift target"),
        }
    }
}

impl<'gc> Not for StackValue<'gc> {
    type Output = Self;
    fn not(self) -> Self::Output {
        use StackValue::*;
        match self {
            Int32(i) => Int32(!i),
            Int64(i) => Int64(!i),
            NativeInt(i) => NativeInt(!i),
            _ => panic!("invalid type for not"),
        }
    }
}

impl<'gc> Neg for StackValue<'gc> {
    type Output = Self;
    fn neg(self) -> Self::Output {
        use StackValue::*;
        match self {
            Int32(i) => Int32(-i),
            Int64(i) => Int64(-i),
            NativeInt(i) => NativeInt(-i),
            NativeFloat(f) => NativeFloat(-f),
            _ => panic!("invalid type for neg"),
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

        let sum = v1.clone() + v2.clone();
        assert_eq!(sum, StackValue::Int32(30));

        let v3 = StackValue::Int32(30);
        let v4 = StackValue::Int32(10);
        let diff = v3 - v4;
        assert_eq!(diff, StackValue::Int32(20));
    }
}
