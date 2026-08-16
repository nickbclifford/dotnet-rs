use crate::{
    AssemblyLoader,
    error::AssemblyLoadError,
    loader::{SUPPORT_ASSEMBLY, SUPPORT_LIBRARY},
    support_contract::{SlotDescriptor, SlotKind, SupportSlotRegistry},
};
use dotnet_types::{TypeDescription, resolution::ResolutionS};
use dotnetdll::prelude::*;
use std::ptr;

struct ExpectedSupportSlot {
    type_name: &'static str,
    field_name: &'static str,
    kind: SlotKind,
    is_static: bool,
}

const EXPECTED_SUPPORT_SLOTS: &[ExpectedSupportSlot] = &[
    ExpectedSupportSlot {
        type_name: "System.RuntimeTypeHandle",
        field_name: "_value",
        kind: SlotKind::Handle,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "System.RuntimeFieldHandle",
        field_name: "_value",
        kind: SlotKind::Handle,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "System.RuntimeMethodHandle",
        field_name: "_value",
        kind: SlotKind::Handle,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "System.RuntimeType",
        field_name: "index",
        kind: SlotKind::Index,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.MethodInfo",
        field_name: "index",
        kind: SlotKind::Index,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.ConstructorInfo",
        field_name: "index",
        kind: SlotKind::Index,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.FieldInfo",
        field_name: "index",
        kind: SlotKind::Index,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.ParameterInfo",
        field_name: "method_index",
        kind: SlotKind::Index,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.ParameterInfo",
        field_name: "position",
        kind: SlotKind::ScalarInt,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.PropertyInfo",
        field_name: "name",
        kind: SlotKind::GcRef,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.PropertyInfo",
        field_name: "getter",
        kind: SlotKind::GcRef,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.PropertyInfo",
        field_name: "setter",
        kind: SlotKind::GcRef,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.PropertyInfo",
        field_name: "declaringType",
        kind: SlotKind::GcRef,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.PropertyInfo",
        field_name: "propertyType",
        kind: SlotKind::GcRef,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "System.Delegate",
        field_name: "_target",
        kind: SlotKind::GcRef,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "System.Delegate",
        field_name: "_method",
        kind: SlotKind::Index,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "System.MulticastDelegate",
        field_name: "targets",
        kind: SlotKind::GcRef,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "System.Span`1",
        field_name: "_reference",
        kind: SlotKind::Byref,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "System.Span`1",
        field_name: "_length",
        kind: SlotKind::ScalarInt,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "System.ReadOnlySpan`1",
        field_name: "_reference",
        kind: SlotKind::Byref,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "System.ReadOnlySpan`1",
        field_name: "_length",
        kind: SlotKind::ScalarInt,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.Module",
        field_name: "resolution",
        kind: SlotKind::NativePtr,
        is_static: false,
    },
    ExpectedSupportSlot {
        type_name: "DotnetRs.Assembly",
        field_name: "resolution",
        kind: SlotKind::NativePtr,
        is_static: false,
    },
];

