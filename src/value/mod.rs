use crate::{types::TypeDescription, vm::{GCHandle, context::ResolutionContext}};
use dotnetdll::prelude::*;
use gc_arena::{Collect, Collection, Gc, lock::RefLock};
use std::{
    cmp::Ordering, fmt::Debug, ops::{Add, BitAnd, BitOr, BitXor, Mul, Neg, Not, Shl, Sub},
};

pub mod layout;
pub mod object;
pub mod pointer;
pub mod storage;
pub mod string;

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
}
unsafe impl<'gc> Collect for StackValue<'gc> {
    fn trace(&self, cc: &Collection) {
        match self {
            Self::ObjectRef(o) => o.trace(cc),
            Self::ManagedPtr(m) => m.trace(cc),
            Self::ValueType(v) => v.as_ref().trace(cc),
            _ => {}
        }
    }
}
impl<'gc> StackValue<'gc> {
    pub fn unmanaged_ptr(ptr: *mut u8) -> Self {
        Self::UnmanagedPtr(UnmanagedPtr(ptr))
    }
    pub fn managed_ptr(
        ptr: *mut u8,
        target_type: TypeDescription,
        owner: Option<ObjectHandle<'gc>>,
        pinned: bool,
    ) -> Self {
        Self::ManagedPtr(ManagedPtr {
            value: ptr,
            inner_type: target_type,
            owner,
            pinned,
        })
    }
    pub fn null() -> Self {
        Self::ObjectRef(ObjectRef(None))
    }
    pub fn string(gc: GCHandle<'gc>, s: CLRString) -> Self {
        Self::ObjectRef(ObjectRef(Some(Gc::new(
            gc,
            RefLock::new(HeapStorage::Str(s)),
        ))))
    }

    pub fn data_location(&self) -> *const u8 {
        fn ref_to_ptr<T>(r: &T) -> *const u8 {
            (r as *const T) as *const u8
        }

        match self {
            Self::Int32(i) => ref_to_ptr(i),
            Self::Int64(i) => ref_to_ptr(i),
            Self::NativeInt(i) => ref_to_ptr(i),
            Self::NativeFloat(f) => ref_to_ptr(f),
            Self::ObjectRef(ObjectRef(o)) => ref_to_ptr(o),
            Self::UnmanagedPtr(UnmanagedPtr(u)) => ref_to_ptr(u),
            Self::ManagedPtr(m) => ref_to_ptr(&m.value),
            Self::ValueType(o) => o.instance_storage.get().as_ptr(),
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
        }
    }

    pub fn as_ptr(&self) -> *mut u8 {
        match self {
            Self::NativeInt(i) => *i as *mut u8,
            Self::UnmanagedPtr(UnmanagedPtr(p)) => *p,
            Self::ManagedPtr(m) => m.value,
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
        use NumberSign::*;
        use StackValue::*;
        let amount = match other {
            Int32(i) => i as u32,
            NativeInt(i) => i as u32,
            _ => panic!("invalid shift amount"),
        };
        match (self, sgn) {
            (Int32(i), Signed) => Int32(i >> amount),
            (Int32(i), Unsigned) => Int32(((i as u32) >> amount) as i32),
            (Int64(i), Signed) => Int64(i >> amount),
            (Int64(i), Unsigned) => Int64(((i as u64) >> amount) as i64),
            (NativeInt(i), Signed) => NativeInt(i >> amount),
            (NativeInt(i), Unsigned) => NativeInt(((i as usize) >> amount) as isize),
            (v, _) => panic!("invalid shift target: {:?}", v),
        }
    }

    pub fn div(self, other: Self, sgn: NumberSign) -> Result<Self, &'static str> {
        use NumberSign::*;
        use StackValue::*;
        if other.is_zero() {
            return Err("System.DivideByZeroException");
        }
        match (self, other, sgn) {
            (Int32(l), Int32(r), Signed) => Ok(Int32(l / r)),
            (Int32(l), Int32(r), Unsigned) => Ok(Int32(((l as u32) / (r as u32)) as i32)),
            (Int64(l), Int64(r), Signed) => Ok(Int64(l / r)),
            (Int64(l), Int64(r), Unsigned) => Ok(Int64(((l as u64) / (r as u64)) as i64)),
            (NativeInt(l), NativeInt(r), Signed) => Ok(NativeInt(l / r)),
            (NativeInt(l), NativeInt(r), Unsigned) => {
                Ok(NativeInt(((l as usize) / (r as usize)) as isize))
            }
            (NativeFloat(l), NativeFloat(r), _) => Ok(NativeFloat(l / r)),
            (l, r, _) => panic!("invalid types for div: {:?}, {:?}", l, r),
        }
    }

