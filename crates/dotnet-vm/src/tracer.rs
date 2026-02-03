//! IO-optimized runtime state debug tracer for the .NET VM
//!
//! This module now provides a backward-compatible wrapper around `tracing`.
use crate::{metrics::RuntimeMetrics, stack::CallStack};
use dotnet_value::object::HeapStorage;
use gc_arena::{unsafe_empty_collect, Collect, Gc};
use std::{
    env,
    fs::File,
    io::{stderr, stdout},
    sync::Once,
};
use tracing::{debug, error, info, trace, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter, Registry};

static INIT: Once = Once::new();

/// Trace level for filtering messages (Legacy)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TraceLevel {
    Error = 0,
    Info = 1,
    Debug = 2,
    Trace = 3,
    Instruction = 4,
}

pub struct Tracer;

unsafe_empty_collect!(Tracer);

impl Tracer {
    pub fn new() -> Self {
        INIT.call_once(|| {
            init_tracing();
        });
        Self
    }

    #[inline(always)]
    pub fn is_enabled(&self) -> bool {
        // Check if any relevant level is enabled in tracing
        // We use ERROR as the baseline check, but really we want to know if *anything* is enabled.
        // `tracing` doesn't have a cheap global "is anything enabled" check that matches our exact filter,
        // but checking if INFO is enabled is a reasonable proxy for "tracing is on".
        // However, if the user set level=ERROR, INFO might be disabled.
        // Let's use ERROR.
        tracing::enabled!(Level::ERROR)
    }

    pub fn msg(&mut self, level: TraceLevel, indent: usize, args: std::fmt::Arguments) {
        let msg = format!("{:indent$}{}", "", args, indent = indent * 2);
        match level {
            TraceLevel::Error => error!("{}", msg),
            TraceLevel::Info => info!("{}", msg),
            TraceLevel::Debug => debug!("{}", msg),
            TraceLevel::Trace => debug!("{}", msg), // Map Trace to Debug for now
            TraceLevel::Instruction => trace!("{}", msg),
        }
    }

    pub fn flush(&mut self) {
        // No-op for now
    }

    pub fn trace_instruction(&mut self, indent: usize, ip: usize, instruction: &str) {
        trace!(
            target: "instruction",
            indent = indent,
            ip = ip,
            instruction = instruction,
            "[IP:{:04}] {:indent$}{}",
            ip,
            "",
            instruction,
            indent = indent * 2
        );
    }

    pub fn trace_method_entry(&mut self, indent: usize, name: &str, signature: &str) {
        debug!(
            target: "method",
            indent = indent,
            name = name,
            signature = signature,
            "{:indent$}→ CALL {} ({})",
            "",
            name,
            signature,
            indent = indent * 2
        );
    }

    pub fn trace_method_exit(&mut self, indent: usize, name: &str) {
        debug!(
            target: "method",
            indent = indent,
            name = name,
            "{:indent$}← RET  {}",
            "",
            name,
            indent = indent * 2
        );
    }

    pub fn trace_exception(&mut self, indent: usize, exception: &str, location: &str) {
        error!(
            target: "exception",
            indent = indent,
            exception = exception,
            location = location,
            "{:indent$}⚠ EXC  {} at {}",
            "",
            exception,
            location,
            indent = indent * 2
        );
    }

    pub fn trace_gc_event(&mut self, indent: usize, event: &str, details: &str) {
        info!(
            target: "gc",
            indent = indent,
            event = event,
            details = details,
            "{:indent$}♻ GC   {} ({})",
            "",
            event,
            details,
            indent = indent * 2
        );
    }

    pub fn trace_stack_op(&mut self, indent: usize, op: &str, value: &str) {
        trace!(
            target: "stack",
            indent = indent,
            op = op,
            value = value,
            "{:indent$}  STACK {} {}",
            "",
            op,
            value,
            indent = indent * 2
        );
    }

    pub fn trace_field_access(&mut self, indent: usize, op: &str, field: &str, value: &str) {
        trace!(
            target: "field",
            indent = indent,
            op = op,
            field = field,
            value = value,
            "{:indent$}  FIELD {} {} = {}",
            "",
            op,
            field,
            value,
            indent = indent * 2
        );
    }

