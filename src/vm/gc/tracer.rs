//! IO-optimized runtime state debug tracer for the .NET VM
//!
//! This module provides a comprehensive tracing system for capturing runtime execution
//! state with minimal performance impact through aggressive buffering and lazy evaluation.
//!
//! ## Features
//!
//! ### Instruction-level tracing
//! - Method calls and returns
//! - Stack operations (push/pop)
//! - Branch instructions
//! - Field accesses
//! - GC events (collection, allocation, finalization, resurrection, handles, pinning)
//! - Threading events (create, start, exit, safepoint, suspend, resume, STW pauses)
//! - Exception handling
//!
//! ### State snapshots
//! - Complete stack visualization with frame markers
//! - Heap object inspection
//! - Static storage dumps
//! - Frame-by-frame analysis
//! - GC statistics
//!
//! ## Environment Variables
//!
//! - `DOTNET_RS_TRACE`: Enable tracing
//!   - `"1"`, `"true"`, or `"stdout"`: Write to stdout
//!   - `"stderr"`: Write to stderr
//!   - `<path>`: Write to file at path
//!
//! - `DOTNET_RS_TRACE_FLUSH_INTERVAL`: Number of messages before auto-flush (default: 10000)
//!
//! - `DOTNET_RS_TRACE_STATS`: Enable detailed statistics collection (`"1"` or `"true"`)
//!
//! ## Performance Characteristics
//!
//! - 256KB write buffer to minimize syscalls
//! - Automatic periodic flushing to prevent buffer overflow
//! - Early-exit checks when tracing is disabled (zero-cost when off)
//! - Lazy string formatting only when tracing is active
//! - Optional statistics collection (disabled by default for minimal overhead)
//!
//! ## Usage Examples
//!
//! ### Basic tracing
//! ```bash
//! # Trace to stdout
//! DOTNET_RS_TRACE=stdout cargo run
//!
//! # Trace to file with statistics
//! DOTNET_RS_TRACE=/tmp/trace.log DOTNET_RS_TRACE_STATS=1 cargo run
//!
//! # Custom flush interval (flush every 1000 messages)
//! DOTNET_RS_TRACE=stdout DOTNET_RS_TRACE_FLUSH_INTERVAL=1000 cargo run
//! ```
//!
//! ### Code integration
//! ```ignore
//! // Trace individual operations
//! vm_msg!(ctx, "Custom message: {}", value);
//! vm_trace_instruction!(ctx, ip, "ldarg.0");
//! vm_trace_method_entry!(ctx, "MyClass::Method", "()V");
//!
//! // Capture complete state snapshots
//! vm_trace_full_state!(ctx);           // Everything: frames, stack, heap, statics
//! vm_trace_stack_snapshot!(ctx);        // Just the stack
//! vm_trace_heap_snapshot!(ctx);         // Just the heap
//! ```
use crate::{value::object::HeapStorage, vm::CallStack};
use gc_arena::{Collect, Gc, unsafe_empty_collect};
use std::{
    cell::{Cell, RefCell},
    env, fs::File, io::{BufWriter, Write, stderr, stdout},
};

const BUFFER_SIZE: usize = 256 * 1024; // 256KB buffer for better IO performance
const AUTO_FLUSH_INTERVAL: usize = 10_000; // Auto-flush every N messages

/// Statistics for runtime execution tracing
#[derive(Debug, Clone, Default)]
pub struct TraceStats {
    pub total_messages: usize,
    pub instructions_traced: usize,
    pub method_calls: usize,
    pub method_returns: usize,
    pub gc_events: usize,
    pub exceptions: usize,
    pub branches: usize,
    pub stack_ops: usize,
    pub field_accesses: usize,
    pub gc_collections: usize,
    pub gc_allocations: usize,
    pub gc_finalizations: usize,
    #[cfg(feature = "multithreading")]
    pub thread_events: usize,
    #[cfg(feature = "multithreading")]
    pub thread_safepoints: usize,
    #[cfg(feature = "multithreading")]
    pub thread_suspensions: usize,
}

pub struct Tracer {
    enabled: bool,
    writer: RefCell<Option<BufWriter<Box<dyn Write + Send>>>>,
    message_count: Cell<usize>,
    auto_flush_interval: usize,
    stats: RefCell<TraceStats>,
    detailed_stats: bool,
}
unsafe_empty_collect!(Tracer);

