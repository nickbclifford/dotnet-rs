use dotnet_build_tools::{
    find_repo_root, shared_msbuild_input_candidates, should_skip_dotnet_build,
    support_slots::{AccessorType, FieldSignature, SlotView, StorageShape, SupportSlots},
};
use std::{fmt::Write as _, path::Path, process::Command};

fn rust_signature(signature: &FieldSignature) -> String {
    match signature {
        FieldSignature::IntPtr => "RuntimeSlotSignature::IntPtr".into(),
        FieldSignature::Int32 => "RuntimeSlotSignature::Int32".into(),
        FieldSignature::Object => "RuntimeSlotSignature::Object".into(),
        FieldSignature::String => "RuntimeSlotSignature::String".into(),
        FieldSignature::Class { well_known } => {
            format!("RuntimeSlotSignature::Class(WellKnown::{well_known})")
        }
        FieldSignature::SzArray { element_well_known } => {
            format!("RuntimeSlotSignature::SzArray(WellKnown::{element_well_known})")
        }
        FieldSignature::ByRefTypeGeneric { index } => {
            format!("RuntimeSlotSignature::ByRefTypeGeneric({index})")
        }
    }
}

fn rust_storage(storage: StorageShape) -> &'static str {
    match storage {
        StorageShape::HandleObjectRef => "RuntimeSlotStorage::HandleObjectRef",
        StorageShape::ObjectRef => "RuntimeSlotStorage::ObjectRef",
        StorageShape::Usize => "RuntimeSlotStorage::Usize",
        StorageShape::I32 => "RuntimeSlotStorage::I32",
        StorageShape::ManagedPtr => "RuntimeSlotStorage::ManagedPtr",
    }
}

fn rust_accessor_type(value_type: AccessorType) -> &'static str {
    match value_type {
        AccessorType::ObjectRef => "RuntimeSlotAccessorType::ObjectRef",
        AccessorType::Usize => "RuntimeSlotAccessorType::Usize",
        AccessorType::I32 => "RuntimeSlotAccessorType::I32",
        AccessorType::ManagedPtr => "RuntimeSlotAccessorType::ManagedPtr",
    }
}

fn rust_accessor_value_type(value_type: AccessorType) -> &'static str {
    match value_type {
        AccessorType::ObjectRef => "ObjectRef<'gc>",
        AccessorType::Usize => "usize",
        AccessorType::I32 => "i32",
        AccessorType::ManagedPtr => "ManagedPtr<'gc>",
    }
}

fn rust_accessor_lifetimes(value_type: AccessorType) -> &'static str {
    match value_type {
        AccessorType::ObjectRef | AccessorType::ManagedPtr => "'storage, 'gc",
        AccessorType::Usize | AccessorType::I32 => "'storage",
    }
}

fn generate_support_slot_ops(slots: &SupportSlots) -> String {
    let mut output = String::from(
        "/// Typed access to fields governed by the support-assembly ABI contract.\n///\n/// This complete surface is generated from `support_slots.def`.\npub trait SupportSlotOps {\n",
    );
    for slot in &slots.slots {
        writeln!(output, "    fn {name}<{lifetimes}>(&self, storage: &'storage FieldStorage, desc: TypeDescription) -> Option<FieldRef<'storage, {value_type}>>;", name = slot.accessor.name, lifetimes = rust_accessor_lifetimes(slot.accessor.value_type), value_type = rust_accessor_value_type(slot.accessor.value_type)).unwrap();
        for view in &slot.views {
            match view {
                SlotView::Alternate(accessor) => writeln!(output, "    fn {name}<{lifetimes}>(&self, storage: &'storage FieldStorage, desc: TypeDescription) -> Option<FieldRef<'storage, {value_type}>>;", name = accessor.name, lifetimes = rust_accessor_lifetimes(accessor.value_type), value_type = rust_accessor_value_type(accessor.value_type)).unwrap(),
                SlotView::Layout { name } => writeln!(output, "    fn {name}<'layout>(&self, layout: &'layout FieldLayoutManager) -> Option<&'layout FieldLayout>;").unwrap(),
            }
        }
    }
    output.push_str("\n    /// Explicitly bounded convenience for code that accepts either span representation.\n    fn span_or_readonly_span_reference_field<'storage, 'gc>(&self, storage: &'storage FieldStorage, desc: TypeDescription) -> Option<FieldRef<'storage, ManagedPtr<'gc>>> {\n        self.span_reference_field(storage, desc.clone()).or_else(|| self.readonly_span_reference_field(storage, desc))\n    }\n\n    /// Explicitly bounded convenience for code that accepts either span representation.\n    fn span_or_readonly_span_length_field<'storage>(&self, storage: &'storage FieldStorage, desc: TypeDescription) -> Option<FieldRef<'storage, i32>> {\n        self.span_length_field(storage, desc.clone()).or_else(|| self.readonly_span_length_field(storage, desc))\n    }\n\n    /// Explicitly bounded convenience for layout-only code that accepts either span representation.\n    fn span_or_readonly_span_reference_layout<'layout>(&self, layout: &'layout FieldLayoutManager) -> Option<&'layout FieldLayout> {\n        self.span_reference_layout(layout).or_else(|| self.readonly_span_reference_layout(layout))\n    }\n\n    /// Explicitly bounded convenience for layout-only code that accepts either span representation.\n    fn span_or_readonly_span_length_layout<'layout>(&self, layout: &'layout FieldLayoutManager) -> Option<&'layout FieldLayout> {\n        self.span_length_layout(layout).or_else(|| self.readonly_span_length_layout(layout))\n    }\n}\n\nimpl SupportSlotOps for AssemblyLoader {\n");
    for slot in &slots.slots {
        writeln!(output, "    fn {name}<{lifetimes}>(&self, storage: &'storage FieldStorage, desc: TypeDescription) -> Option<FieldRef<'storage, {value_type}>> {{ self.support_slot_registry.field(storage, desc, RuntimeSlotId::{id}) }}", name = slot.accessor.name, lifetimes = rust_accessor_lifetimes(slot.accessor.value_type), value_type = rust_accessor_value_type(slot.accessor.value_type), id = slot.semantic_name).unwrap();
        for view in &slot.views {
            match view {
                SlotView::Alternate(accessor) => writeln!(output, "    fn {name}<{lifetimes}>(&self, storage: &'storage FieldStorage, desc: TypeDescription) -> Option<FieldRef<'storage, {value_type}>> {{ self.support_slot_registry.reinterpreted_field(storage, desc, RuntimeSlotId::{id}) }}", name = accessor.name, lifetimes = rust_accessor_lifetimes(accessor.value_type), value_type = rust_accessor_value_type(accessor.value_type), id = slot.semantic_name).unwrap(),
                SlotView::Layout { name } => writeln!(output, "    fn {name}<'layout>(&self, layout: &'layout FieldLayoutManager) -> Option<&'layout FieldLayout> {{ self.support_slot_registry.layout_field(layout, RuntimeSlotId::{id}) }}", id = slot.semantic_name).unwrap(),
            }
        }
    }
    output.push_str("}\n");
    output
}