    pub fn trace_branch(&mut self, indent: usize, branch_type: &str, target: usize, taken: bool) {
        trace!(
            target: "branch",
            indent = indent,
            branch_type = branch_type,
            target = target,
            taken = taken,
            "{:indent$}↷ {} to {:04} ({})",
            "",
            branch_type,
            target,
            if taken { "TAKEN" } else { "NOT TAKEN" },
            indent = indent * 2
        );
    }

    pub fn trace_type_info(&mut self, indent: usize, operation: &str, type_name: &str) {
        debug!(
            target: "type",
            indent = indent,
            operation = operation,
            type_name = type_name,
            "{:indent$}  TYPE {} {}",
            "",
            operation,
            type_name,
            indent = indent * 2
        );
    }

    pub fn trace_intrinsic(&mut self, indent: usize, operation: &str, details: &str) {
        debug!(
            target: "intrinsic",
            indent = indent,
            operation = operation,
            details = details,
            "{:indent$}⚡ INTRINSIC {} {}",
            "",
            operation,
            details,
            indent = indent * 2
        );
    }

    pub fn trace_interop(&mut self, indent: usize, operation: &str, details: &str) {
        debug!(
            target: "interop",
            indent = indent,
            operation = operation,
            details = details,
            "{:indent$}🔗 INTEROP {} {}",
            "",
            operation,
            details,
            indent = indent * 2
        );
    }

    // GC Helpers
    pub fn trace_gc_collection_start(&mut self, indent: usize, generation: usize, reason: &str) {
        info!(
            target: "gc",
            indent = indent,
            generation = generation,
            reason = reason,
            "{:indent$}♻ GC   COLLECTION START [Gen {}] ({})",
            "",
            generation,
            reason,
            indent = indent * 2
        );
    }

    pub fn trace_gc_collection_end(
        &mut self,
        indent: usize,
        generation: usize,
        collected: usize,
        duration_us: u64,
    ) {
        info!(
            target: "gc",
            indent = indent,
            generation = generation,
            collected = collected,
            duration_us = duration_us,
            "{:indent$}♻ GC   COLLECTION END [Gen {}] ({} objects collected, {} μs)",
            "",
            generation,
            collected,
            duration_us,
            indent = indent * 2
        );
    }

    pub fn trace_gc_allocation(&mut self, indent: usize, type_name: &str, size_bytes: usize) {
        debug!(
            target: "gc",
            indent = indent,
            type_name = type_name,
            size_bytes = size_bytes,
            "{:indent$}♻ GC   ALLOC {} ({} bytes)",
            "",
            type_name,
            size_bytes,
            indent = indent * 2
        );
    }

    pub fn trace_gc_finalization(&mut self, indent: usize, obj_type: &str, obj_addr: usize) {
        debug!(
            target: "gc",
            indent = indent,
            obj_type = obj_type,
            obj_addr = obj_addr,
            "{:indent$}♻ GC   FINALIZE {} @ {:#x}",
            "",
            obj_type,
            obj_addr,
            indent = indent * 2
        );
    }

    pub fn trace_gc_handle(
        &mut self,
        indent: usize,
        operation: &str,
        handle_type: &str,
        addr: usize,
    ) {
        debug!(
            target: "gc",
            indent = indent,
            operation = operation,
            handle_type = handle_type,
            addr = addr,
            "{:indent$}♻ GC   HANDLE {} [{}] @ {:#x}",
            "",
            operation,
            handle_type,
            addr,
            indent = indent * 2
        );
    }

    pub fn trace_gc_pin(&mut self, indent: usize, operation: &str, obj_addr: usize) {
        debug!(
            target: "gc",
            indent = indent,
            operation = operation,
            obj_addr = obj_addr,
            "{:indent$}♻ GC   PIN {} @ {:#x}",
            "",
            operation,
            obj_addr,
            indent = indent * 2
        );
    }

    pub fn trace_gc_weak_ref(&mut self, indent: usize, operation: &str, handle_id: usize) {
        debug!(
            target: "gc",
            indent = indent,
            operation = operation,
            handle_id = handle_id,
            "{:indent$}♻ GC   WEAK {} (handle {})",
            "",
            operation,
            handle_id,
            indent = indent * 2
        );
    }

    pub fn trace_gc_resurrection(&mut self, indent: usize, obj_type: &str, obj_addr: usize) {
        debug!(
            target: "gc",
            indent = indent,
            obj_type = obj_type,
            obj_addr = obj_addr,
            "{:indent$}♻ GC   RESURRECT {} @ {:#x}",
            "",
            obj_type,
            obj_addr,
            indent = indent * 2
        );
    }