impl Tracer {
    pub fn new() -> Self {
        let trace_env = env::var("DOTNET_RS_TRACE");
        let (enabled, writer): (bool, Option<Box<dyn Write + Send>>) = match trace_env {
            Ok(val) if val == "1" || val == "true" || val == "stdout" => {
                (true, Some(Box::new(stdout())))
            }
            Ok(val) if val == "stderr" => (true, Some(Box::new(stderr()))),
            Ok(val) if !val.is_empty() => {
                // assume it's a file path
                match File::create(&val) {
                    Ok(f) => (true, Some(Box::new(f))),
                    Err(e) => {
                        eprintln!("Failed to create trace file {}: {}", val, e);
                        (false, None)
                    }
                }
            }
            _ => (false, None),
        };

        // Check for custom auto-flush interval
        let auto_flush_interval = env::var("DOTNET_RS_TRACE_FLUSH_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(AUTO_FLUSH_INTERVAL);

        // Check if detailed statistics should be collected
        let detailed_stats = env::var("DOTNET_RS_TRACE_STATS")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);

        Self {
            enabled,
            writer: RefCell::new(writer.map(|w| BufWriter::with_capacity(BUFFER_SIZE, w))),
            message_count: Cell::new(0),
            auto_flush_interval,
            stats: RefCell::new(TraceStats::default()),
            detailed_stats,
        }
    }

    #[inline(always)]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[inline(always)]
    fn write_msg(&self, indent: usize, args: std::fmt::Arguments) {
        if let Some(ref mut writer) = *self.writer.borrow_mut() {
            // Write indentation
            for _ in 0..indent {
                let _ = writer.write_all(b"  ");
            }
            // Write message
            let _ = writer.write_fmt(args);
            let _ = writer.write_all(b"\n");

            // Periodic auto-flush to prevent buffer overflow and ensure visibility
            let count = self.message_count.get() + 1;
            self.message_count.set(count);
            if count >= self.auto_flush_interval {
                let _ = writer.flush();
                self.message_count.set(0);
            }
        }
    }

    pub fn msg(&self, indent: usize, args: std::fmt::Arguments) {
        if !self.enabled {
            return;
        }

        if self.detailed_stats {
            self.stats.borrow_mut().total_messages += 1;
        }

        self.write_msg(indent, args);
    }

    pub fn flush(&self) {
        if self.enabled {
            if let Some(ref mut writer) = *self.writer.borrow_mut() {
                let _ = writer.flush();
            }
            self.message_count.set(0);
        }
    }

    pub fn trace_instruction(&self, indent: usize, ip: usize, instruction: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().instructions_traced += 1;
        }
        self.write_msg(indent, format_args!("[IP:{:04}] {}", ip, instruction));
    }

    pub fn trace_method_entry(&self, indent: usize, name: &str, signature: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().method_calls += 1;
        }
        if signature.is_empty() {
            self.write_msg(indent, format_args!("→ CALL {}", name));
        } else {
            self.write_msg(indent, format_args!("→ CALL {} ({})", name, signature));
        }
    }

    pub fn trace_method_exit(&self, indent: usize, name: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().method_returns += 1;
        }
        self.write_msg(indent, format_args!("← RET  {}", name));
    }

    pub fn trace_exception(&self, indent: usize, exception: &str, location: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().exceptions += 1;
        }
        self.write_msg(indent, format_args!("⚠ EXC  {} at {}", exception, location));
    }

    pub fn trace_gc_event(&self, indent: usize, event: &str, details: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().gc_events += 1;
        }
        self.write_msg(indent, format_args!("♻ GC   {} ({})", event, details));
    }

    pub fn trace_stack_op(&self, indent: usize, op: &str, value: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().stack_ops += 1;
        }
        self.write_msg(indent, format_args!("  STACK {} {}", op, value));
    }

    pub fn trace_field_access(&self, indent: usize, op: &str, field: &str, value: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().field_accesses += 1;
        }
        self.write_msg(indent, format_args!("  FIELD {} {} = {}", op, field, value));
    }

    pub fn trace_branch(&self, indent: usize, branch_type: &str, target: usize, taken: bool) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().branches += 1;
        }
        let status = if taken { "TAKEN" } else { "NOT TAKEN" };
        self.write_msg(
            indent,
            format_args!("↷ {} to {:04} ({})", branch_type, target, status),
        );
    }

    pub fn trace_type_info(&self, indent: usize, operation: &str, type_name: &str) {
        if !self.enabled {
            return;
        }
        self.write_msg(indent, format_args!("  TYPE {} {}", operation, type_name));
    }

    // Performance counter helpers
    pub fn get_message_count(&self) -> usize {
        self.message_count.get()
    }

    pub fn reset_message_count(&self) {
        self.message_count.set(0);
    }

    pub fn get_stats(&self) -> TraceStats {
        self.stats.borrow().clone()
    }

    pub fn reset_stats(&self) {
        *self.stats.borrow_mut() = TraceStats::default();
    }

    pub fn print_stats(&self) {
        if !self.detailed_stats {
            return;
        }
        let stats = self.stats.borrow();
        eprintln!("\n=== Tracer Statistics ===");
        eprintln!("Total messages:      {:>12}", stats.total_messages);
        eprintln!("Instructions traced: {:>12}", stats.instructions_traced);
        eprintln!("Method calls:        {:>12}", stats.method_calls);
        eprintln!("Method returns:      {:>12}", stats.method_returns);
        eprintln!("GC events:           {:>12}", stats.gc_events);
        eprintln!("  Collections:       {:>12}", stats.gc_collections);
        eprintln!("  Allocations:       {:>12}", stats.gc_allocations);
        eprintln!("  Finalizations:     {:>12}", stats.gc_finalizations);
        #[cfg(feature = "multithreading")]
        {
            eprintln!("Thread events:       {:>12}", stats.thread_events);
            eprintln!("  Safepoints:        {:>12}", stats.thread_safepoints);
            eprintln!("  Suspensions:       {:>12}", stats.thread_suspensions);
        }
        eprintln!("Exceptions:          {:>12}", stats.exceptions);
        eprintln!("Branches:            {:>12}", stats.branches);
        eprintln!("Stack operations:    {:>12}", stats.stack_ops);
        eprintln!("Field accesses:      {:>12}", stats.field_accesses);
        eprintln!("========================\n");
    }
}

