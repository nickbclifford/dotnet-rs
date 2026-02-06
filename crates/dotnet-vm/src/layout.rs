use crate::{context::ResolutionContext, metrics::RuntimeMetrics, sync::Arc};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::FieldDescription,
};
use dotnet_value::layout::{
    ArrayLayoutManager, FieldKey, FieldLayout, FieldLayoutManager, GcDesc, HasLayout,
    LayoutManager, Scalar, align_up,
};
use dotnetdll::prelude::*;
use tracing::trace;

pub struct LayoutFactory;

impl LayoutFactory {
    fn populate_gc_desc(layout: &LayoutManager, base_offset: usize, desc: &mut GcDesc) {
        let ptr_size = size_of::<usize>();
        match layout {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                desc.set(base_offset / ptr_size);
            }
            LayoutManager::Scalar(Scalar::ManagedPtr) => {
                // ManagedPtr is (Owner, Offset). The first word is the ObjectRef.
                // The pointer is recomputed from owner + offset on each read.
                desc.set(base_offset / ptr_size);
            }
            LayoutManager::Field(m) => {
                let offset_words = base_offset / ptr_size;
                for word_idx in m.gc_desc.bitmap.iter_ones() {
                    desc.set(offset_words + word_idx);
                }
            }
            LayoutManager::Array(arr) => {
                let elem_size = arr.element_layout.size();
                // Optimization: if element has no refs, skip
                if arr.element_layout.is_or_contains_refs() {
                    for i in 0..arr.length {
                        Self::populate_gc_desc(
                            &arr.element_layout,
                            base_offset + i * elem_size,
                            desc,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn create_field_layout<'a>(
        fields: impl IntoIterator<Item = (TypeDescription, &'a str, Option<usize>, Arc<LayoutManager>)>,
        layout: Layout,
        base_size: usize,
        base_alignment: usize,
    ) -> FieldLayoutManager {
        let mut mapping = std::collections::HashMap::new();
        let mut gc_desc = GcDesc::default();
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

                    Self::populate_gc_desc(&layout, aligned_offset, &mut gc_desc);

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

                    Self::populate_gc_desc(&layout, aligned_offset, &mut gc_desc);

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
                // Keep track of fields to check for overlaps: (start, end, layout)
                let mut placed_fields: Vec<(usize, usize, Arc<LayoutManager>)> = Vec::new();

                for (owner, name, o, layout) in fields {
                    max_alignment = max_alignment.max(layout.alignment());
                    match o {
                        None => panic!(
                            "explicit field layout requires all fields to have defined offsets"
                        ),
                        Some(o) => {
                            // Add base size to the explicit offset
                            let actual_offset = base_size + o;
                            let size = layout.size();
                            let actual_end = actual_offset + size;

                            // Validation: Check for overlaps with existing fields if refs are involved
                            for (prev_start, prev_end, prev_layout) in &placed_fields {
                                let overlap = actual_offset < *prev_end && *prev_start < actual_end;
                                if overlap
                                    && (layout.is_or_contains_refs()
                                        || prev_layout.is_or_contains_refs())
                                {
                                    panic!("explicit field layout overlaps reference type fields");
                                }
                            }
                            placed_fields.push((actual_offset, actual_end, layout.clone()));

                            Self::populate_gc_desc(&layout, actual_offset, &mut gc_desc);

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
                            offset = offset.max(actual_end);
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

        FieldLayoutManager {
            fields: mapping,
            total_size,
            alignment: max_alignment,
            gc_desc,
        }
    }

    fn collect_fields(
        td: TypeDescription,
        context: &ResolutionContext,
        predicate: &mut dyn FnMut(&Field) -> bool,
        metrics: Option<&RuntimeMetrics>,
        include_base: bool,
    ) -> FieldLayoutManager {
        let ancestors: Vec<_> = context.get_ancestors(td).collect();

        // Build layout hierarchically: first compute base class layout, then add derived fields
        let base_layout = if include_base && ancestors.len() > 1 {
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
                caches: context.caches.clone(),
                shared: context.shared.clone(),
            };

            // Recursively compute base layout
            Some(Self::collect_fields(
                *base_td,
                &base_ctx,
                predicate,
                metrics,
                include_base,
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

            let layout = if f.by_ref {
                Arc::new(Scalar::ManagedPtr.into())
            } else {
                let t = context.get_field_type(FieldDescription {
                    parent: td,
                    field_resolution: td.resolution,
                    field: f,
                });
                type_layout_with_metrics(t, context, metrics)
            };

            current_type_fields.push((td, f.name.as_ref(), f.offset, layout));
        }

        // Create layout with hierarchical approach
        let mut result = Self::create_field_layout(
            current_type_fields,
            td.definition().flags.layout,
            base_size,
            base_alignment,
        );

        // Add all base class fields to the result
        if let Some(base_layout) = base_layout {
            // Merge base fields into result
            for (key, field_layout) in base_layout.fields.clone() {
                result.fields.insert(key, field_layout);
            }
            result.gc_desc.merge(&base_layout.gc_desc);
        }

        result
    }

    pub fn instance_fields(td: TypeDescription, context: &ResolutionContext) -> FieldLayoutManager {
        Self::instance_fields_with_metrics(td, context, None)
    }

    pub fn instance_field_layout_cached(
        td: TypeDescription,
        context: &ResolutionContext,
        metrics: Option<&RuntimeMetrics>,
    ) -> Arc<FieldLayoutManager> {
        let key = (td, context.generics.clone());

        if let Some(cached) = context.caches.instance_field_layout_cache.get(&key) {
            if let Some(m) = metrics {
                m.record_instance_field_layout_cache_hit();
            }
            return Arc::clone(&cached);
        }

        if let Some(m) = metrics {
            m.record_instance_field_layout_cache_miss();
        }
        let result = Arc::new(Self::instance_fields_with_metrics(td, context, metrics));
        context
            .caches
            .instance_field_layout_cache
            .insert(key, Arc::clone(&result));
        result
    }

    pub fn instance_fields_with_metrics(
        td: TypeDescription,
        context: &ResolutionContext,
        metrics: Option<&RuntimeMetrics>,
    ) -> FieldLayoutManager {
        let context = context.for_type(td);
        Self::collect_fields(td, &context, &mut |f| !f.static_member, metrics, true)
    }

    pub fn static_fields(td: TypeDescription, context: &ResolutionContext) -> FieldLayoutManager {
        Self::static_fields_with_metrics(td, context, None)
    }

    pub fn static_fields_with_metrics(
        td: TypeDescription,
        context: &ResolutionContext,
        metrics: Option<&RuntimeMetrics>,
    ) -> FieldLayoutManager {
        let context = context.for_type(td);
        Self::collect_fields(td, &context, &mut |f| f.static_member, metrics, false)
    }

    pub fn create_array_layout(
        element: ConcreteType,
        length: usize,
        context: &ResolutionContext,
    ) -> ArrayLayoutManager {
        Self::create_array_layout_with_metrics(element, length, context, None)
    }

    pub fn create_array_layout_with_metrics(
        element: ConcreteType,
        length: usize,
        context: &ResolutionContext,
        metrics: Option<&RuntimeMetrics>,
    ) -> ArrayLayoutManager {
        let element_layout = type_layout_with_metrics(element, context, metrics);
        // Defensive check for massive allocations (e.g. from corrupted metadata/stack)
        if length > 0x4000_0000 || (length > 0 && element_layout.size() > usize::MAX / length) {
            panic!(
                "massive array allocation attempt: length={}, element_size={}",
                length,
                element_layout.size()
            );
        }
        ArrayLayoutManager {
            element_layout,
            length,
        }
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

    if let Some(cached) = context.caches.layout_cache.get(&t) {
        if let Some(m) = metrics {
            m.record_layout_cache_hit();
        }
        return Arc::clone(&cached);
    }

    if let Some(m) = metrics {
        m.record_layout_cache_miss();
    }
    let result = Arc::new(type_layout_internal(t.clone(), context, metrics));
    context.caches.layout_cache.insert(t, Arc::clone(&result));
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
        BaseType::Boolean | BaseType::UInt8 => Scalar::UInt8.into(),
        BaseType::Int8 => Scalar::Int8.into(),
        BaseType::Char | BaseType::UInt16 => Scalar::UInt16.into(),
        BaseType::Int16 => Scalar::Int16.into(),
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

            // Intrinsic: System.ByReference<T> is a ManagedPtr
            let name = t.type_name();
            trace!("Computing layout for: {}", name);
            if name == "System.ByReference`1"
                || name == "System.ReadOnlyByReference`1"
                || name == "System.ByReference"
                || name == "System.Runtime.CompilerServices.ByReference`1"
                || name == "System.Runtime.CompilerServices.ReadOnlyByReference`1"
            {
                return Scalar::ManagedPtr.into();
            }

            let new_lookup = GenericLookup::new(type_generics);
            let ctx = context.with_generics(&new_lookup);

            if let Some(inner) = t.is_enum() {
                (*type_layout_with_metrics(ctx.make_concrete(inner), &ctx, metrics)).clone()
            } else {
                LayoutFactory::instance_fields_with_metrics(t, &ctx, metrics).into()
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