fn generate_rust_slots(slots: &SupportSlots) -> String {
    let mut output = String::from(
        "// @generated by crates/dotnet-assemblies/build.rs from support_slots.def.\n\
         // Do not edit this file directly.\n\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\n\
         #[repr(i32)]\n\
         pub enum RuntimeSlotId {\n",
    );
    for slot in &slots.slots {
        writeln!(output, "    {} = {},", slot.semantic_name, slot.id).unwrap();
    }
    output.push_str(
        "}\n\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\n\
         pub enum RuntimeSlotSignature {\n\
             IntPtr,\n\
             Int32,\n\
             Object,\n\
             String,\n\
             Class(WellKnown),\n\
             SzArray(WellKnown),\n\
             ByRefTypeGeneric(u16),\n\
         }\n\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\n\
         pub enum RuntimeSlotStorage {\n\
             HandleObjectRef,\n\
             ObjectRef,\n\
             Usize,\n\
             I32,\n\
             ManagedPtr,\n\
         }\n\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\n\
         pub enum RuntimeSlotAccessorType {\n\
             ObjectRef,\n\
             Usize,\n\
             I32,\n\
             ManagedPtr,\n\
         }\n\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\n\
         pub struct RuntimeSlotAccessor {\n\
             pub name: &'static str,\n\
             pub value_type: RuntimeSlotAccessorType,\n\
         }\n\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\n\
         pub enum RuntimeSlotView {\n\
             Alternate(RuntimeSlotAccessor),\n\
             Layout { name: &'static str },\n\
         }\n\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\n\
         pub struct RuntimeSlotDescriptor {\n\
             pub id: RuntimeSlotId,\n\
             pub semantic_name: &'static str,\n\
             pub declaring_well_known: WellKnown,\n\
             pub field_name: &'static str,\n\
             pub is_static: bool,\n\
             pub signature: RuntimeSlotSignature,\n\
             pub storage: RuntimeSlotStorage,\n\
             pub primary_accessor: RuntimeSlotAccessor,\n\
             pub views: &'static [RuntimeSlotView],\n\
         }\n\n\
         pub static RUNTIME_SLOT_DESCRIPTORS: &[RuntimeSlotDescriptor] = &[\n",
    );
    for slot in &slots.slots {
        writeln!(
            output,
            "    RuntimeSlotDescriptor {{ id: RuntimeSlotId::{semantic}, semantic_name: {semantic:?}, declaring_well_known: WellKnown::{owner}, field_name: {field:?}, is_static: {is_static}, signature: {signature}, storage: {storage}, primary_accessor: RuntimeSlotAccessor {{ name: {accessor:?}, value_type: {accessor_type} }}, views: {views} }},",
            semantic = slot.semantic_name,
            owner = slot.declaring_well_known,
            field = slot.field_name,
            is_static = slot.is_static,
            signature = rust_signature(&slot.signature),
            storage = rust_storage(slot.storage),
            accessor = slot.accessor.name,
            accessor_type = rust_accessor_type(slot.accessor.value_type),
            views = {
                let mut views = String::from("&[");
                for view in &slot.views {
                    match view {
                        SlotView::Alternate(accessor) => write!(
                            views,
                            "RuntimeSlotView::Alternate(RuntimeSlotAccessor {{ name: {:?}, value_type: {} }}),",
                            accessor.name,
                            rust_accessor_type(accessor.value_type),
                        )
                        .unwrap(),
                        SlotView::Layout { name } => {
                            write!(views, "RuntimeSlotView::Layout {{ name: {name:?} }},").unwrap()
                        }
                    }
                }
                views.push(']');
                views
            },
        )
        .unwrap();
    }
    output.push_str("\n];\n\nimpl RuntimeSlotId {\n");
    writeln!(
        output,
        "    pub const COUNT: usize = {};",
        slots.slots.len()
    )
    .unwrap();
    output.push_str(
        "\n    pub const fn from_i32(value: i32) -> Option<Self> {\n        match value {\n",
    );
    for slot in &slots.slots {
        writeln!(
            output,
            "            {} => Some(Self::{}),",
            slot.id, slot.semantic_name
        )
        .unwrap();
    }
    output.push_str(
        "            _ => None,\n        }\n    }\n\n    pub const fn descriptor(self) -> &'static RuntimeSlotDescriptor {\n        &RUNTIME_SLOT_DESCRIPTORS[self as usize - 1]\n    }\n}\n",
    );
    output.push('\n');
    output.push_str(&generate_support_slot_ops(slots));
    output
}