impl Drop for Tracer {
    fn drop(&mut self) {
        // Print stats on drop if enabled
        if self.detailed_stats && self.enabled {
            self.print_stats();
        }
        // Final flush on drop
        self.flush();
    }
}

impl Default for Tracer {
    fn default() -> Self {
        Self::new()
    }
}

// Structured state capture methods for comprehensive debugging
impl Tracer {
    /// Writes a stack snapshot to the trace output
    pub fn dump_stack_state(&self, stack_contents: &[String], frame_markers: &[(usize, String)]) {
        if !self.enabled {
            return;
        }

        self.msg(0, format_args!(""));
        self.msg(
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(0, format_args!("║ STACK SNAPSHOT"));
        self.msg(
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );

        if stack_contents.is_empty() {
            self.msg(0, format_args!("║ (empty stack)"));
        } else {
            for (idx, content) in stack_contents.iter().enumerate().rev() {
                // Check for frame markers at this position
                let markers: Vec<_> = frame_markers
                    .iter()
                    .filter(|(pos, _)| *pos == idx)
                    .map(|(_, label)| label.as_str())
                    .collect();

                if !markers.is_empty() {
                    for marker in markers {
                        self.msg(0, format_args!("╟─ {} ", marker));
                    }
                }

                self.msg(0, format_args!("║ [{:4}] {}", idx, content));
            }
        }

        self.msg(
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes frame information to the trace output
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
        if !self.enabled {
            return;
        }

        self.msg(0, format_args!(""));
        self.msg(
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(0, format_args!("║ FRAME #{} - {}", frame_idx, method_name));
        self.msg(
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );
        self.msg(0, format_args!("║ IP:           {:04}", ip));
        self.msg(0, format_args!("║ Args base:    {}", args_base));
        self.msg(0, format_args!("║ Locals base:  {}", locals_base));
        self.msg(0, format_args!("║ Stack base:   {}", stack_base));
        self.msg(0, format_args!("║ Stack height: {}", stack_height));
        self.msg(
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes heap object information to the trace output
    pub fn dump_heap_object(&self, ptr_addr: usize, obj_type: &str, details: &str) {
        if !self.enabled {
            return;
        }

        self.msg(
            0,
            format_args!("HEAP[{:#x}] {} => {}", ptr_addr, obj_type, details),
        );
    }

    /// Writes a heap snapshot header
    pub fn dump_heap_snapshot_start(&self, object_count: usize) {
        if !self.enabled {
            return;
        }

        self.msg(0, format_args!(""));
        self.msg(
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(
            0,
            format_args!("║ HEAP SNAPSHOT ({} objects)", object_count),
        );
        self.msg(
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes a heap snapshot footer
    pub fn dump_heap_snapshot_end(&self) {
        if !self.enabled {
            return;
        }

        self.msg(
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes static storage information
    pub fn dump_statics_snapshot(&self, statics_debug: &str) {
        if !self.enabled {
            return;
        }

        self.msg(0, format_args!(""));
        self.msg(
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(0, format_args!("║ STATIC STORAGE SNAPSHOT"));
        self.msg(
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );

        for line in statics_debug.lines() {
            self.msg(0, format_args!("║ {}", line));
        }

        self.msg(
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes a complete runtime state snapshot
    pub fn dump_full_state_header(&self) {
        if !self.enabled {
            return;
        }

        self.msg(0, format_args!(""));
        self.msg(
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(0, format_args!("║ FULL RUNTIME STATE SNAPSHOT"));
        self.msg(
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes GC statistics
    pub fn dump_gc_stats(
        &self,
        finalization_queue: usize,
        pending_finalization: usize,
        pinned_objects: usize,
        gc_handles: usize,
        all_objects: usize,
    ) {
        if !self.enabled {
            return;
        }

        self.msg(0, format_args!(""));
        self.msg(
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(0, format_args!("║ GC STATISTICS"));
        self.msg(
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );
        self.msg(0, format_args!("║ All objects:          {}", all_objects));
        self.msg(
            0,
            format_args!("║ Finalization queue:   {}", finalization_queue),
        );
        self.msg(
            0,
            format_args!("║ Pending finalization: {}", pending_finalization),
        );
        self.msg(
            0,
            format_args!("║ Pinned objects:       {}", pinned_objects),
        );
        self.msg(0, format_args!("║ GC handles:           {}", gc_handles));
        self.msg(
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    // === GC-specific tracing methods ===

    /// Trace the start of a GC collection cycle
    pub fn trace_gc_collection_start(&self, indent: usize, generation: usize, reason: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().gc_collections += 1;
            self.stats.borrow_mut().gc_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("♻ GC   COLLECTION START [Gen {}] ({})", generation, reason),
        );
    }

    /// Trace the end of a GC collection cycle
    pub fn trace_gc_collection_end(
        &self,
        indent: usize,
        generation: usize,
        collected: usize,
        duration_us: u64,
    ) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().gc_events += 1;
        }
        self.write_msg(
            indent,
            format_args!(
                "♻ GC   COLLECTION END [Gen {}] ({} objects collected, {} μs)",
                generation, collected, duration_us
            ),
        );
    }

    /// Trace heap allocation
    pub fn trace_gc_allocation(&self, indent: usize, type_name: &str, size_bytes: usize) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().gc_allocations += 1;
            self.stats.borrow_mut().gc_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("♻ GC   ALLOC {} ({} bytes)", type_name, size_bytes),
        );
    }

    /// Trace object finalization
    pub fn trace_gc_finalization(&self, indent: usize, obj_type: &str, obj_addr: usize) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().gc_finalizations += 1;
            self.stats.borrow_mut().gc_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("♻ GC   FINALIZE {} @ {:#x}", obj_type, obj_addr),
        );
    }

    /// Trace GC handle operations
    pub fn trace_gc_handle(&self, indent: usize, operation: &str, handle_type: &str, addr: usize) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().gc_events += 1;
        }
        self.write_msg(
            indent,
            format_args!(
                "♻ GC   HANDLE {} [{}] @ {:#x}",
                operation, handle_type, addr
            ),
        );
    }

    /// Trace GC pinning operations
    pub fn trace_gc_pin(&self, indent: usize, operation: &str, obj_addr: usize) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().gc_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("♻ GC   PIN {} @ {:#x}", operation, obj_addr),
        );
    }

    /// Trace weak reference updates
    pub fn trace_gc_weak_ref(&self, indent: usize, operation: &str, handle_id: usize) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().gc_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("♻ GC   WEAK {} (handle {})", operation, handle_id),
        );
    }

    /// Trace resurrection during finalization
    pub fn trace_gc_resurrection(&self, indent: usize, obj_type: &str, obj_addr: usize) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().gc_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("♻ GC   RESURRECT {} @ {:#x}", obj_type, obj_addr),
        );
    }

    // === Threading-specific tracing methods ===

    #[cfg(feature = "multithreading")]
    /// Trace thread creation
    pub fn trace_thread_create(&self, indent: usize, thread_id: u64, name: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().thread_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("⚙ THREAD CREATE [ID:{}] \"{}\"", thread_id, name),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread start
    pub fn trace_thread_start(&self, indent: usize, thread_id: u64) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().thread_events += 1;
        }
        self.write_msg(indent, format_args!("⚙ THREAD START [ID:{}]", thread_id));
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread exit
    pub fn trace_thread_exit(&self, indent: usize, thread_id: u64) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().thread_events += 1;
        }
        self.write_msg(indent, format_args!("⚙ THREAD EXIT [ID:{}]", thread_id));
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread reaching a safe point
    pub fn trace_thread_safepoint(&self, indent: usize, thread_id: u64, location: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().thread_safepoints += 1;
            self.stats.borrow_mut().thread_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("⚙ THREAD SAFEPOINT [ID:{}] at {}", thread_id, location),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread suspension for GC
    pub fn trace_thread_suspend(&self, indent: usize, thread_id: u64, reason: &str) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().thread_suspensions += 1;
            self.stats.borrow_mut().thread_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("⚙ THREAD SUSPEND [ID:{}] ({})", thread_id, reason),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread resumption after GC
    pub fn trace_thread_resume(&self, indent: usize, thread_id: u64) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().thread_events += 1;
        }
        self.write_msg(indent, format_args!("⚙ THREAD RESUME [ID:{}]", thread_id));
    }

    #[cfg(feature = "multithreaded-gc")]
    /// Trace stop-the-world GC pause start
    pub fn trace_stw_start(&self, indent: usize, active_threads: usize) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().thread_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("⚙ STOP-THE-WORLD START ({} active threads)", active_threads),
        );
    }

