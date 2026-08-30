use crate::{
    AssemblyLoader,
    error::AssemblyLoadError,
    loader::{SUPPORT_ASSEMBLY, SUPPORT_LIBRARY},
    support_contract::{
        RUNTIME_SLOT_DESCRIPTORS, RuntimeSlotId, RuntimeSlotSignature, SupportSlotRegistry,
    },
};
use dotnet_types::{TypeDescription, resolution::ResolutionS};
use dotnetdll::prelude::*;
use std::ptr;

type StubMaps = (
    std::collections::HashMap<String, TypeDescription>,
    std::collections::HashMap<String, String>,
);

pub(crate) fn parse_runtime_slot_id(
    constructor_args: &[FixedArg<'_>],
) -> Result<RuntimeSlotId, Box<str>> {
    match constructor_args {
        // C# stores an enum-valued custom-attribute argument as its underlying integral value.
        // The RuntimeSlotAttribute constructor itself declares RuntimeSlotId, while dotnetdll
        // decodes its payload as Integral rather than retaining the enum type name.
        [FixedArg::Integral(IntegralParam::Int32(value))] => RuntimeSlotId::from_i32(*value)
            .ok_or_else(|| format!("unknown RuntimeSlot ID {value}").into()),
        [FixedArg::Enum(enum_name, IntegralParam::Int32(value))]
            if enum_name.as_ref() == "DotnetRs.RuntimeSlotId" => RuntimeSlotId::from_i32(*value)
                .ok_or_else(|| format!("unknown RuntimeSlot ID {value}").into()),
        [FixedArg::Enum(enum_name, _)] => Err(format!(
            "RuntimeSlot enum argument must be DotnetRs.RuntimeSlotId with an Int32 payload, found {enum_name:?}"
        )
        .into()),
        _ => Err(format!(
            "RuntimeSlot requires one RuntimeSlotId Int32 argument, found {constructor_args:?}"
        )
        .into()),
    }
}

fn runtime_slot_constructor_uses_generated_id(
    constructor: &Method<'_>,
    resolution: &Resolution<'_>,
) -> bool {
    matches!(
        constructor.signature.parameters.as_slice(),
        [Parameter(_, ParameterType::Value(MethodType::Base(base)))]
            if matches!(
                base.as_ref(),
                BaseType::Type {
                    source: TypeSource::User(id),
                    ..
                } if id.type_name(resolution) == "DotnetRs.RuntimeSlotId"
            )
    )
}

fn user_type_matches(
    source: &TypeSource<MemberType>,
    expected: dotnet_types::WellKnown,
    loader: &AssemblyLoader,
    resolution: &Resolution<'_>,
) -> bool {
    matches!(
        source,
        TypeSource::User(id) if loader.canonical_type_name(&id.type_name(resolution)) == expected.name()
    )
}

fn slot_signature_matches(
    expected: RuntimeSlotSignature,
    field: &Field<'_>,
    loader: &AssemblyLoader,
    resolution: &Resolution<'_>,
) -> bool {
    match (expected, field.by_ref, field.return_type.as_base()) {
        (RuntimeSlotSignature::IntPtr, false, Some(BaseType::IntPtr))
        | (RuntimeSlotSignature::Int32, false, Some(BaseType::Int32))
        | (RuntimeSlotSignature::Object, false, Some(BaseType::Object))
        | (RuntimeSlotSignature::String, false, Some(BaseType::String)) => true,
        (
            RuntimeSlotSignature::Class(expected),
            false,
            Some(BaseType::Type {
                value_kind: Some(ValueKind::Class),
                source,
            }),
        ) => user_type_matches(source, expected, loader, resolution),
        (
            RuntimeSlotSignature::SzArray(expected),
            false,
            Some(BaseType::Vector(modifiers, element)),
        ) => {
            modifiers.is_empty()
                && matches!(
                    element,
                    MemberType::Base(base)
                        if matches!(
                            base.as_ref(),
                            BaseType::Type {
                                value_kind: Some(ValueKind::Class),
                                source,
                            } if user_type_matches(source, expected, loader, resolution)
                        )
                )
        }
        (RuntimeSlotSignature::ByRefTypeGeneric(expected), true, None) => {
            matches!(field.return_type, MemberType::TypeGeneric(index) if index == expected as usize)
        }
        _ => false,
    }
}

#[derive(Debug)]
pub(crate) struct StubCandidate {
    target: String,
    support_type_name: String,
    type_index: TypeIndex,
}

impl AssemblyLoader {
    pub(crate) fn add_support_library(&mut self) -> Result<(), AssemblyLoadError> {
        // Ensure alignment for SUPPORT_LIBRARY
        let len = SUPPORT_LIBRARY.len();
        let cap = len.div_ceil(8);
        let mut aligned: Vec<u64> = vec![0u64; cap];
        // SAFETY: The source and destination are both valid for 'len' bytes.
        // The destination buffer 'aligned' has enough capacity as it's allocated with 'div_ceil(8)'.
        // Both buffers are non-overlapping as 'aligned' is newly allocated.
        unsafe {
            ptr::copy_nonoverlapping(
                SUPPORT_LIBRARY.as_ptr(),
                aligned.as_mut_ptr() as *mut u8,
                len,
            );
        }
        let aligned_boxed = aligned.into_boxed_slice();
        let aligned_ptr = Box::into_raw(aligned_boxed);
        // SAFETY: `aligned_ptr` came from `Box::into_raw` immediately above. AssemblyLoader owns
        // the `Arc<MetadataArena>` that tracks and eventually reclaims the allocation, so this
        // mutable slice remains live for every metadata user given the `'static` reference.
        let aligned_slice: &'static mut [u64] = unsafe { &mut *aligned_ptr };
        // SAFETY: `aligned_ptr` is a live allocation from `Box::into_raw`; MetadataArena takes
        // ownership of reclaiming it and does not dereference it here.
        unsafe {
            self.metadata.add_u64_slice(aligned_ptr);
        }

        let byte_slice =
            // SAFETY: 'aligned_slice' is a leaked Box<[u64]> which is valid for its entire length.
            // Converting it to a *const u8 slice of length 'len' is safe because it was initialized
            // with exactly 'len' bytes from SUPPORT_LIBRARY.
            unsafe { std::slice::from_raw_parts(aligned_slice.as_ptr() as *const u8, len) };

        // Attributes are accessed below through `type_attributes()` and `field_attributes()`, so
        // they can remain lazy. Attribute instantiation reads constructor signatures directly,
        // however, so support-library method signatures must be decoded eagerly.
        let support_res_raw = Resolution::parse(
            byte_slice,
            ReadOptions {
                lazy_method_bodies: true,
                lazy_method_signatures: false,
                lazy_attributes: true,
                ..Default::default()
            },
        )
        .map_err(AssemblyLoadError::from)?;
        let support_res_box = Box::new(support_res_raw);
        let support_res_ptr = Box::into_raw(support_res_box);
        // SAFETY: `support_res_ptr` came from `Box::into_raw` immediately above. MetadataArena
        // tracks and eventually reclaims it, and its owning Arc outlives every ResolutionS that
        // receives this `'static` reference.
        let support_res: &'static mut Resolution<'static> = unsafe { &mut *support_res_ptr };
        // SAFETY: `support_res_ptr` is the live allocation just produced by `Box::into_raw`, and
        // MetadataArena records responsibility for reclaiming it without accessing it now.
        unsafe {
            self.metadata.add_resolution(support_res_ptr);
        }

        let res_s = ResolutionS::new(support_res, self.metadata.clone());
        {
            let mut external = self.external.write();
            external.insert(SUPPORT_ASSEMBLY.to_string(), Some(res_s.clone()));
        }

        let (stubs, reverse_stubs) = self.validate_stub_schema(support_res, res_s.clone())?;

        // The schema validator builds both maps before this point, so malformed metadata cannot
        // leave partially registered stubs behind.
        self.stubs = stubs;
        self.reverse_stubs = reverse_stubs;

        let support_slots = self.validate_support_contract(support_res)?;

        self.support_slot_registry = support_slots;
        Ok(())
    }

    fn collect_stub_candidates(
        &self,
        support_res: &Resolution<'_>,
    ) -> Result<Vec<StubCandidate>, AssemblyLoadError> {
        let mut stub_attribute_types = support_res
            .enumerate_type_definitions()
            .filter(|(_, ty)| ty.type_name() == "DotnetRs.StubAttribute");
        let Some((stub_attribute_index, _)) = stub_attribute_types.next() else {
            return Err(AssemblyLoadError::InvalidFormat(
                "invalid StubAttribute schema: missing DotnetRs.StubAttribute".into(),
            ));
        };
        if stub_attribute_types.next().is_some() {
            return Err(AssemblyLoadError::InvalidFormat(
                "invalid StubAttribute schema: multiple DotnetRs.StubAttribute definitions".into(),
            ));
        }

        let fields: Vec<_> = support_res.enumerate_fields(stub_attribute_index).collect();
        match fields.as_slice() {
            [(_, field)]
                if field.name == "InPlaceOf"
                    && !field.static_member
                    && !field.by_ref
                    && matches!(field.return_type.as_base(), Some(BaseType::String)) => {}
            _ => {
                return Err(AssemblyLoadError::InvalidFormat(
                    "invalid StubAttribute schema for DotnetRs.StubAttribute: expected exactly one instance string field named InPlaceOf".into(),
                ));
            }
        }

        let mut candidates = Vec::new();
        for (type_index, ty) in support_res.enumerate_type_definitions() {
            let type_name = ty.type_name();
            let attrs = support_res.type_attributes(type_index).map_err(|e| {
                AssemblyLoadError::InvalidFormat(
                    format!("failed to read [Stub] metadata on {type_name}: {e}").into(),
                )
            })?;
            for attribute in attrs {
                // StubAttribute is defined in this assembly, so a valid use must refer to its
                // definition rather than an unrelated external attribute with the same name.
                let is_stub_attribute = matches!(
                    attribute.constructor,
                    UserMethod::Definition(definition)
                        if definition.parent_type() == stub_attribute_index
                );
                if !is_stub_attribute {
                    continue;
                }

                let data = attribute
                    .instantiation_data(&(self as &AssemblyLoader), support_res)
                    .map_err(|e| {
                        AssemblyLoadError::InvalidFormat(
                            format!("failed to parse [Stub] metadata on {type_name}: {e}").into(),
                        )
                    })?;
                let target = match data.named_args.as_slice() {
                    [NamedArg::Field(name, FixedArg::String(Some(target)))]
                        if name == "InPlaceOf" && !target.is_empty() =>
                    {
                        target.to_string()
                    }
                    [NamedArg::Field(name, FixedArg::String(Some(_)))] if name == "InPlaceOf" => {
                        return Err(AssemblyLoadError::InvalidFormat(
                            format!("invalid [Stub] metadata on {type_name}: InPlaceOf must be non-empty").into(),
                        ));
                    }
                    _ => {
                        return Err(AssemblyLoadError::InvalidFormat(
                            format!("invalid [Stub] metadata on {type_name}: expected exactly one non-null string field argument InPlaceOf").into(),
                        ));
                    }
                };

                candidates.push(StubCandidate {
                    target,
                    support_type_name: type_name.clone(),
                    type_index,
                });
            }
        }

        let mut completed_stub_map = std::collections::HashMap::new();
        let mut completed_reverse_stub_map = std::collections::HashMap::new();
        for candidate in &candidates {
            if let Some(previous) =
                completed_stub_map.insert(candidate.target.as_str(), candidate.type_index)
            {
                return Err(AssemblyLoadError::InvalidFormat(
                    format!(
                        "invalid [Stub] metadata on {}: InPlaceOf target {:?} is already mapped to type index {:?}",
                        candidate.support_type_name, candidate.target, previous,
                    )
                    .into(),
                ));
            }
            if completed_reverse_stub_map
                .insert(
                    candidate.support_type_name.as_str(),
                    candidate.target.as_str(),
                )
                .is_some()
            {
                return Err(AssemblyLoadError::InvalidFormat(
                    format!(
                        "invalid [Stub] metadata on {}: multiple [Stub] attributes are not allowed",
                        candidate.support_type_name,
                    )
                    .into(),
                ));
            }
        }

        Ok(candidates)
    }

    /// Validates the metadata schema used to register support-library stub types.
    ///
    /// This is deliberately independent from the runtime storage-slot contract: `InPlaceOf` is
    /// custom-attribute metadata, not a field whose storage Rust accesses.
    fn validate_stub_schema(
        &self,
        support_res: &Resolution<'_>,
        support_resolution: ResolutionS,
    ) -> Result<StubMaps, AssemblyLoadError> {
        let candidates = self.collect_stub_candidates(support_res)?;
        let mut stubs = std::collections::HashMap::new();
        let mut reverse_stubs = std::collections::HashMap::new();

        for candidate in &candidates {
            let annotated_support_type =
                TypeDescription::new(support_resolution.clone(), candidate.type_index);
            if let Some(previous) = stubs.insert(candidate.target.clone(), annotated_support_type) {
                return Err(AssemblyLoadError::InvalidFormat(
                    format!(
                        "invalid [Stub] metadata on {}: InPlaceOf target {:?} is already mapped to {previous:?}",
                        candidate.support_type_name, candidate.target,
                    )
                    .into(),
                ));
            }
            if reverse_stubs
                .insert(
                    candidate.support_type_name.clone(),
                    candidate.target.clone(),
                )
                .is_some()
            {
                return Err(AssemblyLoadError::InvalidFormat(
                    format!(
                        "invalid [Stub] metadata on {}: multiple [Stub] attributes are not allowed",
                        candidate.support_type_name,
                    )
                    .into(),
                ));
            }
        }

        for candidate in &candidates {
            let annotated_support_type =
                TypeDescription::new(support_resolution.clone(), candidate.type_index);
            match stubs.get(&candidate.target) {
                Some(resolved) if resolved == &annotated_support_type => {}
                _ => {
                    return Err(AssemblyLoadError::InvalidFormat(
                        format!(
                            "invalid [Stub] metadata on {}: InPlaceOf target {:?} does not resolve to its annotated support type",
                            candidate.support_type_name, candidate.target,
                        )
                        .into(),
                    ));
                }
            }
        }

        Ok((stubs, reverse_stubs))
    }

    #[cfg(test)]
    pub(crate) fn validate_stub_schema_metadata(
        &self,
        support_res: &Resolution<'_>,
    ) -> Result<(), AssemblyLoadError> {
        self.collect_stub_candidates(support_res).map(|_| ())
    }

    /// Parses and validates the support-slot annotations in a support resolution.
    ///
    /// Kept separate from support-library registration so synthetic resolutions can exercise the
    /// same metadata and contract validation path in tests.
    pub(crate) fn validate_support_contract(
        &self,
        support_res: &Resolution<'_>,
    ) -> Result<SupportSlotRegistry, AssemblyLoadError> {
        let mut scanned_slots: [Option<(Box<str>, Box<str>)>; RuntimeSlotId::COUNT] =
            std::array::from_fn(|_| None);

        for (type_index, t) in support_res.enumerate_type_definitions() {
            let raw_type_name = t.type_name();
            let type_name = self.canonical_type_name(&raw_type_name);
            for (field_index, field) in support_res.enumerate_fields(type_index) {
                let attrs = support_res.field_attributes(field_index).map_err(|e| {
                    AssemblyLoadError::InvalidFormat(
                        format!("failed to read support field attributes: {e}").into(),
                    )
                })?;
                let mut used_implicitly = false;
                let mut has_runtime_slot = false;
                for a in attrs {
                    used_implicitly |= match a.constructor {
                        UserMethod::Definition(d) => {
                            support_res[d.parent_type()].type_name()
                                == "JetBrains.Annotations.UsedImplicitlyAttribute"
                        }
                        UserMethod::Reference(r) => {
                            matches!(
                                &support_res[r].parent,
                                MethodReferenceParent::Type(parent)
                                    if parent
                                        .show(support_res)
                                        .ends_with("JetBrains.Annotations.UsedImplicitlyAttribute")
                            )
                        }
                    };
                    // RuntimeSlotAttribute is defined by the support assembly, so a valid use
                    // must refer to its local constructor definition.
                    let constructor = match a.constructor {
                        UserMethod::Definition(definition) => definition,
                        UserMethod::Reference(_) => continue,
                    };
                    let parent = &support_res[constructor.parent_type()];
                    if parent.type_name() != "DotnetRs.RuntimeSlotAttribute" {
                        continue;
                    }
                    has_runtime_slot = true;

                    let field_name = field.name.as_ref();
                    if !runtime_slot_constructor_uses_generated_id(
                        &support_res[constructor],
                        support_res,
                    ) {
                        return Err(AssemblyLoadError::SupportContractViolation {
                            type_name: type_name.into(),
                            field_name: field_name.into(),
                            reason: "RuntimeSlot constructor must take DotnetRs.RuntimeSlotId"
                                .into(),
                        });
                    }
                    let data = a
                        .instantiation_data(&(self as &AssemblyLoader), support_res)
                        .map_err(|e| AssemblyLoadError::SupportContractViolation {
                            type_name: type_name.into(),
                            field_name: field_name.into(),
                            reason: format!("failed to parse RuntimeSlot attribute data: {e}")
                                .into(),
                        })?;
                    let id = parse_runtime_slot_id(&data.constructor_args).map_err(|reason| {
                        AssemblyLoadError::SupportContractViolation {
                            type_name: type_name.into(),
                            field_name: field_name.into(),
                            reason,
                        }
                    })?;
                    let expected = id.descriptor();
                    let expected_type = expected.declaring_well_known.name();
                    if type_name != expected_type || field_name != expected.field_name {
                        return Err(AssemblyLoadError::SupportContractViolation {
                            type_name: type_name.into(),
                            field_name: field_name.into(),
                            reason: format!(
                                "RuntimeSlot ID {} is misplaced: found {}.{}, expected {}.{}",
                                expected.semantic_name,
                                type_name,
                                field_name,
                                expected_type,
                                expected.field_name,
                            )
                            .into(),
                        });
                    }
                    if field.static_member != expected.is_static {
                        return Err(AssemblyLoadError::SupportContractViolation {
                            type_name: type_name.into(),
                            field_name: field_name.into(),
                            reason: format!(
                                "RuntimeSlot ID {} has wrong staticness at {}.{}: expected {} field, found {} field",
                                expected.semantic_name,
                                expected_type,
                                expected.field_name,
                                if expected.is_static { "static" } else { "instance" },
                                if field.static_member { "static" } else { "instance" },
                            )
                            .into(),
                        });
                    }
                    if !slot_signature_matches(expected.signature, field, self, support_res) {
                        return Err(AssemblyLoadError::SupportContractViolation {
                            type_name: type_name.into(),
                            field_name: field_name.into(),
                            reason: format!(
                                "RuntimeSlot ID {} has an exact signature mismatch at {}.{}: expected {:?}, found by_ref={} type {:?}",
                                expected.semantic_name,
                                expected_type,
                                expected.field_name,
                                expected.signature,
                                field.by_ref,
                                field.return_type,
                            )
                            .into(),
                        });
                    }
                    if let Some((previous_type, previous_field)) = &scanned_slots[id as usize - 1] {
                        return Err(AssemblyLoadError::SupportContractViolation {
                            type_name: type_name.into(),
                            field_name: field_name.into(),
                            reason: format!(
                                "duplicate RuntimeSlot ID {}: found {}.{}, already used by {}.{}; expected {}.{}",
                                expected.semantic_name,
                                type_name,
                                field_name,
                                previous_type,
                                previous_field,
                                expected_type,
                                expected.field_name,
                            )
                            .into(),
                        });
                    }
                    scanned_slots[id as usize - 1] = Some((type_name.into(), field_name.into()));
                }
                if has_runtime_slot && !used_implicitly {
                    return Err(AssemblyLoadError::SupportContractViolation {
                        type_name: type_name.into(),
                        field_name: field.name.clone().into(),
                        reason: "fields marked [RuntimeSlot(...)] must carry [UsedImplicitly]"
                            .into(),
                    });
                }
            }
        }

        let mut support_slots = SupportSlotRegistry::default();
        for expected in RUNTIME_SLOT_DESCRIPTORS {
            if scanned_slots[expected.id as usize - 1].is_none() {
                return Err(AssemblyLoadError::SupportContractViolation {
                    type_name: expected.declaring_well_known.name().into(),
                    field_name: expected.field_name.into(),
                    reason: format!(
                        "missing RuntimeSlot ID {}: expected {}.{} with signature {:?}",
                        expected.semantic_name,
                        expected.declaring_well_known.name(),
                        expected.field_name,
                        expected.signature,
                    )
                    .into(),
                });
            }
            let declaring_type =
                self.corlib_wkt(expected.declaring_well_known)
                    .map_err(|error| AssemblyLoadError::SupportContractViolation {
                        type_name: expected.declaring_well_known.name().into(),
                        field_name: expected.field_name.into(),
                        reason: format!(
                            "failed to resolve declaring descriptor for RuntimeSlot ID {}: {error}",
                            expected.semantic_name,
                        )
                        .into(),
                    })?;
            support_slots.insert(expected.id, declaring_type);
        }

        Ok(support_slots)
    }
}
