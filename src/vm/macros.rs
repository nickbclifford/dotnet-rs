#[macro_export]
macro_rules! vm_pop {
    ($stack:expr) => {
        $stack.pop_stack()
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
            $src.tracer().msg($crate::vm::gc::tracer::TraceLevel::Debug, $src.indent(), format_args!($($format)*))
        }
    }
}

// Level-specific message macros for convenience
#[macro_export]
macro_rules! vm_error {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::vm::gc::tracer::TraceLevel::Error, $src.indent(), format_args!($($format)*))
        }
    }
}

#[macro_export]
macro_rules! vm_info {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::vm::gc::tracer::TraceLevel::Info, $src.indent(), format_args!($($format)*))
        }
    }
}

#[macro_export]
macro_rules! vm_debug {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::vm::gc::tracer::TraceLevel::Debug, $src.indent(), format_args!($($format)*))
        }
    }
}

#[macro_export]
macro_rules! vm_trace {
    ($src:expr, $($format:tt)*) => {
        if $src.tracer_enabled() {
            $src.tracer().msg($crate::vm::gc::tracer::TraceLevel::Trace, $src.indent(), format_args!($($format)*))
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
