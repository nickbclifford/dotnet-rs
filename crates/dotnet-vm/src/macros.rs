#[macro_export]
macro_rules! vm_pop {
    ($stack:expr, $gc:expr) => {
        $stack.pop_stack($gc)
    };
}

#[macro_export]
macro_rules! vm_push {
    ($stack:expr, $gc:expr, string($val:expr)) => {
        {
            let obj = dotnet_value::object::ObjectRef::new(
                $gc,
                dotnet_value::object::HeapStorage::Str(
                    dotnet_value::string::CLRString::from($val)
                )
            );
            $stack.register_new_object(&obj);
            $stack.push_stack($gc, dotnet_value::StackValue::ObjectRef(obj))
        }
    };
    ($stack:expr, $gc:expr, $variant:ident ( $($args:expr),* )) => {
        $stack.push_stack($gc, dotnet_value::StackValue::$variant($($args),*))
    };
    ($stack:expr, $gc:expr, $val:expr) => {
        $stack.push_stack($gc, $val)
    };
}

#[macro_export]
macro_rules! vm_expect_stack {
    (let $variant:ident ( $inner:ident $(as $t:ty)? ) = $v:expr) => {
        let $inner = match $v {
            dotnet_value::StackValue::$variant($inner) => $inner,
            err => panic!(
                "invalid type on stack ({:?}), expected {}",
                err,
                stringify!($variant)
            ),
        };
        $(
            let $inner = $inner as $t;
        )?
    };
}

#[macro_export]
macro_rules! vm_msg {
    // Default to Debug level - format string with args
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::tracer::TraceLevel::Debug, $src.indent(), format_args!($($format)*))
        }
    }
}

// Level-specific message macros for convenience
#[macro_export]
macro_rules! vm_error {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::tracer::TraceLevel::Error, $src.indent(), format_args!($($format)*))
        }
    }
}

#[macro_export]
macro_rules! vm_info {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::tracer::TraceLevel::Info, $src.indent(), format_args!($($format)*))
        }
    }
}

#[macro_export]
macro_rules! vm_debug {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::tracer::TraceLevel::Debug, $src.indent(), format_args!($($format)*))
        }
    }
}

#[macro_export]
macro_rules! vm_trace {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::tracer::TraceLevel::Trace, $src.indent(), format_args!($($format)*))
        }
    }
}

// Specialized tracing macros for better performance and readability
#[macro_export]
macro_rules! vm_trace_instruction {
    ($src:expr, $ip:expr, $instr:expr) => {
        if $src.tracer_enabled() {
            $src.tracer().trace_instruction($src.indent(), $ip, $instr);
        }
    };
}

#[macro_export]
macro_rules! vm_trace_method_entry {
    ($src:expr, $name:expr, $sig:expr) => {
        if $src.tracer_enabled() {
            $src.tracer().trace_method_entry($src.indent(), $name, $sig);
        }
    };
}

#[macro_export]
macro_rules! vm_trace_intrinsic {
    ($src:expr, $op:expr, $details:expr) => {
        if $src.tracer_enabled() {
            $src.tracer().trace_intrinsic($src.indent(), $op, $details);
        }
    };
}

