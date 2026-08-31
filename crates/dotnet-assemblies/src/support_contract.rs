//! Runtime-facing descriptions of support-assembly fields.

use crate::AssemblyLoader;
use dotnet_types::{TypeDescription, WellKnown};

use dotnet_value::{
    layout::{FieldLayout, FieldLayoutManager, FieldType, HasLayout, LayoutManager},
    object::ObjectRef,
    pointer::ManagedPtr,
    storage::{FieldRef, FieldStorage},
};

include!(concat!(env!("OUT_DIR"), "/support_slots.rs"));

/// All support-assembly fields registered by the ABI contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupportSlotRegistry {
    // IDs are dense in the current contract, so this is a direct `id - 1` lookup. A future
    // reserved-ID scheme must update this representation before the definition permits holes.
    slots: [Option<TypeDescription>; RuntimeSlotId::COUNT],
}

impl Default for SupportSlotRegistry {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
        }
    }
}

impl SupportSlotRegistry {
    pub(crate) fn insert(&mut self, id: RuntimeSlotId, declaring_type: TypeDescription) {
        self.slots[id as usize - 1] = Some(declaring_type);
    }

    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    fn slot(&self, id: RuntimeSlotId) -> Option<&TypeDescription> {
        self.slots[id as usize - 1].as_ref()
    }

    /// Maps metadata at a layout-collection boundary to its validated semantic ID.
    ///
    /// Field names are metadata inputs at this boundary only; all downstream support ABI use is
    /// by `RuntimeSlotId` and requires exact descriptor identity.
    fn id_for_field(&self, desc: &TypeDescription, field_name: &str) -> Option<RuntimeSlotId> {
        RUNTIME_SLOT_DESCRIPTORS.iter().find_map(|spec| {
            (self.slot(spec.id) == Some(desc) && spec.field_name == field_name).then_some(spec.id)
        })
    }

    /// Resolves a pre-validated slot to its position in this particular storage layout.
    ///
    /// Layouts are constructed lazily and can differ for constructed generic types, so the
    /// generated contract selects the metadata name while the supplied storage provides the
    /// process-local byte offset.
    fn field<'storage, T: FieldType>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
        id: RuntimeSlotId,
    ) -> Option<FieldRef<'storage, T>> {
        let slot = self.slot(id)?;
        if !Self::accessor_accepts_descriptor(slot, &desc) {
            return None;
        }
        let field = storage
            .layout()
            .get_field(desc, id.descriptor().field_name)?;
        if !matches!(
            field.layout.as_ref(),
            LayoutManager::Scalar(scalar) if *scalar == T::SCALAR
        ) {
            return None;
        }
        storage.field_at_offset(field.position.as_usize())
    }

    /// Resolves the one intentional same-width reinterpretation in the contract.
    fn reinterpreted_field<'storage, T: FieldType>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
        id: RuntimeSlotId,
    ) -> Option<FieldRef<'storage, T>> {
        let slot = self.slot(id)?;
        if !Self::accessor_accepts_descriptor(slot, &desc) {
            return None;
        }
        let field = storage
            .layout()
            .get_field(desc, id.descriptor().field_name)?;
        if field.layout.size().as_usize() != T::SCALAR.size_const() {
            return None;
        }
        storage.field_at_offset(field.position.as_usize())
    }

    /// Runtime accessors may receive a framework descriptor for a type replaced by an
    /// embedded support stub. The field still has to be owned by the same declared
    /// metadata type name and satisfy the slot's exact storage shape below. Layout
    /// registration remains exact-descriptor-only in `id_for_field`, so an unrelated
    /// same-named type cannot acquire a support layout override.
    fn accessor_accepts_descriptor(slot: &TypeDescription, desc: &TypeDescription) -> bool {
        slot == desc || slot.type_name() == desc.type_name()
    }

    fn layout_field<'layout>(
        &self,
        layout: &'layout FieldLayoutManager,
        id: RuntimeSlotId,
    ) -> Option<&'layout FieldLayout> {
        let slot = self.slot(id)?;
        layout
            .get_field(slot.clone(), id.descriptor().field_name)
            .or_else(|| {
                layout.fields.iter().find_map(|(key, field)| {
                    (key.name == id.descriptor().field_name
                        && key.owner.type_name() == slot.type_name())
                    .then_some(field)
                })
            })
    }
}

impl AssemblyLoader {
    /// Maps a field selected during layout collection to its support ABI semantic ID.
    ///
    /// The returned ID is bound to the exact declaring [`TypeDescription`], not to a canonical
    /// name, so a same-named type in another assembly cannot select a support layout override.
    pub fn support_slot_id_for_field(
        &self,
        desc: &TypeDescription,
        field_name: &str,
    ) -> Option<RuntimeSlotId> {
        self.support_slot_registry.id_for_field(desc, field_name)
    }

    /// Returns the support-assembly ABI slots validated while this loader was initialized.
    pub fn support_slots(&self) -> &SupportSlotRegistry {
        &self.support_slot_registry
    }
}
