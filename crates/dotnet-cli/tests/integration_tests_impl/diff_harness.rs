macro_rules! diff_test {
    ($name:ident, $fixture_path:expr) => {
        #[cfg(not(feature = "fuzzing"))]
        #[test]
        fn $name() {
            use crate::integration_tests_impl::harness::TestHarness;
            use std::{path::Path, process::Command};

            let harness = TestHarness::get();
            let dll_path = harness.ensure_dll(Path::new($fixture_path));

            let dotnet_output = Command::new("dotnet")
                .arg(&dll_path)
                .output()
                .expect("failed to run fixture with dotnet");
            let dotnet_exit_code = dotnet_output.status.code().unwrap_or(255) as u8;
            let dotnet_stdout = String::from_utf8_lossy(&dotnet_output.stdout).to_string();

            let (dotnet_rs_exit_code, dotnet_rs_stdout) = harness.run_cli(&dll_path);

            assert_eq!(
                dotnet_exit_code, 42,
                "sanity check failed: dotnet should return fixture exit code 42"
            );
            assert_eq!(
                dotnet_rs_exit_code, dotnet_exit_code,
                "dotnet-rs exit code diverged from dotnet"
            );
            assert_eq!(
                dotnet_rs_stdout, dotnet_stdout,
                "dotnet-rs stdout diverged from dotnet"
            );
        }
    };
}

macro_rules! diff_test_exit_code {
    ($name:ident, $fixture_path:expr, $expected_exit_code:expr) => {
        #[cfg(not(feature = "fuzzing"))]
        #[test]
        fn $name() {
            use crate::integration_tests_impl::harness::TestHarness;
            use std::{path::Path, process::Command};

            let harness = TestHarness::get();
            let dll_path = harness.ensure_dll(Path::new($fixture_path));

            let dotnet_output = Command::new("dotnet")
                .arg(&dll_path)
                .output()
                .expect("failed to run fixture with dotnet");
            let dotnet_exit_code = dotnet_output.status.code().unwrap_or(255) as u8;

            let (dotnet_rs_exit_code, _) = harness.run_cli(&dll_path);
            let expected_exit_code: u8 = $expected_exit_code;

            assert_eq!(
                dotnet_exit_code, expected_exit_code,
                "sanity check failed: dotnet should return fixture exit code {}",
                expected_exit_code
            );
            assert_eq!(
                dotnet_rs_exit_code, dotnet_exit_code,
                "dotnet-rs exit code diverged from dotnet"
            );
        }
    };
}

#[cfg(feature = "multithreading")]
diff_test!(
    memmove_overlap_gc_matches_dotnet,
    "tests/fixtures/unsafe/memmove_overlap_gc_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    bulk_fill_gc_matches_dotnet,
    "tests/fixtures/unsafe/bulk_fill_gc_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    string_equality_gc_matches_dotnet,
    "tests/fixtures/strings/equality_gc_42.cs"
);

diff_test!(
    string_char_repeat_matches_dotnet,
    "tests/fixtures/strings/string_char_repeat_42.cs"
);

