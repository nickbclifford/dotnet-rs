use std::path::Path;

use crate::integration_tests_impl::harness::TestHarness;

const FORBIDDEN_CORELIB_TRACE_MARKERS: [(&str, &str); 4] = [
    (
        "PortableThreadPool::.ctor",
        "PortableThreadPool construction",
    ),
    ("CreateEventEx", "Kernel32.CreateEventEx"),
    ("LowLevelMonitor_Create", "LowLevelMonitor_Create"),
    ("WaitHandle_WaitOneCore", "WaitHandle_WaitOneCore"),
];

fn forbidden_corelib_trace_path(trace: &str) -> Option<&'static str> {
    FORBIDDEN_CORELIB_TRACE_MARKERS
        .iter()
        .find_map(|(marker, description)| trace.contains(marker).then_some(*description))
}

fn run_cli_with_bounded_trace(
    harness: &TestHarness,
    dll: &Path,
    fixture: &str,
) -> (std::process::Output, String) {
    use std::{
        process::Stdio,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    const RUN_TIMEOUT: Duration = Duration::from_secs(45);
    const MAX_TRACE_BYTES: u64 = 16 * 1024 * 1024;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock precedes the Unix epoch")
        .as_nanos();
    let trace_file = std::env::temp_dir().join(format!(
        "dotnet-rs-corelib-trace-{}-{nonce}-{fixture}.log",
        std::process::id()
    ));
    let mut command = harness.cli_command(dll);
    command
        .env("DOTNET_RS_TRACE", &trace_file)
        .env("DOTNET_RS_TRACE_METHODS", "Program::Main,TaskCompletionSource,ThreadPoolWorkQueue::DispatchWorkItem,ThreadPoolWorkQueue::Dispatch(),EventWaitHandle::.ctor,WaitHandle::WaitOne,TimerQueue::FireNextTimers,System.SR::InternalGetResourceString,ResourceManager::GetString,PortableThreadPool::.ctor")
        .env("RUST_LOG", "method=debug,interop=debug,tracer=warn")
        .env_remove("DOTNET_RS_TRACE_LEVEL")
        .env("DOTNET_RS_TRACE_FORMAT", "text")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|error| panic!("failed to start traced {fixture} fixture: {error}"));
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        if child
            .try_wait()
            .unwrap_or_else(|error| panic!("failed to poll traced {fixture} fixture: {error}"))
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            child
                .kill()
                .expect("failed to terminate timed-out trace fixture");
            let output = child
                .wait_with_output()
                .expect("failed to collect timed-out trace fixture output");
            panic!(
                "traced {fixture} fixture exceeded {RUN_TIMEOUT:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("failed to collect traced {fixture} output: {error}"));
    let metadata = std::fs::metadata(&trace_file)
        .unwrap_or_else(|error| panic!("{fixture} trace file was not created: {error}"));
    assert!(
        metadata.len() <= MAX_TRACE_BYTES,
        "{fixture} trace exceeded the {MAX_TRACE_BYTES}-byte audit limit"
    );
    let trace = std::fs::read_to_string(&trace_file)
        .unwrap_or_else(|error| panic!("failed to read {fixture} trace: {error}"));
    let _ = std::fs::remove_file(trace_file);
    assert!(
        !trace.contains("trace event(s) because tracer channel was full"),
        "{fixture} trace reported dropped events, so forbidden-path evidence is incomplete"
    );
    (output, trace)
}

#[test]
fn forbidden_corelib_trace_controls_are_rejected() {
    for (marker, description) in FORBIDDEN_CORELIB_TRACE_MARKERS {
        let intentional_forbidden_entry = if marker == "PortableThreadPool::.ctor" {
            format!("→ CALL void System.Threading.{marker}() () frame=1")
        } else {
            format!("🔌 NATIVE CALL (Invoking P/Invoke: function [{marker}])")
        };
        assert_eq!(
            forbidden_corelib_trace_path(&intentional_forbidden_entry),
            Some(description),
            "trace audit failed to reject its {description} control"
        );
    }
}

#[cfg(feature = "multithreading")]
#[test]
fn corelib_task_pool_event_timer_and_resource_paths_avoid_forbidden_entries() {
    for (fixture, positive_markers) in [
        (
            "tests/fixtures/async/async_await_task_completion_source_deferred_42.cs",
            &["Program::Main", "TaskCompletionSource"] as &[&str],
        ),
        (
            "tests/fixtures/async/async_pool_run_result_42.cs",
            &["Program::Main", "ThreadPoolWorkQueue::DispatchWorkItem"],
        ),
        (
            "tests/fixtures/threading/event_basic_42.cs",
            &[
                "Program::Main",
                "EventWaitHandle::.ctor",
                "WaitHandle::WaitOne",
            ],
        ),
        (
            "tests/fixtures/threading/threading_timer_callback_pool_thread_42.cs",
            &["Program::Main", "TimerQueue::FireNextTimers"],
        ),
        (
            "tests/fixtures/exceptions/resource_messages_42.cs",
            &[
                "Program::Main",
                "System.SR::InternalGetResourceString",
                "ResourceManager::GetString",
            ],
        ),
    ] {
        let harness = TestHarness::get();
        let dll = harness.ensure_dll(Path::new(fixture));
        let label = Path::new(fixture).file_stem().unwrap().to_str().unwrap();
        let (output, trace) = run_cli_with_bounded_trace(&harness, &dll, label);
        assert_eq!(
            output.status.code(),
            Some(42),
            "{fixture} failed under tracing: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "{fixture} wrote diagnostics under tracing: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            forbidden_corelib_trace_path(&trace),
            None,
            "{fixture} entered a prohibited CoreLib path"
        );
        for marker in positive_markers {
            let relevant_trace = trace
                .lines()
                .filter(|line| {
                    line.contains("Program::Main")
                        || line.contains("ThreadPoolWorkQueue")
                        || line.contains("interop")
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                trace.contains(marker),
                "{fixture} trace did not show its positive control {marker}; relevant trace:\n{relevant_trace}"
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn intentional_native_path_control_is_visible_to_the_trace_audit() {
    let harness = TestHarness::get();
    let dll = harness
        .build(Path::new(
            "tests/debug_fixtures/trace_forbidden_native_paths.cs",
        ))
        .expect("failed to build native-path trace control");
    let (output, trace) = run_cli_with_bounded_trace(&harness, &dll, "forbidden-control");
    assert_eq!(
        output.status.code(),
        Some(42),
        "intentional trace control failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (marker, description) in FORBIDDEN_CORELIB_TRACE_MARKERS {
        if marker == "PortableThreadPool::.ctor" {
            continue;
        }
        assert!(
            trace.contains(marker),
            "intentional control did not emit {description}"
        );
        assert_eq!(
            forbidden_corelib_trace_path(&format!("{marker} {description}")),
            Some(description)
        );
    }
    assert!(
        forbidden_corelib_trace_path(&trace).is_some(),
        "the trace audit accepted the intentional forbidden-path control"
    );
}
