//! Shared support-slot census used by contract tests.

use crate::support_contract::SlotKind;

/// Declares the support-assembly fields that make up the runtime ABI contract.
///
/// A type may be nested below the namespace with dotted path segments. `<*>` marks a
/// one-parameter generic type and is rendered using the metadata name suffix `` `1``.
macro_rules! slots_registry {
    ($(namespace $namespace:ident { $($types:tt)* })+) => {
        slots_registry!(@collect [] $(namespace $namespace { $($types)* })+);
    };

    (@collect [$($entries:tt)*]) => {
        pub(crate) const EXPECTED_SLOTS: &[(&str, &str, SlotKind)] = &[
            $($entries)*
        ];
    };
    (@collect [$($entries:tt)*]
        namespace $namespace:ident { $($types:tt)* }
        $($rest:tt)*
    ) => {
        slots_registry!(@types
            [$($entries)*]
            $namespace
            [$($rest)*]
            $($types)*
        );
    };

    (@types
        [$($entries:tt)*]
        $namespace:ident
        [$($rest:tt)*]
    ) => {
        slots_registry!(@collect [$($entries)*] $($rest)*);
    };
    (@types
        [$($entries:tt)*]
        $namespace:ident
        [$($rest:tt)*]
        $type:ident $(. $nested_type:ident)* <*>
        { $($fields:tt)* }
        $($types:tt)*
    ) => {
        slots_registry!(@fields
            [$($entries)*]
            $namespace
            [$($rest)*]
            [
                concat!(
                    stringify!($namespace), ".",
                    stringify!($type),
                    $( ".", stringify!($nested_type), )*
                    "`1",
                )
            ]
            [$($fields)*]
            [$($types)*]
        );
    };
    (@types
        [$($entries:tt)*]
        $namespace:ident
        [$($rest:tt)*]
        $type:ident $(. $nested_type:ident)*
        { $($fields:tt)* }
        $($types:tt)*
    ) => {
        slots_registry!(@fields
            [$($entries)*]
            $namespace
            [$($rest)*]
            [
                concat!(
                    stringify!($namespace), ".",
                    stringify!($type),
                    $( ".", stringify!($nested_type), )*
                )
            ]
            [$($fields)*]
            [$($types)*]
        );
    };

    (@fields
        [$($entries:tt)*]
        $namespace:ident
        [$($rest:tt)*]
        [$type_name:expr]
        [$kind:ident $field:ident;]
        [$($types:tt)*]
    ) => {
        slots_registry!(@types
            [
                $($entries)*
                ($type_name, stringify!($field), slots_registry!(@kind $kind)),
            ]
            $namespace
            [$($rest)*]
            $($types)*
        );
    };
    (@fields
        [$($entries:tt)*]
        $namespace:ident
        [$($rest:tt)*]
        [$type_name:expr]
        [$kind:ident $field:ident; $($fields:tt)+]
        [$($types:tt)*]
    ) => {
        slots_registry!(@fields
            [
                $($entries)*
                ($type_name, stringify!($field), slots_registry!(@kind $kind)),
            ]
            $namespace
            [$($rest)*]
            [$type_name]
            [$($fields)+]
            [$($types)*]
        );
    };

    (@kind handle) => { SlotKind::Handle };
    (@kind idx) => { SlotKind::Index };
    (@kind gc) => { SlotKind::GcRef };
    (@kind byref) => { SlotKind::Byref };
    (@kind int) => { SlotKind::ScalarInt };
    (@kind bool) => { SlotKind::ScalarBool };
    (@kind generic) => { SlotKind::Generic };
    (@kind value) => { SlotKind::ValueType };
    (@kind ptr) => { SlotKind::NativePtr };
}

slots_registry! {
    namespace System {
        RuntimeTypeHandle { handle _value; }
        RuntimeFieldHandle { handle _value; }
        RuntimeMethodHandle { handle _value; }
        RuntimeType { idx index; }
        Delegate {
            gc _target;
            idx _method;
        }
        MulticastDelegate { gc targets; }
        Span<*> {
            byref _reference;
            int _length;
        }
        ReadOnlySpan<*> {
            byref _reference;
            int _length;
        }
        Threading.Tasks.ValueTask { gc _task; }
        Threading.Tasks.ValueTask<*> {
            gc _task;
            generic _result;
            bool _hasResult;
        }
        Threading.Tasks.Task {
            bool _isCompleted;
            gc _exception;
            gc _continuation;
        }
        Threading.Tasks.Task<*> {
            generic _result;
            bool _hasResult;
        }
        Threading.Tasks.TaskCompletionSource<*> { gc _task; }
        Runtime.CompilerServices.AsyncTaskMethodBuilder { gc _task; }
        Runtime.CompilerServices.AsyncTaskMethodBuilder<*> { gc _task; }
        Runtime.CompilerServices.AsyncValueTaskMethodBuilder { gc _task; }
        Runtime.CompilerServices.AsyncValueTaskMethodBuilder<*> { gc _task; }
        Runtime.CompilerServices.TaskAwaiter { gc _task; }
        Runtime.CompilerServices.TaskAwaiter<*> { gc _task; }
        Runtime.CompilerServices.ValueTaskAwaiter { value _valueTask; }
        Runtime.CompilerServices.ValueTaskAwaiter<*> { value _valueTask; }
    }
    namespace DotnetRs {
        MethodInfo { idx index; }
        ConstructorInfo { idx index; }
        FieldInfo { idx index; }
        ParameterInfo {
            idx method_index;
            int position;
        }
        PropertyInfo {
            gc name;
            gc getter;
            gc setter;
            gc declaringType;
            gc propertyType;
        }
        Module { ptr resolution; }
        Assembly { ptr resolution; }
        StubAttribute { gc InPlaceOf; }
    }
}
