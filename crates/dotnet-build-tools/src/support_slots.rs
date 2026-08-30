//! Parser and model for the support-assembly storage ABI definition.
//!
//! The committed definition is deliberately a small, line-oriented format.  It is
//! consumed by build scripts, so this module has no dependency on runtime crates
//! or on the metadata reader.  A record has nine `|`-separated columns:
//!
//! ```text
//! id | SemanticId | WellKnown | field | instance|static | signature | storage | accessor:type | views
//! ```
//!
//! `views` is `none`, or a comma-separated list of
//! `alternate:accessor:type` and `layout:accessor`.  Supported signatures are
//! `intptr`, `int32`, `object`, `string`, `class(WellKnown)`,
//! `szarray(WellKnown)`, and `byref-generic(index)`.  Supported storage shapes are
//! `handle-object-ref`,
//! `object-ref`, `usize`, `i32`, and `managed-ptr`.

use std::{collections::HashSet, error::Error, fmt};

/// The complete declarative support-slot contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupportSlots {
    pub slots: Vec<SupportSlot>,
}

/// One runtime-managed support-assembly field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupportSlot {
    /// Stable positive semantic ID, suitable for a C# `int` and Rust `i32`.
    pub id: i32,
    pub semantic_name: String,
    pub declaring_well_known: String,
    pub field_name: String,
    pub is_static: bool,
    pub signature: FieldSignature,
    pub storage: StorageShape,
    pub accessor: SlotAccessor,
    pub views: Vec<SlotView>,
}

/// The exact decoded metadata shape expected for a field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldSignature {
    IntPtr,
    Int32,
    Object,
    String,
    Class { well_known: String },
    SzArray { element_well_known: String },
    ByRefTypeGeneric { index: u16 },
}

/// The Rust storage representation prescribed by a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageShape {
    /// An `nint` metadata field stored as a GC-traced `ObjectRef` by the VM.
    HandleObjectRef,
    /// A normal GC-traced managed reference.
    ObjectRef,
    /// A native-width runtime index.
    Usize,
    /// A fixed-width signed integer.
    I32,
    /// A serialized managed by-reference.
    ManagedPtr,
}

/// The typed primary accessor generated for a slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotAccessor {
    pub name: String,
    pub value_type: AccessorType,
}

/// A type exposed by a typed support-slot accessor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessorType {
    ObjectRef,
    Usize,
    I32,
    ManagedPtr,
}

/// An explicit non-primary way to access a support slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotView {
    /// A deliberately bounded alternate typed view of the same storage.
    Alternate(SlotAccessor),
    /// An offset-only accessor for layout consumers that do not hold an object.
    Layout { name: String },
}

/// An error emitted while parsing or validating a support-slot definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for SlotParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "support slot definition line {}: {}",
            self.line, self.message
        )
    }
}

impl Error for SlotParseError {}

