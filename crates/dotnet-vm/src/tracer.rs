//! IO-optimized runtime state debug tracer for the .NET VM
//!
//! This module now provides a backward-compatible wrapper around `tracing`.
use crate::{metrics::RuntimeMetrics, stack::CallStack};
use crossbeam_channel::{Receiver, Sender, unbounded};
use dotnet_value::object::HeapStorage;
use gc_arena::{Collect, Gc, unsafe_empty_collect};
use std::{
    env,
    fmt::Arguments,
    fs::File,
    io::{stderr, stdout, Write},
    sync::Once,
    thread,
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

#[allow(dead_code)]
/// A log entry to be processed by the background flusher thread.
enum LogEntry {
    Msg(TraceLevel, usize, String),
    Instruction(usize, usize, String),
    MethodEntry(usize, String, String),
    MethodExit(usize, String),
    Exception(usize, String, String),
    GcEvent(usize, String, String),
    StackOp(usize, String, String),
    FieldAccess(usize, String, String, String),
    Branch(usize, String, usize, bool),
    TypeInfo(usize, String, String),
    Intrinsic(usize, String, String),
    Interop(usize, String, String),
    GcCollectionStart(usize, usize, String),
    GcCollectionEnd(usize, usize, usize, u64),
    GcAllocation(usize, String, usize),
    GcFinalization(usize, String, usize),
    GcHandle(usize, String, String, usize),
    GcPin(usize, String, usize),
    GcWeakRef(usize, String, usize),
    GcResurrection(usize, String, usize),
    ThreadCreate(usize, dotnet_utils::ArenaId, String),
    ThreadStart(usize, dotnet_utils::ArenaId),
    ThreadExit(usize, dotnet_utils::ArenaId),
    ThreadSafepoint(usize, dotnet_utils::ArenaId, String),
    ThreadSuspend(usize, dotnet_utils::ArenaId, String),
    ThreadResume(usize, dotnet_utils::ArenaId),
    StwStart(usize, usize),
    StwEnd(usize, u64),
    ThreadState(usize, dotnet_utils::ArenaId, String, String),
    ThreadSync(usize, dotnet_utils::ArenaId, String, usize),
    DumpStack(Vec<String>, Vec<(usize, String)>),
    DumpFrame(usize, String, usize, usize, usize, usize, usize),
    DumpHeapObject(usize, String, String),
    DumpHeapSnapshotStart(usize),
    DumpHeapSnapshotEnd,
    DumpStaticsSnapshot(String),
    DumpFullStateHeader,
    DumpGcStats(usize, usize, usize, usize, usize),
    DumpRuntimeMetrics(u64, u64, u64, u64, Box<crate::metrics::CacheStats>),
    Flush,
}

pub struct Tracer {
    sender: Option<Sender<LogEntry>>,
}

unsafe_empty_collect!(Tracer);

impl Tracer {
    pub fn new() -> Self {
        INIT.call_once(|| {
            init_tracing();
        });

        if !tracing::enabled!(Level::ERROR) && env::var("DOTNET_RS_TRACE").is_err() {
            return Self { sender: None };
        }

        let (sender, receiver) = unbounded::<LogEntry>();

        thread::spawn(move || {
            Self::flusher(receiver);
        });

        Self {
            sender: Some(sender),
        }
    }

    fn flusher(receiver: Receiver<LogEntry>) {
        while let Ok(entry) = receiver.recv() {
            match entry {
                LogEntry::Msg(level, _indent, msg) => {
                    match level {
                        TraceLevel::Error => error!("{}", msg),
                        TraceLevel::Info => info!("{}", msg),
                        TraceLevel::Debug => debug!("{}", msg),
                        TraceLevel::Trace => debug!("{}", msg),
                        TraceLevel::Instruction => trace!("{}", msg),
                    }
                }
                LogEntry::Instruction(indent, ip, instruction) => {
                    let padding = " ".repeat(indent * 2);
                    trace!(target: "instruction", frame = indent, "[IP:{:04}] {}{}", ip, padding, instruction);
                }
                LogEntry::MethodEntry(indent, name, signature) => {
                    debug!(target: "method", frame = indent, "{:indent$}→ CALL {} ({})", "", name, signature, indent = indent * 2);
                }
                LogEntry::MethodExit(indent, name) => {
                    debug!(target: "method", frame = indent, "{:indent$}← RET  {}", "", name, indent = indent * 2);
                }
                LogEntry::Exception(indent, exception, location) => {
                    error!(target: "exception", frame = indent, "{:indent$}⚠ EXC  {} at {}", "", exception, location, indent = indent * 2);
                }
                LogEntry::GcEvent(indent, event, details) => {
                    info!(target: "gc", frame = indent, "{:indent$}♻ GC   {} ({})", "", event, details, indent = indent * 2);
                }
                LogEntry::StackOp(indent, op, value) => {
                    trace!(target: "stack", frame = indent, "{:indent$}  STACK {} {}", "", op, value, indent = indent * 2);
                }
                LogEntry::FieldAccess(indent, op, field, value) => {
                    trace!(target: "field", frame = indent, "{:indent$}  FIELD {} {} {}", "", op, field, value, indent = indent * 2);
                }
                LogEntry::Branch(indent, branch_type, target, taken) => {
                    let status = if taken { "TAKEN" } else { "NOT TAKEN" };
                    trace!(target: "branch", frame = indent, "{:indent$}  BR    {} to {:04} {}", "", branch_type, target, status, indent = indent * 2);
                }
                LogEntry::TypeInfo(indent, operation, type_name) => {
                    debug!(target: "type", frame = indent, "{:indent$}  TYPE  {} {}", "", operation, type_name, indent = indent * 2);
                }
                LogEntry::Intrinsic(indent, operation, details) => {
                    debug!(target: "intrinsic", frame = indent, "{:indent$}✨ INTR  {} ({})", "", operation, details, indent = indent * 2);
                }
                LogEntry::Interop(indent, operation, details) => {
                    debug!(target: "interop", frame = indent, "{:indent$}🔌 NATIVE {} ({})", "", operation, details, indent = indent * 2);
                }
                LogEntry::GcCollectionStart(indent, generation, reason) => {
                    info!(target: "gc", frame = indent, "{:indent$}♻ GC   COLLECTION GEN{} START ({})", "", generation, reason, indent = indent * 2);
                }
                LogEntry::GcCollectionEnd(indent, generation, collected, duration_us) => {
                    info!(target: "gc", frame = indent, "{:indent$}♻ GC   COLLECTION GEN{} END ({} bytes, {} μs)", "", generation, collected, duration_us, indent = indent * 2);
                }
                LogEntry::GcAllocation(indent, type_name, size_bytes) => {
                    trace!(target: "gc", frame = indent, "{:indent$}♻ GC   ALLOC {} ({} bytes)", "", type_name, size_bytes, indent = indent * 2);
                }
                LogEntry::GcFinalization(indent, obj_type, obj_addr) => {
                    info!(target: "gc", frame = indent, "{:indent$}♻ GC   FINALIZE {} @ {:#x}", "", obj_type, obj_addr, indent = indent * 2);
                }
                LogEntry::GcHandle(indent, operation, handle_type, addr) => {
                    debug!(target: "gc", frame = indent, "{:indent$}♻ GC   HANDLE {} {} @ {:#x}", "", operation, handle_type, addr, indent = indent * 2);
                }
                LogEntry::GcPin(indent, operation, obj_addr) => {
                    debug!(target: "gc", frame = indent, "{:indent$}♻ GC   PIN    {} @ {:#x}", "", operation, obj_addr, indent = indent * 2);
                }
                LogEntry::GcWeakRef(indent, operation, handle_id) => {
                    debug!(target: "gc", frame = indent, "{:indent$}♻ GC   WEAK   {} handle #{}", "", operation, handle_id, indent = indent * 2);
                }
                LogEntry::GcResurrection(indent, obj_type, obj_addr) => {
                    info!(target: "gc", frame = indent, "{:indent$}♻ GC   RESURRECT {} @ {:#x}", "", obj_type, obj_addr, indent = indent * 2);
                }
                LogEntry::ThreadCreate(indent, thread_id, name) => {
                    info!(target: "thread", frame = indent, "{:indent$}⚙ THREAD CREATE [ID:{}] \"{}\"", "", thread_id, name, indent = indent * 2);
                }
                LogEntry::ThreadStart(indent, thread_id) => {
                    info!(target: "thread", frame = indent, "{:indent$}⚙ THREAD START [ID:{}]", "", thread_id, indent = indent * 2);
                }
                LogEntry::ThreadExit(indent, thread_id) => {
                    info!(target: "thread", frame = indent, "{:indent$}⚙ THREAD EXIT [ID:{}]", "", thread_id, indent = indent * 2);
                }
                LogEntry::ThreadSafepoint(indent, thread_id, location) => {
                    debug!(target: "thread", frame = indent, "{:indent$}⚙ THREAD SAFEPOINT [ID:{}] at {}", "", thread_id, location, indent = indent * 2);
                }
                LogEntry::ThreadSuspend(indent, thread_id, reason) => {
                    debug!(target: "thread", frame = indent, "{:indent$}⚙ THREAD SUSPEND [ID:{}] ({})", "", thread_id, reason, indent = indent * 2);
                }
                LogEntry::ThreadResume(indent, thread_id) => {
                    debug!(target: "thread", frame = indent, "{:indent$}⚙ THREAD RESUME [ID:{}]", "", thread_id, indent = indent * 2);
                }
                LogEntry::StwStart(indent, active_threads) => {
                    info!(target: "thread", frame = indent, "{:indent$}⚙ STOP-THE-WORLD START ({} active threads)", "", active_threads, indent = indent * 2);
                }
                LogEntry::StwEnd(indent, duration_us) => {
                    info!(target: "thread", frame = indent, "{:indent$}⚙ STOP-THE-WORLD END ({} μs)", "", duration_us, indent = indent * 2);
                }
                LogEntry::ThreadState(indent, thread_id, old_state, new_state) => {
                    debug!(target: "thread", frame = indent, "{:indent$}⚙ THREAD STATE [ID:{}] {} → {}", "", thread_id, old_state, new_state, indent = indent * 2);
                }
                LogEntry::ThreadSync(indent, thread_id, operation, obj_addr) => {
                    debug!(target: "thread", frame = indent, "{:indent$}⚙ THREAD SYNC [ID:{}] {} @ {:#x}", "", thread_id, operation, obj_addr, indent = indent * 2);
                }
                LogEntry::DumpStack(stack_contents, frame_markers) => {
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
                LogEntry::DumpFrame(frame_idx, method_name, ip, args_base, locals_base, stack_base, stack_height) => {
                    debug!("FRAME #{} - {}", frame_idx, method_name);
                    debug!("║ IP:           {:04}", ip);
                    debug!("║ Arguments:    base = {}", args_base);
                    debug!("║ Locals:       base = {}", locals_base);
                    debug!("║ Stack:        base = {}, height = {}", stack_base, stack_height);
                }
                LogEntry::DumpHeapObject(ptr_addr, obj_type, details) => {
                    debug!("OBJECT @ {:#x} ({})", ptr_addr, obj_type);
                    if !details.is_empty() {
                        debug!("║ Details: {}", details);
                    }
                }
                LogEntry::DumpHeapSnapshotStart(object_count) => {
                    debug!("=== HEAP SNAPSHOT START ({} objects) ===", object_count);
                }
                LogEntry::DumpHeapSnapshotEnd => {
                    debug!("=== HEAP SNAPSHOT END ===");
                }
                LogEntry::DumpStaticsSnapshot(statics_debug) => {
                    debug!("=== STATICS SNAPSHOT START ===");
                    for line in statics_debug.lines() {
                        debug!("║ {}", line);
                    }
                    debug!("=== STATICS SNAPSHOT END ===");
                }
                LogEntry::DumpFullStateHeader => {
                    debug!("╔══════════════════════════════════════════════════════════════════════════════╗");
                    debug!("║ FULL RUNTIME STATE SNAPSHOT                                                  ║");
                    debug!("╚══════════════════════════════════════════════════════════════════════════════╝");
                }
                LogEntry::DumpGcStats(finalization_queue, pending_finalization, pinned_objects, gc_handles, all_objects) => {
                    debug!("║ GC Statistics:");
                    debug!("║   Finalization Queue:   {:>12}", finalization_queue);
                    debug!("║   Pending Finalizers:   {:>12}", pending_finalization);
                    debug!("║   Pinned Objects:       {:>12}", pinned_objects);
                    debug!("║   GC Handles:           {:>12}", gc_handles);
                    debug!("║   Total Managed Objects:{:>12}", all_objects);
                }
                LogEntry::DumpRuntimeMetrics(gc_count, gc_total_us, lock_count, lock_total_us, stats) => {
                    debug!("║ Runtime Metrics:");
                    debug!("║   GC Pause Count:       {:>12}", gc_count);
                    debug!("║   GC Pause Total:       {:>12} μs", gc_total_us);
                    debug!("║   Lock Wait Count:      {:>12}", lock_count);
                    debug!("║   Lock Wait Total:      {:>12} μs", lock_total_us);
                    debug!("{}", *stats);
                }
                LogEntry::Flush => {
                    // Force flush if supported by subscriber (standard fmt doesn't have a flush)
                }
            }
        }
    }

    fn send(&self, entry: LogEntry) {
        if let Some(ref sender) = self.sender {
            let _ = sender.send(entry);
        }
    }

    #[inline(always)]
    pub fn is_enabled(&self) -> bool {
        self.sender.is_some()
    }

    pub fn msg(&self, level: TraceLevel, indent: usize, args: Arguments) {
        let msg = format!("{:indent$}{}", "", args, indent = indent.min(100) * 2);
        self.send(LogEntry::Msg(level, indent, msg));
    }

    pub fn flush(&self) {
        self.send(LogEntry::Flush);
    }

    pub fn trace_instruction(&self, indent: usize, ip: usize, instruction: &str) {
        self.send(LogEntry::Instruction(indent, ip, instruction.to_string()));
    }

    pub fn trace_method_entry(&self, indent: usize, name: &str, signature: &str) {
        self.send(LogEntry::MethodEntry(indent, name.to_string(), signature.to_string()));
    }

    pub fn trace_method_exit(&self, indent: usize, name: &str) {
        self.send(LogEntry::MethodExit(indent, name.to_string()));
    }

    pub fn trace_exception(&self, indent: usize, exception: &str, location: &str) {
        self.send(LogEntry::Exception(indent, exception.to_string(), location.to_string()));
    }

    pub fn trace_gc_event(&self, indent: usize, event: &str, details: &str) {
        self.send(LogEntry::GcEvent(indent, event.to_string(), details.to_string()));
    }

    pub fn trace_stack_op(&self, indent: usize, op: &str, value: &str) {
        self.send(LogEntry::StackOp(indent, op.to_string(), value.to_string()));
    }

    pub fn trace_field_access(&self, indent: usize, op: &str, field: &str, value: &str) {
        self.send(LogEntry::FieldAccess(indent, op.to_string(), field.to_string(), value.to_string()));
    }

    pub fn trace_branch(&self, indent: usize, branch_type: &str, target: usize, taken: bool) {
        self.send(LogEntry::Branch(indent, branch_type.to_string(), target, taken));
    }

    pub fn trace_type_info(&self, indent: usize, operation: &str, type_name: &str) {
        self.send(LogEntry::TypeInfo(indent, operation.to_string(), type_name.to_string()));
    }

    pub fn trace_intrinsic(&self, indent: usize, operation: &str, details: &str) {
        self.send(LogEntry::Intrinsic(indent, operation.to_string(), details.to_string()));
    }

    pub fn trace_interop(&self, indent: usize, operation: &str, details: &str) {
        self.send(LogEntry::Interop(indent, operation.to_string(), details.to_string()));
    }

    // GC Helpers
    pub fn trace_gc_collection_start(&self, indent: usize, generation: usize, reason: &str) {
        self.send(LogEntry::GcCollectionStart(indent, generation, reason.to_string()));
    }

    pub fn trace_gc_collection_end(
        &self,
        indent: usize,
        generation: usize,
        collected: usize,
        duration_us: u64,
    ) {
        self.send(LogEntry::GcCollectionEnd(indent, generation, collected, duration_us));
    }

    pub fn trace_gc_allocation(&self, indent: usize, type_name: &str, size_bytes: usize) {
        self.send(LogEntry::GcAllocation(indent, type_name.to_string(), size_bytes));
    }

    pub fn trace_gc_finalization(&self, indent: usize, obj_type: &str, obj_addr: usize) {
        self.send(LogEntry::GcFinalization(indent, obj_type.to_string(), obj_addr));
    }

    pub fn trace_gc_handle(
        &self,
        indent: usize,
        operation: &str,
        handle_type: &str,
        addr: usize,
    ) {
        self.send(LogEntry::GcHandle(indent, operation.to_string(), handle_type.to_string(), addr));
    }

    pub fn trace_gc_pin(&self, indent: usize, operation: &str, obj_addr: usize) {
        self.send(LogEntry::GcPin(indent, operation.to_string(), obj_addr));
    }

    pub fn trace_gc_weak_ref(&self, indent: usize, operation: &str, handle_id: usize) {
        self.send(LogEntry::GcWeakRef(indent, operation.to_string(), handle_id));
    }

    pub fn trace_gc_resurrection(&self, indent: usize, obj_type: &str, obj_addr: usize) {
        self.send(LogEntry::GcResurrection(indent, obj_type.to_string(), obj_addr));
    }

    // Threading Helpers
    #[cfg(feature = "multithreading")]
    pub fn trace_thread_create(
        &self,
        indent: usize,
        thread_id: dotnet_utils::ArenaId,
        name: &str,
    ) {
        self.send(LogEntry::ThreadCreate(indent, thread_id, name.to_string()));
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_start(&self, indent: usize, thread_id: dotnet_utils::ArenaId) {
        self.send(LogEntry::ThreadStart(indent, thread_id));
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_exit(&self, indent: usize, thread_id: dotnet_utils::ArenaId) {
        self.send(LogEntry::ThreadExit(indent, thread_id));
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_safepoint(
        &self,
        indent: usize,
        thread_id: dotnet_utils::ArenaId,
        location: &str,
    ) {
        self.send(LogEntry::ThreadSafepoint(indent, thread_id, location.to_string()));
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_suspend(
        &self,
        indent: usize,
        thread_id: dotnet_utils::ArenaId,
        reason: &str,
    ) {
        self.send(LogEntry::ThreadSuspend(indent, thread_id, reason.to_string()));
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_resume(&self, indent: usize, thread_id: dotnet_utils::ArenaId) {
        self.send(LogEntry::ThreadResume(indent, thread_id));
    }

    #[cfg(feature = "multithreaded-gc")]
    pub fn trace_stw_start(&self, indent: usize, active_threads: usize) {
        self.send(LogEntry::StwStart(indent, active_threads));
    }

    #[cfg(feature = "multithreaded-gc")]
    pub fn trace_stw_end(&self, indent: usize, duration_us: u64) {
        self.send(LogEntry::StwEnd(indent, duration_us));
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_state(
        &self,
        indent: usize,
        thread_id: dotnet_utils::ArenaId,
        old_state: &str,
        new_state: &str,
    ) {
        self.send(LogEntry::ThreadState(indent, thread_id, old_state.to_string(), new_state.to_string()));
    }

    #[cfg(feature = "multithreading")]
    pub fn trace_thread_sync(
        &self,
        indent: usize,
        thread_id: dotnet_utils::ArenaId,
        operation: &str,
        obj_addr: usize,
    ) {
        self.send(LogEntry::ThreadSync(indent, thread_id, operation.to_string(), obj_addr));
    }

    // Snapshots
    pub fn dump_stack_state(
        &self,
        stack_contents: &[String],
        frame_markers: &[(usize, String)],
    ) {
        self.send(LogEntry::DumpStack(stack_contents.to_vec(), frame_markers.to_vec()));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dump_frame_state(
        &self,
        frame_idx: usize,
        method_name: &str,
        ip: usize,
        args_base: usize,
        locals_base: usize,
        stack_base: usize,
        stack_height: usize,
    ) {
        self.send(LogEntry::DumpFrame(frame_idx, method_name.to_string(), ip, args_base, locals_base, stack_base, stack_height));
    }

    pub fn dump_heap_object(&self, ptr_addr: usize, obj_type: &str, details: &str) {
        self.send(LogEntry::DumpHeapObject(ptr_addr, obj_type.to_string(), details.to_string()));
    }

    pub fn dump_heap_snapshot_start(&self, object_count: usize) {
        self.send(LogEntry::DumpHeapSnapshotStart(object_count));
    }

    pub fn dump_heap_snapshot_end(&self) {
        self.send(LogEntry::DumpHeapSnapshotEnd);
    }

    pub fn dump_statics_snapshot(&self, statics_debug: &str) {
        self.send(LogEntry::DumpStaticsSnapshot(statics_debug.to_string()));
    }

    pub fn dump_full_state_header(&self) {
        self.send(LogEntry::DumpFullStateHeader);
    }

    pub fn dump_gc_stats(
        &self,
        finalization_queue: usize,
        pending_finalization: usize,
        pinned_objects: usize,
        gc_handles: usize,
        all_objects: usize,
    ) {
        self.send(LogEntry::DumpGcStats(finalization_queue, pending_finalization, pinned_objects, gc_handles, all_objects));
    }

    pub fn dump_runtime_metrics(&self, metrics: &RuntimeMetrics) {
        use std::sync::atomic::Ordering;
        use crate::metrics::CacheSizes;

        let gc_pause_total = metrics.gc_pause_total_us.load(Ordering::Relaxed);
        let gc_pause_count = metrics.gc_pause_count.load(Ordering::Relaxed);
        let lock_count = metrics.lock_contention_count.load(Ordering::Relaxed);
        let lock_total = metrics.lock_contention_total_us.load(Ordering::Relaxed);

        // We need CacheSizes to get CacheStats. We'll use dummy sizes for now as the tracer doesn't have them easily.
        // Actually, the original code had them.
        let stats = metrics.cache_statistics(CacheSizes {
            layout_size: 0,
            vmt_size: 0,
            intrinsic_size: 0,
            intrinsic_field_size: 0,
            hierarchy_size: 0,
            static_field_layout_size: 0,
            instance_field_layout_size: 0,
            value_type_size: 0,
            has_finalizer_size: 0,
            assembly_type_info: (0, 0, 0),
            assembly_method_info: (0, 0, 0),
        });

        self.send(LogEntry::DumpRuntimeMetrics(
            gc_pause_count,
            gc_pause_total,
            lock_count,
            lock_total,
            Box::new(stats),
        ));
    }

    pub fn trace_gc_raw(&self, msg: &str) {
        // Direct output to stderr to bypass any channel or mutex
        let _ = writeln!(stderr(), "♻ GC RAW: {}", msg);
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
        // Only enable the specified level for dotnet_vm targets, not all crates (like dotnetdll)
        // Also enable specific trace targets used by the tracer (instruction, method, etc.)
        EnvFilter::default()
            .add_directive(format!("dotnet_vm={}", target_level).parse().unwrap())
            .add_directive(format!("dotnet_cli={}", target_level).parse().unwrap())
            .add_directive(format!("instruction={}", target_level).parse().unwrap())
            .add_directive(format!("method={}", target_level).parse().unwrap())
            .add_directive(format!("stack={}", target_level).parse().unwrap())
            .add_directive(format!("field={}", target_level).parse().unwrap())
            .add_directive(format!("branch={}", target_level).parse().unwrap())
            .add_directive(format!("type={}", target_level).parse().unwrap())
            .add_directive(format!("intrinsic={}", target_level).parse().unwrap())
            .add_directive(format!("interop={}", target_level).parse().unwrap())
            .add_directive(format!("gc={}", target_level).parse().unwrap())
            .add_directive(format!("thread={}", target_level).parse().unwrap())
            .add_directive(format!("exception={}", target_level).parse().unwrap())
            .add_directive("dotnetdll=warn".parse().unwrap()) // Suppress verbose dotnetdll logs
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
            markers.push((base.stack.as_usize(), format!("Stack base of frame #{}", i)));
            if base.locals != base.stack {
                markers.push((
                    base.locals.as_usize(),
                    format!("Locals base of frame #{}", i),
                ));
            }
            markers.push((
                base.arguments.as_usize(),
                format!("Arguments base of frame #{}", i),
            ));
        }

        self.tracer().dump_stack_state(&contents, &markers);
    }

    pub fn trace_dump_frames(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let tracer = self.tracer();
        for (idx, frame) in self.execution.frame_stack.frames.iter().enumerate() {
            let method_name = format!("{:?}", frame.state.info_handle.source);
            tracer.dump_frame_state(
                idx,
                &method_name,
                frame.state.ip,
                frame.base.arguments.as_usize(),
                frame.base.locals.as_usize(),
                frame.base.stack.as_usize(),
                frame.stack_height.as_usize(),
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
        let tracer = self.tracer();
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
