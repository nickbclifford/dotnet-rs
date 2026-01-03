use crate::{
    resolve::{Ancestor, Assemblies},
    utils::{decompose_type_source, ResolutionS},
    vm::GCHandle,
};

use description::{FieldDescription, MethodDescription, TypeDescription};
use dotnetdll::prelude::*;
use gc_arena::{lock::RefLock, Collect, Collection, Gc};
use std::{
    cmp::Ordering,
    collections::{HashSet, VecDeque},
    fmt::{Debug, Formatter},
    hash::Hash,
    ops::{Add, BitAnd, BitOr, BitXor, Mul, Neg, Not, Shl, Sub},
};

pub mod description;
pub mod layout;
pub mod object;
pub mod pointer;
pub mod storage;
pub mod string;

use object::{HeapStorage, Object, ObjectHandle, ObjectRef};
use pointer::{ManagedPtr, UnmanagedPtr};
use string::CLRString;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Collect)]
#[collect(require_static)]
pub enum GCHandleType {
    Weak = 0,
    WeakTrackResurrection = 1,
    Normal = 2,
    Pinned = 3,
}

impl From<i32> for GCHandleType {
    fn from(i: i32) -> Self {
        match i {
            0 => GCHandleType::Weak,
            1 => GCHandleType::WeakTrackResurrection,
            2 => GCHandleType::Normal,
            3 => GCHandleType::Pinned,
            _ => panic!("invalid GCHandleType: {}", i),
        }
    }
}

#[derive(Clone, Copy)]
pub struct ResolutionContext<'a> {
    pub generics: &'a GenericLookup,
    pub assemblies: &'a Assemblies,
    pub resolution: ResolutionS,
    pub type_owner: Option<TypeDescription>,
    pub method_owner: Option<MethodDescription>,
}
impl<'a> ResolutionContext<'a> {
    pub fn new(
        generics: &'a GenericLookup,
        assemblies: &'a Assemblies,
        resolution: ResolutionS,
    ) -> Self {
        Self {
            generics,
            assemblies,
            resolution,
            type_owner: None,
            method_owner: None,
        }
    }

    pub fn for_method(
        method: MethodDescription,
        assemblies: &'a Assemblies,
        generics: &'a GenericLookup,
    ) -> Self {
        Self {
            generics,
            assemblies,
            resolution: method.resolution(),
            type_owner: Some(method.parent),
            method_owner: Some(method),
        }
    }

    pub fn with_generics(&self, generics: &'a GenericLookup) -> ResolutionContext<'a> {
        ResolutionContext { generics, ..*self }
    }

    pub fn for_type(&self, td: TypeDescription) -> ResolutionContext<'a> {
        self.for_type_with_generics(td, self.generics)
    }

