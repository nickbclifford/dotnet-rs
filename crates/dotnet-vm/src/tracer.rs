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
//! - `DOTNET_RS_TRACE_FORMAT`: Output format (`"text"` or `"json"`, default: `"text"`)
//!
//! - `DOTNET_RS_TRACE_LEVEL`: Minimum trace level to output
//!   - `"error"`: Only errors
//!   - `"info"`: Info and above (includes errors)
//!   - `"debug"`: Debug and above (includes info and errors)
//!   - `"trace"`: Trace and above (includes debug, info, and errors)
//!   - `"instruction"`: All messages including instruction-level tracing (default)
//!
//! ## Performance Characteristics
//!
//! - **Asynchronous I/O**: Trace messages are sent to a background thread via a bounded channel,
//!   decoupling tracing from VM execution for minimal performance impact
//! - **Lock-free checks**: Tracer enabled status uses atomic operations, avoiding mutex contention
//! - **256KB write buffer**: Minimizes syscalls in the background writer thread
//! - **Automatic periodic flushing**: Prevents buffer overflow and ensures trace visibility
//! - **Early-exit checks**: Zero-cost when tracing is disabled
//! - **Lazy string formatting**: Strings are only formatted when tracing is active and not filtered
//! - **Level filtering**: Messages are filtered before formatting to avoid unnecessary work
//! - **Optional statistics collection**: Disabled by default for minimal overhead
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
use dotnet_value::object::HeapStorage;
use crate::{
    metrics::RuntimeMetrics,
    stack::CallStack,
};
use crossbeam_channel::{bounded, Sender};
use gc_arena::{unsafe_empty_collect, Collect, Gc};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::File,
    io::{stderr, stdout, BufWriter, Write},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    thread::{self, JoinHandle},
};

const BUFFER_SIZE: usize = 256 * 1024; // 256KB buffer for better IO performance
const AUTO_FLUSH_INTERVAL: usize = 10_000; // Auto-flush every N messages
const CHANNEL_CAPACITY: usize = 10_000; // Channel capacity for trace messages

/// Trace level for filtering messages
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TraceLevel {
    Error = 0,
    Info = 1,
    Debug = 2,
    Trace = 3,
    Instruction = 4,
}

impl TraceLevel {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "error" => Some(Self::Error),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            "instruction" => Some(Self::Instruction),
            _ => None,
        }
    }
}

/// Output format for trace messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TraceFormat {
    Text,
    Json,
}

/// Trace message types for asynchronous tracing
#[derive(Debug)]
enum TraceMessage {
    /// Generic message with indentation and level
    Message {
        level: TraceLevel,
        indent: usize,
        text: String,
        metadata: MessageMetadata,
    },
    /// Flush the output
    Flush,
    /// Shutdown the tracer thread
    Shutdown,
}

/// Metadata for trace messages
#[derive(Debug, Clone, Serialize)]
struct MessageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp_us: Option<u64>,
}

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
    enabled: AtomicBool,
    sender: Option<Sender<TraceMessage>>,
    writer_thread: Option<JoinHandle<()>>,
    message_count: AtomicUsize,
    stats: TraceStats,
    detailed_stats: bool,
    min_level: TraceLevel,
}
unsafe_empty_collect!(Tracer);