/// Parses and validates a complete support-slot definition.
///
/// IDs must be contiguous from one. This prevents a deleted row from silently
/// renumbering later semantic IDs. Supporting retired IDs requires an explicit
/// schema and generated-lookup migration rather than an implicit gap.
pub fn parse_support_slots(input: &str) -> Result<SupportSlots, SlotParseError> {
    let mut slots = Vec::new();
    let mut ids = HashSet::new();
    let mut semantic_names = HashSet::new();
    let mut fields = HashSet::new();
    let mut primary_accessors = HashSet::new();
    let mut all_accessors = HashSet::new();

    for (line_index, raw_line) in input.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line
            .split_once('#')
            .map_or(raw_line, |(before, _)| before)
            .trim();
        if line.is_empty() {
            continue;
        }

        let columns: Vec<_> = line.split('|').map(str::trim).collect();
        if columns.len() != 9 {
            return Err(error(
                line_number,
                "expected exactly nine `|`-separated columns",
            ));
        }
        if columns.iter().any(|column| column.is_empty()) {
            return Err(error(line_number, "columns must not be empty"));
        }

        let id = columns[0]
            .parse::<i32>()
            .map_err(|_| error(line_number, "ID must be a positive 32-bit integer"))?;
        if id <= 0 {
            return Err(error(line_number, "ID must be positive"));
        }
        if !ids.insert(id) {
            return Err(error(line_number, format!("duplicate ID {id}")));
        }

        let semantic_name = parse_identifier(columns[1], line_number, "semantic ID")?;
        if !semantic_names.insert(semantic_name.clone()) {
            return Err(error(
                line_number,
                format!("duplicate semantic ID `{semantic_name}`"),
            ));
        }

        let declaring_well_known = parse_identifier(columns[2], line_number, "WellKnown")?;
        let field_name = parse_identifier(columns[3], line_number, "field name")?;
        if !fields.insert((declaring_well_known.clone(), field_name.clone())) {
            return Err(error(
                line_number,
                format!("duplicate field `{}.{}`", declaring_well_known, field_name),
            ));
        }

        let is_static = match columns[4] {
            "instance" => false,
            "static" => true,
            _ => {
                return Err(error(
                    line_number,
                    "staticness must be `instance` or `static`",
                ));
            }
        };
        let signature = parse_signature(columns[5], line_number)?;
        let storage = parse_storage(columns[6], line_number)?;
        let accessor = parse_accessor(columns[7], line_number, "primary accessor")?;
        if !primary_accessors.insert(accessor.name.clone()) {
            return Err(error(
                line_number,
                format!("duplicate primary accessor `{}`", accessor.name),
            ));
        }
        if !all_accessors.insert(accessor.name.clone()) {
            return Err(error(
                line_number,
                format!("duplicate accessor `{}`", accessor.name),
            ));
        }

        validate_primary_accessor(line_number, &signature, storage, &accessor)?;
        let views = parse_views(columns[8], line_number, &mut all_accessors)?;
        validate_views(line_number, &signature, storage, &accessor, &views)?;

        slots.push(SupportSlot {
            id,
            semantic_name,
            declaring_well_known,
            field_name,
            is_static,
            signature,
            storage,
            accessor,
            views,
        });
    }

    if slots.is_empty() {
        return Err(error(1, "definition must contain at least one slot"));
    }

    let mut sorted_ids: Vec<_> = ids.into_iter().collect();
    sorted_ids.sort_unstable();
    for (index, id) in sorted_ids.into_iter().enumerate() {
        let expected = i32::try_from(index + 1).expect("slot count exceeds i32 ID range");
        if id != expected {
            return Err(error(
                1,
                format!("IDs must be dense from 1; expected ID {expected}, found {id}"),
            ));
        }
    }

    slots.sort_by_key(|slot| slot.id);
    Ok(SupportSlots { slots })
}

fn parse_signature(value: &str, line: usize) -> Result<FieldSignature, SlotParseError> {
    match value {
        "intptr" => Ok(FieldSignature::IntPtr),
        "int32" => Ok(FieldSignature::Int32),
        "object" => Ok(FieldSignature::Object),
        "string" => Ok(FieldSignature::String),
        _ => {
            if let Some(well_known) = value
                .strip_prefix("class(")
                .and_then(|value| value.strip_suffix(')'))
            {
                return Ok(FieldSignature::Class {
                    well_known: parse_identifier(well_known, line, "class WellKnown")?,
                });
            }
            if let Some(well_known) = value
                .strip_prefix("szarray(")
                .and_then(|value| value.strip_suffix(')'))
            {
                return Ok(FieldSignature::SzArray {
                    element_well_known: parse_identifier(
                        well_known,
                        line,
                        "SZARRAY element WellKnown",
                    )?,
                });
            }
            if let Some(index) = value
                .strip_prefix("byref-generic(")
                .and_then(|value| value.strip_suffix(')'))
            {
                return index
                    .parse::<u16>()
                    .map(|index| FieldSignature::ByRefTypeGeneric { index })
                    .map_err(|_| {
                        error(
                            line,
                            "byref generic index must be an unsigned 16-bit integer",
                        )
                    });
            }
            Err(error(
                line,
                "unsupported signature; expected intptr, int32, object, string, class(WellKnown), szarray(WellKnown), or byref-generic(index)",
            ))
        }
    }
}

fn parse_storage(value: &str, line: usize) -> Result<StorageShape, SlotParseError> {
    match value {
        "handle-object-ref" => Ok(StorageShape::HandleObjectRef),
        "object-ref" => Ok(StorageShape::ObjectRef),
        "usize" => Ok(StorageShape::Usize),
        "i32" => Ok(StorageShape::I32),
        "managed-ptr" => Ok(StorageShape::ManagedPtr),
        _ => Err(error(
            line,
            "unsupported storage; expected handle-object-ref, object-ref, usize, i32, or managed-ptr",
        )),
    }
}

