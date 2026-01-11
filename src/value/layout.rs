use crate::{
    types::{
        generics::{ConcreteType, GenericLookup},
        members::FieldDescription,
        TypeDescription,
    },
    value::object::ObjectRef,
    vm::{context::ResolutionContext, metrics::RuntimeMetrics, sync::Arc},
};
use dotnetdll::prelude::*;
use enum_dispatch::enum_dispatch;
use gc_arena::{Collect, Collection};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

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
        matches!(
            self,
            LayoutManager::Scalar(Scalar::ObjectRef) | LayoutManager::Scalar(Scalar::ManagedPtr)
        )
    }

    pub fn is_or_contains_refs(&self) -> bool {
        match self {
            LayoutManager::FieldLayoutManager(f) => {
                f.fields.values().any(|f| f.layout.is_or_contains_refs())
            }
            LayoutManager::ArrayLayoutManager(a) => a.element_layout.is_or_contains_refs(),
            LayoutManager::Scalar(Scalar::ObjectRef)
            | LayoutManager::Scalar(Scalar::ManagedPtr) => true,
            _ => false,
        }
    }

    pub fn type_tag(&self) -> &'static str {
        match &self {
            LayoutManager::FieldLayoutManager(_) => "struct",
            LayoutManager::ArrayLayoutManager(_) => "arr",
            LayoutManager::Scalar(s) => match s {
                Scalar::ObjectRef => "obj",
                Scalar::ManagedPtr => "ptr",
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

    pub fn alignment(&self) -> usize {
        match self {
            LayoutManager::FieldLayoutManager(f) => f.alignment,
            LayoutManager::ArrayLayoutManager(a) => a.element_layout.alignment(),
            LayoutManager::Scalar(s) => s.alignment(),
        }
    }

    pub fn trace(&self, storage: &[u8], cc: &Collection) {
        match self {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                ObjectRef::read(storage).trace(cc);
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // NOTE: ManagedPtr in memory is now pointer-sized (8 bytes).
                // The GC metadata (owner handle) is stored in the Object's side-table.
                // Tracing of managed pointer owners is handled by Object::trace,
                // which has access to the side-table. Here we do nothing since
                // the raw pointer value doesn't need tracing.
            }
            LayoutManager::FieldLayoutManager(f) => {
                for field in f.fields.values() {
                    field.layout.trace(&storage[field.position..], cc);
                }
            }
            LayoutManager::ArrayLayoutManager(a) => {
                let elem_size = a.element_layout.size();
                for i in 0..a.length {
                    a.element_layout.trace(&storage[i * elem_size..], cc);
                }
            }
            _ => {}
        }
    }

    pub fn resurrect<'gc>(
        &self,
        storage: &[u8],
        fc: &gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
    ) {
        match self {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                ObjectRef::read(storage).resurrect(fc, visited);
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // NOTE: ManagedPtr resurrection is handled by Object::resurrect
                // which has access to the side-table containing the owner handles.
            }
            LayoutManager::FieldLayoutManager(f) => {
                for field in f.fields.values() {
                    field
                        .layout
                        .resurrect(&storage[field.position..], fc, visited);
                }
            }
            LayoutManager::ArrayLayoutManager(a) => {
                let elem_size = a.element_layout.size();
                for i in 0..a.length {
                    a.element_layout
                        .resurrect(&storage[i * elem_size..], fc, visited);
                }
            }
            _ => {}
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
    pub layout: Arc<LayoutManager>,
}
impl FieldLayout {
    pub fn as_range(&self) -> Range<usize> {
        self.position..self.position + self.layout.size()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldKey {
    pub owner: TypeDescription,
    pub name: String,
}

impl std::fmt::Display for FieldKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldLayoutManager {
    pub fields: HashMap<FieldKey, FieldLayout>,
    pub total_size: usize,
    pub alignment: usize,
}
impl HasLayout for FieldLayoutManager {
    fn size(&self) -> usize {
        self.total_size
    }
}
impl FieldLayoutManager {
    pub fn trace(&self, storage: &[u8], cc: &Collection) {
        for field in self.fields.values() {
            field.layout.trace(&storage[field.position..], cc);
        }
    }

    pub fn resurrect<'gc>(
        &self,
        storage: &[u8],
        fc: &gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
    ) {
        for field in self.fields.values() {
            field
                .layout
                .resurrect(&storage[field.position..], fc, visited);
        }
    }

    fn new<'a>(
        fields: impl IntoIterator<Item = (TypeDescription, &'a str, Option<usize>, Arc<LayoutManager>)>,
        layout: Layout,
        base_size: usize,
        base_alignment: usize,
    ) -> Self {
        let mut mapping = HashMap::new();
        let total_size;
        let mut max_alignment = base_alignment.max(1);

        let fields: Vec<_> = fields.into_iter().collect();

        match layout {
            Layout::Automatic => {
                // For automatic layout, we can optimize by reordering fields
                // Start after base class fields
                let mut offset = base_size;

                // Sort fields by alignment (largest first) to minimize padding
                let mut sorted_fields = fields;
                sorted_fields
                    .sort_by_key(|(_, _, _, layout)| std::cmp::Reverse(layout.alignment()));

                for (owner, name, _, layout) in sorted_fields {
                    let field_align = layout.alignment();
                    max_alignment = max_alignment.max(field_align);

                    let aligned_offset = align_up(offset, field_align);
                    mapping.insert(
                        FieldKey {
                            owner,
                            name: name.to_string(),
                        },
                        FieldLayout {
                            position: aligned_offset,
                            layout: layout.clone(),
                        },
                    );
                    offset = aligned_offset + layout.size();
                }

                total_size = align_up(offset, max_alignment);
            }
            Layout::Sequential(s) => {
                let (packing_size, class_size) = match s {
                    None => (8, 0),
                    Some(SequentialLayout {
                        packing_size,
                        class_size,
                    }) => (if packing_size == 0 { 8 } else { packing_size }, class_size),
                };

                // Start after base class fields
                let mut offset = base_size;

                for (owner, name, _, layout) in fields {
                    let field_align = layout.alignment().min(packing_size);
                    max_alignment = max_alignment.max(field_align);

                    let aligned_offset = align_up(offset, field_align);
                    mapping.insert(
                        FieldKey {
                            owner,
                            name: name.to_string(),
                        },
                        FieldLayout {
                            position: aligned_offset,
                            layout: layout.clone(),
                        },
                    );
                    offset = aligned_offset + layout.size();
                }

                total_size = align_up(offset, max_alignment).max(class_size);
            }
            Layout::Explicit(e) => {
                // For explicit layout, offsets are relative to the current type's fields
                // We need to add base_size to these offsets
                let mut offset = base_size;
                for (owner, name, o, layout) in fields {
                    max_alignment = max_alignment.max(layout.alignment());
                    match o {
                        None => panic!(
                            "explicit field layout requires all fields to have defined offsets"
                        ),
                        Some(o) => {
                            // Add base size to the explicit offset
                            let actual_offset = base_size + o;
                            mapping.insert(
                                FieldKey {
                                    owner,
                                    name: name.to_string(),
                                },
                                FieldLayout {
                                    position: actual_offset,
                                    layout: layout.clone(),
                                },
                            );
                            offset = offset.max(actual_offset + layout.size());
                        }
                    };
                }
                total_size = match e {
                    Some(ExplicitLayout { class_size }) => class_size.max(offset),
                    None => offset,
                };
            }
        }

        if total_size > 0x1000_0000 {
            panic!("massive field layout detected: {} bytes", total_size);
        }

        Self {
            fields: mapping,
            total_size,
            alignment: max_alignment,
        }
    }

    fn collect_fields(
        td: TypeDescription,
        context: &ResolutionContext,
        predicate: &mut dyn FnMut(&Field) -> bool,
        metrics: Option<&RuntimeMetrics>,
    ) -> Self {
        let ancestors: Vec<_> = context.get_ancestors(td).collect();

        // Build layout hierarchically: first compute base class layout, then add derived fields
        let base_layout = if ancestors.len() > 1 {
            // We have a base class - compute its layout first
            let (base_td, base_generics) = &ancestors[1];
            let base_res = base_td.resolution;

            let new_lookup = GenericLookup::new(
                base_generics
                    .iter()
                    .map(|t| context.make_concrete(*t))
                    .collect(),
            );
            let base_ctx = ResolutionContext {
                generics: &new_lookup,
                resolution: base_res,
                loader: context.loader,
                type_owner: Some(*base_td),
                method_owner: None,
                shared: context.shared.clone(),
            };

            // Recursively compute base layout
            Some(Self::collect_fields(
                *base_td, &base_ctx, predicate, metrics,
            ))
        } else {
            None
        };

        let (base_size, base_alignment) = base_layout
            .as_ref()
            .map(|l| (l.total_size, l.alignment))
            .unwrap_or((0, 1));

        // Now lay out this type's fields on top of the base
        let mut current_type_fields = vec![];

        for f in &td.definition().fields {
            if !predicate(f) {
                continue;
            }

            let t = context.get_field_type(FieldDescription {
                parent: td,
                field_resolution: td.resolution,
                field: f,
            });
            let layout = type_layout_with_metrics(t, context, metrics);
            current_type_fields.push((td, f.name.as_ref(), f.offset, layout));
        }

        // Create layout with hierarchical approach
        let mut result = Self::new(
            current_type_fields,
            td.definition().flags.layout,
            base_size,
            base_alignment,
        );

        // Add all base class fields to the result
        if let Some(base_layout) = base_layout {
            // Merge base fields into result
            for (key, field_layout) in base_layout.fields {
                result.fields.insert(key, field_layout);
            }
        }

        result
    }

    pub fn instance_fields(td: TypeDescription, context: &ResolutionContext) -> Self {
        Self::instance_fields_with_metrics(td, context, None)
    }

    pub fn instance_field_layout_cached(
        td: TypeDescription,
        context: &ResolutionContext,
        metrics: Option<&RuntimeMetrics>,
    ) -> Arc<Self> {
        let key = (td, context.generics.clone());

        if let Some(cached) = context.shared.instance_field_layout_cache.get(&key) {
            return Arc::clone(&cached);
        }

        if let Some(m) = metrics {
            m.record_instance_field_layout_cache_miss();
        }
        let result = Arc::new(Self::instance_fields_with_metrics(td, context, metrics));
        context
            .shared
            .instance_field_layout_cache
            .insert(key, Arc::clone(&result));
        result
    }

    pub fn instance_fields_with_metrics(
        td: TypeDescription,
        context: &ResolutionContext,
        metrics: Option<&RuntimeMetrics>,
    ) -> Self {
        let context = context.for_type(td);
        Self::collect_fields(td, &context, &mut |f| !f.static_member, metrics)
    }

    pub fn static_fields(td: TypeDescription, context: &ResolutionContext) -> Self {
        Self::static_fields_with_metrics(td, context, None)
    }

    pub fn static_fields_with_metrics(
        td: TypeDescription,
        context: &ResolutionContext,
        metrics: Option<&RuntimeMetrics>,
    ) -> Self {
        let context = context.for_type(td);
        Self::collect_fields(td, &context, &mut |f| f.static_member, metrics)
    }

    /// Get a field's layout by owner and name
    pub fn get_field(&self, owner: TypeDescription, name: &str) -> Option<&FieldLayout> {
        self.fields.get(&FieldKey {
            owner,
            name: name.to_string(),
        })
    }

    /// Get a field's layout by name, searching through all types
    /// This is a fallback for cases where we don't know the exact owner
    pub fn get_field_by_name(&self, name: &str) -> Option<&FieldLayout> {
        self.fields
            .iter()
            .find(|(k, _)| k.name == name)
            .map(|(_, v)| v)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArrayLayoutManager {
    pub element_layout: Arc<LayoutManager>,
    pub length: usize,
}
impl HasLayout for ArrayLayoutManager {
    fn size(&self) -> usize {
        self.element_layout.size() * self.length
    }
}
impl ArrayLayoutManager {
    pub fn resurrect<'gc>(
        &self,
        storage: &[u8],
        fc: &gc_arena::Finalization<'gc>,
        visited: &mut HashSet<usize>,
    ) {
        let elem_size = self.element_layout.size();
        for i in 0..self.length {
            self.element_layout
                .resurrect(&storage[i * elem_size..], fc, visited);
        }
    }
    pub fn new(element: ConcreteType, length: usize, context: &ResolutionContext) -> Self {
        let element_layout = type_layout(element, context);
        // Defensive check for massive allocations (e.g. from corrupted metadata/stack)
        // Limit to ~1GB elements or overflow of usize
        if length > 0x4000_0000 || (length > 0 && element_layout.size() > usize::MAX / length) {
            panic!(
                "massive array allocation attempt: length={}, element_size={}",
                length,
                element_layout.size()
            );
        }
        Self {
            element_layout,
            length,
        }
    }

    pub fn new_with_metrics(
        element: ConcreteType,
        length: usize,
        context: &ResolutionContext,
        metrics: Option<&RuntimeMetrics>,
    ) -> Self {
        let element_layout = type_layout_with_metrics(element, context, metrics);
        // Defensive check for massive allocations (e.g. from corrupted metadata/stack)
        if length > 0x4000_0000 || (length > 0 && element_layout.size() > usize::MAX / length) {
            panic!(
                "massive array allocation attempt: length={}, element_size={}",
                length,
                element_layout.size()
            );
        }
        Self {
            element_layout,
            length,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    ObjectRef,
    ManagedPtr,
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
            Scalar::ObjectRef | Scalar::NativeInt => ObjectRef::SIZE,
            // ManagedPtr is pointer-sized in memory (metadata stored in side-table)
            Scalar::ManagedPtr => ObjectRef::SIZE,
            Scalar::Float32 => 4,
            Scalar::Float64 => 8,
        }
    }
}
impl Scalar {
    pub fn alignment(&self) -> usize {
        self.size()
    }
}

pub fn type_layout(t: ConcreteType, context: &ResolutionContext) -> Arc<LayoutManager> {
    type_layout_with_metrics(t, context, None)
}

pub fn type_layout_with_metrics(
    t: ConcreteType,
    context: &ResolutionContext,
    metrics: Option<&RuntimeMetrics>,
) -> Arc<LayoutManager> {
    let t = context.normalize_type(t);

    if let Some(cached) = context.shared.layout_cache.get(&t) {
        return Arc::clone(&cached);
    }

    if let Some(m) = metrics {
        m.record_layout_cache_miss();
    }
    let result = Arc::new(type_layout_internal(t.clone(), context, metrics));
    context.shared.layout_cache.insert(t, Arc::clone(&result));
    result
}

fn type_layout_internal(
    t: ConcreteType,
    context: &ResolutionContext,
    metrics: Option<&RuntimeMetrics>,
) -> LayoutManager {
    let mut context = context.clone();
    context.resolution = t.resolution();
    match t.get() {
        BaseType::Boolean | BaseType::Int8 | BaseType::UInt8 => Scalar::Int8.into(),
        BaseType::Char | BaseType::Int16 | BaseType::UInt16 => Scalar::Int16.into(),
        BaseType::Int32 | BaseType::UInt32 => Scalar::Int32.into(),
        BaseType::Int64 | BaseType::UInt64 => Scalar::Int64.into(),
        BaseType::IntPtr | BaseType::UIntPtr | BaseType::FunctionPointer(_) => {
            Scalar::NativeInt.into()
        }
        BaseType::ValuePointer(_, _) => Scalar::ManagedPtr.into(),
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
            let ctx = context.with_generics(&new_lookup);

            if let Some(inner) = t.is_enum() {
                (*type_layout_with_metrics(ctx.make_concrete(inner), &ctx, metrics)).clone()
            } else {
                FieldLayoutManager::instance_fields_with_metrics(t, &ctx, metrics).into()
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