    pub fn rem(self, other: Self, sgn: NumberSign) -> Result<Self, &'static str> {
        use NumberSign::*;
        use StackValue::*;
        if other.is_zero() {
            return Err("System.DivideByZeroException");
        }
        match (self, other, sgn) {
            (Int32(l), Int32(r), Signed) => Ok(Int32(l % r)),
            (Int32(l), Int32(r), Unsigned) => Ok(Int32(((l as u32) % (r as u32)) as i32)),
            (Int64(l), Int64(r), Signed) => Ok(Int64(l % r)),
            (Int64(l), Int64(r), Unsigned) => Ok(Int64(((l as u64) % (r as u64)) as i64)),
            (NativeInt(l), NativeInt(r), Signed) => Ok(NativeInt(l % r)),
            (NativeInt(l), NativeInt(r), Unsigned) => {
                Ok(NativeInt(((l as usize) % (r as usize)) as isize))
            }
            (NativeFloat(l), NativeFloat(r), _) => Ok(NativeFloat(l % r)),
            (l, r, _) => panic!("invalid types for rem: {:?}, {:?}", l, r),
        }
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
        use NumberSign::*;
        use StackValue::*;
        match (self, other, sgn) {
            (Int32(l), Int32(r), Signed) => l
                .checked_add(r)
                .map(Int32)
                .ok_or("System.OverflowException"),
            (Int32(l), Int32(r), Unsigned) => (l as u32)
                .checked_add(r as u32)
                .map(|v| Int32(v as i32))
                .ok_or("System.OverflowException"),
            (Int64(l), Int64(r), Signed) => l
                .checked_add(r)
                .map(Int64)
                .ok_or("System.OverflowException"),
            (Int64(l), Int64(r), Unsigned) => (l as u64)
                .checked_add(r as u64)
                .map(|v| Int64(v as i64))
                .ok_or("System.OverflowException"),
            (NativeInt(l), NativeInt(r), Signed) => l
                .checked_add(r)
                .map(NativeInt)
                .ok_or("System.OverflowException"),
            (NativeInt(l), NativeInt(r), Unsigned) => (l as usize)
                .checked_add(r as usize)
                .map(|v| NativeInt(v as isize))
                .ok_or("System.OverflowException"),
            (l, r, _) => panic!("invalid types for checked_add: {:?}, {:?}", l, r),
        }
    }

    pub fn checked_sub(self, other: Self, sgn: NumberSign) -> Result<Self, &'static str> {
        use NumberSign::*;
        use StackValue::*;
        match (self, other, sgn) {
            (Int32(l), Int32(r), Signed) => l
                .checked_sub(r)
                .map(Int32)
                .ok_or("System.OverflowException"),
            (Int32(l), Int32(r), Unsigned) => (l as u32)
                .checked_sub(r as u32)
                .map(|v| Int32(v as i32))
                .ok_or("System.OverflowException"),
            (Int64(l), Int64(r), Signed) => l
                .checked_sub(r)
                .map(Int64)
                .ok_or("System.OverflowException"),
            (Int64(l), Int64(r), Unsigned) => (l as u64)
                .checked_sub(r as u64)
                .map(|v| Int64(v as i64))
                .ok_or("System.OverflowException"),
            (NativeInt(l), NativeInt(r), Signed) => l
                .checked_sub(r)
                .map(NativeInt)
                .ok_or("System.OverflowException"),
            (NativeInt(l), NativeInt(r), Unsigned) => (l as usize)
                .checked_sub(r as usize)
                .map(|v| NativeInt(v as isize))
                .ok_or("System.OverflowException"),
            (l, r, _) => panic!("invalid types for checked_sub: {:?}, {:?}", l, r),
        }
    }

    pub fn checked_mul(self, other: Self, sgn: NumberSign) -> Result<Self, &'static str> {
        use NumberSign::*;
        use StackValue::*;
        match (self, other, sgn) {
            (Int32(l), Int32(r), Signed) => l
                .checked_mul(r)
                .map(Int32)
                .ok_or("System.OverflowException"),
            (Int32(l), Int32(r), Unsigned) => (l as u32)
                .checked_mul(r as u32)
                .map(|v| Int32(v as i32))
                .ok_or("System.OverflowException"),
            (Int64(l), Int64(r), Signed) => l
                .checked_mul(r)
                .map(Int64)
                .ok_or("System.OverflowException"),
            (Int64(l), Int64(r), Unsigned) => (l as u64)
                .checked_mul(r as u64)
                .map(|v| Int64(v as i64))
                .ok_or("System.OverflowException"),
            (NativeInt(l), NativeInt(r), Signed) => l
                .checked_mul(r)
                .map(NativeInt)
                .ok_or("System.OverflowException"),
            (NativeInt(l), NativeInt(r), Unsigned) => (l as usize)
                .checked_mul(r as usize)
                .map(|v| NativeInt(v as isize))
                .ok_or("System.OverflowException"),
            (l, r, _) => panic!("invalid types for checked_mul: {:?}, {:?}", l, r),
        }
    }

    pub unsafe fn load(ptr: *const u8, t: LoadType) -> Self {
        match t {
            LoadType::Int8 => Self::Int32(*(ptr as *const i8) as i32),
            LoadType::UInt8 => Self::Int32(*ptr as i32),
            LoadType::Int16 => Self::Int32(*(ptr as *const i16) as i32),
            LoadType::UInt16 => Self::Int32(*(ptr as *const u16) as i32),
            LoadType::Int32 => Self::Int32(*(ptr as *const i32)),
            LoadType::UInt32 => Self::Int32(*(ptr as *const u32) as i32),
            LoadType::Int64 => Self::Int64(*(ptr as *const i64)),
            LoadType::Float32 => Self::NativeFloat(*(ptr as *const f32) as f64),
            LoadType::Float64 => Self::NativeFloat(*(ptr as *const f64)),
            LoadType::IntPtr => Self::NativeInt(*(ptr as *const isize)),
            LoadType::Object => Self::ObjectRef(*(ptr as *const ObjectRef)),
        }
    }

    pub unsafe fn store(self, ptr: *mut u8, t: StoreType) {
        match t {
            StoreType::Int8 => *(ptr as *mut i8) = self.as_i32() as i8,
            StoreType::Int16 => *(ptr as *mut i16) = self.as_i32() as i16,
            StoreType::Int32 => *(ptr as *mut i32) = self.as_i32(),
            StoreType::Int64 => *(ptr as *mut i64) = self.as_i64(),
            StoreType::Float32 => *(ptr as *mut f32) = self.as_f64() as f32,
            StoreType::Float64 => *(ptr as *mut f64) = self.as_f64(),
            StoreType::IntPtr => *(ptr as *mut isize) = self.as_isize(),
            StoreType::Object => *(ptr as *mut ObjectRef) = self.as_object_ref(),
        }
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
            (ManagedPtr(l), NativeInt(r)) => (l.value as isize) == *r,
            (NativeInt(l), ManagedPtr(r)) => *l == (r.value as isize),
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
            (ManagedPtr(l), NativeInt(r)) => (l.value as isize).partial_cmp(r),
            (NativeInt(l), ManagedPtr(r)) => l.partial_cmp(&(r.value as isize)),
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
            (Int32(l), Int32(r)) => Int32(l.wrapping_add(r)),
            (Int32(l), NativeInt(r)) => NativeInt((l as isize).wrapping_add(r)),
            (Int64(l), Int64(r)) => Int64(l.wrapping_add(r)),
            (NativeInt(l), Int32(r)) => NativeInt(l.wrapping_add(r as isize)),
            (NativeInt(l), NativeInt(r)) => NativeInt(l.wrapping_add(r)),
            (NativeFloat(l), NativeFloat(r)) => NativeFloat(l + r),
            (Int32(i), ManagedPtr(m)) | (ManagedPtr(m), Int32(i)) => unsafe {
                ManagedPtr(m.map_value(|p| p.offset(i as isize)))
            },
            (NativeInt(i), ManagedPtr(m)) | (ManagedPtr(m), NativeInt(i)) => unsafe {
                ManagedPtr(m.map_value(|p| p.offset(i)))
            },
            (l, r) => panic!("invalid types for add: {:?}, {:?}", l, r),
        }
    }
}