fn parse_accessor(value: &str, line: usize, context: &str) -> Result<SlotAccessor, SlotParseError> {
    let Some((name, value_type)) = value.split_once(':') else {
        return Err(error(
            line,
            format!("{context} must use `name:type` syntax"),
        ));
    };
    if value_type.contains(':') {
        return Err(error(line, format!("{context} must use exactly one `:`")));
    }
    Ok(SlotAccessor {
        name: parse_identifier(name, line, context)?,
        value_type: parse_accessor_type(value_type, line)?,
    })
}

fn parse_accessor_type(value: &str, line: usize) -> Result<AccessorType, SlotParseError> {
    match value {
        "object-ref" => Ok(AccessorType::ObjectRef),
        "usize" => Ok(AccessorType::Usize),
        "i32" => Ok(AccessorType::I32),
        "managed-ptr" => Ok(AccessorType::ManagedPtr),
        _ => Err(error(
            line,
            "unsupported accessor type; expected object-ref, usize, i32, or managed-ptr",
        )),
    }
}

fn parse_views(
    value: &str,
    line: usize,
    all_accessors: &mut HashSet<String>,
) -> Result<Vec<SlotView>, SlotParseError> {
    if value == "none" {
        return Ok(Vec::new());
    }

    let mut views = Vec::new();
    for view in value.split(',') {
        let pieces: Vec<_> = view.split(':').map(str::trim).collect();
        let parsed = match pieces.as_slice() {
            ["alternate", name, value_type] => {
                let accessor = SlotAccessor {
                    name: parse_identifier(name, line, "alternate accessor")?,
                    value_type: parse_accessor_type(value_type, line)?,
                };
                SlotView::Alternate(accessor)
            }
            ["layout", name] => SlotView::Layout {
                name: parse_identifier(name, line, "layout accessor")?,
            },
            _ => {
                return Err(error(
                    line,
                    "views must be `none`, `alternate:name:type`, or `layout:name`",
                ));
            }
        };

        let name = match &parsed {
            SlotView::Alternate(accessor) => &accessor.name,
            SlotView::Layout { name } => name,
        };
        if !all_accessors.insert(name.clone()) {
            return Err(error(line, format!("duplicate accessor `{name}`")));
        }
        views.push(parsed);
    }
    Ok(views)
}

fn validate_primary_accessor(
    line: usize,
    signature: &FieldSignature,
    storage: StorageShape,
    accessor: &SlotAccessor,
) -> Result<(), SlotParseError> {
    let expected_type = match storage {
        StorageShape::HandleObjectRef | StorageShape::ObjectRef => AccessorType::ObjectRef,
        StorageShape::Usize => AccessorType::Usize,
        StorageShape::I32 => AccessorType::I32,
        StorageShape::ManagedPtr => AccessorType::ManagedPtr,
    };
    if accessor.value_type != expected_type {
        return Err(error(
            line,
            format!(
                "storage `{}` requires a `{}` primary accessor",
                storage_name(storage),
                accessor_type_name(expected_type)
            ),
        ));
    }

    let supported = matches!(
        (signature, storage),
        (
            FieldSignature::IntPtr,
            StorageShape::HandleObjectRef | StorageShape::Usize
        ) | (FieldSignature::Int32, StorageShape::I32)
            | (FieldSignature::Object, StorageShape::ObjectRef)
            | (FieldSignature::String, StorageShape::ObjectRef)
            | (FieldSignature::Class { .. }, StorageShape::ObjectRef)
            | (FieldSignature::SzArray { .. }, StorageShape::ObjectRef)
            | (
                FieldSignature::ByRefTypeGeneric { .. },
                StorageShape::ManagedPtr
            )
    );
    if !supported {
        return Err(error(
            line,
            format!(
                "unsupported signature/storage combination: `{}` with `{}`",
                signature_name(signature),
                storage_name(storage)
            ),
        ));
    }
    Ok(())
}

