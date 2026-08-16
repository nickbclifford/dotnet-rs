//! Runtime-facing descriptions of support-assembly fields.

use crate::AssemblyLoader;
use dotnet_types::TypeDescription;
use dotnet_value::{
    layout::{FieldLayout, FieldLayoutManager, FieldType, HasLayout, LayoutManager},
    object::ObjectRef,
    pointer::ManagedPtr,
    storage::{FieldRef, FieldStorage},
};

/// The runtime representation expected for a support-assembly field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    /// An `nint` that stores an `ObjectRef`.
    Handle,
    /// An `nint` that stores a runtime index.
    Index,
    /// A reference type that is already traced by the GC.
    GcRef,
    /// A managed by-reference.
    Byref,
    /// A fixed-width integer.
    ScalarInt,
    /// An `nint` or `IntPtr` that stores an unmanaged pointer.
    NativePtr,
}

/// A single support-assembly field in the runtime ABI contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotDescriptor {
    pub type_name: Box<str>,
    pub field_name: Box<str>,
    pub kind: SlotKind,
    pub is_static: bool,
}

/// All support-assembly fields registered by the ABI contract.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SupportSlotRegistry {
    pub slots: Vec<SlotDescriptor>,
}

impl SupportSlotRegistry {
    fn slot(&self, type_name: &str, field_name: &str) -> Option<&SlotDescriptor> {
        self.slots.iter().find(|slot| {
            slot.type_name.as_ref() == type_name && slot.field_name.as_ref() == field_name
        })
    }

    fn is_handle_value_slot(&self, type_name: &str, field_name: &str) -> bool {
        field_name == "_value"
            && self
                .slot(type_name, field_name)
                .is_some_and(|slot| slot.kind == SlotKind::Handle)
    }

    /// Resolves a pre-validated slot to its position in this particular storage layout.
    ///
    /// Layouts are constructed lazily and can differ for constructed generic types, so the
    /// registry retains the validated metadata name rather than a process-global byte offset.
    /// The resulting offset is always obtained from the supplied storage's own layout.
    fn field<'storage, T: FieldType>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
        type_name: &str,
        field_name: &str,
    ) -> Option<FieldRef<'storage, T>> {
        let slot = self.slot(type_name, field_name)?;
        let field = storage.layout().get_field(desc, slot.field_name.as_ref())?;
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
        type_name: &str,
        field_name: &str,
    ) -> Option<FieldRef<'storage, T>> {
        let slot = self.slot(type_name, field_name)?;
        let field = storage.layout().get_field(desc, slot.field_name.as_ref())?;
        if field.layout.size().as_usize() != T::SCALAR.size_const() {
            return None;
        }
        storage.field_at_offset(field.position.as_usize())
    }

    fn span_field<'storage, T: FieldType>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
        type_name: &str,
        field_name: &str,
    ) -> Option<FieldRef<'storage, T>> {
        let slot = self.slot(type_name, field_name)?;
        let field = storage.layout().get_field(desc, slot.field_name.as_ref())?;
        if !matches!(
            field.layout.as_ref(),
            LayoutManager::Scalar(scalar) if *scalar == T::SCALAR
        ) {
            return None;
        }
        storage.field_at_offset(field.position.as_usize())
    }

    fn span_layout_field<'layout>(
        &self,
        layout: &'layout FieldLayoutManager,
        field_name: &str,
    ) -> Option<&'layout FieldLayout> {
        self.slots
            .iter()
            .filter(|slot| {
                matches!(
                    slot.type_name.as_ref(),
                    "System.Span`1" | "System.ReadOnlySpan`1"
                ) && slot.field_name.as_ref() == field_name
            })
            .find_map(|slot| layout.get_field_by_name(slot.field_name.as_ref()))
    }
}

impl AssemblyLoader {
    /// Whether this field uses the support ABI's GC-traced handle representation.
    ///
    /// The support library declares these fields as `nint` for its public `IntPtr` API, while
    /// the VM stores `ObjectRef` values in the three validated `Handle` slots.
    pub fn is_handle_value_slot(&self, desc: &TypeDescription, field_name: &str) -> bool {
        let raw_type_name = desc.type_name();
        let type_name = self.canonical_type_name(&raw_type_name);
        self.stubs
            .get(type_name)
            .is_some_and(|support_type| support_type == desc)
            && self
                .support_slot_registry
                .is_handle_value_slot(type_name, field_name)
    }

    /// Returns the support-assembly ABI slots validated while this loader was initialized.
    pub fn support_slots(&self) -> &SupportSlotRegistry {
        &self.support_slot_registry
    }
}

/// Typed access to fields governed by the support-assembly ABI contract.
///
/// The trait belongs to this crate rather than `dotnet-types`: the helpers expose
/// [`FieldStorage`] and [`FieldRef`], which are defined by `dotnet-value`, and
/// `dotnet-value` already depends on `dotnet-types`. Keeping the trait here
/// therefore avoids a `dotnet-types -> dotnet-value -> dotnet-types` dependency
/// cycle. `dotnet-vm-ops` re-exports the trait for intrinsic consumers.
pub trait SupportSlotOps {
    fn rth_value_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>>;