    #[cfg(feature = "multithreaded-gc")]
    /// Trace stop-the-world GC pause end
    pub fn trace_stw_end(&self, indent: usize, duration_us: u64) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().thread_events += 1;
        }
        self.write_msg(
            indent,
            format_args!("⚙ STOP-THE-WORLD END ({} μs)", duration_us),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread state transition
    pub fn trace_thread_state(
        &self,
        indent: usize,
        thread_id: u64,
        old_state: &str,
        new_state: &str,
    ) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().thread_events += 1;
        }
        self.write_msg(
            indent,
            format_args!(
                "⚙ THREAD STATE [ID:{}] {} → {}",
                thread_id, old_state, new_state
            ),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread synchronization (Monitor.Enter/Exit, etc.)
    pub fn trace_thread_sync(
        &self,
        indent: usize,
        thread_id: u64,
        operation: &str,
        obj_addr: usize,
    ) {
        if !self.enabled {
            return;
        }
        if self.detailed_stats {
            self.stats.borrow_mut().thread_events += 1;
        }
        self.write_msg(
            indent,
            format_args!(
                "⚙ THREAD SYNC [ID:{}] {} @ {:#x}",
                thread_id, operation, obj_addr
            ),
        );
    }
}

// this block is all for runtime debug methods
#[allow(dead_code)]
impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    // Tracer-integrated dump methods for comprehensive state capture
    pub fn trace_dump_stack(&self) {
        if !self.tracer_enabled() {
            return;
        }

        let contents: Vec<_> = self.execution.stack[..self.top_of_stack()]
            .iter()
            .map(|h| format!("{:?}", self.get_slot(h)))
            .collect();

        let mut markers = Vec::new();
        for (i, frame) in self.execution.frames.iter().enumerate() {
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

        let tracer = self.tracer();
        for (idx, frame) in self.execution.frames.iter().enumerate() {
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

        let objects: Vec<_> = self.heap()._all_objs.borrow().iter().copied().collect();
        let tracer = self.tracer();
        tracer.dump_heap_snapshot_start(objects.len());

        for obj in objects {
            let Some(ptr) = obj.0 else {
                continue;
            };
            let raw_ptr = Gc::as_ptr(ptr) as *const _ as usize;
            match &ptr.borrow().storage {
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

        let s = self.statics_read();
        let debug_str = format!("{:#?}", &*s);
        self.tracer().dump_statics_snapshot(&debug_str);
    }

    pub fn trace_dump_gc_stats(&self) {
        if !self.tracer_enabled() {
            return;
        }

        self.tracer().dump_gc_stats(
            self.heap().finalization_queue.borrow().len(),
            self.heap().pending_finalization.borrow().len(),
            self.heap().pinned_objects.borrow().len(),
            self.heap().gchandles.borrow().len(),
            self.heap()._all_objs.borrow().len(),
        );
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
        self.tracer().flush();
    }
}