// Rejected candidates: structs/interlocked_misaligned_1 (intentional ECMA alignment
// divergence);
// exceptions/unhandled_exception_254 and stack_trace_*_254 (the in-process fixture oracle is
// reserved code 254, while the unchanged subprocess behavior is real .NET exit 134 versus
// dotnet-rs CLI exit 1 for unhandled exceptions); exceptions/intrinsic_trace_42 (rejected from
// full stdout comparison for its GC.Collect mismatch, but covered by the qualified exit-code-only
// differential test);
// structs/explicit_layout_0 (real .NET exits 134); unsafe/managed_ptr_size_0 (fat pointer
// representation); and threading/threading_*_0 (exit-code-only comparison has no signal).
// The original expression-compilation case is retained while the canonical set expands coverage.
#[cfg(all(feature = "multithreading", not(feature = "fuzzing"), unix))]
#[test]
fn threading_fatal_exceptions_match_dotnet() {
    use crate::integration_tests_impl::harness::TestHarness;
    use std::{
        os::unix::process::ExitStatusExt,
        path::Path,
        process::{Command, Output, Stdio},
        time::{Duration, Instant},
    };

    fn output_with_timeout(command: &mut Command) -> Output {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if child.try_wait().unwrap().is_some() {
                return child.wait_with_output().unwrap();
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "fixture timed out: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    let harness = TestHarness::get();
    for (fixture, expected_stdout, expected_message) in [
        ("pool_callback", "before worker\n", "worker failure"),
        ("worker_joined", "before worker\n", "worker failure"),
        ("worker_unjoined", "before worker\n", "worker failure"),
        ("worker_after_return", "before worker\n", "worker failure"),
        ("worker_computation", "before worker\n", "worker failure"),
        ("entry_locked", "before entry failure\n", "entry failure"),
        ("entry_sleeping", "before entry failure\n", "entry failure"),
    ] {
        let source = format!("tests/debug_fixtures/fatal_{fixture}.cs");
        let dll = harness.ensure_dll(Path::new(&source));
        let clr = output_with_timeout(Command::new("dotnet").arg(&dll));
        let vm = output_with_timeout(&mut harness.cli_command(&dll));
        assert_eq!(
            clr.status.signal(),
            Some(6),
            "CLR {fixture}: {:?}",
            clr.status
        );
        assert_eq!(
            vm.status.code(),
            Some(1),
            "VM {fixture}: {}",
            String::from_utf8_lossy(&vm.stderr)
        );
        assert_eq!(vm.stdout, clr.stdout, "{fixture} stdout");
        assert_eq!(
            vm.stdout,
            expected_stdout.as_bytes(),
            "{fixture} continued after throw"
        );
        let clr_stderr = String::from_utf8_lossy(&clr.stderr);
        assert!(
            clr_stderr.contains("System.InvalidOperationException")
                && clr_stderr.contains(expected_message),
            "CLR {fixture}: {clr_stderr}"
        );
        let stderr = String::from_utf8_lossy(&vm.stderr);
        assert!(
            stderr.contains("System.InvalidOperationException")
                && stderr.contains(expected_message),
            "VM {fixture}: {stderr}"
        );
    }
}

diff_test!(
    statics_cctor_filter_matches_dotnet,
    "tests/fixtures/statics/cctor_filter_42.cs"
);
diff_test!(
    statics_cctor_filter_newobj_matches_dotnet,
    "tests/fixtures/statics/cctor_filter_newobj_42.cs"
);
diff_test!(
    statics_cctor_filter_call_matches_dotnet,
    "tests/fixtures/statics/cctor_filter_call_42.cs"
);
diff_test!(
    statics_cctor_filter_delegate_matches_dotnet,
    "tests/fixtures/statics/cctor_filter_delegate_42.cs"
);
diff_test!(
    statics_cctor_filter_delegate_multicast_matches_dotnet,
    "tests/fixtures/statics/cctor_filter_delegate_multicast_42.cs"
);
diff_test!(
    statics_cctor_filter_reflection_matches_dotnet,
    "tests/fixtures/statics/cctor_filter_reflection_42.cs"
);
diff_test!(
    statics_cctor_filter_run_class_constructor_matches_dotnet,
    "tests/fixtures/statics/cctor_filter_run_class_constructor_42.cs"
);
diff_test!(
    statics_cctor_run_class_constructor_matches_dotnet,
    "tests/fixtures/statics/cctor_run_class_constructor_42.cs"
);
diff_test!(
    statics_cctor_generic_handle_matches_dotnet,
    "tests/fixtures/statics/cctor_generic_handle_42.cs"
);
diff_test!(
    statics_cctor_field_operations_matches_dotnet,
    "tests/fixtures/statics/cctor_field_operations_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    statics_cctor_wait_success_matches_dotnet,
    "tests/fixtures/statics/cctor_wait_success_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    statics_cctor_wait_failure_matches_dotnet,
    "tests/fixtures/statics/cctor_wait_failure_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    statics_cctor_wait_triggers_success_matches_dotnet,
    "tests/fixtures/statics/cctor_wait_triggers_success_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    statics_cctor_wait_triggers_failure_matches_dotnet,
    "tests/fixtures/statics/cctor_wait_triggers_failure_42.cs"
);
diff_test!(
    statics_cctor_recursive_triggers_matches_dotnet,
    "tests/fixtures/statics/cctor_recursive_triggers_42.cs"
);
diff_test!(
    statics_cctor_success_triggers_matches_dotnet,
    "tests/fixtures/statics/cctor_success_triggers_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    reflection_invoke_argument_race_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_argument_race_42.cs"
);
diff_test!(
    reflection_invoke_reference_byref_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_reference_byref_42.cs"
);
diff_test!(
    reflection_invoke_fallback_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_fallback_42.cs"
);
diff_test!(
    reflection_invoke_values_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_values_42.cs"
);
diff_test!(
    reflection_invoke_byref_like_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_byref_like_42.cs"
);
diff_test!(
    reflection_invoke_target_validation_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_target_validation_42.cs"
);
diff_test!(
    reflection_invoke_argument_count_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_argument_count_42.cs"
);
diff_test!(
    reflection_invoke_argument_types_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_argument_types_42.cs"
);
diff_test!(
    delegates_multicast_values_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_values_42.cs"
);
diff_test!(
    delegates_invocation_list_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_invocation_list_42.cs"
);
diff_test!(
    delegates_dynamic_invoke_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_dynamic_invoke_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    delegates_multicast_contended_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_contended_42.cs"
);
diff_test!(
    delegates_multicast_unmanaged_ref_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_unmanaged_ref_42.cs"
);
diff_test!(
    delegates_multicast_typed_reference_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_typed_reference_42.cs"
);
diff_test!(
    delegates_multicast_byref_return_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_byref_return_42.cs"
);
diff_test!(
    delegates_multicast_ref_struct_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_ref_struct_42.cs"
);
diff_test!(
    reflection_invoke_constructors_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_constructors_42.cs"
);
diff_test!(
    reflection_invoke_results_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_results_42.cs"
);
diff_test!(
    reflection_invoke_intrinsic_value_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_intrinsic_value_42.cs"
);
diff_test!(
    reflection_invoke_byref_gc_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_byref_gc_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    reflection_invoke_byref_foreign_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_byref_foreign_42.cs"
);
diff_test!(
    reflection_invoke_collect_42_matches_dotnet,
    "tests/fixtures/reflection/invoke_collect_42.cs"
);
diff_test!(
    fields_enum_frame_homes_42_matches_dotnet,
    "tests/fixtures/fields/enum_frame_homes_42.cs"
);
diff_test!(
    delegates_multicast_static_ref_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_static_ref_42.cs"
);
diff_test!(
    delegates_multicast_many_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_many_42.cs"
);
diff_test!(
    delegates_multicast_scalar_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_scalar_42.cs"
);
diff_test!(
    delegates_multicast_aggregate_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_aggregate_42.cs"
);
diff_test!(
    delegates_multicast_nested_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_nested_42.cs"
);
diff_test!(
    delegates_multicast_reflection_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_reflection_42.cs"
);
diff_test!(
    delegates_multicast_finalizers_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_finalizers_42.cs"
);
diff_test!(
    delegates_multicast_filters_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_filters_42.cs"
);
diff_test!(
    delegates_multicast_static_ref_filters_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_static_ref_filters_42.cs"
);
diff_test!(
    delegates_multicast_refs_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_refs_42.cs"
);
diff_test!(
    delegates_multicast_ref_cctor_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_ref_cctor_42.cs"
);
diff_test!(
    delegates_multicast_cctor_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_cctor_42.cs"
);
diff_test!(
    delegates_multicast_static_ref_cctor_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_static_ref_cctor_42.cs"
);
diff_test!(
    delegates_multicast_gc_collect_42_matches_dotnet,
    "tests/fixtures/delegates/delegate_multicast_gc_collect_42.cs"
);
diff_test!(
    strings_substring_chunks_42_matches_dotnet,
    "tests/fixtures/strings/substring_chunks_42.cs"
);
diff_test!(
    strings_substring_fallback_42_matches_dotnet,
    "tests/fixtures/strings/substring_fallback_42.cs"
);
diff_test!(
    strings_ctor_char_array_range_42_matches_dotnet,
    "tests/fixtures/strings/ctor_char_array_range_42.cs"
);
diff_test!(
    expressions_expression_compile_42_matches_dotnet,
    "tests/fixtures/expressions/expression_compile_42.cs"
);
diff_test!(
    arithmetic_arithmetic_operations_42_matches_dotnet,
    "tests/fixtures/arithmetic/arithmetic_operations_42.cs"
);
diff_test!(
    exceptions_exception_batch_42_matches_dotnet,
    "tests/fixtures/exceptions/exception_batch_42.cs"
);
diff_test!(
    exceptions_nested_exceptions_42_matches_dotnet,
    "tests/fixtures/exceptions/nested_exceptions_42.cs"
);
diff_test!(
    memory_nullable_boxing_42_matches_dotnet,
    "tests/fixtures/memory/nullable_boxing_42.cs"
);
diff_test!(
    threading_interlocked_42_matches_dotnet,
    "tests/fixtures/threading/interlocked_42.cs"
);
diff_test!(
    threading_volatile_intrinsics_42_matches_dotnet,
    "tests/fixtures/threading/volatile_intrinsics_42.cs"
);
diff_test_exit_code!(
    exceptions_intrinsic_trace_42_matches_dotnet_exit_code,
    "tests/fixtures/exceptions/intrinsic_trace_42.cs",
    42
);

diff_test!(
    nested_filter_catches_matches_dotnet,
    "tests/fixtures/exceptions/nested_filter_catches_42.cs"
);

diff_test!(
    nested_filter_throws_matches_dotnet,
    "tests/fixtures/exceptions/nested_filter_throws_42.cs"
);

diff_test!(
    nested_filter_depth_matches_dotnet,
    "tests/fixtures/exceptions/nested_filter_depth_42.cs"
);

diff_test!(
    nested_filter_byref_matches_dotnet,
    "tests/fixtures/exceptions/nested_filter_byref_42.cs"
);

diff_test!(
    nested_filter_rethrow_matches_dotnet,
    "tests/fixtures/exceptions/nested_filter_rethrow_42.cs"
);

diff_test_exit_code!(
    exceptions_exception_filter_7_matches_dotnet,
    "tests/fixtures/exceptions/exception_filter_7.cs",
    7
);

diff_test_exit_code!(
    exceptions_filter_throws_keeps_original_matches_dotnet,
    "tests/fixtures/exceptions/filter_throws_keeps_original_42.cs",
    42
);

#[cfg(feature = "multithreading")]
diff_test!(
    canonical_type_objects_42_matches_dotnet,
    "tests/fixtures/threading/canonical_type_objects_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    canonical_member_objects_42_matches_dotnet,
    "tests/fixtures/threading/canonical_member_objects_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    threading_monitor_contended_lock_42_matches_dotnet,
    "tests/fixtures/threading/monitor_contended_lock_42.cs"
);

diff_test!(
    threading_synchronized_value_type_42_matches_dotnet,
    "tests/fixtures/threading/synchronized_value_type_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    threading_monitor_fresh_object_race_42_matches_dotnet,
    "tests/fixtures/threading/monitor_fresh_object_race_42.cs"
);

diff_test!(
    threading_monitor_byref_origins_42_matches_dotnet,
    "tests/fixtures/threading/monitor_byref_origins_42.cs"
);

diff_test!(
    threading_monitor_argument_validation_matches_dotnet,
    "tests/fixtures/threading/monitor_argument_validation_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    threading_foreground_worker_42_matches_dotnet,
    "tests/fixtures/threading/foreground_worker_42.cs"
);

diff_test!(
    weak_reference_liveness_matches_dotnet,
    "tests/fixtures/gc/weak_reference_liveness_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    retired_arena_cycles_matches_dotnet,
    "tests/fixtures/gc/retired_arena_cycles_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    retired_arena_static_matches_dotnet,
    "tests/debug_fixtures/retired_arena_static_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    retired_finalizer_matches_dotnet,
    "tests/debug_fixtures/retired_finalizer_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    retired_finalizer_monitor_matches_dotnet,
    "tests/debug_fixtures/retired_finalizer_monitor_42.cs"
);
diff_test!(
    finalizer_collect_matches_dotnet,
    "tests/fixtures/gc/finalizer_collect_42.cs"
);
diff_test!(
    finalizer_self_wait_matches_dotnet,
    "tests/fixtures/gc/finalizer_self_wait_42.cs"
);
diff_test!(
    finalizer_cctor_self_wait_matches_dotnet,
    "tests/fixtures/gc/finalizer_cctor_self_wait_42.cs"
);

diff_test!(
    finalizer_allocation_matches_dotnet,
    "tests/fixtures/gc/finalizer_allocation_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    retired_finalizer_collect_matches_dotnet,
    "tests/debug_fixtures/retired_finalizer_collect_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    global_finalizer_wait_matches_dotnet,
    "tests/debug_fixtures/global_finalizer_wait_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    live_finalizer_wait_matches_dotnet,
    "tests/debug_fixtures/live_finalizer_wait_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    foreground_finalizer_wait_matches_dotnet,
    "tests/debug_fixtures/foreground_finalizer_wait_42.cs"
);

#[cfg(feature = "multithreading")]
diff_test!(
    thread_finalization_matches_dotnet,
    "tests/debug_fixtures/thread_finalization_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    thread_retired_creator_finalization_matches_dotnet,
    "tests/debug_fixtures/thread_retired_creator_finalization_42.cs"
);

diff_test!(
    exceptions_handlers_inside_handlers_matches_dotnet,
    "tests/fixtures/exceptions/handlers_inside_handlers_42.cs"
);
diff_test!(
    exceptions_nested_finally_replacement_matches_dotnet,
    "tests/fixtures/exceptions/nested_finally_42.cs"
);

// C# has no fault clause. Generate the IL so the same cases run on the CLR and VM.
#[cfg(not(feature = "fuzzing"))]
#[test]
fn exceptions_fault_inside_finally_matches_dotnet() {
    use crate::integration_tests_impl::harness::TestHarness;
    use dotnetdll::prelude::*;
    use std::{fs, path::Path, process::Command};

    let harness = TestHarness::get();
    let reference =
        harness.ensure_dll(Path::new("tests/fixtures/exceptions/exception_batch_42.cs"));
    let runtime_config = fs::read(reference.with_extension("runtimeconfig.json"))
        .expect("read fixture runtime configuration");
    let dir = std::env::temp_dir().join(format!("dotnet-rs-fault-fixture-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create fixture directory");

    for pending in [false, true] {
        let name = if pending {
            "fault_pending"
        } else {
            "fault_normal"
        };
        let mut resolution = Resolution::new(Module::new(format!("{name}.dll")));
        resolution.assembly = Some(Assembly::new(name));
        let core = resolution.push_assembly_reference(ExternalAssemblyReference::new("mscorlib"));
        let object = resolution.push_type_reference(ExternalTypeReference::new(
            Some("System".into()),
            "Object",
            ResolutionScope::Assembly(core),
        ));
        let exception = resolution.push_type_reference(ExternalTypeReference::new(
            Some("System".into()),
            "Exception",
            ResolutionScope::Assembly(core),
        ));
        let exception_type: MethodType = BaseType::class(exception).into();
        let ctor = resolution.push_method_reference(ExternalMethodReference {
            parent: MethodReferenceParent::Type(exception_type.clone()),
            name: ".ctor".into(),
            signature: ManagedMethod::instance(ReturnType::VOID, vec![]),
            attributes: vec![],
        });
        let program = resolution.push_type_definition(TypeDefinition::new(None, "Program"));
        resolution[program].set_extends(object);

        let (
            mut instructions,
            outer_try,
            outer_finally,
            inner_try,
            inner_fault,
            inner_catch,
            outer_end,
            outer_catch,
            done,
        ) = asm! {
            LoadConstantInt32 0;
            StoreLocal 0;
            +outer_try NoOperation;
            NoOperation;
            +outer_finally NoOperation;
            +inner_try NewObject UserMethod::Reference(ctor);
            Throw;
            +inner_fault LoadConstantInt32 40;
            StoreLocal 0;
            EndFinally;
            +inner_catch Pop;
            Leave outer_end;
            +outer_end LoadLocal 0;
            LoadConstantInt32 1;
            Add;
            StoreLocal 0;
            EndFinally;
            +outer_catch Pop;
            LoadLocal 0;
            LoadConstantInt32 1;
            Add;
            StoreLocal 0;
            Leave done;
            +done LoadLocal 0;
            Return;
        };
        if pending {
            instructions[outer_try] = Instruction::NewObject(UserMethod::Reference(ctor));
            instructions[outer_try + 1] = Instruction::Throw;
        } else {
            instructions[outer_try] = Instruction::Leave(done);
            instructions[outer_try + 1] = Instruction::Leave(done);
        }
        let clauses = vec![
            body::Exception {
                kind: body::ExceptionKind::Fault,
                try_offset: inner_try,
                try_length: inner_fault - inner_try,
                handler_offset: inner_fault,
                handler_length: inner_catch - inner_fault,
            },
            body::Exception {
                kind: body::ExceptionKind::TypedException(exception_type.clone()),
                try_offset: inner_try,
                try_length: inner_catch - inner_try,
                handler_offset: inner_catch,
                handler_length: outer_end - inner_catch,
            },
            body::Exception {
                kind: body::ExceptionKind::Finally,
                try_offset: outer_try,
                try_length: outer_finally - outer_try,
                handler_offset: outer_finally,
                handler_length: outer_catch - outer_finally,
            },
            body::Exception {
                kind: body::ExceptionKind::TypedException(exception_type),
                try_offset: outer_try,
                try_length: outer_catch - outer_try,
                handler_offset: outer_catch,
                handler_length: done - outer_catch,
            },
        ];
        let main = resolution.push_method(
            program,
            Method::new(
                Accessibility::Public,
                ManagedMethod::static_member(
                    ReturnType::value(MethodType::Base(Box::new(BaseType::Int32))),
                    vec![],
                ),
                "Main",
                Some(body::Method {
                    data_sections: vec![body::DataSection::ExceptionHandlers(clauses)],
                    ..body::Method::with_locals(
                        vec![LocalVariable::new(MethodType::Base(Box::new(
                            BaseType::Int32,
                        )))],
                        instructions,
                    )
                }),
            ),
        );
        resolution.set_entry_point(main);
        let bytes = resolution
            .write(WriteOptions {
                is_32_bit: false,
                is_executable: true,
            })
            .expect("write fault fixture");
        let dll = dir.join(format!("{name}.dll"));
        fs::write(&dll, bytes).expect("write fault fixture");
        fs::write(dll.with_extension("runtimeconfig.json"), &runtime_config)
            .expect("write runtime configuration");
        let clr = Command::new("dotnet")
            .arg(&dll)
            .output()
            .expect("run CLR fixture");
        let expected = if pending { 42 } else { 41 };
        assert_eq!(
            clr.status.code(),
            Some(expected),
            "CLR {name}: {}",
            String::from_utf8_lossy(&clr.stderr)
        );
        let (vm_exit, vm_stdout) = harness.run_cli(&dll);
        assert_eq!(vm_exit, expected as u8, "VM {name} exit code");
        assert_eq!(vm_stdout.as_bytes(), clr.stdout, "{name} stdout");
    }
    fs::remove_dir_all(dir).expect("remove fixture directory");
}

diff_test!(
    threading_sleep_yield_42_matches_dotnet,
    "tests/fixtures/threading/thread_sleep_yield_42.cs"
);
diff_test!(
    threading_current_identity_42_matches_dotnet,
    "tests/fixtures/threading/thread_current_identity_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    threading_identity_start_42_matches_dotnet,
    "tests/fixtures/threading/thread_identity_start_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    threading_cross_start_42_matches_dotnet,
    "tests/fixtures/threading/thread_cross_start_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    threading_retired_creator_42_matches_dotnet,
    "tests/fixtures/threading/thread_retired_creator_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    threading_sleep_gc_42_matches_dotnet,
    "tests/fixtures/threading/thread_sleep_gc_42.cs"
);

diff_test!(
    threading_background_current_42_matches_dotnet,
    "tests/fixtures/threading/thread_background_current_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    threading_background_42_matches_dotnet,
    "tests/fixtures/threading/thread_background_42.cs"
);

diff_test!(
    monitor_static_flags_single_matches_dotnet,
    "tests/fixtures/threading/monitor_static_flags_single_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    monitor_static_flags_contended_matches_dotnet,
    "tests/fixtures/threading/monitor_static_flags_42.cs"
);

diff_test!(
    threading_monitor_wait_single_matches_dotnet,
    "tests/fixtures/threading/monitor_wait_single_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    threading_monitor_wait_threads_matches_dotnet,
    "tests/fixtures/threading/monitor_wait_threads_42.cs"
);

diff_test!(
    delegate_sleep_42_matches_dotnet,
    "tests/fixtures/threading/delegate_sleep_42.cs"
);
#[cfg(feature = "multithreading")]
diff_test!(
    delegate_monitor_enter_42_matches_dotnet,
    "tests/fixtures/threading/delegate_monitor_enter_42.cs"
);
diff_test!(
    delegate_substring_chunks_42_matches_dotnet,
    "tests/fixtures/strings/delegate_substring_chunks_42.cs"
);

#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    async_pool_asynclocal_task_run_matches_dotnet,
    "tests/fixtures/async/async_pool_asynclocal_task_run_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    async_pool_asynclocal_yield_matches_dotnet,
    "tests/fixtures/async/async_pool_asynclocal_yield_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    threading_pool_asynclocal_isolation_matches_dotnet,
    "tests/fixtures/threading/threading_pool_asynclocal_isolation_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    threading_pool_asynclocal_safe_matches_dotnet,
    "tests/fixtures/threading/threading_pool_asynclocal_safe_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    threading_pool_asynclocal_unsafe_matches_dotnet,
    "tests/fixtures/threading/threading_pool_asynclocal_unsafe_42.cs"
);

diff_test!(
    threading_event_basic_matches_dotnet,
    "tests/fixtures/threading/event_basic_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    threading_event_workers_matches_dotnet,
    "tests/fixtures/threading/event_workers_42.cs"
);

#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    async_task_delay_elapsed_matches_dotnet,
    "tests/fixtures/async/async_task_delay_elapsed_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    async_task_delay_cancel_after_matches_dotnet,
    "tests/fixtures/async/async_task_delay_cancel_after_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    async_task_when_any_timeout_matches_dotnet,
    "tests/fixtures/async/async_task_when_any_timeout_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    threading_timer_callback_pool_thread_matches_dotnet,
    "tests/fixtures/threading/threading_timer_callback_pool_thread_42.cs"
);
#[cfg(all(feature = "multithreading", not(feature = "fuzzing")))]
diff_test!(
    threading_timer_idle_background_exit_matches_dotnet,
    "tests/fixtures/threading/threading_timer_idle_background_exit_42.cs"
);

// The VM can construct a Transient pointer from an eval-stack struct, but the CLR rejects
// this candidate before Monitor receives it. Reflection.Emit supplies the handwritten IL.
#[cfg(not(feature = "fuzzing"))]
#[test]
fn monitor_transient_stack_value_il_is_rejected_by_clr() {
    use crate::integration_tests_impl::harness::TestHarness;
    let dll = TestHarness::get().ensure_dll(std::path::Path::new(
        "tests/debug_fixtures/monitor_transient_il_42.cs",
    ));
    let output = std::process::Command::new("dotnet")
        .arg(dll)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(42));
    assert_eq!(output.stdout, b"rejected\n");
}

diff_test!(
    jagged_interface_cast_matches_dotnet,
    "tests/fixtures/arrays/jagged_interface_cast_42.cs"
);

diff_test!(
    type_names_match_dotnet,
    "tests/fixtures/reflection/type_names_42.cs"
);

diff_test!(
    is_byref_like_matches_dotnet,
    "tests/fixtures/reflection/is_byref_like_42.cs"
);
