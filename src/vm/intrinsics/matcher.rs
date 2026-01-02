use crate::utils::decompose_type_source;
use crate::value::{FieldDescription, MethodDescription};
use dotnetdll::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum TypeMatcher {
    String,
    Int32,
    NativeInt,
    Object,
    Bool,
    Char,
    Void,
    Any,
    ValueType(&'static str),
    MethodGeneric(u16),
    ManagedPtr(Box<TypeMatcher>),
    Pointer(Box<TypeMatcher>),
    Array(Box<TypeMatcher>),
    // Special case for common ones
    ReadOnlySpan(Box<TypeMatcher>),
}

impl TypeMatcher {
    pub fn matches(&self, t: &ParameterType<MethodType>) -> bool {
        match (self, t) {
            (TypeMatcher::Any, _) => true,
            (TypeMatcher::ManagedPtr(inner), ParameterType::Ref(t)) => inner.matches_type(t),
            (inner, ParameterType::Value(t)) => inner.matches_type(t),
            _ => false,
        }
    }

    fn matches_type(&self, t: &MethodType) -> bool {
        match (self, t) {
            (TypeMatcher::Any, _) => true,
            (TypeMatcher::String, MethodType::Base(b)) => matches!(**b, BaseType::String),
            (TypeMatcher::Int32, MethodType::Base(b)) => matches!(**b, BaseType::Int32),
            (TypeMatcher::NativeInt, MethodType::Base(b)) => {
                matches!(**b, BaseType::IntPtr | BaseType::UIntPtr)
            }
            (TypeMatcher::Object, MethodType::Base(b)) => matches!(**b, BaseType::Object),
            (TypeMatcher::Bool, MethodType::Base(b)) => matches!(**b, BaseType::Boolean),
            (TypeMatcher::Char, MethodType::Base(b)) => matches!(**b, BaseType::Char),
            (TypeMatcher::ValueType(_name), MethodType::Base(b)) => {
                if let BaseType::Type { source, .. } = &**b {
                    let (_ut, _) = decompose_type_source(source);
                    match _ut {
                        UserType::Definition(_d) => {
                            // This is still a bit hacky, we should ideally check the name properly
                            true
                        }
                        _ => false,
                    }
                } else {
                    false
                }
            }
            (TypeMatcher::MethodGeneric(i), MethodType::MethodGeneric(mi)) => *i as usize == *mi,
            (TypeMatcher::Pointer(inner), MethodType::Base(b)) => match &**b {
                BaseType::ValuePointer(_, Some(t)) => inner.matches_type(t),
                BaseType::ValuePointer(_, None) => {
                    matches!(**inner, TypeMatcher::Void | TypeMatcher::Any)
                }
                _ => false,
            },
            (TypeMatcher::ReadOnlySpan(inner), MethodType::Base(b)) => {
                if let BaseType::Type { source, .. } = &**b {
                    let (_ut, generics) = decompose_type_source(source);
                    // Check if _ut is System.ReadOnlySpan`1
                    // For now we just check if it has 1 generic param and that matches
                    generics.len() == 1 && inner.matches_type(&generics[0])
                } else {
                    false
                }
            }
            (TypeMatcher::Array(inner), MethodType::Base(b)) => match &**b {
                BaseType::Vector(_, t) => inner.matches_type(t),
                BaseType::Array(t, _) => inner.matches_type(t),
                _ => false,
            },
            _ => false,
        }
    }
}

pub fn matches_method(
    method: MethodDescription,
    is_static: bool,
    type_name: &str,
    method_name: &str,
    params: &[TypeMatcher],
    generic_params: u16,
) -> bool {
    if is_static == method.method.signature.instance
        || (method.parent.type_name() != type_name)
        || (method.method.name != method_name)
        || (method.method.signature.parameters.len() != params.len())
        || (method.method.generic_parameters.len() as u16 != generic_params)
    {
        return false;
    }

    for (i, p) in params.iter().enumerate() {
        if !p.matches(&method.method.signature.parameters[i].1) {
            return false;
        }
    }

    true
}

pub fn matches_field(
    field: FieldDescription,
    is_static: bool,
    type_name: &str,
    field_name: &str,
) -> bool {
    (is_static == field.field.static_member)
        && (field.parent.type_name() == type_name)
        && (field.field.name == field_name)
}

#[macro_export]
macro_rules! parse_type {
    (string) => { $crate::vm::intrinsics::matcher::TypeMatcher::String };
    (int) => { $crate::vm::intrinsics::matcher::TypeMatcher::Int32 };
    (nint) => { $crate::vm::intrinsics::matcher::TypeMatcher::NativeInt };
    (object) => { $crate::vm::intrinsics::matcher::TypeMatcher::Object };
    (bool) => { $crate::vm::intrinsics::matcher::TypeMatcher::Bool };
    (char) => { $crate::vm::intrinsics::matcher::TypeMatcher::Char };
    (void) => { $crate::vm::intrinsics::matcher::TypeMatcher::Void };
    (any) => { $crate::vm::intrinsics::matcher::TypeMatcher::Any };
    (! ! $n:literal) => { $crate::vm::intrinsics::matcher::TypeMatcher::MethodGeneric($n) };
    ($t:ident [ ]) => { $crate::vm::intrinsics::matcher::TypeMatcher::Array(Box::new($crate::parse_type!($t))) };
    ($($t:ident).+ [ ]) => { $crate::vm::intrinsics::matcher::TypeMatcher::Array(Box::new($crate::parse_type!($($t).+))) };
    (ReadOnlySpan < ! ! $n:literal >) => { $crate::vm::intrinsics::matcher::TypeMatcher::ReadOnlySpan(Box::new($crate::vm::intrinsics::matcher::TypeMatcher::MethodGeneric($n))) };
    (ReadOnlySpan < $t:ident >) => { $crate::vm::intrinsics::matcher::TypeMatcher::ReadOnlySpan(Box::new($crate::parse_type!($t))) };
    (ReadOnlySpan < $($t:ident).+ >) => { $crate::vm::intrinsics::matcher::TypeMatcher::ReadOnlySpan(Box::new($crate::parse_type!($($t).+))) };
    (ref ! ! $n:literal) => { $crate::vm::intrinsics::matcher::TypeMatcher::ManagedPtr(Box::new($crate::vm::intrinsics::matcher::TypeMatcher::MethodGeneric($n))) };
    (ref $t:ident) => { $crate::vm::intrinsics::matcher::TypeMatcher::ManagedPtr(Box::new($crate::parse_type!($t))) };
    (ref $($t:ident).+) => { $crate::vm::intrinsics::matcher::TypeMatcher::ManagedPtr(Box::new($crate::parse_type!($($t).+))) };
    (ref $name:literal) => { $crate::vm::intrinsics::matcher::TypeMatcher::ManagedPtr(Box::new($crate::vm::intrinsics::matcher::TypeMatcher::ValueType($name))) };
    (* ! ! $n:literal) => { $crate::vm::intrinsics::matcher::TypeMatcher::Pointer(Box::new($crate::vm::intrinsics::matcher::TypeMatcher::MethodGeneric($n))) };
    (* $t:ident) => { $crate::vm::intrinsics::matcher::TypeMatcher::Pointer(Box::new($crate::parse_type!($t))) };
    (* $($t:ident).+) => { $crate::vm::intrinsics::matcher::TypeMatcher::Pointer(Box::new($crate::parse_type!($($t).+))) };
    (* $name:literal) => { $crate::vm::intrinsics::matcher::TypeMatcher::Pointer(Box::new($crate::vm::intrinsics::matcher::TypeMatcher::ValueType($name))) };
    ($($t:ident).+) => { $crate::vm::intrinsics::matcher::TypeMatcher::ValueType(stringify!($($t).+)) };
    ($name:literal) => { $crate::vm::intrinsics::matcher::TypeMatcher::ValueType($name) };
    ($other:expr) => { $other };
}

#[macro_export]
macro_rules! __munch_types {
    ([ $($acc:expr,)* ] ) => {
        [ $($acc),* ]
    };

    // Comma
    ([ $($acc:expr,)* ] , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* ] $($rest)*)
    };

    // ReadOnlySpan<T>
    ([ $($acc:expr,)* ] ReadOnlySpan < ! ! $n:literal > , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ReadOnlySpan < ! ! $n >), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] ReadOnlySpan < ! ! $n:literal >) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ReadOnlySpan < ! ! $n >), ])
    };
    ([ $($acc:expr,)* ] ReadOnlySpan < $t:ident > , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ReadOnlySpan < $t >), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] ReadOnlySpan < $t:ident >) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ReadOnlySpan < $t >), ])
    };
    ([ $($acc:expr,)* ] ReadOnlySpan < $($t:ident).+ > , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ReadOnlySpan < $($t).+ >), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] ReadOnlySpan < $($t:ident).+ >) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ReadOnlySpan < $($t).+ >), ])
    };

    // ref T
    ([ $($acc:expr,)* ] ref ! ! $n:literal , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ref ! ! $n), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] ref ! ! $n:literal) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ref ! ! $n), ])
    };
    ([ $($acc:expr,)* ] ref $t:ident , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ref $t), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] ref $t:ident) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ref $t), ])
    };
    ([ $($acc:expr,)* ] ref $($t:ident).+ , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ref $($t).+), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] ref $($t:ident).+) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ref $($t).+), ])
    };
    ([ $($acc:expr,)* ] ref $t:literal , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ref $t), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] ref $t:literal) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(ref $t), ])
    };

    // * T
    ([ $($acc:expr,)* ] * ! ! $n:literal , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(* ! ! $n), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] * ! ! $n:literal) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(* ! ! $n), ])
    };
    ([ $($acc:expr,)* ] * $t:ident , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(* $t), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] * $t:ident) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(* $t), ])
    };
    ([ $($acc:expr,)* ] * $($t:ident).+ , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(* $($t).+), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] * $($t:ident).+) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(* $($t).+), ])
    };
    ([ $($acc:expr,)* ] * $t:literal , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(* $t), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] * $t:literal) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(* $t), ])
    };

    // !!N
    ([ $($acc:expr,)* ] ! ! $n:literal $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(! ! $n), ] $($rest)*)
    };

    // Array T[]
    ([ $($acc:expr,)* ] $t:ident [ ] , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!($t [ ]), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] $t:ident [ ]) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!($t [ ]), ])
    };
    ([ $($acc:expr,)* ] $($t:ident).+ [ ] , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!($($t).+ [ ]), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] $($t:ident).+ [ ]) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!($($t).+ [ ]), ])
    };

    // Primitives and idents
    ([ $($acc:expr,)* ] string , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(string), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] string) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(string), ])
    };
    ([ $($acc:expr,)* ] int , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(int), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] int) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(int), ])
    };
    ([ $($acc:expr,)* ] nint , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(nint), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] nint) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(nint), ])
    };
    ([ $($acc:expr,)* ] object , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(object), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] object) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(object), ])
    };
    ([ $($acc:expr,)* ] bool , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(bool), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] bool) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(bool), ])
    };
    ([ $($acc:expr,)* ] char , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(char), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] char) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(char), ])
    };
    ([ $($acc:expr,)* ] void , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(void), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] void) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(void), ])
    };
    ([ $($acc:expr,)* ] any , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(any), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] any) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!(any), ])
    };

    // Literal type names
    ([ $($acc:expr,)* ] $t:literal , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!($t), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] $t:literal) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!($t), ])
    };

    // Non-quoted type names
    ([ $($acc:expr,)* ] $($t:ident).+ , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!($($t).+), ] $($rest)*)
    };
    ([ $($acc:expr,)* ] $($t:ident).+) => {
        $crate::__munch_types!([ $($acc,)* $crate::parse_type!($($t).+), ])
    };

    // Fallback for expressions
    ([ $($acc:expr,)* ] $t:expr) => {
        $crate::__munch_types!([ $($acc,)* $t, ] )
    };
    ([ $($acc:expr,)* ] $t:expr , $($rest:tt)*) => {
        $crate::__munch_types!([ $($acc,)* $t, ] , $($rest)*)
    };
}