impl<'gc> Sub for StackValue<'gc> {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        use StackValue::*;
        match (self, rhs) {
            (Int32(l), Int32(r)) => Int32(l.wrapping_sub(r)),
            (Int32(l), NativeInt(r)) => NativeInt((l as isize).wrapping_sub(r)),
            (Int64(l), Int64(r)) => Int64(l.wrapping_sub(r)),
            (NativeInt(l), Int32(r)) => NativeInt(l.wrapping_sub(r as isize)),
            (NativeInt(l), NativeInt(r)) => NativeInt(l.wrapping_sub(r)),
            (NativeFloat(l), NativeFloat(r)) => NativeFloat(l - r),
            (ManagedPtr(m), Int32(i)) => unsafe {
                ManagedPtr(m.map_value(|p| p.offset(-(i as isize))))
            },
            (ManagedPtr(m), NativeInt(i)) => unsafe { ManagedPtr(m.map_value(|p| p.offset(-i))) },
            (ManagedPtr(m1), ManagedPtr(m2)) => {
                NativeInt((m1.value as isize) - (m2.value as isize))
            }
            (l, r) => panic!("invalid types for sub: {:?}, {:?}", l, r),
        }
    }
}

impl<'gc> Mul for StackValue<'gc> {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self::Output {
        use StackValue::*;
        match (self, rhs) {
            (Int32(l), Int32(r)) => Int32(l.wrapping_mul(r)),
            (Int32(l), NativeInt(r)) => NativeInt((l as isize).wrapping_mul(r)),
            (Int64(l), Int64(r)) => Int64(l.wrapping_mul(r)),
            (NativeInt(l), Int32(r)) => NativeInt(l.wrapping_mul(r as isize)),
            (NativeInt(l), NativeInt(r)) => NativeInt(l.wrapping_mul(r)),
            (NativeFloat(l), NativeFloat(r)) => NativeFloat(l * r),
            (l, r) => panic!("invalid types for mul: {:?}, {:?}", l, r),
        }
    }
}