    fn rth_value_usize_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>>;

    fn rfh_value_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>>;

    fn rmh_value_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>>;

    fn runtime_type_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>>;

    fn method_info_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>>;

    fn constructor_info_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>>;

    fn field_info_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>>;

    fn parameter_info_method_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>>;

    fn parameter_info_position_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, i32>>;

    fn property_info_name_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>>;

    fn property_info_getter_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>>;

    fn property_info_setter_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>>;

    fn property_info_declaring_type_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>>;

    fn property_info_property_type_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>>;

    fn delegate_target_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>>;

    fn delegate_method_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>>;

    fn multicast_delegate_targets_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>>;

    fn span_reference_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ManagedPtr<'gc>>>;

    fn span_length_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, i32>>;

    fn span_reference_layout<'layout>(
        &self,
        layout: &'layout FieldLayoutManager,
    ) -> Option<&'layout FieldLayout>;

    fn span_length_layout<'layout>(
        &self,
        layout: &'layout FieldLayoutManager,
    ) -> Option<&'layout FieldLayout>;
}

impl SupportSlotOps for AssemblyLoader {
    fn rth_value_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>> {
        self.support_slot_registry
            .field(storage, desc, "System.RuntimeTypeHandle", "_value")
    }

    fn rth_value_usize_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>> {
        self.support_slot_registry.reinterpreted_field(
            storage,
            desc,
            "System.RuntimeTypeHandle",
            "_value",
        )
    }

    fn rfh_value_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>> {
        self.support_slot_registry
            .field(storage, desc, "System.RuntimeFieldHandle", "_value")
    }

    fn rmh_value_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>> {
        self.support_slot_registry
            .field(storage, desc, "System.RuntimeMethodHandle", "_value")
    }

    fn runtime_type_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>> {
        self.support_slot_registry
            .field(storage, desc, "System.RuntimeType", "index")
    }

    fn method_info_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>> {
        self.support_slot_registry
            .field(storage, desc, "DotnetRs.MethodInfo", "index")
    }

    fn constructor_info_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>> {
        self.support_slot_registry
            .field(storage, desc, "DotnetRs.ConstructorInfo", "index")
    }

    fn field_info_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>> {
        self.support_slot_registry
            .field(storage, desc, "DotnetRs.FieldInfo", "index")
    }

    fn parameter_info_method_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>> {
        self.support_slot_registry
            .field(storage, desc, "DotnetRs.ParameterInfo", "method_index")
    }

    fn parameter_info_position_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, i32>> {
        self.support_slot_registry
            .field(storage, desc, "DotnetRs.ParameterInfo", "position")
    }

    fn property_info_name_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>> {
        self.support_slot_registry
            .field(storage, desc, "DotnetRs.PropertyInfo", "name")
    }

    fn property_info_getter_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>> {
        self.support_slot_registry
            .field(storage, desc, "DotnetRs.PropertyInfo", "getter")
    }

    fn property_info_setter_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>> {
        self.support_slot_registry
            .field(storage, desc, "DotnetRs.PropertyInfo", "setter")
    }

    fn property_info_declaring_type_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>> {
        self.support_slot_registry
            .field(storage, desc, "DotnetRs.PropertyInfo", "declaringType")
    }

    fn property_info_property_type_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>> {
        self.support_slot_registry
            .field(storage, desc, "DotnetRs.PropertyInfo", "propertyType")
    }

    fn delegate_target_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>> {
        self.support_slot_registry
            .field(storage, desc, "System.Delegate", "_target")
    }

    fn delegate_method_index_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, usize>> {
        self.support_slot_registry
            .field(storage, desc, "System.Delegate", "_method")
    }

    fn multicast_delegate_targets_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ObjectRef<'gc>>> {
        self.support_slot_registry
            .field(storage, desc, "System.MulticastDelegate", "targets")
    }

    fn span_reference_field<'storage, 'gc>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, ManagedPtr<'gc>>> {
        let raw_type_name = desc.type_name();
        let type_name = self.canonical_type_name(&raw_type_name);
        self.support_slot_registry
            .span_field(storage, desc, type_name, "_reference")
    }

    fn span_length_field<'storage>(
        &self,
        storage: &'storage FieldStorage,
        desc: TypeDescription,
    ) -> Option<FieldRef<'storage, i32>> {
        let raw_type_name = desc.type_name();
        let type_name = self.canonical_type_name(&raw_type_name);
        self.support_slot_registry
            .span_field(storage, desc, type_name, "_length")
    }

    fn span_reference_layout<'layout>(
        &self,
        layout: &'layout FieldLayoutManager,
    ) -> Option<&'layout FieldLayout> {
        self.support_slot_registry
            .span_layout_field(layout, "_reference")
    }

    fn span_length_layout<'layout>(
        &self,
        layout: &'layout FieldLayoutManager,
    ) -> Option<&'layout FieldLayout> {
        self.support_slot_registry
            .span_layout_field(layout, "_length")
    }
}
