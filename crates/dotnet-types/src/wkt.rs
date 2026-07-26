//! Well-known core-library and support-library type handles.

/// A type name used statically by the runtime.
///
/// Each variant's discriminant is a contiguous index into the well-known type
/// cache maintained by a [`crate::TypeResolver`] implementation.
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WellKnown {
    // System primitives.
    Boolean,
    SByte,
    Byte,
    Single,
    Char,
    UInt16,
    Double,
    UInt32,
    Int16,
    UInt64,
    Int32,
    UIntPtr,
    Int64,
    IntPtr,

    // System core types.
    Array,
    Attribute,
    ByRef1,
    Delegate,
    Enum,
    Exception,
    ExceptionDispatchState,
    MemoryExtensions,
    MulticastDelegate,
    Nullable1,
    Object,
    ReadOnlySpan1,
    String,
    Type,
    TypedReference,
    ValueType,
    Void,

    // System.Collections.Generic types.
    ICollection1,
    IEnumerable1,
    IList1,
    IReadOnlyCollection1,
    IReadOnlyList1,

    // System.IO types.
    IoMemoryStream,

    // System.Reflection types.
    ReflectionAssembly,
    ReflectionAssemblyName,
    ReflectionConstructorInfo,
    ReflectionFieldInfo,
    ReflectionMemberInfo,
    ReflectionMethodInfo,
    ReflectionParameterInfo,
    ReflectionPropertyInfo,
    ReflectionRuntimeAssembly,
    ReflectionTypeDelegator,

    // System.Runtime handles and types.
    RuntimeFieldHandle,
    RuntimeMethodHandle,
    RuntimeType,
    RuntimeTypeHandle,

    // DotnetRs support-library types.
    SupportComparersEqualityComparer1,
    SupportComparersOrderingComparer1,
    SupportConstructorInfo,
    SupportFieldInfo,
    SupportMethodInfo,
    SupportParameterInfo,
    SupportPropertyInfo,
    SupportSZArrayHelper1,
}

impl WellKnown {
    /// Number of well-known type handles.
    pub const COUNT: usize = Self::SupportSZArrayHelper1 as usize + 1;