fn validate_views(
    line: usize,
    signature: &FieldSignature,
    storage: StorageShape,
    primary: &SlotAccessor,
    views: &[SlotView],
) -> Result<(), SlotParseError> {
    let mut has_alternate = false;
    let mut has_layout = false;
    for view in views {
        match view {
            SlotView::Alternate(alternate) => {
                if has_alternate {
                    return Err(error(
                        line,
                        "a slot may have at most one alternate accessor",
                    ));
                }
                has_alternate = true;
                if !matches!(
                    (signature, storage, alternate.value_type),
                    (
                        FieldSignature::IntPtr,
                        StorageShape::HandleObjectRef,
                        AccessorType::Usize
                    )
                ) {
                    return Err(error(
                        line,
                        "an alternate accessor is only supported as the usize view of an intptr handle-object-ref slot",
                    ));
                }
                if alternate.value_type == primary.value_type {
                    return Err(error(
                        line,
                        "alternate accessor must expose a distinct type",
                    ));
                }
            }
            SlotView::Layout { .. } => {
                if has_layout {
                    return Err(error(line, "a slot may have at most one layout accessor"));
                }
                has_layout = true;
            }
        }
    }
    Ok(())
}

fn parse_identifier(value: &str, line: usize, context: &str) -> Result<String, SlotParseError> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(error(line, format!("{context} must not be empty")));
    };
    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(error(
            line,
            format!("{context} `{value}` must be an ASCII identifier"),
        ));
    }
    Ok(value.to_owned())
}

fn error(line: usize, message: impl Into<String>) -> SlotParseError {
    SlotParseError {
        line,
        message: message.into(),
    }
}

fn storage_name(storage: StorageShape) -> &'static str {
    match storage {
        StorageShape::HandleObjectRef => "handle-object-ref",
        StorageShape::ObjectRef => "object-ref",
        StorageShape::Usize => "usize",
        StorageShape::I32 => "i32",
        StorageShape::ManagedPtr => "managed-ptr",
    }
}

fn accessor_type_name(value_type: AccessorType) -> &'static str {
    match value_type {
        AccessorType::ObjectRef => "object-ref",
        AccessorType::Usize => "usize",
        AccessorType::I32 => "i32",
        AccessorType::ManagedPtr => "managed-ptr",
    }
}

fn signature_name(signature: &FieldSignature) -> &'static str {
    match signature {
        FieldSignature::IntPtr => "intptr",
        FieldSignature::Int32 => "int32",
        FieldSignature::Object => "object",
        FieldSignature::String => "string",
        FieldSignature::Class { .. } => "class(...)",
        FieldSignature::SzArray { .. } => "szarray(...)",
        FieldSignature::ByRefTypeGeneric { .. } => "byref-generic(...)",
    }
}

#[cfg(test)]
mod tests {
    use super::{AccessorType, FieldSignature, SlotView, StorageShape, parse_support_slots};

    const SUPPORT_SLOTS: &str = include_str!("../../dotnet-assemblies/support_slots.def");

    #[test]
    fn parses_committed_twenty_one_slot_contract() {
        let contract = parse_support_slots(SUPPORT_SLOTS).expect("committed definition must parse");

        assert_eq!(contract.slots.len(), 21);
        assert_eq!(contract.slots[0].id, 1);
        assert_eq!(contract.slots[0].semantic_name, "RuntimeTypeHandleValue");
        assert_eq!(contract.slots[0].declaring_well_known, "RuntimeTypeHandle");
        assert_eq!(contract.slots[0].signature, FieldSignature::IntPtr);
        assert_eq!(contract.slots[0].storage, StorageShape::HandleObjectRef);
        assert_eq!(
            contract.slots[0].accessor.value_type,
            AccessorType::ObjectRef
        );
        assert_eq!(contract.slots[14].signature, FieldSignature::Object);
        assert_eq!(contract.slots[20].id, 21);
        assert_eq!(contract.slots[20].semantic_name, "ReadOnlySpanLength");
    }