#[macro_export]
macro_rules! match_method {
    ($method:expr, { $( [ $($rule:tt)+ ] $(| [ $($rule_or:tt)+ ] )* => $body:expr ),* $(,)? }) => {
        {
            #[allow(unreachable_code)]
            $(
                if $crate::match_method!($method, $($rule)+) $( || $crate::match_method!($method, $($rule_or)+) )* {
                    Some({ $body })
                } else
            )* {
                None
            }
        }
    };
    ($method:expr, static $($type:ident).+ :: $name:ident < $generic_params:tt > ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            true,
            stringify!($($type).+),
            stringify!($name),
            &$crate::__munch_types!([] $($param)*),
            $generic_params
        )
    };
    ($method:expr, static $($type:ident).+ :: $name:literal < $generic_params:tt > ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            true,
            stringify!($($type).+),
            $name,
            &$crate::__munch_types!([] $($param)*),
            $generic_params
        )
    };
    ($method:expr, $($type:ident).+ :: $name:ident < $generic_params:tt > ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            false,
            stringify!($($type).+),
            stringify!($name),
            &$crate::__munch_types!([] $($param)*),
            $generic_params
        )
    };
    ($method:expr, $($type:ident).+ :: $name:literal < $generic_params:tt > ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            false,
            stringify!($($type).+),
            $name,
            &$crate::__munch_types!([] $($param)*),
            $generic_params
        )
    };
    ($method:expr, static $($type:ident).+ :: $name:ident ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            true,
            stringify!($($type).+),
            stringify!($name),
            &$crate::__munch_types!([] $($param)*),
            0
        )
    };
    ($method:expr, static $($type:ident).+ :: $name:literal ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            true,
            stringify!($($type).+),
            $name,
            &$crate::__munch_types!([] $($param)*),
            0
        )
    };
    ($method:expr, $($type:ident).+ :: $name:ident ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            false,
            stringify!($($type).+),
            stringify!($name),
            &$crate::__munch_types!([] $($param)*),
            0
        )
    };
    ($method:expr, $($type:ident).+ :: $name:literal ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            false,
            stringify!($($type).+),
            $name,
            &$crate::__munch_types!([] $($param)*),
            0
        )
    };

    ($method:expr, static $type:literal :: $name:ident < $generic_params:tt > ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            true,
            $type,
            stringify!($name),
            &$crate::__munch_types!([] $($param)*),
            $generic_params
        )
    };
    ($method:expr, static $type:literal :: $name:literal < $generic_params:tt > ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            true,
            $type,
            $name,
            &$crate::__munch_types!([] $($param)*),
            $generic_params
        )
    };
    ($method:expr, $type:literal :: $name:ident < $generic_params:tt > ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            false,
            $type,
            stringify!($name),
            &$crate::__munch_types!([] $($param)*),
            $generic_params
        )
    };
    ($method:expr, $type:literal :: $name:literal < $generic_params:tt > ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            false,
            $type,
            $name,
            &$crate::__munch_types!([] $($param)*),
            $generic_params
        )
    };
    ($method:expr, static $type:literal :: $name:ident ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            true,
            $type,
            stringify!($name),
            &$crate::__munch_types!([] $($param)*),
            0
        )
    };
    ($method:expr, static $type:literal :: $name:literal ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            true,
            $type,
            $name,
            &$crate::__munch_types!([] $($param)*),
            0
        )
    };
    ($method:expr, $type:literal :: $name:ident ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            false,
            $type,
            stringify!($name),
            &$crate::__munch_types!([] $($param)*),
            0
        )
    };
    ($method:expr, $type:literal :: $name:literal ( $($param:tt)* )) => {
        $crate::vm::intrinsics::matcher::matches_method(
            $method,
            false,
            $type,
            $name,
            &$crate::__munch_types!([] $($param)*),
            0
        )
    };
}