impl<'gc> BitAnd for StackValue<'gc> {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        use StackValue::*;
        match (self, rhs) {
            (Int32(l), Int32(r)) => Int32(l & r),
            (Int32(l), NativeInt(r)) => NativeInt((l as isize) & r),
            (Int64(l), Int64(r)) => Int64(l & r),
            (NativeInt(l), Int32(r)) => NativeInt(l & (r as isize)),
            (NativeInt(l), NativeInt(r)) => NativeInt(l & r),
            (l, r) => panic!("invalid types for and: {:?}, {:?}", l, r),
        }
    }
}

impl<'gc> BitOr for StackValue<'gc> {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        use StackValue::*;
        match (self, rhs) {
            (Int32(l), Int32(r)) => Int32(l | r),
            (Int32(l), NativeInt(r)) => NativeInt((l as isize) | r),
            (Int64(l), Int64(r)) => Int64(l | r),
            (NativeInt(l), Int32(r)) => NativeInt(l | (r as isize)),
            (NativeInt(l), NativeInt(r)) => NativeInt(l | r),
            (l, r) => panic!("invalid types for or: {:?}, {:?}", l, r),
        }
    }
}

impl<'gc> BitXor for StackValue<'gc> {
    type Output = Self;
    fn bitxor(self, rhs: Self) -> Self::Output {
        use StackValue::*;
        match (self, rhs) {
            (Int32(l), Int32(r)) => Int32(l ^ r),
            (Int32(l), NativeInt(r)) => NativeInt((l as isize) ^ r),
            (Int64(l), Int64(r)) => Int64(l ^ r),
            (NativeInt(l), Int32(r)) => NativeInt(l ^ (r as isize)),
            (NativeInt(l), NativeInt(r)) => NativeInt(l ^ r),
            (l, r) => panic!("invalid types for xor: {:?}, {:?}", l, r),
        }
    }
}

impl<'gc> Shl for StackValue<'gc> {
    type Output = Self;
    fn shl(self, rhs: Self) -> Self::Output {
        use StackValue::*;
        let amount = match rhs {
            Int32(i) => i as u32,
            NativeInt(i) => i as u32,
            _ => panic!("invalid shift amount"),
        };
        match self {
            Int32(i) => Int32(i << amount),
            Int64(i) => Int64(i << amount),
            NativeInt(i) => NativeInt(i << amount),
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
