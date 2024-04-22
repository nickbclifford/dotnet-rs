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
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutManager {
    FieldLayoutManager,
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

fn align_up(value: usize, align: usize) -> usize {
    let misalignment = value % align;
    if misalignment == 0 {
        value
    } else {
        value + align - misalignment
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldLayout {
    pub position: usize,
    pub layout: LayoutManager,
}
#[derive(Clone, Debug, PartialEq)]
pub struct FieldLayoutManager {
    pub fields: HashMap<String, FieldLayout>,
    pub total_size: usize,
}
impl HasLayout for FieldLayoutManager {
    fn size(&self) -> usize {
        self.total_size
    }
}
impl FieldLayoutManager {
    fn new<'a>(
        fields: impl IntoIterator<Item = &'a Field<'static>>,
        layout: Layout,
        context: Context,
    ) -> Self {
        let mut mapping = HashMap::new();
        let total_size;

        let fields: Vec<_> = fields
            .into_iter()
            .map(|f| {
                (
                    f.name.as_ref(),
                    f.offset,
                    type_layout(
                        context.generics.make_concrete(f.return_type.clone()),
                        context.clone(),
                    ),
                )
            })
            .collect();

        match layout {
            Layout::Automatic => {
                let mut offset = 0;

                for (name, _, layout) in fields {
                    let size = layout.size();
                    mapping.insert(
                        name.to_string(),
                        FieldLayout {
                            position: offset,
                            layout,
                        },
                    );
                    offset += align_up(size, size_of::<usize>());
                }

                total_size = offset;
            }
            Layout::Sequential(s) => match s {
                None => {
                    let mut offset = 0;

                    for (name, _, layout) in fields {
                        let size = layout.size();
                        mapping.insert(
                            name.to_string(),
                            FieldLayout {
                                position: offset,
                                layout,
                            },
                        );
                        offset += size;
                    }

                    total_size = offset;
                }
                Some(SequentialLayout {
                    packing_size,
                    class_size,
                }) => {
                    let mut offset = 0;

                    for (name, _, layout) in fields {
                        let size = layout.size();
                        let aligned_offset = align_up(offset, packing_size);
                        mapping.insert(
                            name.to_string(),
                            FieldLayout {
                                position: aligned_offset,
                                layout,
                            },
                        );
                        offset = aligned_offset + size;
                    }
                    total_size = align_up(offset, class_size);
                }
            },
            Layout::Explicit(e) => {
                for (name, offset, layout) in fields {
                    match offset {
                        None => panic!(
                            "explicit field layout requires all fields to have defined offsets"
                        ),
                        Some(o) => mapping.insert(
                            name.to_string(),
                            FieldLayout {
                                position: o,
                                layout,
                            },
                        ),
                    };
                }
                total_size = match e {
                    Some(ExplicitLayout { class_size }) => class_size,
                    None => match mapping.values().max_by_key(|l| l.position) {
                        None => 0,
                        Some(FieldLayout { position, layout }) => position + layout.size(),
                    },
                }
            }
        }

        Self {
            fields: mapping,
            total_size,
        }
    }

    pub fn instance_fields(
        TypeDescription(_, description): TypeDescription,
        context: Context,
    ) -> Self {
        Self::new(
            description.fields.iter().filter(|f| !f.static_member),
            description.flags.layout,
            context,
        )
    }

    pub fn static_fields(
        TypeDescription(_, description): TypeDescription,
        context: Context,
    ) -> Self {
        Self::new(
            description.fields.iter().filter(|f| f.static_member),
            description.flags.layout,
            context,
        )
    }

    pub fn field_offset(&self, name: &str) -> usize {
        match self.fields.get(name) {
            Some(l) => l.position,
            None => todo!("field not present in class"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

pub fn type_layout(t: ConcreteType, context: Context) -> LayoutManager {
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
            let t = match source {
                TypeSource::User(u) => context.locate_type(*u),
                TypeSource::Generic { base, parameters } => {
                    // TODO: new generic lookup context
                    context.locate_type(*base)
                }
            };
            FieldLayoutManager::instance_fields(t, context).into()
        }
        BaseType::Type { .. }
        | BaseType::Object
        | BaseType::String
        | BaseType::Vector(_, _)
        | BaseType::Array(_, _) => Scalar::ObjectRef.into(),
        // note that arrays with specified sizes are still objects, not value types
    }
}
