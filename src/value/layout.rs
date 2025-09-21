use super::{ConcreteType, Context, FieldDescription, GenericLookup, TypeDescription};

use dotnetdll::prelude::*;
use enum_dispatch::enum_dispatch;
use std::{collections::HashMap, mem::size_of, ops::Range};

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
        matches!(self, LayoutManager::Scalar(Scalar::ObjectRef))
    }

    pub fn is_or_contains_refs(&self) -> bool {
        match self {
            LayoutManager::FieldLayoutManager(f) => {
                f.fields.values().any(|f| f.layout.is_or_contains_refs())
            }
            LayoutManager::ArrayLayoutManager(a) => a.element_layout.is_or_contains_refs(),
            LayoutManager::Scalar(Scalar::ObjectRef) => true,
            _ => false,
        }
    }

    pub fn type_tag(&self) -> &'static str {
        match &self {
            LayoutManager::FieldLayoutManager(_) => "struct",
            LayoutManager::ArrayLayoutManager(_) => "arr",
            LayoutManager::Scalar(s) => match s {
                Scalar::ObjectRef => "obj",
                Scalar::Int8 => "i8",
                Scalar::Int16 => "i16",
                Scalar::Int32 => "i32",
                Scalar::Int64 => "i64",
                Scalar::NativeInt => "ptr",
                Scalar::Float32 => "f32",
                Scalar::Float64 => "f64",
            },
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
impl FieldLayout {
    pub fn as_range(&self) -> Range<usize> {
        self.position..self.position + self.layout.size()
    }
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
        fields: impl IntoIterator<Item = (&'a str, Option<usize>, LayoutManager)>,
        layout: Layout,
    ) -> Self {
        let mut mapping = HashMap::new();
        let total_size;

        let fields: Vec<_> = fields.into_iter().collect();

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
                    mut packing_size,
                    class_size,
                }) => {
                    if packing_size == 0 {
                        packing_size = size_of::<usize>();
                    }

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

                    total_size = align_up(offset, usize::max(packing_size, class_size));
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

    fn collect_fields(
        td: TypeDescription,
        context: Context,
        mut predicate: impl FnMut(&Field) -> bool,
    ) -> Self {
        // TODO: check for layout flags all the way down the chain? (II.22.8)
        let mut ancestors: Vec<_> = context.get_ancestors(td).collect();
        ancestors.reverse();
        // remove current type, since it gets special treatment
        ancestors.pop();

        let mut total_fields = vec![];
        let mut add_field_layout = |f: FieldDescription, ctx: &Context| {
            let t = ctx.get_field_type(f);
            let layout = type_layout(t, ctx.clone());
            total_fields.push((f.field.name.as_ref(), f.field.offset, layout));
        };

        for (
            td @ TypeDescription {
                resolution: res,
                definition: a,
            },
            generic_params,
        ) in ancestors
        {
            let new_lookup = GenericLookup::new(
                generic_params
                    .into_iter()
                    .map(|t| context.make_concrete(t))
                    .collect(),
            );
            let new_ctx = Context {
                generics: &new_lookup,
                resolution: res,
                assemblies: context.assemblies,
            };
            for f in &a.fields {
                if !predicate(f) {
                    continue;
                }

                add_field_layout(
                    FieldDescription {
                        parent: td,
                        field: f,
                    },
                    &new_ctx,
                );
            }
        }

        // now for the type's actually declared fields
        for f in &td.definition.fields {
            if !predicate(f) {
                continue;
            }

            add_field_layout(
                FieldDescription {
                    parent: td,
                    field: f,
                },
                &context,
            );
        }

        Self::new(total_fields, td.definition.flags.layout)
    }

    pub fn instance_fields(td: TypeDescription, context: Context) -> Self {
        Self::collect_fields(td, context, |f| !f.static_member)
    }

    pub fn static_fields(td: TypeDescription, context: Context) -> Self {
        Self::collect_fields(td, context, |f| f.static_member)
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
            let mut type_generics = vec![];
            let t = match source {
                TypeSource::User(u) => context.locate_type(*u),
                TypeSource::Generic { base, parameters } => {
                    type_generics = parameters.clone();
                    context.locate_type(*base)
                }
            };

            let new_lookup = GenericLookup::new(type_generics);
            let ctx = Context::with_generics(context, &new_lookup);

            if let Some(inner) = t.is_enum() {
                type_layout(ctx.make_concrete(inner), ctx)
            } else {
                FieldLayoutManager::instance_fields(t, ctx).into()
            }
        }
        BaseType::Type { .. }
        | BaseType::Object
        | BaseType::String
        | BaseType::Vector(_, _)
        | BaseType::Array(_, _) => Scalar::ObjectRef.into(),
        // note that arrays with specified sizes are still objects, not value types
    }
}