fn validate_slot_metadata(kind: SlotKind, field: &Field<'_>) -> Result<(), Box<str>> {
    let base = field.return_type.as_base();
    let matches = match kind {
        SlotKind::Handle | SlotKind::Index | SlotKind::NativePtr => {
            !field.by_ref && matches!(base, Some(BaseType::IntPtr))
        }
        SlotKind::GcRef => {
            !field.by_ref
                && matches!(
                    base,
                    Some(
                        BaseType::Object
                            | BaseType::String
                            | BaseType::Vector(_, _)
                            | BaseType::Array(_, _)
                            | BaseType::Type {
                                value_kind: Some(ValueKind::Class),
                                ..
                            }
                    )
                )
        }
        SlotKind::Byref => field.by_ref,
        SlotKind::ScalarInt => !field.by_ref && matches!(base, Some(BaseType::Int32)),
    };

    if matches {
        return Ok(());
    }

    let expected = match kind {
        SlotKind::Handle => "a non-byref native-int field whose layout is overridden to ObjectRef",
        SlotKind::Index => "a non-byref native-int field",
        SlotKind::GcRef => "a non-byref managed-reference field",
        SlotKind::Byref => "a managed byref field",
        SlotKind::ScalarInt => "a non-byref Int32 field",
        SlotKind::NativePtr => "a non-byref native-int field",
    };
    Err(format!(
        "RuntimeSlot({kind:?}) requires {expected}; found by_ref={} metadata type {:?}",
        field.by_ref, field.return_type
    )
    .into())
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

        for (index, t) in support_res.type_definitions.iter().enumerate() {
            let type_index = support_res.type_definition_index(index).ok_or_else(|| {
                AssemblyLoadError::InvalidFormat("failed to find type definition index".into())
            })?;
            let attrs = support_res
                .type_attributes(type_index)
                .map_err(|e| AssemblyLoadError::InvalidFormat(format!("{e}").into()))?;
            for a in attrs {
                // the target stub attribute is internal to the support library,
                // so the constructor reference will always be a Definition variant
                let parent = match a.constructor {
                    UserMethod::Definition(d) => &support_res[d.parent_type()],
                    UserMethod::Reference(_) => {
                        continue;
                    }
                };
                if parent.type_name() == "DotnetRs.StubAttribute" {
                    let data = a
                        .instantiation_data(&(self as &AssemblyLoader), &*support_res)
                        .map_err(|e| {
                            AssemblyLoadError::InvalidFormat(
                                format!("failed to parse stub attribute data: {}", e).into(),
                            )
                        })?;
                    for n in data.named_args {
                        match n {
                            NamedArg::Field(name, FixedArg::String(Some(target)))
                                if name == "InPlaceOf" =>
                            {
                                let support_type_name = t.type_name();
                                self.stubs.insert(
                                    target.to_string(),
                                    TypeDescription::new(res_s.clone(), type_index),
                                );
                                self.reverse_stubs
                                    .insert(support_type_name, target.to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let support_slots = self.validate_support_contract(support_res)?;

        self.support_slot_registry = support_slots;
        Ok(())
    }

    /// Parses and validates the support-slot annotations in a support resolution.
    ///
    /// Kept separate from support-library registration so synthetic resolutions can exercise the
    /// same metadata and contract validation path in tests.
    pub(crate) fn validate_support_contract(
        &self,
        support_res: &Resolution<'_>,
    ) -> Result<SupportSlotRegistry, AssemblyLoadError> {
        let mut support_slots = SupportSlotRegistry::default();
        for (type_index, t) in support_res.enumerate_type_definitions() {
            let raw_type_name = t.type_name();
            let type_name = self.canonical_type_name(&raw_type_name);
            for (field_index, field) in support_res.enumerate_fields(type_index) {
                let attrs = support_res.field_attributes(field_index).map_err(|e| {
                    AssemblyLoadError::InvalidFormat(
                        format!("failed to read support field attributes: {e}").into(),
                    )
                })?;
                for a in attrs {
                    // Like StubAttribute above, RuntimeSlotAttribute is defined by the support
                    // assembly, so an attribute constructor reference must be a definition.
                    let parent = match a.constructor {
                        UserMethod::Definition(d) => &support_res[d.parent_type()],
                        UserMethod::Reference(_) => continue,
                    };
                    if parent.type_name() != "DotnetRs.RuntimeSlotAttribute" {
                        continue;
                    }

                    let field_name = field.name.as_ref();
                    let data = a
                        .instantiation_data(&(self as &AssemblyLoader), support_res)
                        .map_err(|e| AssemblyLoadError::SupportContractViolation {
                            type_name: type_name.into(),
                            field_name: field_name.into(),
                            reason: format!("failed to parse RuntimeSlot attribute data: {e}")
                                .into(),
                        })?;
                    let kind = match data.constructor_args.as_slice() {
                        [FixedArg::String(Some(kind))] => match kind.as_ref() {
                            "Handle" => SlotKind::Handle,
                            "Index" => SlotKind::Index,
                            "GcRef" => SlotKind::GcRef,
                            "Byref" => SlotKind::Byref,
                            "ScalarInt" => SlotKind::ScalarInt,
                            "NativePtr" => SlotKind::NativePtr,
                            other => {
                                return Err(AssemblyLoadError::SupportContractViolation {
                                    type_name: type_name.into(),
                                    field_name: field_name.into(),
                                    reason: format!("unrecognized RuntimeSlot kind {other:?}")
                                        .into(),
                                });
                            }
                        },
                        _ => {
                            return Err(AssemblyLoadError::SupportContractViolation {
                                type_name: type_name.into(),
                                field_name: field_name.into(),
                                reason:
                                    "RuntimeSlot requires one non-null string constructor argument"
                                        .into(),
                            });
                        }
                    };
                    if let Err(reason) = validate_slot_metadata(kind, field) {
                        return Err(AssemblyLoadError::SupportContractViolation {
                            type_name: type_name.into(),
                            field_name: field_name.into(),
                            reason,
                        });
                    }
                    support_slots.slots.push(SlotDescriptor {
                        type_name: type_name.into(),
                        field_name: field_name.into(),
                        kind,
                        is_static: field.static_member,
                    });
                }
            }
        }

        for expected in EXPECTED_SUPPORT_SLOTS {
            let matching_slots: Vec<_> = support_slots
                .slots
                .iter()
                .filter(|slot| {
                    slot.type_name.as_ref() == expected.type_name
                        && slot.field_name.as_ref() == expected.field_name
                })
                .collect();
            match matching_slots.as_slice() {
                [] => {
                    return Err(AssemblyLoadError::SupportContractViolation {
                        type_name: expected.type_name.into(),
                        field_name: expected.field_name.into(),
                        reason: format!("missing required {:?} support slot", expected.kind).into(),
                    });
                }
                [slot] if slot.kind == expected.kind && slot.is_static == expected.is_static => {}
                [slot] => {
                    return Err(AssemblyLoadError::SupportContractViolation {
                        type_name: expected.type_name.into(),
                        field_name: expected.field_name.into(),
                        reason: format!(
                            "expected {:?} {} field, found {:?} {} field",
                            expected.kind,
                            if expected.is_static {
                                "static"
                            } else {
                                "instance"
                            },
                            slot.kind,
                            if slot.is_static { "static" } else { "instance" },
                        )
                        .into(),
                    });
                }
                slots => {
                    return Err(AssemblyLoadError::SupportContractViolation {
                        type_name: expected.type_name.into(),
                        field_name: expected.field_name.into(),
                        reason: format!("expected exactly one slot, found {}", slots.len()).into(),
                    });
                }
            }
        }

        for slot in &support_slots.slots {
            let is_known_slot = EXPECTED_SUPPORT_SLOTS.iter().any(|expected| {
                slot.type_name.as_ref() == expected.type_name
                    && slot.field_name.as_ref() == expected.field_name
            });
            if !is_known_slot {
                return Err(AssemblyLoadError::UnrecognizedSupportSlot {
                    type_name: slot.type_name.clone(),
                    field_name: slot.field_name.clone(),
                });
            }
        }

        Ok(support_slots)
    }
}
