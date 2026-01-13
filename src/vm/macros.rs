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
            let obj = $crate::value::object::ObjectRef::new(
                $gc,
                $crate::value::object::HeapStorage::Str(
                    $crate::value::string::CLRString::from($val)
                )
            );
            $stack.register_new_object(&obj);
            $stack.push_stack($gc, $crate::value::StackValue::ObjectRef(obj))
        }
    };
    ($stack:expr, $gc:expr, $variant:ident ( $($args:expr),* )) => {
        $stack.push_stack($gc, $crate::value::StackValue::$variant($($args),*))
    };
    ($stack:expr, $gc:expr, $val:expr) => {
        $stack.push_stack($gc, $val)
    };
}

#[macro_export]
macro_rules! vm_expect_stack {
    (let $variant:ident ( $inner:ident $(as $t:ty)? ) = $v:expr) => {
        let $inner = match $v {
            $crate::value::StackValue::$variant($inner) => $inner,
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
            $src.tracer().msg($crate::vm::tracer::TraceLevel::Debug, $src.indent(), format_args!($($format)*))
        }
    }
}

// Level-specific message macros for convenience
#[macro_export]
macro_rules! vm_error {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::vm::tracer::TraceLevel::Error, $src.indent(), format_args!($($format)*))
        }
    }
}

#[macro_export]
macro_rules! vm_info {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::vm::tracer::TraceLevel::Info, $src.indent(), format_args!($($format)*))
        }
    }
}

#[macro_export]
macro_rules! vm_debug {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::vm::tracer::TraceLevel::Debug, $src.indent(), format_args!($($format)*))
        }
    }
}

#[macro_export]
macro_rules! vm_trace {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::vm::tracer::TraceLevel::Trace, $src.indent(), format_args!($($format)*))
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
/// with type validation. Arguments are listed in REVERSE stack order
/// (i.e., last parameter first, which is how they appear on the stack).
///
/// # Syntax
/// ```ignore
/// pop_args!(stack, [
///     VariantN(nameN),  // last parameter (top of stack)
///     ...
///     Variant1(name1),  // first parameter (bottom of argument block)
/// ])
/// ```
///
/// # Example
/// ```ignore
/// // For System.String::Substring(int startIndex, int length)
/// pop_args!(stack, [
///     Int32(length),      // popped first (last param)
///     Int32(start_index)  // popped second (first param)
/// ]);
/// ```
///
/// The macro uses vm_expect_stack! internally, so it will panic with a
/// descriptive message if the stack value doesn't match the expected type.
#[macro_export]
macro_rules! pop_args {
    ($stack:expr, $gc:expr, [ $($variant:ident($name:ident)),+ $(,)? ]) => {
        $(
            $crate::vm_expect_stack!(let $variant($name) = $crate::vm_pop!($stack, $gc));
        )+
    };
}