    pub fn for_type_with_generics(
        &self,
        td: TypeDescription,
        generics: &'a GenericLookup,
    ) -> ResolutionContext<'a> {
        ResolutionContext {
            resolution: td.resolution,
            generics,
            assemblies: self.assemblies,
            type_owner: Some(td),
            method_owner: None,
        }
    }

    pub fn locate_type(&self, handle: UserType) -> TypeDescription {
        self.assemblies.locate_type(self.resolution, handle)
    }

    pub fn locate_method(
        &self,
        handle: UserMethod,
        generic_inst: &GenericLookup,
    ) -> MethodDescription {
        self.assemblies
            .locate_method(self.resolution, handle, generic_inst)
    }

    pub fn locate_field(&self, field: FieldSource) -> (FieldDescription, GenericLookup) {
        self.assemblies
            .locate_field(self.resolution, field, self.generics)
    }

    pub fn get_ancestors(
        &self,
        child_type: TypeDescription,
    ) -> impl Iterator<Item = Ancestor<'a>> + 'a {
        self.assemblies.ancestors(child_type)
    }

    pub fn is_a(&self, value: TypeDescription, ancestor: TypeDescription) -> bool {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::new();

        for (a, _) in self.get_ancestors(value) {
            queue.push_back(a);
        }

        while let Some(current) = queue.pop_front() {
            if current == ancestor {
                return true;
            }
            if !seen.insert(current) {
                continue;
            }

            for (_, interface_source) in &current.definition.implements {
                let handle = match interface_source {
                    TypeSource::User(h) | TypeSource::Generic { base: h, .. } => *h,
                };
                let interface = self.assemblies.locate_type(current.resolution, handle);
                queue.push_back(interface);
            }
        }

        false
    }

    pub fn get_heap_description(&self, object: ObjectHandle) -> TypeDescription {
        use object::HeapStorage::*;
        match &*object.as_ref().borrow() {
            Obj(o) => o.description,
            Vec(_) => self.assemblies.corlib_type("System.Array"),
            Str(_) => self.assemblies.corlib_type("System.String"),
            Boxed(v) => v.description(self),
        }
    }

    pub fn make_concrete<T: Clone + Into<MethodType>>(&self, t: &T) -> ConcreteType {
        self.generics.make_concrete(self.resolution, t.clone())
    }

    pub fn get_field_type(&self, field: FieldDescription) -> ConcreteType {
        let return_type = &field.field.return_type;
        if field.field.by_ref {
            let by_ref_t: MemberType = BaseType::pointer(return_type.clone()).into();
            self.make_concrete(&by_ref_t)
        } else {
            self.make_concrete(return_type)
        }
    }

    pub fn get_field_desc(&self, field: FieldDescription) -> TypeDescription {
        self.assemblies
            .find_concrete_type(self.get_field_type(field))
    }

    pub fn normalize_type(&self, mut t: ConcreteType) -> ConcreteType {
        let (ut, res) = match t.get() {
            BaseType::Type { source, .. } => (decompose_type_source(source).0, t.resolution()),
            _ => return t,
        };

        let name = ut.type_name(res.0);
        let base = match name.as_ref() {
            "System.Boolean" => Some(BaseType::Boolean),
            "System.Char" => Some(BaseType::Char),
            "System.Byte" => Some(BaseType::UInt8),
            "System.SByte" => Some(BaseType::Int8),
            "System.Int16" => Some(BaseType::Int16),
            "System.UInt16" => Some(BaseType::UInt16),
            "System.Int32" => Some(BaseType::Int32),
            "System.UInt32" => Some(BaseType::UInt32),
            "System.Int64" => Some(BaseType::Int64),
            "System.UInt64" => Some(BaseType::UInt64),
            "System.Single" => Some(BaseType::Float32),
            "System.Double" => Some(BaseType::Float64),
            "System.IntPtr" => Some(BaseType::IntPtr),
            "System.UIntPtr" => Some(BaseType::UIntPtr),
            "System.Object" => Some(BaseType::Object),
            "System.String" => Some(BaseType::String),
            _ => None,
        };

        if let Some(base) = base {
            ConcreteType::new(res, base)
        } else {
            if let BaseType::Type { source, value_kind } = t.get_mut() {
                if value_kind.is_none() {
                    let (ut, _) = decompose_type_source(source);
                    let td = self.locate_type(ut);
                    if td.is_value_type(self) {
                        *value_kind = Some(ValueKind::ValueType);
                    }
                }
            }
            t
        }
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
            Self::Int32(_) => ctx.assemblies.corlib_type("System.Int32"),
            Self::Int64(_) => ctx.assemblies.corlib_type("System.Int64"),
            Self::NativeInt(_) | Self::UnmanagedPtr(_) => {
                ctx.assemblies.corlib_type("System.IntPtr")
            }
            Self::NativeFloat(_) => ctx.assemblies.corlib_type("System.Double"),
            Self::ObjectRef(ObjectRef(Some(o))) => ctx.get_heap_description(*o),
            Self::ObjectRef(ObjectRef(None)) => ctx.assemblies.corlib_type("System.Object"),
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

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ConcreteType {
    source: ResolutionS,
    base: Box<BaseType<Self>>,
}
unsafe impl Collect for ConcreteType {
    fn trace(&self, _cc: &Collection) {}
}
impl From<TypeDescription> for ConcreteType {
    fn from(td: TypeDescription) -> Self {
        let index = td
            .resolution
            .0
            .type_definitions
            .iter()
            .position(|t| std::ptr::eq(t, td.definition))
            .expect("TypeDescription has invalid definition pointer");
        Self::new(
            td.resolution,
            BaseType::Type {
                source: TypeSource::User(UserType::Definition(
                    td.resolution.0.type_definition_index(index).unwrap(),
                )),
                value_kind: None,
            },
        )
    }
}

impl ConcreteType {
    pub fn new(source: ResolutionS, base: BaseType<Self>) -> Self {
        ConcreteType {
            source,
            base: Box::new(base),
        }
    }

    pub fn get(&self) -> &BaseType<Self> {
        &self.base
    }

    pub fn get_mut(&mut self) -> &mut BaseType<Self> {
        &mut self.base
    }

    pub fn resolution(&self) -> ResolutionS {
        self.source
    }
}
impl Debug for ConcreteType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get().show(self.source.0))
    }
}
impl ResolvedDebug for ConcreteType {
    fn show(&self, _res: &Resolution) -> String {
        format!("{:?}", self)
    }
}

#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct GenericLookup {
    pub type_generics: Vec<ConcreteType>,
    pub method_generics: Vec<ConcreteType>,
}
unsafe impl Collect for GenericLookup {
    fn trace(&self, cc: &Collection) {
        self.type_generics.trace(cc);
        self.method_generics.trace(cc);
    }
}
impl GenericLookup {
    pub fn new(type_generics: Vec<ConcreteType>) -> Self {
        Self {
            type_generics,
            method_generics: vec![],
        }
    }

    pub fn make_concrete(&self, res: ResolutionS, t: impl Into<MethodType>) -> ConcreteType {
        match t.into() {
            MethodType::Base(b) => ConcreteType::new(res, b.map(|t| self.make_concrete(res, t))),
            MethodType::TypeGeneric(i) => self.type_generics[i].clone(),
            MethodType::MethodGeneric(i) => self.method_generics[i].clone(),
        }
    }
}
impl Debug for GenericLookup {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        struct GenericIndexFormatter(char, usize);
        impl Debug for GenericIndexFormatter {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}{}", self.0, self.1)
            }
        }

        f.debug_map()
            .entries(
                self.type_generics
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (GenericIndexFormatter('T', i), t)),
            )
            .entries(
                self.method_generics
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (GenericIndexFormatter('M', i), t)),
            )
            .finish()
    }
}