fn generate_csharp_slots(slots: &SupportSlots) -> String {
    let mut output = String::from(
        "// <auto-generated />\n// Generated by crates/dotnet-assemblies/build.rs from support_slots.def.\n\nnamespace DotnetRs;\n\ninternal enum RuntimeSlotId\n{\n",
    );
    for slot in &slots.slots {
        writeln!(output, "    {} = {},", slot.semantic_name, slot.id).unwrap();
    }
    output.push_str("}\n");
    output
}

fn main() {
    println!("cargo:rerun-if-changed=src/support/support.csproj");
    println!("cargo:rerun-if-changed=src/support");
    println!("cargo:rerun-if-env-changed=DOTNET_SKIP_BUILD");
    println!("cargo:rerun-if-env-changed=RUSTC_WORKSPACE_WRAPPER");
    println!("cargo:rerun-if-env-changed=RUSTC_WRAPPER");
    println!("cargo:rerun-if-env-changed=CLIPPY_ARGS");
    println!("cargo:rerun-if-changed=support_slots.def");

    fn watch_dir(dir: &Path) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap().to_str().unwrap();
                if name != "bin" && name != "obj" {
                    watch_dir(&path);
                }
            } else {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let support_dir = manifest_dir.join("src/support");
    let repo_root = find_repo_root(manifest_dir);
    for input in shared_msbuild_input_candidates(&support_dir, &repo_root) {
        // Emit these even when missing so introducing one later retriggers build.rs.
        println!("cargo:rerun-if-changed={}", input.display());
    }

    if support_dir.exists() {
        watch_dir(&support_dir);
    }

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let support_slots_path = manifest_dir.join("support_slots.def");
    let support_slots = dotnet_build_tools::support_slots::parse_support_slots(
        &std::fs::read_to_string(&support_slots_path).expect("failed to read support_slots.def"),
    )
    .expect("invalid support_slots.def");
    let rust_slots_path = Path::new(&out_dir).join("support_slots.rs");
    std::fs::write(&rust_slots_path, generate_rust_slots(&support_slots))
        .expect("failed to generate support_slots.rs");
    let generated_support_dir = Path::new(&out_dir).join("support-generated");
    std::fs::create_dir_all(&generated_support_dir)
        .expect("failed to create generated support source directory");
    let generated_csharp_path = generated_support_dir.join("RuntimeSlotId.g.cs");
    std::fs::write(
        &generated_csharp_path,
        generate_csharp_slots(&support_slots),
    )
    .expect("failed to generate RuntimeSlotId.g.cs");
    let dll_path = Path::new(&out_dir).join("support.dll");

    if should_skip_dotnet_build() {
        // Create an empty stub so that `include_bytes!` in lib.rs can resolve the path.
        // The real DLL is only needed at runtime, not during check/clippy.
        if !dll_path.exists() {
            std::fs::write(&dll_path, b"").expect("failed to create stub support.dll");
        }
        return;
    }

    let status = Command::new("dotnet")
        .args([
            "build",
            "src/support/support.csproj",
            "-c",
            "Debug",
            "-o",
            &out_dir,
            &format!("-p:IntermediateOutputPath={}/support-obj/", out_dir),
            &format!(
                "-p:RuntimeSlotGeneratedFile={}",
                generated_csharp_path.display()
            ),
        ])
        .status()
        .expect("failed to run dotnet build");

    if !status.success() {
        panic!("dotnet build failed for support library");
    }
}