#[macro_export]
macro_rules! vm_trace_interop {
    ($src:expr, $op:expr, $($arg:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().trace_interop($src.indent(), $op, &format!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! vm_trace_method_exit {
    ($src:expr, $name:expr) => {
        if $src.tracer_enabled() {
            $src.tracer().trace_method_exit($src.indent(), $name);
        }
    };
}

#[macro_export]
macro_rules! vm_trace_stack {
    ($src:expr, $op:expr, $val:expr) => {
        if $src.tracer_enabled() {
            $src.tracer()
                .trace_stack_op($src.indent(), $op, &format!("{:?}", $val));
        }
    };
}

#[macro_export]
macro_rules! vm_trace_gc {
    ($src:expr, $event:expr, $details:expr) => {
        if $src.tracer_enabled() {
            $src.tracer()
                .trace_gc_event($src.indent(), $event, $details);
        }
    };
}

#[macro_export]
macro_rules! vm_trace_gc_allocation {
    ($src:expr, $type_name:expr, $size:expr) => {
        if $src.tracer_enabled() {
            $src.tracer()
                .trace_gc_allocation($src.indent(), $type_name, $size);
        }
    };
}

#[macro_export]
macro_rules! vm_trace_gc_collection_start {
    ($src:expr, $gen:expr, $reason:expr) => {
        if $src.tracer_enabled() {
            $src.tracer()
                .trace_gc_collection_start($src.indent(), $gen, $reason);
        }
    };
}

#[macro_export]
macro_rules! vm_trace_gc_collection_end {
    ($src:expr, $gen:expr, $collected:expr, $duration:expr) => {
        if $src.tracer_enabled() {
            $src.tracer()
                .trace_gc_collection_end($src.indent(), $gen, $collected, $duration);
        }
    };
}

#[macro_export]
macro_rules! vm_trace_branch {
    ($src:expr, $type:expr, $target:expr, $taken:expr) => {
        if $src.tracer_enabled() {
            $src.tracer()
                .trace_branch($src.indent(), $type, $target, $taken);
        }
    };
}

#[macro_export]
macro_rules! vm_trace_field {
    ($src:expr, $op:expr, $field:expr, $val:expr) => {
        if $src.tracer_enabled() {
            $src.tracer()
                .trace_field_access($src.indent(), $op, $field, &format!("{:?}", $val));
        }
    };
}

// Comprehensive state snapshot macros
#[macro_export]
macro_rules! vm_trace_full_state {
    ($src:expr) => {
        if $src.tracer_enabled() {
            $src.trace_full_state();
        }
    };
}

#[macro_export]
macro_rules! vm_trace_stack_snapshot {
    ($src:expr) => {
        if $src.tracer_enabled() {
            $src.trace_dump_stack();
        }
    };
}

#[macro_export]
macro_rules! vm_trace_heap_snapshot {
    ($src:expr) => {
        if $src.tracer_enabled() {
            $src.trace_dump_heap();
        }
    };
}

/// Pop and validate multiple arguments from the stack in one call.
///
/// This macro simplifies the common pattern of popping multiple arguments
/// with type validation. Arguments are listed in PARAMETER order
/// (i.e., first parameter first).
///
/// # Syntax
/// ```ignore
/// pop_args!(stack, [
///     Variant1(name1),  // first parameter
///     ...
///     VariantN(nameN),  // last parameter
/// ])
/// ```
///
/// # Example
/// ```ignore
/// // For System.String::Substring(int startIndex, int length)
/// pop_args!(stack, [
///     Int32(start_index), // popped second (first param)
///     Int32(length)       // popped first (last param)
/// ]);
/// ```
///
/// The macro uses vm_expect_stack! internally, so it will panic with a
/// descriptive message if the stack value doesn't match the expected type.
#[macro_export]
#[doc(hidden)]
macro_rules! pop_args_impl {
    // Base case: input list empty. Accumulator has reversed items.
    ($stack:expr, $gc:expr, [ $($variant:ident($name:ident)),* $(,)? ], []) => {
        $(
            $crate::vm_expect_stack!(let $variant($name) = $crate::vm_pop!($stack, $gc));
        )*
    };

    // Recursive: head, rest...
    ($stack:expr, $gc:expr, [ $($acc:tt)* ], [ $variant:ident($name:ident), $($rest:tt)* ]) => {
        $crate::pop_args_impl!($stack, $gc, [$variant($name), $($acc)*], [ $($rest)* ])
    };

    // Recursive: last item (no trailing comma in input list)
    ($stack:expr, $gc:expr, [ $($acc:tt)* ], [ $variant:ident($name:ident) ]) => {
        $crate::pop_args_impl!($stack, $gc, [$variant($name), $($acc)*], [])
    };
}

#[macro_export]
macro_rules! pop_args {
    ($stack:expr, $gc:expr, [ $($args:tt)* ]) => {
        $crate::pop_args_impl!($stack, $gc, [], [ $($args)* ])
    };
}

#[macro_export]
macro_rules! define_intrinsic {
    ($name:ident ($gc:ident, $stack:ident) ( $($arg_pat:ident ( $arg_name:ident )),* ) $body:block) => {
        #[allow(unused_variables)]
        pub fn $name<'gc, 'm: 'gc>(
            $gc: dotnet_utils::gc::GCHandle<'gc>,
            $stack: &mut $crate::CallStack<'gc, 'm>,
            _method: dotnet_types::members::MethodDescription,
            _generics: &dotnet_types::generics::GenericLookup,
        ) -> $crate::StepResult {
            $crate::pop_args!($stack, $gc, [ $($arg_pat($arg_name)),* ]);
            $body
            $crate::StepResult::InstructionStepped
        }
    };
    // Variant for when we need method/generics
    ($name:ident ($gc:ident, $stack:ident, $method:ident, $generics:ident) ( $($arg_pat:ident ( $arg_name:ident )),* ) $body:block) => {
        #[allow(unused_variables)]
        pub fn $name<'gc, 'm: 'gc>(
            $gc: dotnet_utils::gc::GCHandle<'gc>,
            $stack: &mut $crate::CallStack<'gc, 'm>,
            $method: dotnet_types::members::MethodDescription,
            $generics: &dotnet_types::generics::GenericLookup,
        ) -> $crate::StepResult {
            $crate::pop_args!($stack, $gc, [ $($arg_pat($arg_name)),* ]);
            $body
            $crate::StepResult::InstructionStepped
        }
    };
}