impl Tracer {
    pub fn new() -> Self {
        let trace_env = env::var("DOTNET_RS_TRACE");
        let writer: Option<Box<dyn Write + Send>> = match trace_env {
            Ok(val) if val == "1" || val == "true" || val == "stdout" => Some(Box::new(stdout())),
            Ok(val) if val == "stderr" => Some(Box::new(stderr())),
            Ok(val) if !val.is_empty() => {
                // assume it's a file path
                match File::create(&val) {
                    Ok(f) => Some(Box::new(f)),
                    Err(e) => {
                        eprintln!("Failed to create trace file {}: {}", val, e);
                        None
                    }
                }
            }
            _ => None,
        };
        let enabled = writer.is_some();

        // Check for custom auto-flush interval
        let auto_flush_interval = env::var("DOTNET_RS_TRACE_FLUSH_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(AUTO_FLUSH_INTERVAL);

        // Check if detailed statistics should be collected
        let detailed_stats = env::var("DOTNET_RS_TRACE_STATS")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);

        // Determine output format
        let format = env::var("DOTNET_RS_TRACE_FORMAT")
            .ok()
            .and_then(|v| match v.to_lowercase().as_str() {
                "json" => Some(TraceFormat::Json),
                "text" => Some(TraceFormat::Text),
                _ => None,
            })
            .unwrap_or(TraceFormat::Text);

        // Determine minimum trace level
        let min_level = env::var("DOTNET_RS_TRACE_LEVEL")
            .ok()
            .and_then(|v| TraceLevel::from_str(&v))
            .unwrap_or(TraceLevel::Instruction);

        // Set up asynchronous tracing if enabled
        let (sender, writer_thread) = if enabled {
            let (tx, rx) = bounded(CHANNEL_CAPACITY);
            let mut buffered_writer = writer.map(|w| BufWriter::with_capacity(BUFFER_SIZE, w));

            let handle = thread::spawn(move || {
                let mut message_count = 0usize;

                while let Ok(msg) = rx.recv() {
                    match msg {
                        TraceMessage::Message {
                            level,
                            indent,
                            text,
                            metadata,
                        } => {
                            if let Some(ref mut writer) = buffered_writer {
                                match format {
                                    TraceFormat::Text => {
                                        // Write indentation
                                        for _ in 0..indent {
                                            let _ = writer.write_all(b"  ");
                                        }
                                        // Write message
                                        let _ = writer.write_all(text.as_bytes());
                                        let _ = writer.write_all(b"\n");
                                    }
                                    TraceFormat::Json => {
                                        // Serialize as JSON
                                        let json_msg = serde_json::json!({
                                            "level": level,
                                            "indent": indent,
                                            "message": text,
                                            "metadata": metadata,
                                        });
                                        if let Ok(json_str) = serde_json::to_string(&json_msg) {
                                            let _ = writer.write_all(json_str.as_bytes());
                                            let _ = writer.write_all(b"\n");
                                        }
                                    }
                                }

                                // Periodic auto-flush
                                message_count += 1;
                                if message_count >= auto_flush_interval {
                                    let _ = writer.flush();
                                    message_count = 0;
                                }
                            }
                        }
                        TraceMessage::Flush => {
                            if let Some(ref mut writer) = buffered_writer {
                                let _ = writer.flush();
                            }
                            message_count = 0;
                        }
                        TraceMessage::Shutdown => {
                            // Final flush before shutdown
                            if let Some(ref mut writer) = buffered_writer {
                                let _ = writer.flush();
                            }
                            break;
                        }
                    }
                }
            });

            (Some(tx), Some(handle))
        } else {
            (None, None)
        };

        Self {
            enabled: AtomicBool::new(enabled),
            sender,
            writer_thread,
            message_count: AtomicUsize::new(0),
            stats: TraceStats::default(),
            detailed_stats,
            min_level,
        }
    }

    #[inline(always)]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    #[inline(always)]
    fn write_msg(&mut self, level: TraceLevel, indent: usize, args: std::fmt::Arguments) {
        // Early exit if level is filtered out
        if level > self.min_level {
            return;
        }

        if let Some(ref sender) = self.sender {
            let text = format!("{}", args);
            let metadata = MessageMetadata {
                thread_id: None,    // TODO: Add thread ID support
                timestamp_us: None, // TODO: Add timestamp support
            };
            // Use blocking send to ensure message delivery. If the channel is full,
            // this indicates the trace consumer cannot keep up, which is acceptable
            // since tracing is a debugging tool and blocking prevents message loss.
            let _ = sender.send(TraceMessage::Message {
                level,
                indent,
                text,
                metadata,
            });

            // Track message count for statistics
            self.message_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn msg(&mut self, level: TraceLevel, indent: usize, args: std::fmt::Arguments) {
        if !self.is_enabled() {
            return;
        }

        if self.detailed_stats {
            self.stats.total_messages += 1;
        }

        self.write_msg(level, indent, args);
    }

    pub fn flush(&mut self) {
        if self.is_enabled() {
            if let Some(ref sender) = self.sender {
                let _ = sender.try_send(TraceMessage::Flush);
            }
        }
    }

    pub fn trace_instruction(&mut self, indent: usize, ip: usize, instruction: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.instructions_traced += 1;
        }
        self.write_msg(
            TraceLevel::Instruction,
            indent,
            format_args!("[IP:{:04}] {}", ip, instruction),
        );
    }

    pub fn trace_method_entry(&mut self, indent: usize, name: &str, signature: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.method_calls += 1;
        }
        if signature.is_empty() {
            self.write_msg(TraceLevel::Trace, indent, format_args!("→ CALL {}", name));
        } else {
            self.write_msg(
                TraceLevel::Trace,
                indent,
                format_args!("→ CALL {} ({})", name, signature),
            );
        }
    }

