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
