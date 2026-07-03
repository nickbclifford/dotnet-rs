#[macro_export]
macro_rules! vm_msg {
    // Default to Debug level - format string with args
    ($src:expr, $($format:tt)*) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.msg(dotnet_tracer::TraceLevel::Debug, format_args!($($format)*));
        });
    }
}

// Level-specific message macros for convenience
#[macro_export]
macro_rules! vm_error {
    ($src:expr, $($format:tt)*) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.msg(dotnet_tracer::TraceLevel::Error, format_args!($($format)*));
        });
    }
}

#[macro_export]
macro_rules! vm_info {
    ($src:expr, $($format:tt)*) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.msg(dotnet_tracer::TraceLevel::Info, format_args!($($format)*));
        });
    }
}

#[macro_export]
macro_rules! vm_debug {
    ($src:expr, $($format:tt)*) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.msg(dotnet_tracer::TraceLevel::Debug, format_args!($($format)*));
        });
    }
}

#[macro_export]
macro_rules! vm_trace {
    ($src:expr, $($format:tt)*) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.msg(dotnet_tracer::TraceLevel::Trace, format_args!($($format)*));
        });
    }
}

// Specialized tracing macros for better performance and readability
#[macro_export]
macro_rules! vm_trace_instruction {
    ($src:expr, $ip:expr, $instr:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.instruction($ip, $instr);
        });
    };
}

#[macro_export]
macro_rules! vm_trace_method_entry {
    ($src:expr, $name:expr, $sig:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.method_entry($name, $sig);
        });
    };
}

#[macro_export]
macro_rules! vm_trace_intrinsic {
    ($src:expr, $op:expr, $details:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.intrinsic($op, $details);
        });
    };
}

#[macro_export]
macro_rules! vm_trace_interop {
    ($src:expr, $op:expr, $($arg:tt)*) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.interop($op, &format!($($arg)*));
        });
    };
}

#[macro_export]
macro_rules! vm_trace_method_exit {
    ($src:expr, $name:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.method_exit($name);
        });
    };
}

#[macro_export]
macro_rules! vm_trace_stack {
    ($src:expr, $op:expr, $val:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.stack_op($op, &format!("{:?}", $val));
        });
    };
}

#[macro_export]
macro_rules! vm_trace_gc {
    ($src:expr, $event:expr, $details:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.gc_event($event, $details);
        });
    };
}

#[macro_export]
macro_rules! vm_trace_gc_allocation {
    ($src:expr, $type_name:expr, $size:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.gc_allocation($type_name, $size);
        });
    };
}

#[macro_export]
macro_rules! vm_trace_gc_collection_start {
    ($src:expr, $gen:expr, $reason:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.gc_collection_start($gen, $reason);
        });
    };
}

#[macro_export]
macro_rules! vm_trace_gc_collection_end {
    ($src:expr, $gen:expr, $collected:expr, $duration:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.gc_collection_end($gen, $collected, $duration);
        });
    };
}

#[macro_export]
macro_rules! vm_trace_branch {
    ($src:expr, $type:expr, $target:expr, $taken:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.branch($type, $target, $taken);
        });
    };
}

#[macro_export]
macro_rules! vm_trace_field {
    ($src:expr, $op:expr, $field:expr, $val:expr) => {
        let _ = $src.tracer().enabled_emit($src.indent(), |trace| {
            trace.field_access($op, $field, &format!("{:?}", $val));
        });
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

/// Branch prediction hint wrappers for stable toolchains.
///
/// These call through `branch_hint` shims that keep the uncommon side in a
/// `#[cold]` function while preserving the original boolean value.
#[macro_export]
macro_rules! vm_likely {
    ($expr:expr) => {
        $crate::branch_hint::likely($expr)
    };
}

#[macro_export]
macro_rules! vm_unlikely {
    ($expr:expr) => {
        $crate::branch_hint::unlikely($expr)
    };
}

/// Define a cold panic helper in one line.
#[macro_export]
macro_rules! vm_cold_panic {
    (fn $name:ident ( $($arg:ident : $ty:ty),* $(,)? ) => $($panic_args:tt)+) => {
        #[cold]
        #[inline(never)]
        fn $name($($arg: $ty),*) -> ! {
            panic!($($panic_args)+);
        }
    };
}

#[cfg(test)]
macro_rules! make_test_method {
    (
        max_stack: $max_stack:expr,
        instructions: $instructions:expr
        $(, locals: $locals:expr)?
        $(, data_sections: $data_sections:expr)?
        $(,)?
    ) => {
        dotnetdll::prelude::body::Method {
            header: dotnetdll::prelude::body::Header {
                maximum_stack_size: $max_stack,
                local_variables: make_test_method!(@locals $($locals)?),
                initialize_locals: true,
            },
            instructions: $instructions,
            data_sections: make_test_method!(@data_sections $($data_sections)?),
        }
    };
    (@locals $locals:expr) => {
        $locals
    };
    (@locals) => {
        vec![]
    };
    (@data_sections $data_sections:expr) => {
        $data_sections
    };
    (@data_sections) => {
        vec![]
    };
}

#[cfg(test)]
macro_rules! make_test_assembly {
    ($module_name:expr, $assembly_name:expr, $type_name:expr) => {{
        let mut res =
            dotnetdll::prelude::Resolution::new(dotnetdll::prelude::Module::new($module_name));
        res.assembly = Some(dotnetdll::prelude::Assembly::new($assembly_name));
        let system_runtime = res.push_assembly_reference(
            dotnetdll::resolved::assembly::ExternalAssemblyReference::new("System.Runtime"),
        );
        let object_type_ref =
            res.push_type_reference(dotnetdll::resolved::types::ExternalTypeReference::new(
                Some("System".into()),
                "Object",
                dotnetdll::resolved::types::ResolutionScope::Assembly(system_runtime),
            ));
        let mut type_def = dotnetdll::resolved::types::TypeDefinition::new(None, $type_name);
        type_def.extends = Some(object_type_ref.into());
        let type_idx = res.push_type_definition(type_def);
        (res, system_runtime, type_idx)
    }};
}
