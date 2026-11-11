use crate::integration_tests_impl::harness::TestHarness;
use dotnetdll::prelude::{Instruction, ReadOptions, Resolution};
use std::{fs, process::Command};

#[test]
fn reflection_tail_dispatch_matches_dotnet() {
    let dir = std::env::temp_dir().join(format!("dotnet-reflection-tail-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temporary project");
    fs::write(
        dir.join("reflection_tail.fs"),
        include_str!("../fixtures_il/reflection_tail.fs"),
    )
    .unwrap();
    fs::write(
        dir.join("reflection_tail.fsproj"),
        include_str!("../fixtures_il/reflection_tail.fsproj"),
    )
    .unwrap();
    let output = Command::new("dotnet")
        .arg("build")
        .arg(dir.join("reflection_tail.fsproj"))
        .args([
            "--configuration",
            "Release",
            "--nologo",
            "--verbosity",
            "quiet",
            "--ignore-failed-sources",
        ])
        .output()
        .expect("build tail-call assembly");
    assert!(
        output.status.success(),
        "F# compilation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dll = dir.join("bin/Release/net10.0/reflection_tail.dll");
    let bytes = fs::read(&dll).expect("read tail-call assembly");
    let resolution =
        Resolution::parse(&bytes, ReadOptions::default()).expect("parse tail-call assembly");
    for name in ["invoke", "invokeCtor"] {
        let method = resolution
            .type_definitions
            .iter()
            .flat_map(|ty| &ty.methods)
            .find(|method| method.name == name)
            .expect("tail-call method");
        assert!(
            method
                .body
                .as_ref()
                .expect("tail-call body")
                .instructions
                .iter()
                .any(|instruction| matches!(
                    instruction,
                    Instruction::CallVirtualTail(_)
                        | Instruction::Call {
                            tail_call: true,
                            ..
                        }
                )),
            "{name} must contain a tail-prefixed call"
        );
    }
    let clr = Command::new("dotnet").arg(&dll).output().expect("run CLR");
    assert_eq!(clr.status.code(), Some(42));
    let harness = TestHarness::get();
    let fsharp_core = harness
        .loader
        .load_resolution_from_file(dll.parent().unwrap().join("FSharp.Core.dll"))
        .expect("load FSharp.Core from the compiled project");
    harness.loader.register_assembly(fsharp_core);
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tracing::{
        Event, Subscriber,
        field::{Field, Visit},
    };
    use tracing_subscriber::{
        layer::{Context, Layer},
        prelude::*,
    };
    struct Capture {
        fallback: Arc<AtomicUsize>,
        asynchronous: Arc<AtomicUsize>,
    }
    #[derive(Default)]
    struct Fields {
        fallback: bool,
        asynchronous: bool,
    }
    impl Visit for Fields {
        fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "reflection_invoke_path" && value == "fallback" {
                self.fallback = true;
            }
            if field.name() == "reflection_invoke_path" && value == "async" {
                self.asynchronous = true;
            }
        }
    }
    impl<S: Subscriber> Layer<S> for Capture {
        fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
            let mut fields = Fields::default();
            event.record(&mut fields);
            if fields.fallback {
                self.fallback.fetch_add(1, Ordering::Relaxed);
            }
            if fields.asynchronous {
                self.asynchronous.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    let fallback_count = Arc::new(AtomicUsize::new(0));
    let async_count = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(
        Capture {
            fallback: Arc::clone(&fallback_count),
            asynchronous: Arc::clone(&async_count),
        }
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.target() == "dotnet_vm::stack::reflection_ops_impl"
                || metadata
                    .target()
                    .starts_with("dotnet_intrinsics_reflection::")
        })),
    );
    assert_eq!(
        crate::integration_tests_impl::harness::with_counting_subscriber(subscriber, || harness
            .run(&dll)),
        42
    );
    // Plan 16 slice 1 admits struct constructor and `Activator` results, the last shapes this
    // fixture sent to the synchronous fallback; slice 3 removes that fallback.
    assert_eq!(
        fallback_count.load(Ordering::Relaxed),
        0,
        "every reflection shape in this fixture is transportable"
    );
    assert!(
        async_count.load(Ordering::Relaxed) >= 8,
        "tail-prefixed reflection calls must admit supported async targets"
    );
    fs::remove_dir_all(dir).expect("remove temporary project");
}