    #[test]
    fn parses_exact_signature_shapes_and_explicit_views() {
        let contract = parse_support_slots(
            "1 | Example | Owner | field | static | class(Type) | object-ref | primary:object-ref | none\n\
             2 | Generic | Owner2 | field | instance | byref-generic(7) | managed-ptr | generic:managed-ptr | layout:generic_layout\n\
             3 | Array | Owner3 | field | instance | szarray(Delegate) | object-ref | array:object-ref | none\n\
             4 | Handle | Owner4 | field | instance | intptr | handle-object-ref | handle:object-ref | alternate:raw:usize",
        )
        .expect("valid grammar must parse");

        assert!(contract.slots[0].is_static);
        assert_eq!(
            contract.slots[0].signature,
            FieldSignature::Class {
                well_known: "Type".into()
            }
        );
        assert_eq!(
            contract.slots[1].signature,
            FieldSignature::ByRefTypeGeneric { index: 7 }
        );
        assert_eq!(
            contract.slots[1].views,
            vec![SlotView::Layout {
                name: "generic_layout".into()
            }]
        );
        assert_eq!(
            contract.slots[2].signature,
            FieldSignature::SzArray {
                element_well_known: "Delegate".into()
            }
        );
        assert_eq!(
            contract.slots[3].views,
            vec![SlotView::Alternate(super::SlotAccessor {
                name: "raw".into(),
                value_type: AccessorType::Usize,
            })]
        );
    }

    #[test]
    fn rejects_malformed_grammar() {
        let error = parse_support_slots(
            "1 | Name | Owner | field | instance | intptr | usize | accessor:usize",
        )
        .unwrap_err();
        assert_eq!(error.line, 1);
        assert!(error.message.contains("nine"));

        let error = parse_support_slots(
            "1 | Name | Owner | field | instance | class(not-an-identifier) | object-ref | accessor:object-ref | none",
        )
        .unwrap_err();
        assert!(error.message.contains("ASCII identifier"));
    }

    #[test]
    fn rejects_duplicate_ids_semantic_names_fields_and_primary_accessors() {
        for (input, expected) in [
            (
                "1 | First | Owner | first | instance | intptr | usize | first_accessor:usize | none\n\
                 1 | Second | Owner | second | instance | intptr | usize | second_accessor:usize | none",
                "duplicate ID",
            ),
            (
                "1 | First | Owner | first | instance | intptr | usize | first_accessor:usize | none\n\
                 2 | First | Owner | second | instance | intptr | usize | second_accessor:usize | none",
                "duplicate semantic ID",
            ),
            (
                "1 | First | Owner | field | instance | intptr | usize | first_accessor:usize | none\n\
                 2 | Second | Owner | field | instance | intptr | usize | second_accessor:usize | none",
                "duplicate field",
            ),
            (
                "1 | First | Owner | first | instance | intptr | usize | accessor:usize | none\n\
                 2 | Second | Owner | second | instance | intptr | usize | accessor:usize | none",
                "duplicate primary accessor",
            ),
        ] {
            let error = parse_support_slots(input).unwrap_err();
            assert!(error.message.contains(expected), "{error}");
        }
    }

    #[test]
    fn rejects_zero_non_dense_and_missing_accessors() {
        for (input, expected) in [
            (
                "0 | First | Owner | field | instance | intptr | usize | accessor:usize | none",
                "ID must be positive",
            ),
            (
                "1 | First | Owner | first | instance | intptr | usize | first_accessor:usize | none\n\
                 3 | Third | Owner | third | instance | intptr | usize | third_accessor:usize | none",
                "IDs must be dense",
            ),
            (
                "1 | First | Owner | field | instance | intptr | usize | :usize | none",
                "must not be empty",
            ),
        ] {
            let error = parse_support_slots(input).unwrap_err();
            assert!(error.message.contains(expected), "{error}");
        }
    }

    #[test]
    fn rejects_unsupported_signature_storage_and_views() {
        for (input, expected) in [
            (
                "1 | Bad | Owner | field | instance | int32 | object-ref | accessor:object-ref | none",
                "unsupported signature/storage combination",
            ),
            (
                "1 | Bad | Owner | field | instance | intptr | usize | accessor:usize | alternate:raw:object-ref",
                "alternate accessor is only supported",
            ),
            (
                "1 | Bad | Owner | field | instance | intptr | handle-object-ref | accessor:object-ref | alternate:raw:usize,layout:first,layout:second",
                "at most one layout accessor",
            ),
        ] {
            let error = parse_support_slots(input).unwrap_err();
            assert!(error.message.contains(expected), "{error}");
        }
    }
}
