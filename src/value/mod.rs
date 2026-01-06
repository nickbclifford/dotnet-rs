use crate::{
    types::TypeDescription,
    vm::{
        context::ResolutionContext,
        sync::{
            AtomicI16, AtomicI32, AtomicI64, AtomicI8, AtomicIsize, AtomicU16, AtomicU32,
            AtomicU64, AtomicU8, AtomicUsize, Ordering as AtomicOrdering,
        },
        GCHandle,
    },
};
use dotnetdll::prelude::*;
use gc_arena::{Collect, Collection, Gc};
use std::{
    cmp::Ordering,
    fmt::Debug,
    ops::{Add, BitAnd, BitOr, BitXor, Mul, Neg, Not, Shl, Sub},
    ptr::NonNull,
};

pub mod layout;
pub mod object;
pub mod pointer;
pub mod storage;
pub mod string;

#[cfg(feature = "multithreaded-gc")]
use object::ObjectPtr;

use object::{HeapStorage, Object, ObjectHandle, ObjectRef};
use pointer::{ManagedPtr, UnmanagedPtr};
use string::CLRString;

#[derive(Clone, Debug)]
pub enum StackValue<'gc> {
    Int32(i32),
    Int64(i64),
    NativeInt(isize),
    NativeFloat(f64),
    ObjectRef(ObjectRef<'gc>),
    UnmanagedPtr(UnmanagedPtr),
    ManagedPtr(ManagedPtr<'gc>),
    ValueType(Box<Object<'gc>>),
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
            Self::ValueType(v) => v.as_ref().trace(cc),
            #[cfg(feature = "multithreaded-gc")]
            Self::CrossArenaObjectRef(ptr, tid) => {
                crate::vm::gc::coordinator::record_cross_arena_ref(*tid, *ptr);
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
            (StackValue::NativeInt(l), StackValue::Int32(r)) => StackValue::NativeInt(l $op (r as isize)),
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
            (StackValue::NativeInt(l), StackValue::Int32(r)) => StackValue::NativeInt(l.$op(r as isize)),
            (StackValue::NativeInt(l), StackValue::NativeInt(r)) => StackValue::NativeInt(l.$op(r)),
            (StackValue::NativeFloat(l), StackValue::NativeFloat(r)) => StackValue::NativeFloat(l $float_op r),
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
    pub fn managed_ptr(
        ptr: *mut u8,
        target_type: TypeDescription,
        owner: Option<ObjectHandle<'gc>>,
        pinned: bool,
    ) -> Self {
        Self::ManagedPtr(ManagedPtr {
            value: NonNull::new(ptr).expect("ManagedPtr should not be null"),
            inner_type: target_type,
            owner,
            pinned,
        })
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
            Self::ObjectRef(ObjectRef(o)) => ref_to_ptr(o),
            Self::UnmanagedPtr(UnmanagedPtr(u)) => *u,
            Self::ManagedPtr(m) => m.value,
            Self::ValueType(o) => {
                NonNull::new(o.instance_storage.get().as_ptr() as *mut u8).unwrap()
            }
            #[cfg(feature = "multithreaded-gc")]
            Self::CrossArenaObjectRef(_, _) => todo!("handle CrossArenaObjectRef in data_location"),
        }
    }

    pub fn contains_type(&self, ctx: &ResolutionContext) -> TypeDescription {
        match self {
            Self::Int32(_) => ctx.loader.corlib_type("System.Int32"),
            Self::Int64(_) => ctx.loader.corlib_type("System.Int64"),
            Self::NativeInt(_) | Self::UnmanagedPtr(_) => ctx.loader.corlib_type("System.IntPtr"),
            Self::NativeFloat(_) => ctx.loader.corlib_type("System.Double"),
            Self::ObjectRef(ObjectRef(Some(o))) => ctx.get_heap_description(*o),
            Self::ObjectRef(ObjectRef(None)) => ctx.loader.corlib_type("System.Object"),
            Self::ManagedPtr(m) => m.inner_type,
            Self::ValueType(o) => o.description,
            #[cfg(feature = "multithreaded-gc")]
            Self::CrossArenaObjectRef(_, _) => todo!("handle CrossArenaObjectRef in contains_type"),
        }
    }

    pub fn as_ptr(&self) -> *mut u8 {
        match self {
            Self::NativeInt(i) => *i as *mut u8,
            Self::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
            Self::ManagedPtr(m) => m.value.as_ptr(),
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
        Self::load_atomic(ptr, t, AtomicOrdering::Relaxed)
    }

    /// # Safety
    /// `ptr` must be a valid, aligned pointer to a value of the type specified by `t`.
    pub unsafe fn load_atomic(ptr: *const u8, t: LoadType, ordering: AtomicOrdering) -> Self {
        debug_assert!(!ptr.is_null(), "Attempted to load from a null pointer");
        let alignment = load_type_alignment(t);
        debug_assert!(
            (ptr as usize).is_multiple_of(alignment),
            "Attempted to load from an unaligned pointer {:?} for type {:?}",
            ptr,
            t
        );

        match t {
            LoadType::Int8 => Self::Int32((*(ptr as *const AtomicI8)).load(ordering) as i32),
            LoadType::UInt8 => Self::Int32((*(ptr as *const AtomicU8)).load(ordering) as i32),
            LoadType::Int16 => Self::Int32((*(ptr as *const AtomicI16)).load(ordering) as i32),
            LoadType::UInt16 => Self::Int32((*(ptr as *const AtomicU16)).load(ordering) as i32),
            LoadType::Int32 => Self::Int32((*(ptr as *const AtomicI32)).load(ordering)),
            LoadType::UInt32 => Self::Int32((*(ptr as *const AtomicU32)).load(ordering) as i32),
            LoadType::Int64 => Self::Int64((*(ptr as *const AtomicI64)).load(ordering)),
            LoadType::Float32 => {
                let val = (*(ptr as *const AtomicU32)).load(ordering);
                Self::NativeFloat(f32::from_bits(val) as f64)
            }
            LoadType::Float64 => {
                let val = (*(ptr as *const AtomicU64)).load(ordering);
                Self::NativeFloat(f64::from_bits(val))
            }
            LoadType::IntPtr => Self::NativeInt((*(ptr as *const AtomicIsize)).load(ordering)),
            LoadType::Object => {
                let val = (*(ptr as *const AtomicUsize)).load(ordering);
                let ptr = val as *const crate::vm::threadsafe_lock::ThreadSafeLock<
                    object::ObjectInner<'gc>,
                >;
                if ptr.is_null() {
                    Self::ObjectRef(ObjectRef(None))
                } else {
                    Self::ObjectRef(ObjectRef(Some(Gc::from_ptr(ptr))))
                }
            }
        }
    }

    /// # Safety
    /// `ptr` must be a valid, aligned pointer to a location with sufficient space for the type specified by `t`.
    pub unsafe fn store(self, ptr: *mut u8, t: StoreType) {
        self.store_atomic(ptr, t, AtomicOrdering::Relaxed);
    }

    /// # Safety
    /// `ptr` must be a valid, aligned pointer to a location with sufficient space for the type specified by `t`.
    pub unsafe fn store_atomic(self, ptr: *mut u8, t: StoreType, ordering: AtomicOrdering) {
        debug_assert!(!ptr.is_null(), "Attempted to store to a null pointer");
        let alignment = store_type_alignment(t);
        debug_assert!(
            (ptr as usize).is_multiple_of(alignment),
            "Attempted to store to an unaligned pointer {:?} for type {:?}",
            ptr,
            t
        );

        match t {
            StoreType::Int8 => (*(ptr as *const AtomicI8)).store(self.as_i32() as i8, ordering),
            StoreType::Int16 => (*(ptr as *const AtomicI16)).store(self.as_i32() as i16, ordering),
            StoreType::Int32 => (*(ptr as *const AtomicI32)).store(self.as_i32(), ordering),
            StoreType::Int64 => (*(ptr as *const AtomicI64)).store(self.as_i64(), ordering),
            StoreType::Float32 => {
                let val = (self.as_f64() as f32).to_bits();
                (*(ptr as *const AtomicU32)).store(val, ordering);
            }
            StoreType::Float64 => {
                let val = self.as_f64().to_bits();
                (*(ptr as *const AtomicU64)).store(val, ordering);
            }
            StoreType::IntPtr => (*(ptr as *const AtomicIsize)).store(self.as_isize(), ordering),
            StoreType::Object => {
                let obj = self.as_object_ref();
                let val = match obj.0 {
                    Some(h) => Gc::as_ptr(h) as usize,
                    None => 0,
                };
                (*(ptr as *const AtomicUsize)).store(val, ordering);
            }
        }
    }
}

fn load_type_alignment(t: LoadType) -> usize {
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

fn store_type_alignment(t: StoreType) -> usize {
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
            (ManagedPtr(l), NativeInt(r)) => (l.value.as_ptr() as isize) == *r,
            (NativeInt(l), ManagedPtr(r)) => *l == (r.value.as_ptr() as isize),
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
            (ManagedPtr(l), NativeInt(r)) => (l.value.as_ptr() as isize).partial_cmp(r),
            (NativeInt(l), ManagedPtr(r)) => l.partial_cmp(&(r.value.as_ptr() as isize)),
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
            (ManagedPtr(m1), ManagedPtr(m2)) => {
                NativeInt((m1.value.as_ptr() as isize) - (m2.value.as_ptr() as isize))
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
