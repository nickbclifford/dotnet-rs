use std::collections::HashMap;
use std::mem::size_of;

use dotnetdll::prelude::*;
use enum_dispatch::enum_dispatch;

use super::{ConcreteType, Context, TypeDescription};

#[enum_dispatch]
pub trait HasLayout {
    fn size(&self) -> usize;
}

#[enum_dispatch(HasLayout)]
#[derive(Clone, Debug)]
pub enum LayoutManager {
    ClassLayoutManager,
    ArrayLayoutManager,
    Scalar,
}
impl LayoutManager {
    pub fn is_gc_ptr(&self) -> bool {
        match self {
            LayoutManager::Scalar(Scalar::ObjectRef) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FieldLayout {
    pub position: usize,
    pub layout: LayoutManager,
}
#[derive(Clone, Debug)]
pub struct ClassLayoutManager {
    pub fields: HashMap<String, FieldLayout>,
}
impl HasLayout for ClassLayoutManager {
    fn size(&self) -> usize {
        self.fields.values().map(|f| f.layout.size()).sum()
    }
}
impl ClassLayoutManager {
    pub fn new(description: TypeDescription, context: Context) -> Self {
        let layout = description.0.flags.layout;
        let fields: Vec<_> = description
            .0
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.as_ref(),
                    f.offset,
                    type_layout(
                        context.generics.make_concrete(f.return_type.clone()),
                        context.clone(),
                    )
                    .size(),
                )
            })
            .collect();

        match layout {
            Layout::Automatic => {}
            Layout::Sequential(_) => {}
            Layout::Explicit(_) => {}
        }
        todo!()
    }

    pub fn field_offset(&self, name: &str) -> usize {
        match self.fields.get(name) {
            Some(l) => l.position,
            None => todo!("field not present in class"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ArrayLayoutManager {
    pub element_layout: Box<LayoutManager>,
    pub length: usize,
}
impl HasLayout for ArrayLayoutManager {
    fn size(&self) -> usize {
        self.element_layout.size() * self.length
    }
}
impl ArrayLayoutManager {
    pub fn new(element: ConcreteType, length: usize, context: Context) -> Self {
        Self {
            element_layout: Box::new(type_layout(element, context)),
            length,
        }
    }

    pub fn element_offset(&self, index: usize) -> usize {
        self.element_layout.size() * index
    }
}

#[derive(Clone, Debug)]
pub enum Scalar {
    ObjectRef,
    Int8,
    Int16,
    Int32,
    Int64,
    NativeInt,
    Float32,
    Float64,
}
impl HasLayout for Scalar {
    fn size(&self) -> usize {
        match self {
            Scalar::Int8 => 1,
            Scalar::Int16 => 2,
            Scalar::Int32 => 4,
            Scalar::Int64 => 8,
            Scalar::ObjectRef | Scalar::NativeInt => size_of::<usize>(),
            Scalar::Float32 => 4,
            Scalar::Float64 => 8,
        }
    }
}

pub fn type_layout(t: ConcreteType, mut context: Context) -> LayoutManager {
    match t.get() {
        BaseType::Boolean | BaseType::Int8 | BaseType::UInt8 => Scalar::Int8.into(),
        BaseType::Char | BaseType::Int16 | BaseType::UInt16 => Scalar::Int16.into(),
        BaseType::Int32 | BaseType::UInt32 => Scalar::Int32.into(),
        BaseType::Int64 | BaseType::UInt64 => Scalar::Int64.into(),
        BaseType::IntPtr
        | BaseType::UIntPtr
        | BaseType::ValuePointer(_, _)
        | BaseType::FunctionPointer(_) => Scalar::NativeInt.into(),
        BaseType::Float32 => Scalar::Float32.into(),
        BaseType::Float64 => Scalar::Float64.into(),
        BaseType::Type {
            value_kind: Some(ValueKind::ValueType),
            source,
        } => {
            let (_, t) = match source {
                TypeSource::User(u) => context.locate_type(*u),
                TypeSource::Generic { base, parameters } => {
                    // TODO: new generic lookup context
                    context.locate_type(*base)
                }
            };
            ClassLayoutManager::new(t, context).into()
        }
        BaseType::Type { .. }
        | BaseType::Object
        | BaseType::String
        | BaseType::Vector(_, _)
        | BaseType::Array(_, _) => Scalar::ObjectRef.into(),
        // note that arrays with specified sizes are still objects, not value types
    }
}