    pub fn trace_method_exit(&mut self, indent: usize, name: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.method_returns += 1;
        }
        self.write_msg(TraceLevel::Trace, indent, format_args!("← RET  {}", name));
    }

    pub fn trace_exception(&mut self, indent: usize, exception: &str, location: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.exceptions += 1;
        }
        self.write_msg(
            TraceLevel::Error,
            indent,
            format_args!("⚠ EXC  {} at {}", exception, location),
        );
    }

    pub fn trace_gc_event(&mut self, indent: usize, event: &str, details: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.gc_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("♻ GC   {} ({})", event, details),
        );
    }

    pub fn trace_stack_op(&mut self, indent: usize, op: &str, value: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.stack_ops += 1;
        }
        self.write_msg(
            TraceLevel::Instruction,
            indent,
            format_args!("  STACK {} {}", op, value),
        );
    }

    pub fn trace_field_access(&mut self, indent: usize, op: &str, field: &str, value: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.field_accesses += 1;
        }
        self.write_msg(
            TraceLevel::Instruction,
            indent,
            format_args!("  FIELD {} {} = {}", op, field, value),
        );
    }

    pub fn trace_branch(&mut self, indent: usize, branch_type: &str, target: usize, taken: bool) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.branches += 1;
        }
        let status = if taken { "TAKEN" } else { "NOT TAKEN" };
        self.write_msg(
            TraceLevel::Instruction,
            indent,
            format_args!("↷ {} to {:04} ({})", branch_type, target, status),
        );
    }

    pub fn trace_type_info(&mut self, indent: usize, operation: &str, type_name: &str) {
        if !self.is_enabled() {
            return;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("  TYPE {} {}", operation, type_name),
        );
    }

    pub fn trace_intrinsic(&mut self, indent: usize, operation: &str, details: &str) {
        if !self.is_enabled() {
            return;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("⚡ INTRINSIC {} {}", operation, details),
        );
    }

    // Performance counter helpers
    pub fn get_message_count(&self) -> usize {
        self.message_count.load(Ordering::Relaxed)
    }

    pub fn reset_message_count(&mut self) {
        self.message_count.store(0, Ordering::Relaxed);
    }

    pub fn get_stats(&self) -> TraceStats {
        self.stats.clone()
    }

    pub fn reset_stats(&mut self) {
        self.stats = TraceStats::default();
    }

    pub fn print_stats(&self) {
        if !self.detailed_stats {
            return;
        }
        let stats = &self.stats;
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
        if self.detailed_stats && self.is_enabled() {
            self.print_stats();
        }

        // Send shutdown signal to background thread.
        // If sending fails, the channel is already disconnected (thread panicked).
        if let Some(ref sender) = self.sender {
            let _ = sender.send(TraceMessage::Shutdown);
        }

        // Wait for background thread to finish flushing and exit.
        // If the thread panicked, join() will return Err, which we ignore to avoid
        // panicking during drop. The thread owns no critical resources.
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
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
    pub fn dump_stack_state(
        &mut self,
        stack_contents: &[String],
        frame_markers: &[(usize, String)],
    ) {
        if !self.is_enabled() {
            return;
        }

        self.msg(TraceLevel::Debug, 0, format_args!(""));
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(TraceLevel::Debug, 0, format_args!("║ STACK SNAPSHOT"));
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );

        if stack_contents.is_empty() {
            self.msg(TraceLevel::Debug, 0, format_args!("║ (empty stack)"));
        } else {
            for (idx, content) in stack_contents.iter().enumerate().rev() {
                // Check for frame markers at this position
                let markers: Vec<_> = frame_markers
                    .iter()
                    .filter(|(pos, _)| *pos == idx)
                    .map(|(_, label)| label.as_str())
                    .collect();

                for marker in markers {
                    self.msg(TraceLevel::Debug, 0, format_args!("╟─ {} ", marker));
                }

                self.msg(
                    TraceLevel::Debug,
                    0,
                    format_args!("║ [{:4}] {}", idx, content),
                );
            }
        }

        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes frame information to the trace output
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
        if !self.is_enabled() {
            return;
        }

        self.msg(TraceLevel::Debug, 0, format_args!(""));
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ FRAME #{} - {}", frame_idx, method_name),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ IP:           {:04}", ip),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ Args base:    {}", args_base),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ Locals base:  {}", locals_base),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ Stack base:   {}", stack_base),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ Stack height: {}", stack_height),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes heap object information to the trace output
    pub fn dump_heap_object(&mut self, ptr_addr: usize, obj_type: &str, details: &str) {
        if !self.is_enabled() {
            return;
        }

        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("HEAP[{:#x}] {} => {}", ptr_addr, obj_type, details),
        );
    }

    /// Writes a heap snapshot header
    pub fn dump_heap_snapshot_start(&mut self, object_count: usize) {
        if !self.is_enabled() {
            return;
        }

        self.msg(TraceLevel::Debug, 0, format_args!(""));
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ HEAP SNAPSHOT ({} objects)", object_count),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes a heap snapshot footer
    pub fn dump_heap_snapshot_end(&mut self) {
        if !self.is_enabled() {
            return;
        }

        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes static storage information
    pub fn dump_statics_snapshot(&mut self, statics_debug: &str) {
        if !self.is_enabled() {
            return;
        }

        self.msg(TraceLevel::Debug, 0, format_args!(""));
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ STATIC STORAGE SNAPSHOT"),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );

        for line in statics_debug.lines() {
            self.msg(TraceLevel::Debug, 0, format_args!("║ {}", line));
        }

        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes a complete runtime state snapshot
    pub fn dump_full_state_header(&mut self) {
        if !self.is_enabled() {
            return;
        }

        self.msg(TraceLevel::Debug, 0, format_args!(""));
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ FULL RUNTIME STATE SNAPSHOT"),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes GC statistics
    pub fn dump_gc_stats(
        &mut self,
        finalization_queue: usize,
        pending_finalization: usize,
        pinned_objects: usize,
        gc_handles: usize,
        all_objects: usize,
    ) {
        if !self.is_enabled() {
            return;
        }

        self.msg(TraceLevel::Debug, 0, format_args!(""));
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(TraceLevel::Debug, 0, format_args!("║ GC STATISTICS"));
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ All objects:          {}", all_objects),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ Finalization queue:   {}", finalization_queue),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ Pending finalization: {}", pending_finalization),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ Pinned objects:       {}", pinned_objects),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ GC handles:           {}", gc_handles),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    /// Writes runtime metrics to the trace output
    pub fn dump_runtime_metrics(&mut self, metrics: &RuntimeMetrics) {
        if !self.is_enabled() {
            return;
        }

        let gc_pause_total = metrics.gc_pause_total_us.load(Ordering::Relaxed);
        let gc_pause_count = metrics.gc_pause_count.load(Ordering::Relaxed);
        let gc_allocated = metrics.current_gc_allocated.load(Ordering::Relaxed);
        let ext_allocated = metrics.current_external_allocated.load(Ordering::Relaxed);
        let lock_count = metrics.lock_contention_count.load(Ordering::Relaxed);
        let lock_total = metrics.lock_contention_total_us.load(Ordering::Relaxed);

        self.msg(TraceLevel::Debug, 0, format_args!(""));
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╔════════════════════════════════════════════════════════════"),
        );
        self.msg(TraceLevel::Debug, 0, format_args!("║ RUNTIME METRICS"));
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╠════════════════════════════════════════════════════════════"),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ GC Pause Total:       {:>12} μs", gc_pause_total),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ GC Pause Count:       {:>12}", gc_pause_count),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ GC Allocated:         {:>12} bytes", gc_allocated),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ External Allocated:   {:>12} bytes", ext_allocated),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ Lock Contention:      {:>12}", lock_count),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("║ Lock Wait Total:      {:>12} μs", lock_total),
        );
        self.msg(
            TraceLevel::Debug,
            0,
            format_args!("╚════════════════════════════════════════════════════════════"),
        );
    }

    // === GC-specific tracing methods ===

    /// Trace the start of a GC collection cycle
    pub fn trace_gc_collection_start(&mut self, indent: usize, generation: usize, reason: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.gc_collections += 1;
            self.stats.gc_events += 1;
        }
        self.write_msg(
            TraceLevel::Info,
            indent,
            format_args!("♻ GC   COLLECTION START [Gen {}] ({})", generation, reason),
        );
    }

    /// Trace the end of a GC collection cycle
    pub fn trace_gc_collection_end(
        &mut self,
        indent: usize,
        generation: usize,
        collected: usize,
        duration_us: u64,
    ) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.gc_events += 1;
        }
        self.write_msg(
            TraceLevel::Info,
            indent,
            format_args!(
                "♻ GC   COLLECTION END [Gen {}] ({} objects collected, {} μs)",
                generation, collected, duration_us
            ),
        );
    }

    /// Trace heap allocation
    pub fn trace_gc_allocation(&mut self, indent: usize, type_name: &str, size_bytes: usize) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.gc_allocations += 1;
            self.stats.gc_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("♻ GC   ALLOC {} ({} bytes)", type_name, size_bytes),
        );
    }

    /// Trace object finalization
    pub fn trace_gc_finalization(&mut self, indent: usize, obj_type: &str, obj_addr: usize) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.gc_finalizations += 1;
            self.stats.gc_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("♻ GC   FINALIZE {} @ {:#x}", obj_type, obj_addr),
        );
    }

    /// Trace GC handle operations
    pub fn trace_gc_handle(
        &mut self,
        indent: usize,
        operation: &str,
        handle_type: &str,
        addr: usize,
    ) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.gc_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!(
                "♻ GC   HANDLE {} [{}] @ {:#x}",
                operation, handle_type, addr
            ),
        );
    }

    /// Trace GC pinning operations
    pub fn trace_gc_pin(&mut self, indent: usize, operation: &str, obj_addr: usize) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.gc_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("♻ GC   PIN {} @ {:#x}", operation, obj_addr),
        );
    }

    /// Trace weak reference updates
    pub fn trace_gc_weak_ref(&mut self, indent: usize, operation: &str, handle_id: usize) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.gc_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("♻ GC   WEAK {} (handle {})", operation, handle_id),
        );
    }

    /// Trace resurrection during finalization
    pub fn trace_gc_resurrection(&mut self, indent: usize, obj_type: &str, obj_addr: usize) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.gc_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("♻ GC   RESURRECT {} @ {:#x}", obj_type, obj_addr),
        );
    }

    // === Threading-specific tracing methods ===

    #[cfg(feature = "multithreading")]
    /// Trace thread creation
    pub fn trace_thread_create(&mut self, indent: usize, thread_id: u64, name: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.thread_events += 1;
        }
        self.write_msg(
            TraceLevel::Info,
            indent,
            format_args!("⚙ THREAD CREATE [ID:{}] \"{}\"", thread_id, name),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread start
    pub fn trace_thread_start(&mut self, indent: usize, thread_id: u64) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.thread_events += 1;
        }
        self.write_msg(
            TraceLevel::Info,
            indent,
            format_args!("⚙ THREAD START [ID:{}]", thread_id),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread exit
    pub fn trace_thread_exit(&mut self, indent: usize, thread_id: u64) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.thread_events += 1;
        }
        self.write_msg(
            TraceLevel::Info,
            indent,
            format_args!("⚙ THREAD EXIT [ID:{}]", thread_id),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread reaching a safe point
    pub fn trace_thread_safepoint(&mut self, indent: usize, thread_id: u64, location: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.thread_safepoints += 1;
            self.stats.thread_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("⚙ THREAD SAFEPOINT [ID:{}] at {}", thread_id, location),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread suspension for GC
    pub fn trace_thread_suspend(&mut self, indent: usize, thread_id: u64, reason: &str) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.thread_suspensions += 1;
            self.stats.thread_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("⚙ THREAD SUSPEND [ID:{}] ({})", thread_id, reason),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread resumption after GC
    pub fn trace_thread_resume(&mut self, indent: usize, thread_id: u64) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.thread_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
            indent,
            format_args!("⚙ THREAD RESUME [ID:{}]", thread_id),
        );
    }

    #[cfg(feature = "multithreaded-gc")]
    /// Trace stop-the-world GC pause start
    pub fn trace_stw_start(&mut self, indent: usize, active_threads: usize) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.thread_events += 1;
        }
        self.write_msg(
            TraceLevel::Info,
            indent,
            format_args!("⚙ STOP-THE-WORLD START ({} active threads)", active_threads),
        );
    }

    #[cfg(feature = "multithreaded-gc")]
    /// Trace stop-the-world GC pause end
    pub fn trace_stw_end(&mut self, indent: usize, duration_us: u64) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.thread_events += 1;
        }
        self.write_msg(
            TraceLevel::Info,
            indent,
            format_args!("⚙ STOP-THE-WORLD END ({} μs)", duration_us),
        );
    }

    #[cfg(feature = "multithreading")]
    /// Trace thread state transition
    pub fn trace_thread_state(
        &mut self,
        indent: usize,
        thread_id: u64,
        old_state: &str,
        new_state: &str,
    ) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.thread_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
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
        &mut self,
        indent: usize,
        thread_id: u64,
        operation: &str,
        obj_addr: usize,
    ) {
        if !self.is_enabled() {
            return;
        }
        if self.detailed_stats {
            self.stats.thread_events += 1;
        }
        self.write_msg(
            TraceLevel::Debug,
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

        let mut tracer = self.tracer();
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
        let mut tracer = self.tracer();
        tracer.dump_heap_snapshot_start(objects.len());

        for obj in objects {
            let Some(ptr) = obj.0 else {
                continue;
            };
            let raw_ptr = Gc::as_ptr(ptr) as *const _ as usize;
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

        let s = self.statics();
        let debug_str = format!("{:#?}", s);
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