    /// Returns the canonical metadata name for this type.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Boolean => "System.Boolean",
            Self::SByte => "System.SByte",
            Self::Byte => "System.Byte",
            Self::Single => "System.Single",
            Self::Char => "System.Char",
            Self::UInt16 => "System.UInt16",
            Self::Double => "System.Double",
            Self::UInt32 => "System.UInt32",
            Self::Int16 => "System.Int16",
            Self::UInt64 => "System.UInt64",
            Self::Int32 => "System.Int32",
            Self::UIntPtr => "System.UIntPtr",
            Self::Int64 => "System.Int64",
            Self::IntPtr => "System.IntPtr",
            Self::Array => "System.Array",
            Self::Attribute => "System.Attribute",
            Self::ByRef1 => "System.ByReference`1",
            Self::Delegate => "System.Delegate",
            Self::Enum => "System.Enum",
            Self::Exception => "System.Exception",
            Self::ExceptionDispatchState => "System.Exception+DispatchState",
            Self::MemoryExtensions => "System.MemoryExtensions",
            Self::MulticastDelegate => "System.MulticastDelegate",
            Self::Nullable1 => "System.Nullable`1",
            Self::Object => "System.Object",
            Self::ReadOnlySpan1 => "System.ReadOnlySpan`1",
            Self::String => "System.String",
            Self::Type => "System.Type",
            Self::TypedReference => "System.TypedReference",
            Self::ValueType => "System.ValueType",
            Self::Void => "System.Void",
            Self::ICollection1 => "System.Collections.Generic.ICollection`1",
            Self::IEnumerable1 => "System.Collections.Generic.IEnumerable`1",
            Self::IList1 => "System.Collections.Generic.IList`1",
            Self::IReadOnlyCollection1 => "System.Collections.Generic.IReadOnlyCollection`1",
            Self::IReadOnlyList1 => "System.Collections.Generic.IReadOnlyList`1",
            Self::IoMemoryStream => "System.IO.MemoryStream",
            Self::ReflectionAssembly => "System.Reflection.Assembly",
            Self::ReflectionAssemblyName => "System.Reflection.AssemblyName",
            Self::ReflectionConstructorInfo => "System.Reflection.ConstructorInfo",
            Self::ReflectionFieldInfo => "System.Reflection.FieldInfo",
            Self::ReflectionMemberInfo => "System.Reflection.MemberInfo",
            Self::ReflectionMethodInfo => "System.Reflection.MethodInfo",
            Self::ReflectionParameterInfo => "System.Reflection.ParameterInfo",
            Self::ReflectionPropertyInfo => "System.Reflection.PropertyInfo",
            Self::ReflectionRuntimeAssembly => "System.Reflection.RuntimeAssembly",
            Self::ReflectionTypeDelegator => "System.Reflection.TypeDelegator",
            Self::RuntimeFieldHandle => "System.RuntimeFieldHandle",
            Self::RuntimeMethodHandle => "System.RuntimeMethodHandle",
            Self::RuntimeType => "System.RuntimeType",
            Self::RuntimeTypeHandle => "System.RuntimeTypeHandle",
            Self::SupportComparersEqualityComparer1 => {
                "DotnetRs.Comparers.Equality/GenericEqualityComparer`1"
            }
            Self::SupportComparersOrderingComparer1 => {
                "DotnetRs.Comparers.Ordering/FallbackComparer`1"
            }
            Self::SupportConstructorInfo => "DotnetRs.ConstructorInfo",
            Self::SupportFieldInfo => "DotnetRs.FieldInfo",
            Self::SupportMethodInfo => "DotnetRs.MethodInfo",
            Self::SupportParameterInfo => "DotnetRs.ParameterInfo",
            Self::SupportPropertyInfo => "DotnetRs.PropertyInfo",
            Self::SupportSZArrayHelper1 => "DotnetRs.SZArrayHelper`1",
        }
    }

    /// Returns the handle corresponding to a metadata type name, if known.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "System.Boolean" => Self::Boolean,
            "System.SByte" => Self::SByte,
            "System.Byte" => Self::Byte,
            "System.Single" => Self::Single,
            "System.Char" => Self::Char,
            "System.UInt16" => Self::UInt16,
            "System.Double" => Self::Double,
            "System.UInt32" => Self::UInt32,
            "System.Int16" => Self::Int16,
            "System.UInt64" => Self::UInt64,
            "System.Int32" => Self::Int32,
            "System.UIntPtr" => Self::UIntPtr,
            "System.Int64" => Self::Int64,
            "System.IntPtr" => Self::IntPtr,
            "System.Array" => Self::Array,
            "System.Attribute" => Self::Attribute,
            "System.ByReference`1" => Self::ByRef1,
            "System.Delegate" => Self::Delegate,
            "System.Enum" => Self::Enum,
            "System.Exception" => Self::Exception,
            "System.Exception+DispatchState" | "System.Exception/DispatchState" => {
                Self::ExceptionDispatchState
            }
            "System.MemoryExtensions" => Self::MemoryExtensions,
            "System.MulticastDelegate" => Self::MulticastDelegate,
            "System.Nullable`1" => Self::Nullable1,
            "System.Object" => Self::Object,
            "System.ReadOnlySpan`1" => Self::ReadOnlySpan1,
            "System.String" => Self::String,
            "System.Type" => Self::Type,
            "System.TypedReference" => Self::TypedReference,
            "System.ValueType" => Self::ValueType,
            "System.Void" => Self::Void,
            "System.Collections.Generic.ICollection`1" => Self::ICollection1,
            "System.Collections.Generic.IEnumerable`1" => Self::IEnumerable1,
            "System.Collections.Generic.IList`1" => Self::IList1,
            "System.Collections.Generic.IReadOnlyCollection`1" => Self::IReadOnlyCollection1,
            "System.Collections.Generic.IReadOnlyList`1" => Self::IReadOnlyList1,
            "System.IO.MemoryStream" => Self::IoMemoryStream,
            "System.Reflection.Assembly" => Self::ReflectionAssembly,
            "System.Reflection.AssemblyName" => Self::ReflectionAssemblyName,
            "System.Reflection.ConstructorInfo" => Self::ReflectionConstructorInfo,
            "System.Reflection.FieldInfo" => Self::ReflectionFieldInfo,
            "System.Reflection.MemberInfo" => Self::ReflectionMemberInfo,
            "System.Reflection.MethodInfo" => Self::ReflectionMethodInfo,
            "System.Reflection.ParameterInfo" => Self::ReflectionParameterInfo,
            "System.Reflection.PropertyInfo" => Self::ReflectionPropertyInfo,
            "System.Reflection.RuntimeAssembly" => Self::ReflectionRuntimeAssembly,
            "System.Reflection.TypeDelegator" => Self::ReflectionTypeDelegator,
            "System.RuntimeFieldHandle" => Self::RuntimeFieldHandle,
            "System.RuntimeMethodHandle" => Self::RuntimeMethodHandle,
            "System.RuntimeType" => Self::RuntimeType,
            "System.RuntimeTypeHandle" => Self::RuntimeTypeHandle,
            "DotnetRs.Comparers.Equality/GenericEqualityComparer`1" => {
                Self::SupportComparersEqualityComparer1
            }
            "DotnetRs.Comparers.Ordering/FallbackComparer`1" => {
                Self::SupportComparersOrderingComparer1
            }
            "DotnetRs.ConstructorInfo" => Self::SupportConstructorInfo,
            "DotnetRs.FieldInfo" => Self::SupportFieldInfo,
            "DotnetRs.MethodInfo" => Self::SupportMethodInfo,
            "DotnetRs.ParameterInfo" => Self::SupportParameterInfo,
            "DotnetRs.PropertyInfo" => Self::SupportPropertyInfo,
            "DotnetRs.SZArrayHelper`1" => Self::SupportSZArrayHelper1,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::WellKnown;
    use crate::{
        TypeDescription, TypeResolver, error::TypeResolutionError, generics::ConcreteType,
        resolution::ResolutionS,
    };
    use dotnetdll::prelude::UserType;
    use std::{cell::RefCell, collections::HashSet};

    const ALL: [WellKnown; WellKnown::COUNT] = [
        WellKnown::Boolean,
        WellKnown::SByte,
        WellKnown::Byte,
        WellKnown::Single,
        WellKnown::Char,
        WellKnown::UInt16,
        WellKnown::Double,
        WellKnown::UInt32,
        WellKnown::Int16,
        WellKnown::UInt64,
        WellKnown::Int32,
        WellKnown::UIntPtr,
        WellKnown::Int64,
        WellKnown::IntPtr,
        WellKnown::Array,
        WellKnown::Attribute,
        WellKnown::ByRef1,
        WellKnown::Delegate,
        WellKnown::Enum,
        WellKnown::Exception,
        WellKnown::ExceptionDispatchState,
        WellKnown::MemoryExtensions,
        WellKnown::MulticastDelegate,
        WellKnown::Nullable1,
        WellKnown::Object,
        WellKnown::ReadOnlySpan1,
        WellKnown::String,
        WellKnown::Type,
        WellKnown::TypedReference,
        WellKnown::ValueType,
        WellKnown::Void,
        WellKnown::ICollection1,
        WellKnown::IEnumerable1,
        WellKnown::IList1,
        WellKnown::IReadOnlyCollection1,
        WellKnown::IReadOnlyList1,
        WellKnown::IoMemoryStream,
        WellKnown::ReflectionAssembly,
        WellKnown::ReflectionAssemblyName,
        WellKnown::ReflectionConstructorInfo,
        WellKnown::ReflectionFieldInfo,
        WellKnown::ReflectionMemberInfo,
        WellKnown::ReflectionMethodInfo,
        WellKnown::ReflectionParameterInfo,
        WellKnown::ReflectionPropertyInfo,
        WellKnown::ReflectionRuntimeAssembly,
        WellKnown::ReflectionTypeDelegator,
        WellKnown::RuntimeFieldHandle,
        WellKnown::RuntimeMethodHandle,
        WellKnown::RuntimeType,
        WellKnown::RuntimeTypeHandle,
        WellKnown::SupportComparersEqualityComparer1,
        WellKnown::SupportComparersOrderingComparer1,
        WellKnown::SupportConstructorInfo,
        WellKnown::SupportFieldInfo,
        WellKnown::SupportMethodInfo,
        WellKnown::SupportParameterInfo,
        WellKnown::SupportPropertyInfo,
        WellKnown::SupportSZArrayHelper1,
    ];

    #[test]
    fn names_round_trip_and_discriminants_are_contiguous() {
        let mut names = HashSet::new();

        for (index, handle) in ALL.into_iter().enumerate() {
            assert_eq!(handle as usize, index);
            assert_eq!(WellKnown::from_name(handle.name()), Some(handle));
            assert!(
                names.insert(handle.name()),
                "duplicate name: {}",
                handle.name()
            );
        }

        assert_eq!(names.len(), WellKnown::COUNT);
    }

    #[test]
    fn from_name_accepts_dispatch_state_alias_and_rejects_unknown_names() {
        assert_eq!(
            WellKnown::from_name("System.Exception/DispatchState"),
            Some(WellKnown::ExceptionDispatchState)
        );
        assert_eq!(WellKnown::from_name("System.NotARealType"), None);
    }

    struct RecordingResolver {
        name: RefCell<Option<String>>,
    }

    impl TypeResolver for RecordingResolver {
        fn corlib_type(&self, name: &str) -> Result<TypeDescription, TypeResolutionError> {
            self.name.replace(Some(name.to_owned()));
            Err(TypeResolutionError::TypeNotFound(
                "expected test error".into(),
            ))
        }

        fn locate_type(
            &self,
            _resolution: ResolutionS,
            _handle: UserType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            unreachable!("not used by this test")
        }

        fn find_concrete_type(
            &self,
            _ty: ConcreteType,
        ) -> Result<TypeDescription, TypeResolutionError> {
            unreachable!("not used by this test")
        }
    }

    #[test]
    fn type_resolver_default_method_uses_the_canonical_name() {
        let resolver = RecordingResolver {
            name: RefCell::new(None),
        };

        assert!(resolver.corlib_wkt(WellKnown::Int32).is_err());
        assert_eq!(resolver.name.borrow().as_deref(), Some("System.Int32"));
    }
}