#[macro_export]
macro_rules! any_match_method {
    ($method:expr, $( [ $($rule:tt)+ ] ),* $(,)?) => {
        $( $crate::match_method!($method, $($rule)+) )||*
    };
}

#[macro_export]
macro_rules! any_match_field {
    ($field:expr, $( [ $($rule:tt)+ ] ),* $(,)?) => {
        $( $crate::match_field!($field, $($rule)+) )||*
    };
}

#[macro_export]
macro_rules! match_field {
    ($field:expr, { $( [ $($rule:tt)+ ] $(| [ $($rule_or:tt)+ ] )* => $body:expr ),* $(,)? }) => {
        $(
            if $crate::match_field!($field, $($rule)+) $( || $crate::match_field!($field, $($rule_or)+) )* {
                Some({ $body })
            } else
        )* {
            None
        }
    };
    ($field:expr, static $($type:ident).+ :: $name:ident) => {
        $crate::vm::intrinsics::matcher::matches_field(
            $field,
            true,
            stringify!($($type).+),
            stringify!($name),
        )
    };
    ($field:expr, static $($type:ident).+ :: $name:literal) => {
        $crate::vm::intrinsics::matcher::matches_field(
            $field,
            true,
            stringify!($($type).+),
            $name,
        )
    };
    ($field:expr, $($type:ident).+ :: $name:ident) => {
        $crate::vm::intrinsics::matcher::matches_field(
            $field,
            false,
            stringify!($($type).+),
            stringify!($name),
        )
    };
    ($field:expr, $($type:ident).+ :: $name:literal) => {
        $crate::vm::intrinsics::matcher::matches_field(
            $field,
            false,
            stringify!($($type).+),
            $name,
        )
    };

    ($field:expr, static $type:literal :: $name:ident) => {
        $crate::vm::intrinsics::matcher::matches_field(
            $field,
            true,
            $type,
            stringify!($name),
        )
    };
    ($field:expr, static $type:literal :: $name:literal) => {
        $crate::vm::intrinsics::matcher::matches_field(
            $field,
            true,
            $type,
            $name,
        )
    };
    ($field:expr, $type:literal :: $name:ident) => {
        $crate::vm::intrinsics::matcher::matches_field(
            $field,
            false,
            $type,
            stringify!($name),
        )
    };
    ($field:expr, $type:literal :: $name:literal) => {
        $crate::vm::intrinsics::matcher::matches_field(
            $field,
            false,
            $type,
            $name,
        )
    };
}
