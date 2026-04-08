use crate::{ResolverExecutionContext, ResolverService};
use dotnet_types::{
    TypeDescription,
    error::TypeResolutionError,
    generics::{ConcreteType, GenericLookup},
    members::FieldDescription,
    resolution::ResolutionS,
};
use dotnet_value::{
    ByteOffset,
    layout::{
        ArrayLayoutManager, FieldKey, FieldLayout, FieldLayoutManager, GcDesc, HasLayout,
        LayoutManager, Scalar, align_up,
    },
    object::ObjectRef,
};
use dotnetdll::prelude::*;
use std::sync::Arc;
use tracing::trace;

pub struct LayoutFactory;

impl LayoutFactory {
    pub fn populate_gc_desc(layout: &LayoutManager, base_offset: usize, desc: &mut GcDesc) {
        let ptr_size = ObjectRef::SIZE;
        match layout {
            LayoutManager::Scalar(Scalar::ObjectRef) => {
                desc.set_offset(base_offset);
            }
            LayoutManager::Field(m) => {
                for word_idx in m.gc_desc.bitmap.iter_ones() {
                    desc.set_offset(base_offset + (word_idx * ptr_size));
                }
                for offset in &m.gc_desc.unaligned_offsets {
                    desc.set_offset(base_offset + *offset);
                }
            }
            LayoutManager::Array(arr) => {
                let elem_size = arr.element_layout.size();
                if arr.element_layout.is_or_contains_refs() {
                    for i in 0..arr.length {
                        Self::populate_gc_desc(
                            &arr.element_layout,
                            base_offset + (elem_size * i).as_usize(),
                            desc,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    pub fn create_field_layout<'a>(
        fields: impl IntoIterator<Item = (TypeDescription, &'a str, Option<usize>, Arc<LayoutManager>)>,
        layout: Layout,
        base_size: usize,
        base_alignment: usize,
    ) -> Result<FieldLayoutManager, TypeResolutionError> {
        let mut mapping = std::collections::HashMap::new();
        let mut gc_desc = GcDesc::default();
        let mut has_ref_fields = false;
        let total_size;
        let mut max_alignment = base_alignment.max(1);

        let fields: Vec<_> = fields.into_iter().collect();

        match layout {
            Layout::Automatic => {
                let mut offset = base_size;

                for (owner, name, _, layout) in fields {
                    let field_align = layout.alignment();
                    max_alignment = max_alignment.max(field_align);

                    let aligned_offset = align_up(offset, field_align);

                    Self::populate_gc_desc(&layout, aligned_offset, &mut gc_desc);
                    if layout.has_managed_ptrs() {
                        has_ref_fields = true;
                    }

                    mapping.insert(
                        FieldKey {
                            owner,
                            name: name.to_string(),
                        },
                        FieldLayout {
                            position: ByteOffset(aligned_offset),
                            layout: layout.clone(),
                        },
                    );
                    offset = aligned_offset + layout.size().as_usize();
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

                let mut offset = base_size;

                for (owner, name, _, layout) in fields {
                    let field_align = layout.alignment().min(packing_size);
                    max_alignment = max_alignment.max(field_align);

                    let aligned_offset = align_up(offset, field_align);

                    Self::populate_gc_desc(&layout, aligned_offset, &mut gc_desc);
                    if layout.has_managed_ptrs() {
                        has_ref_fields = true;
                    }

                    mapping.insert(
                        FieldKey {
                            owner,
                            name: name.to_string(),
                        },
                        FieldLayout {
                            position: ByteOffset(aligned_offset),
                            layout: layout.clone(),
                        },
                    );
                    offset = aligned_offset + layout.size().as_usize();
                }

                total_size = align_up(offset, max_alignment).max(base_size + class_size);
            }
            Layout::Explicit(e) => {
                let mut offset = base_size;
                let mut placed_fields: Vec<(usize, usize, Arc<LayoutManager>)> = Vec::new();

                for (owner, name, o, layout) in fields {
                    max_alignment = max_alignment.max(layout.alignment());
                    if layout.has_managed_ptrs() {
                        has_ref_fields = true;
                    }
                    match o {
                        None => {
                            return Err(TypeResolutionError::InvalidLayout(
                                "explicit field layout requires all fields to have defined offsets"
                                    .to_string(),
                            ));
                        }
                        Some(o) => {
                            let actual_offset = base_size + o;
                            let size = layout.size();
                            let actual_end = actual_offset + size.as_usize();

                            for (prev_start, prev_end, prev_layout) in &placed_fields {
                                let overlap = actual_offset < *prev_end && *prev_start < actual_end;
                                if overlap
                                    && (layout.is_or_contains_refs()
                                        || prev_layout.is_or_contains_refs())
                                {
                                    let same_range =
                                        actual_offset == *prev_start && actual_end == *prev_end;
                                    let both_gc_ptrs =
                                        layout.is_gc_ptr() && prev_layout.is_gc_ptr();

                                    if same_range && both_gc_ptrs {
                                        continue;
                                    }

                                    let type_name = owner.type_name();
                                    if type_name
                                        .contains("System.Runtime.CompilerServices.MethodTable")
                                    {
                                        trace!(
                                            "WARNING: Explicit field layout overlaps reference type fields in type '{}'. Field '{}' (offset {}) overlaps with previous field (range {}-{}). Ignoring.",
                                            type_name, name, actual_offset, prev_start, prev_end
                                        );
                                    } else {
                                        return Err(TypeResolutionError::InvalidLayout(format!(
                                            "explicit field layout overlaps reference type fields in type '{}'. Field '{}' (offset {}) overlaps with previous field (range {}-{}).",
                                            type_name, name, actual_offset, prev_start, prev_end
                                        )));
                                    }
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
                                    position: ByteOffset(actual_offset),
                                    layout: layout.clone(),
                                },
                            );
                            offset = offset.max(actual_end);
                        }
                    };
                }
                total_size = match e {
                    Some(ExplicitLayout { class_size }) => (base_size + class_size).max(offset),
                    None => offset,
                };
            }
        }

        if total_size > 0x1000_0000 {
            return Err(TypeResolutionError::MassiveAllocation(format!(
                "massive field layout detected: {} bytes",
                total_size
            )));
        }

        Ok(FieldLayoutManager {
            fields: mapping,
            total_size,
            alignment: max_alignment,
            gc_desc,
            has_ref_fields,
        })
    }

    fn collect_fields<C, L>(
        resolver: &ResolverService<C, L>,
        td: TypeDescription,
        resolution: ResolutionS,
        generics: &GenericLookup,
        predicate: &mut dyn FnMut(&Field) -> bool,
        include_base: bool,
    ) -> Result<FieldLayoutManager, TypeResolutionError>
    where
        C: crate::ResolverCacheAdapter,
        L: crate::ResolverLayoutAdapter,
    {
        let ancestors: Vec<_> = resolver.loader.ancestors(td.clone()).collect();

        let base_layout = if include_base && ancestors.len() > 1 {
            let (base_td, base_generics) = &ancestors[1];
            let new_lookup = GenericLookup::new(
                base_generics
                    .iter()
                    .map(|t| resolver.make_concrete(resolution.clone(), generics, *t))
                    .collect::<Result<Vec<_>, _>>()?,
            );

            Some(Self::collect_fields(
                resolver,
                base_td.clone(),
                base_td.resolution.clone(),
                &new_lookup,
                predicate,
                include_base,
            )?)
        } else {
            None
        };

        let (base_size, base_alignment) = base_layout
            .as_ref()
            .map(|l| (l.total_size, l.alignment))
            .unwrap_or((0, 1));

        let mut current_type_fields = vec![];

        for (i, f) in td.definition().fields.iter().enumerate() {
            if !predicate(f) {
                continue;
            }

            let layout = if f.by_ref {
                Arc::new(Scalar::ManagedPtr.into())
            } else {
                let t = resolver.get_field_type(
                    resolution.clone(),
                    generics,
                    FieldDescription::new(td.clone(), td.resolution.clone(), i),
                )?;
                resolver.type_layout_cached_with_lookup(t, resolution.clone(), generics)?
            };

            current_type_fields.push((td.clone(), f.name.as_ref(), f.offset, layout));
        }

        let mut result = Self::create_field_layout(
            current_type_fields,
            td.definition().flags.layout,
            base_size,
            base_alignment,
        )?;

        if let Some(base_layout) = base_layout {
            for (key, field_layout) in base_layout.fields.clone() {
                result.fields.insert(key, field_layout);
            }
            result.gc_desc.merge(&base_layout.gc_desc);
            result.has_ref_fields |= base_layout.has_ref_fields;
        }

        Ok(result)
    }

    pub(crate) fn instance_fields_with_lookup<C, L>(
        resolver: &ResolverService<C, L>,
        td: TypeDescription,
        generics: &GenericLookup,
    ) -> Result<FieldLayoutManager, TypeResolutionError>
    where
        C: crate::ResolverCacheAdapter,
        L: crate::ResolverLayoutAdapter,
    {
        let include_base = !resolver.is_value_type(td.clone())?;
        Self::collect_fields(
            resolver,
            td.clone(),
            td.resolution.clone(),
            generics,
            &mut |f| !f.static_member,
            include_base,
        )
    }

    pub(crate) fn static_fields_with_lookup<C, L>(
        resolver: &ResolverService<C, L>,
        td: TypeDescription,
        generics: &GenericLookup,
    ) -> Result<FieldLayoutManager, TypeResolutionError>
    where
        C: crate::ResolverCacheAdapter,
        L: crate::ResolverLayoutAdapter,
    {
        Self::collect_fields(
            resolver,
            td.clone(),
            td.resolution.clone(),
            generics,
            &mut |f| f.static_member,
            false,
        )
    }

    pub(crate) fn create_array_layout_with_lookup<C, L>(
        resolver: &ResolverService<C, L>,
        element: ConcreteType,
        length: usize,
        resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<ArrayLayoutManager, TypeResolutionError>
    where
        C: crate::ResolverCacheAdapter,
        L: crate::ResolverLayoutAdapter,
    {
        let element_layout =
            resolver.type_layout_cached_with_lookup(element, resolution, generics)?;
        if length > 0x4000_0000
            || (length > 0 && element_layout.size().as_usize() > usize::MAX / length)
        {
            return Err(TypeResolutionError::MassiveAllocation(format!(
                "massive array allocation attempt: length={}, element_size={}",
                length,
                element_layout.size()
            )));
        }

        Ok(ArrayLayoutManager {
            element_layout,
            length,
        })
    }
}

impl<C, L> ResolverService<C, L>
where
    C: crate::ResolverCacheAdapter,
    L: crate::ResolverLayoutAdapter,
{
    pub fn type_layout_cached<Ctx: ResolverExecutionContext>(
        &self,
        t: ConcreteType,
        ctx: &Ctx,
    ) -> Result<Arc<LayoutManager>, TypeResolutionError> {
        self.type_layout_cached_with_lookup(t, ctx.resolution().clone(), ctx.generics())
    }

    pub fn type_layout_cached_with_lookup(
        &self,
        t: ConcreteType,
        resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<Arc<LayoutManager>, TypeResolutionError> {
        let t = self.normalize_type(t)?;

        if let Some(cached) = self.layout.get_layout_cached(&t) {
            return Ok(cached);
        }

        let result = Arc::new(self.type_layout_internal(t.clone(), resolution, generics)?);
        self.layout.set_layout_cached(t, Arc::clone(&result));
        Ok(result)
    }

    pub fn instance_fields<Ctx: ResolverExecutionContext>(
        &self,
        td: TypeDescription,
        ctx: &Ctx,
    ) -> Result<FieldLayoutManager, TypeResolutionError> {
        self.instance_fields_with_lookup(td, ctx.generics())
    }

    pub fn instance_fields_with_lookup(
        &self,
        td: TypeDescription,
        generics: &GenericLookup,
    ) -> Result<FieldLayoutManager, TypeResolutionError> {
        LayoutFactory::instance_fields_with_lookup(self, td, generics)
    }

    pub fn instance_field_layout_cached<Ctx: ResolverExecutionContext>(
        &self,
        td: TypeDescription,
        ctx: &Ctx,
    ) -> Result<Arc<FieldLayoutManager>, TypeResolutionError> {
        self.instance_field_layout_cached_with_lookup(td, ctx.generics())
    }

    pub fn instance_field_layout_cached_with_lookup(
        &self,
        td: TypeDescription,
        generics: &GenericLookup,
    ) -> Result<Arc<FieldLayoutManager>, TypeResolutionError> {
        let key = (td.clone(), generics.clone());

        if let Some(cached) = self.layout.get_instance_field_layout_cached(&key) {
            return Ok(cached);
        }

        let result = Arc::new(self.instance_fields_with_lookup(td, &key.1)?);
        self.layout
            .set_instance_field_layout_cached(key, Arc::clone(&result));
        Ok(result)
    }

    pub fn static_fields<Ctx: ResolverExecutionContext>(
        &self,
        td: TypeDescription,
        ctx: &Ctx,
    ) -> Result<FieldLayoutManager, TypeResolutionError> {
        self.static_fields_with_lookup(td, ctx.generics())
    }

    pub fn static_fields_with_lookup(
        &self,
        td: TypeDescription,
        generics: &GenericLookup,
    ) -> Result<FieldLayoutManager, TypeResolutionError> {
        LayoutFactory::static_fields_with_lookup(self, td, generics)
    }

    pub fn create_array_layout<Ctx: ResolverExecutionContext>(
        &self,
        element: ConcreteType,
        length: usize,
        ctx: &Ctx,
    ) -> Result<ArrayLayoutManager, TypeResolutionError> {
        self.create_array_layout_with_lookup(
            element,
            length,
            ctx.resolution().clone(),
            ctx.generics(),
        )
    }

    pub fn create_array_layout_with_lookup(
        &self,
        element: ConcreteType,
        length: usize,
        resolution: ResolutionS,
        generics: &GenericLookup,
    ) -> Result<ArrayLayoutManager, TypeResolutionError> {
        LayoutFactory::create_array_layout_with_lookup(self, element, length, resolution, generics)
    }

    fn type_layout_internal(
        &self,
        t: ConcreteType,
        _resolution: ResolutionS,
        _generics: &GenericLookup,
    ) -> Result<LayoutManager, TypeResolutionError> {
        let resolution = t.resolution();

        Ok(match t.get() {
            BaseType::Boolean | BaseType::UInt8 => Scalar::UInt8.into(),
            BaseType::Int8 => Scalar::Int8.into(),
            BaseType::Char | BaseType::UInt16 => Scalar::UInt16.into(),
            BaseType::Int16 => Scalar::Int16.into(),
            BaseType::Int32 | BaseType::UInt32 => Scalar::Int32.into(),
            BaseType::Int64 | BaseType::UInt64 => Scalar::Int64.into(),
            BaseType::IntPtr
            | BaseType::UIntPtr
            | BaseType::FunctionPointer(_)
            | BaseType::ValuePointer(_, _) => Scalar::NativeInt.into(),
            BaseType::Float32 => Scalar::Float32.into(),
            BaseType::Float64 => Scalar::Float64.into(),
            BaseType::Type {
                value_kind: Some(ValueKind::ValueType),
                source,
            } => {
                let mut type_generics = vec![];
                let t = match source {
                    TypeSource::User(u) => self.locate_type(resolution.clone(), *u)?,
                    TypeSource::Generic { base, parameters } => {
                        type_generics = parameters.clone();
                        self.locate_type(resolution.clone(), *base)?
                    }
                };

                let name = t.type_name();
                trace!("Computing layout for: {}", name);
                if name == "System.ByReference`1"
                    || name == "System.ReadOnlyByReference`1"
                    || name == "System.ByReference"
                    || name == "System.Runtime.CompilerServices.ByReference`1"
                    || name == "System.Runtime.CompilerServices.ReadOnlyByReference`1"
                {
                    return Ok(Scalar::ManagedPtr.into());
                }

                let new_lookup = GenericLookup::new(type_generics);

                if let Some(inner) = t.is_enum() {
                    (*self.type_layout_cached_with_lookup(
                        self.make_concrete(resolution.clone(), &new_lookup, inner)?,
                        resolution.clone(),
                        &new_lookup,
                    )?)
                    .clone()
                } else {
                    LayoutFactory::instance_fields_with_lookup(self, t, &new_lookup)?.into()
                }
            }
            BaseType::Type { .. }
            | BaseType::Object
            | BaseType::String
            | BaseType::Vector(_, _)
            | BaseType::Array(_, _) => Scalar::ObjectRef.into(),
        })
    }
}