    // Threading Helpers
    #[cfg(feature = "multithreading")]
    pub fn trace_thread_create(&mut self, indent: usize, thread_id: u64, name: &str) {
        info!(target: "thread", indent = indent, thread_id = thread_id, name = name, "{:indent$}⚙ THREAD CREATE [ID:{}] \"{}\"", "", thread_id, name, indent = indent * 2);
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_start(&mut self, indent: usize, thread_id: u64) {
        info!(target: "thread", indent = indent, thread_id = thread_id, "{:indent$}⚙ THREAD START [ID:{}]", "", thread_id, indent = indent * 2);
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_exit(&mut self, indent: usize, thread_id: u64) {
        info!(target: "thread", indent = indent, thread_id = thread_id, "{:indent$}⚙ THREAD EXIT [ID:{}]", "", thread_id, indent = indent * 2);
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_safepoint(&mut self, indent: usize, thread_id: u64, location: &str) {
        debug!(target: "thread", indent = indent, thread_id = thread_id, location = location, "{:indent$}⚙ THREAD SAFEPOINT [ID:{}] at {}", "", thread_id, location, indent = indent * 2);
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_suspend(&mut self, indent: usize, thread_id: u64, reason: &str) {
        debug!(target: "thread", indent = indent, thread_id = thread_id, reason = reason, "{:indent$}⚙ THREAD SUSPEND [ID:{}] ({})", "", thread_id, reason, indent = indent * 2);
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_resume(&mut self, indent: usize, thread_id: u64) {
        debug!(target: "thread", indent = indent, thread_id = thread_id, "{:indent$}⚙ THREAD RESUME [ID:{}]", "", thread_id, indent = indent * 2);
    }

    #[cfg(feature = "multithreaded-gc")]
    pub fn trace_stw_start(&mut self, indent: usize, active_threads: usize) {
        info!(target: "thread", indent = indent, active_threads = active_threads, "{:indent$}⚙ STOP-THE-WORLD START ({} active threads)", "", active_threads, indent = indent * 2);
    }

    #[cfg(feature = "multithreaded-gc")]
    pub fn trace_stw_end(&mut self, indent: usize, duration_us: u64) {
        info!(target: "thread", indent = indent, duration_us = duration_us, "{:indent$}⚙ STOP-THE-WORLD END ({} μs)", "", duration_us, indent = indent * 2);
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_state(
        &mut self,
        indent: usize,
        thread_id: u64,
        old_state: &str,
        new_state: &str,
    ) {
        debug!(target: "thread", indent = indent, thread_id = thread_id, old_state = old_state, new_state = new_state, "{:indent$}⚙ THREAD STATE [ID:{}] {} → {}", "", thread_id, old_state, new_state, indent = indent * 2);
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_sync(
        &mut self,
        indent: usize,
        thread_id: u64,
        operation: &str,
        obj_addr: usize,
    ) {
        debug!(target: "thread", indent = indent, thread_id = thread_id, operation = operation, obj_addr = obj_addr, "{:indent$}⚙ THREAD SYNC [ID:{}] {} @ {:#x}", "", thread_id, operation, obj_addr, indent = indent * 2);
    }

    // Snapshots
    pub fn dump_stack_state(
        &mut self,
        stack_contents: &[String],
        frame_markers: &[(usize, String)],
    ) {
        debug!("STACK SNAPSHOT");
        for (idx, content) in stack_contents.iter().enumerate().rev() {
            let markers: Vec<_> = frame_markers
                .iter()
                .filter(|(pos, _)| *pos == idx)
                .map(|(_, label)| label.as_str())
                .collect();
            for marker in markers {
                debug!("╟─ {} ", marker);
            }
            debug!("║ [{:4}] {}", idx, content);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dump_frame_state(
        &mut self,
        frame_idx: usize,
        method_name: &str,
        ip: usize,
        args_base: usize,
        locals_base: usize,
        stack_base: usize,
        stack_height: usize,
    ) {
        debug!("FRAME #{} - {}", frame_idx, method_name);
        debug!("║ IP:           {:04}", ip);
        debug!("║ Args base:    {}", args_base);
        debug!("║ Locals base:  {}", locals_base);
        debug!("║ Stack base:   {}", stack_base);
        debug!("║ Stack height: {}", stack_height);
    }

    pub fn dump_heap_object(&mut self, ptr_addr: usize, obj_type: &str, details: &str) {
        debug!("HEAP[{:#x}] {} => {}", ptr_addr, obj_type, details);
    }

    pub fn dump_heap_snapshot_start(&mut self, object_count: usize) {
        debug!("HEAP SNAPSHOT ({} objects)", object_count);
    }

    pub fn dump_heap_snapshot_end(&mut self) {
        // no-op
    }

    pub fn dump_statics_snapshot(&mut self, statics_debug: &str) {
        debug!("STATIC STORAGE SNAPSHOT");
        for line in statics_debug.lines() {
            debug!("║ {}", line);
        }
    }

    pub fn dump_full_state_header(&mut self) {
        debug!("FULL RUNTIME STATE SNAPSHOT");
    }

    pub fn dump_gc_stats(
        &mut self,
        finalization_queue: usize,
        pending_finalization: usize,
        pinned_objects: usize,
        gc_handles: usize,
        all_objects: usize,
    ) {
        debug!("GC STATISTICS");
        debug!("║ All objects:          {}", all_objects);
        debug!("║ Finalization queue:   {}", finalization_queue);
        debug!("║ Pending finalization: {}", pending_finalization);
        debug!("║ Pinned objects:       {}", pinned_objects);
        debug!("║ GC handles:           {}", gc_handles);
    }

    pub fn dump_runtime_metrics(&mut self, metrics: &RuntimeMetrics) {
        use std::sync::atomic::Ordering;

        let gc_pause_total = metrics.gc_pause_total_us.load(Ordering::Relaxed);
        let gc_pause_count = metrics.gc_pause_count.load(Ordering::Relaxed);
        let gc_allocated = metrics.current_gc_allocated.load(Ordering::Relaxed);
        let ext_allocated = metrics.current_external_allocated.load(Ordering::Relaxed);
        let lock_count = metrics.lock_contention_count.load(Ordering::Relaxed);
        let lock_total = metrics.lock_contention_total_us.load(Ordering::Relaxed);

        debug!("RUNTIME METRICS");
        debug!("║ GC Pause Total:       {:>12} μs", gc_pause_total);
        debug!("║ GC Pause Count:       {:>12}", gc_pause_count);
        debug!("║ GC Allocated:         {:>12} bytes", gc_allocated);
        debug!("║ External Allocated:   {:>12} bytes", ext_allocated);
        debug!("║ Lock Contention:      {:>12}", lock_count);
        debug!("║ Lock Wait Total:      {:>12} μs", lock_total);
    }
}

impl Default for Tracer {
    fn default() -> Self {
        Self::new()
    }
}

fn init_tracing() {
    let trace_env = env::var("DOTNET_RS_TRACE").unwrap_or_default();
    if trace_env.is_empty() {
        return;
    }

    let format_str = env::var("DOTNET_RS_TRACE_FORMAT").unwrap_or_else(|_| "text".to_string());
    let is_json = format_str.eq_ignore_ascii_case("json");

    let make_writer = if trace_env == "1" || trace_env == "true" || trace_env == "stdout" {
        fmt::writer::BoxMakeWriter::new(stdout)
    } else if trace_env == "stderr" {
        fmt::writer::BoxMakeWriter::new(stderr)
    } else {
        let path = trace_env.clone();
        // File output requires explicit Arc/Mutex wrapper if we want to use BoxMakeWriter easily
        // But File::create can fail. We must handle it.
        if let Ok(file) = File::create(&path) {
            fmt::writer::BoxMakeWriter::new(std::sync::Arc::new(file))
        } else {
            // Fallback to stdout if file fails
            fmt::writer::BoxMakeWriter::new(stdout)
        }
    };

    // Respect DOTNET_RS_TRACE_LEVEL or default to info if unspecified,
    // unless RUST_LOG is present (Registry handles EnvFilter which checks RUST_LOG).
    // But we are constructing EnvFilter manually.

    let legacy_level = env::var("DOTNET_RS_TRACE_LEVEL").ok();
    let filter = if let Some(lvl) = legacy_level {
        let target_level = match lvl.to_lowercase().as_str() {
            "error" => Level::ERROR,
            "info" => Level::INFO,
            "debug" => Level::DEBUG,
            "trace" => Level::DEBUG,
            "instruction" => Level::TRACE,
            _ => Level::INFO,
        };
        EnvFilter::default().add_directive(target_level.into())
    } else {
        EnvFilter::from_default_env()
    };

    if is_json {
        Registry::default()
            .with(
                fmt::layer()
                    .json()
                    .with_writer(make_writer)
                    .with_filter(filter),
            )
            .init();
    } else {
        Registry::default()
            .with(fmt::layer().with_writer(make_writer).with_filter(filter))
            .init();
    }
}

#[allow(dead_code)]
impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    // Tracer-integrated dump methods for comprehensive state capture
    pub fn trace_dump_stack(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let contents: Vec<_> = self.execution.evaluation_stack.stack
            [..self.execution.evaluation_stack.stack.len()] // Modified to not depend on top_of_stack if private
            .iter()
            .enumerate()
            .map(|(i, _)| format!("{:?}", self.get_slot(i)))
            .collect();

        let mut markers = Vec::new();
        for (i, frame) in self.execution.frame_stack.frames.iter().enumerate() {
            let base = &frame.base;
            markers.push((base.stack, format!("Stack base of frame #{}", i)));
            if base.locals != base.stack {
                markers.push((base.locals, format!("Locals base of frame #{}", i)));
            }
            markers.push((base.arguments, format!("Arguments base of frame #{}", i)));
        }

        self.tracer().dump_stack_state(&contents, &markers);
    }

    pub fn trace_dump_frames(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let mut tracer = self.tracer();
        for (idx, frame) in self.execution.frame_stack.frames.iter().enumerate() {
            let method_name = format!("{:?}", frame.state.info_handle.source);
            tracer.dump_frame_state(
                idx,
                &method_name,
                frame.state.ip,
                frame.base.arguments,
                frame.base.locals,
                frame.base.stack,
                frame.stack_height,
            );
        }
    }

    pub fn trace_dump_heap(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let objects: Vec<_> = self
            .local
            .heap
            ._all_objs
            .borrow()
            .values()
            .copied()
            .collect(); // access via local.heap
        let mut tracer = self.tracer();
        tracer.dump_heap_snapshot_start(objects.len());

        for obj in objects {
            let Some(ptr) = obj.0 else {
                continue;
            };
            let raw_ptr = Gc::<_>::as_ptr(ptr) as *const _ as usize;
            let borrowed = ptr.borrow();
            match &borrowed.storage {
                HeapStorage::Obj(o) => {
                    let details = format!("{:?}", o);
                    tracer.dump_heap_object(raw_ptr, "Object", &details);
                }
                HeapStorage::Vec(v) => {
                    let details = format!("{:?}", v);
                    tracer.dump_heap_object(raw_ptr, "Vector", &details);
                }
                HeapStorage::Str(s) => {
                    let details = format!("{:?}", s);
                    tracer.dump_heap_object(raw_ptr, "String", &details);
                }
                HeapStorage::Boxed(b) => {
                    let details = format!("{:?}", b);
                    tracer.dump_heap_object(raw_ptr, "Boxed", &details);
                }
            }
        }

        tracer.dump_heap_snapshot_end();
    }

    pub fn trace_dump_statics(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let s = &self.shared.statics; // statics() helper might be missing or private
        let debug_str = format!("{:#?}", s);
        self.tracer().dump_statics_snapshot(&debug_str);
    }

    pub fn trace_dump_gc_stats(&self) {
        if !self.tracer_enabled() {
            return;
        }

        self.tracer().dump_gc_stats(
            self.local.heap.finalization_queue.borrow().len(),
            self.local.heap.pending_finalization.borrow().len(),
            self.local.heap.pinned_objects.borrow().len(),
            self.local.heap.gchandles.borrow().len(),
            self.local.heap._all_objs.borrow().len(),
        );
    }

    pub fn trace_dump_runtime_metrics(&self) {
        if !self.tracer_enabled() {
            return;
        }

        self.tracer().dump_runtime_metrics(&self.shared.metrics);
    }

    /// Captures a complete snapshot of all runtime state to the tracer
    pub fn trace_full_state(&self) {
        if !self.tracer_enabled() {
            return;
        }

        self.tracer().dump_full_state_header();
        self.trace_dump_frames();
        self.trace_dump_stack();
        self.trace_dump_heap();
        self.trace_dump_statics();
        self.trace_dump_gc_stats();
        self.trace_dump_runtime_metrics();
        self.tracer().flush();
    }
}
