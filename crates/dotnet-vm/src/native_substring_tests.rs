//! Executor controls for the traced, finite substring activation.

use super::*;
use crate::{
    StepResult,
    stack::{BasePointer, ops::EvalStackOps},
    test_utils::{parameterless_i32_signature, test_gc_handle},
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_intrinsics_string::IntrinsicStringHost;
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandleType;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
};
use dotnetdll::{prelude::*, resolved::members::Method};

const CHUNK: usize = 64 * 1024;
#[cfg(not(feature = "multithreading"))]
const LENGTH: usize = CHUNK * 2 + 17;

fn executor() -> Executor {
    let loader = AssemblyLoader::new_bare("native_substring_test".to_string()).unwrap();
    let (mut res, _, type_idx) =
        make_test_assembly!("NativeSubstring.dll", "NativeSubstring", "Caller");
    let method_idx = res.push_method(type_idx, Method::new(
        Accessibility::Public,
        parameterless_i32_signature(),
        "Caller",
        Some(make_test_method!(max_stack: 3, instructions: vec![Instruction::LoadConstantInt32(0), Instruction::Return])),
    ));
    let res = loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res.clone(), type_idx);
    let index = ty
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &res.definition()[method_idx]))
        .unwrap();
    let method = MethodDescription::new(
        ty,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(index),
    );
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "single-threaded executor fixture"
    )]
    let shared = Arc::new(SharedGlobalState::new(Arc::new(loader)));
    let info = shared
        .caches
        .get_method_info(method, &Default::default(), shared.clone())
        .unwrap();
    let mut executor = Executor::new(shared);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                engine.stack.execution.frame_stack.push_frame(
                    BasePointer::default(),
                    info,
                    Default::default(),
                    Default::default(),
                );
            })
        })
        .unwrap();
    executor
}

fn payload_len(length: usize) -> Vec<u16> {
    (0..length + 4).map(|i| (i % 65536) as u16).collect()
}

#[cfg(not(feature = "multithreading"))]
fn payload() -> Vec<u16> {
    payload_len(LENGTH)
}

#[cfg(not(feature = "multithreading"))]
fn offer(executor: &mut Executor, eligible: bool) -> Option<StepResult> {
    offer_len(executor, eligible, LENGTH)
}

fn offer_len(executor: &mut Executor, eligible: bool, length: usize) -> Option<StepResult> {
    timed_offer_len(executor, eligible, length).0
}

fn timed_offer_len(
    executor: &mut Executor,
    eligible: bool,
    length: usize,
) -> (Option<StepResult>, std::time::Duration) {
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root throughout this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source =
                    ObjectRef::new(gc, HeapStorage::Str(CLRString::new(payload_len(length))));
                engine
                    .stack
                    .local
                    .heap
                    .gchandles
                    .borrow_mut()
                    .push(Some((source, GCHandleType::Weak)));
                engine.stack.execution.call_state.substring_eligible = eligible;
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(2));
                ctx.push(StackValue::Int32(length as i32));
                let start = std::time::Instant::now();
                let result = ctx.offer_substring_continuation(source, 2, length, 3);
                (result, start.elapsed())
            })
        })
        .unwrap()
}

#[cfg(not(feature = "multithreading"))]
fn snapshot(executor: &mut Executor) -> (usize, usize, usize) {
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let work = engine
                    .stack
                    .execution
                    .call_state
                    .substring
                    .as_ref()
                    .unwrap();
                (
                    work.offset,
                    work.buffer.len(),
                    work.buffer.as_ptr() as usize,
                )
            })
        })
        .unwrap()
}

#[cfg(not(feature = "multithreading"))]
fn collect(executor: &mut Executor) {
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| engine.stack.local.heap.needs_full_collect.set(true))
        })
        .unwrap();
    assert_eq!(executor.collection_check().unwrap(), (true, true));
    executor.perform_full_gc().unwrap();
}

#[cfg(not(feature = "multithreading"))]
fn resume(executor: &mut Executor) -> StepResult {
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root throughout this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.resume_native_substring(gc)
            })
        })
        .unwrap()
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn rooted_progress_survives_real_collections_and_completes_once() {
    let mut executor = executor();
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    let initial = snapshot(&mut executor);
    assert_eq!((initial.0, initial.1), (CHUNK, CHUNK));
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Yield);
    let middle = snapshot(&mut executor);
    assert_eq!(
        (middle.0, middle.1, middle.2),
        (CHUNK * 2, CHUNK * 2, initial.2)
    );
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let execution = &engine.stack.execution;
                assert!(execution.call_state.substring.is_none());
                assert_eq!(execution.frame_stack.current_frame().state.ip, 1);
                assert_eq!(execution.evaluation_stack.stack.len(), 1);
                let StackValue::ObjectRef(result) = &execution.evaluation_stack.stack[0] else {
                    panic!("expected string")
                };
                let handle = result.0.unwrap();
                let inner = handle.borrow();
                let HeapStorage::Str(result) = &inner.storage else {
                    panic!("expected string")
                };
                assert_eq!(&result[..], &payload()[2..2 + LENGTH]);
            })
        })
        .unwrap();
    collect(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let handles = engine.stack.local.heap.gchandles.borrow();
                assert!(
                    handles[0].as_ref().unwrap().0.0.is_none(),
                    "source should no longer be rooted"
                );
            })
        })
        .unwrap();
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn declining_offer_preserves_operands() {
    let mut executor = executor();
    assert_eq!(offer(&mut executor, false), None);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.substring.is_none());
                assert_eq!(engine.stack.execution.evaluation_stack.stack.len(), 3);
            })
        })
        .unwrap();
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn changed_call_site_clears_activation() {
    let mut executor = executor();
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame_mut()
                    .state
                    .ip += 1
            })
        })
        .unwrap();
    assert!(matches!(resume(&mut executor), StepResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| assert!(engine.stack.execution.call_state.substring.is_none()))
        })
        .unwrap();
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn abort_clears_parked_activation_without_more_copying() {
    let mut executor = executor();
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    assert_eq!(snapshot(&mut executor).0, CHUNK);
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.substring.is_none());
                assert!(engine.stack.execution.evaluation_stack.stack.is_empty());
                assert_eq!(
                    engine.stack.execution.frame_stack.current_frame().state.ip,
                    0
                );
            })
        })
        .unwrap();
    // The executor skips collection once aborted; drive its collection primitives directly.
    executor
        .with_arena(|arena| {
            if let Some(marked) = arena.finish_marking() {
                crate::gc::finalize_arena(marked);
            }
            arena.finish_cycle();
            arena.mutate(|_, engine| {
                assert!(
                    engine.stack.local.heap.gchandles.borrow()[0]
                        .as_ref()
                        .unwrap()
                        .0
                        .0
                        .is_none()
                );
            });
        })
        .unwrap();
}

#[cfg(feature = "fuzzing")]
#[cfg(not(feature = "multithreading"))]
#[test]
fn exhausted_budget_clears_parked_activation() {
    let mut executor = executor();
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    executor.instruction_budget = Some(0);
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.substring.is_none());
                assert!(engine.stack.execution.evaluation_stack.stack.is_empty());
                assert_eq!(
                    engine.stack.execution.frame_stack.current_frame().state.ip,
                    0
                );
            })
        })
        .unwrap();
    collect(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(
                    engine.stack.local.heap.gchandles.borrow()[0]
                        .as_ref()
                        .unwrap()
                        .0
                        .0
                        .is_none()
                );
            })
        })
        .unwrap();
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn incomplete_chunks_yield_without_collection_pressure() {
    let mut executor = executor();
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    assert_eq!(snapshot(&mut executor).0, CHUNK);
    assert_eq!(resume(&mut executor), StepResult::Yield);
    assert_eq!(snapshot(&mut executor).0, CHUNK * 2);
    assert_eq!(resume(&mut executor), StepResult::Continue);
}

/// Small real-arena control shared by the pinned Miri configurations.
#[test]
pub(crate) fn native_substring_gc_control() {
    let mut executor = executor();
    let result = offer_len(&mut executor, true, CHUNK + 1);
    #[cfg(not(feature = "multithreading"))]
    {
        assert_eq!(result, Some(StepResult::Yield));
        collect(&mut executor);
        assert_eq!(resume(&mut executor), StepResult::Continue);
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let values = &engine.stack.execution.evaluation_stack.stack;
                    assert_eq!(values.len(), 1);
                    let StackValue::ObjectRef(result) = &values[0] else {
                        panic!("expected string")
                    };
                    let handle = result.0.unwrap();
                    let inner = handle.borrow();
                    let HeapStorage::Str(result) = &inner.storage else {
                        panic!("expected string")
                    };
                    assert_eq!(&result[..], &payload_len(CHUNK + 1)[2..CHUNK + 3]);
                })
            })
            .unwrap();
    }
    #[cfg(feature = "multithreading")]
    {
        assert_eq!(result, None);
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    assert!(engine.stack.execution.call_state.substring.is_none());
                    assert_eq!(engine.stack.execution.evaluation_stack.stack.len(), 3);
                })
            })
            .unwrap();
    }
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn pending_finalizer_waits_until_copy_completes() {
    use crate::stack::ops::VesOps;
    let mut executor = executor();
    // A minimal corlib type lets the production virtual-finalizer resolver run.
    let mut res = Resolution::new(Module::new("mscorlib.dll"));
    res.assembly = Some(Assembly::new("mscorlib"));
    let mut object = TypeDefinition::new(None, "Object");
    object.namespace = Some("System".into());
    let ty_index = res.push_type_definition(object);
    for name in ["Enum", "ValueType"] {
        let mut stub = TypeDefinition::new(None, name);
        stub.namespace = Some("System".into());
        res.push_type_definition(stub);
    }
    let mut signature = parameterless_i32_signature();
    signature.instance = true;
    signature.return_type = dotnetdll::resolved::signature::ReturnType(vec![], None);
    let mut finalizer = Method::new(
        Accessibility::Public,
        signature,
        "Finalize",
        Some(make_test_method!(max_stack: 1, instructions: vec![
            Instruction::LoadConstantInt32(42), Instruction::Pop, Instruction::Return
        ])),
    );
    finalizer.virtual_member = true;
    res.push_method(ty_index, finalizer);
    let res = executor.shared.loader.register_owned_assembly(res);
    let finalizable_type = TypeDescription::new(res, ty_index);
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let layout = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 0,
                    alignment: 1,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                };
                let queued = ObjectRef::new(
                    gc,
                    HeapStorage::Obj(Box::new(dotnet_value::object::Object::new(
                        finalizable_type,
                        GenericLookup::default(),
                        dotnet_value::storage::FieldStorage::new(Arc::new(layout), vec![]),
                    ))),
                );
                engine
                    .stack
                    .local
                    .heap
                    .pending_finalization
                    .borrow_mut()
                    .push(queued);
                assert_eq!(
                    engine.ves_context(gc).process_pending_finalizers(),
                    StepResult::Continue
                );
                assert_eq!(
                    engine.stack.local.heap.pending_finalization.borrow().len(),
                    1
                );
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert_eq!(
                    engine.stack.execution.frame_stack.current_frame().state.ip,
                    0
                );
            })
        })
        .unwrap();
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Yield);
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert_eq!(
                    engine.ves_context(gc).process_pending_finalizers(),
                    StepResult::FramePushed
                );
                assert_eq!(engine.stack.execution.frame_stack.len(), 2);
                assert!(
                    engine
                        .stack
                        .execution
                        .frame_stack
                        .current_frame()
                        .is_finalizer
                );
                // Run the managed body, observing its value before it is popped.
                assert_eq!(engine.step(gc), StepResult::Continue);
                assert!(matches!(
                    engine.stack.execution.evaluation_stack.stack.last(),
                    Some(StackValue::Int32(42))
                ));
                assert_eq!(engine.step(gc), StepResult::Continue);
                let _ = engine.step(gc);
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert_eq!(
                    engine.stack.execution.frame_stack.current_frame().state.ip,
                    1
                );
                assert_eq!(engine.stack.execution.evaluation_stack.stack.len(), 1);
                assert!(!engine.stack.local.heap.processing_finalizer.get());
                assert!(
                    engine
                        .stack
                        .local
                        .heap
                        .pending_finalization
                        .borrow()
                        .is_empty()
                );
            })
        })
        .unwrap();
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn teardown_releases_parked_arena() {
    use gc_arena::{Collect, Gc};
    use std::sync::{
        Arc as StdArc,
        atomic::{AtomicUsize, Ordering},
    };
    #[derive(Collect)]
    #[collect(require_static)]
    struct DropWitness(StdArc<AtomicUsize>);
    impl Drop for DropWitness {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = StdArc::new(AtomicUsize::new(0));
    let mut executor = executor();
    executor
        .with_arena(|arena| {
            arena.mutate(|gc, _| {
                let _ = Gc::new(gc, DropWitness(drops.clone()));
            })
        })
        .unwrap();
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    drop(executor);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[cfg(not(feature = "multithreading"))]
#[test]
fn dispatch_entry_controls_substring_eligibility() {
    use crate::stack::ops::{UnifiedDispatchTarget, VmCallOps};
    use dotnetdll::{
        binary::signature::kinds::CallingConvention,
        resolved::{
            signature::{Parameter, ParameterType, ReturnType},
            types::TypeDefinition,
        },
    };

    for explicit_length in [false, true] {
        for entry in ["direct", "ordinary", "tail", "engine_api"] {
            let mut executor = executor();
            let mut res = Resolution::new(Module::new("TestString.dll"));
            res.assembly = Some(Assembly::new("TestString"));
            let mut string = TypeDefinition::new(None, "String");
            string.namespace = Some("System".into());
            string.flags.before_field_init = true;
            let ty_index = res.push_type_definition(string);
            let int_type = || ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32)));
            let signature = MethodSignature {
                instance: true,
                explicit_this: false,
                calling_convention: CallingConvention::Default,
                parameters: (0..if explicit_length { 2 } else { 1 })
                    .map(|_| Parameter(vec![], int_type()))
                    .collect(),
                return_type: ReturnType(
                    vec![],
                    Some(ParameterType::Value(MethodType::Base(Box::new(
                        BaseType::String,
                    )))),
                ),
                varargs: None,
            };
            let method_index = res.push_method(
                ty_index,
                Method::new(Accessibility::Public, signature, "Substring", None),
            );
            let res = executor.shared.loader.register_owned_assembly(res);
            let ty = TypeDescription::new(res.clone(), ty_index);
            let index = ty
                .definition()
                .methods
                .iter()
                .position(|m| std::ptr::eq(m, &res.definition()[method_index]))
                .unwrap();
            let method = MethodDescription::new(
                ty,
                GenericLookup::default(),
                res,
                MethodMemberIndex::Method(index),
            );
            #[cfg(feature = "bench-instrumentation")]
            dotnet_intrinsics_string::search::substring_metrics::reset();
            let result = executor
                .with_arena(|arena| {
                    arena.mutate_root(|gc, engine| {
                        // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                        let gc = unsafe { test_gc_handle(gc, engine) };
                        let source = ObjectRef::new(
                            gc,
                            HeapStorage::Str(CLRString::new(payload()[..LENGTH + 2].to_vec())),
                        );
                        let mut ctx = engine.ves_context(gc);
                        ctx.push(StackValue::ObjectRef(source));
                        ctx.push(StackValue::Int32(2));
                        if explicit_length {
                            ctx.push(StackValue::Int32(LENGTH as i32));
                        }
                        if entry == "engine_api" {
                            // Even stale admission from an outer direct call must be refused.
                            ctx.call_state.substring_eligible = true;
                            ctx.dispatch_resolved_method_for_engine_api(
                                method,
                                GenericLookup::default(),
                            )
                        } else {
                            let target =
                                UnifiedDispatchTarget::Resolved(method, GenericLookup::default());
                            match entry {
                                "direct" => ctx.unified_dispatch_direct(target, None, None),
                                "tail" => ctx.unified_dispatch_tail(target, None, None),
                                _ => ctx.unified_dispatch(target, None, None),
                            }
                        }
                    })
                })
                .unwrap();
            if entry == "direct" {
                assert_eq!(result, StepResult::Yield);
                assert_eq!(snapshot(&mut executor).0, CHUNK);
                assert_eq!(resume(&mut executor), StepResult::Yield);
                assert_eq!(resume(&mut executor), StepResult::Continue);
            } else {
                assert_eq!(result, StepResult::Continue);
            }
            #[cfg(feature = "bench-instrumentation")]
            {
                let measured = dotnet_intrinsics_string::search::substring_metrics::snapshot();
                assert_eq!(measured.copied_units, LENGTH);
                assert_eq!(measured.buffer_allocations, 1);
                assert_eq!(measured.buffer_reallocations, 0);
                assert!(measured.max_buffer_bytes >= LENGTH * size_of::<u16>());
                eprintln!("substring entry={entry} explicit_length={explicit_length} {measured:?}");
            }
            executor
                .with_arena(|arena| {
                    arena.mutate(|_, engine| {
                        assert!(engine.stack.execution.call_state.substring.is_none());
                        assert!(!engine.stack.execution.call_state.substring_eligible);
                        let values = &engine.stack.execution.evaluation_stack.stack;
                        assert_eq!(values.len(), 1);
                        let StackValue::ObjectRef(result) = &values[0] else {
                            panic!("expected string")
                        };
                        let handle = result.0.unwrap();
                        let inner = handle.borrow();
                        let HeapStorage::Str(result) = &inner.storage else {
                            panic!("expected string")
                        };
                        assert_eq!(&result[..], &payload()[2..2 + LENGTH]);
                    })
                })
                .unwrap();
        }
    }
}

/// `calli` currently has no registered opcode handler. Keep its rejection explicit:
/// the generic dispatch control above must not be mistaken for indirect-call support.
#[test]
fn indirect_opcode_refuses_without_consuming_operands() {
    use dotnetdll::binary::signature::kinds::StandAloneCallingConvention;
    for tail_call in [false, true] {
        let mut executor = executor();
        assert_eq!(offer_len(&mut executor, false, CHUNK + 1), None);
        executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let signature = MethodSignature {
                        instance: true,
                        explicit_this: false,
                        calling_convention: StandAloneCallingConvention::DefaultManaged,
                        parameters: vec![],
                        return_type: dotnetdll::resolved::signature::ReturnType(vec![], None),
                        varargs: None,
                    };
                    let instruction = Instruction::CallIndirect {
                        param0: signature,
                        tail_call,
                    };
                    let result = crate::dispatch::InstructionRegistry::dispatch(
                        &mut engine.ves_context(gc),
                        &instruction,
                    );
                    assert!(matches!(
                        result,
                        StepResult::Error(crate::error::VmError::Execution(
                            dotnet_types::error::ExecutionError::NotImplemented(_)
                        ))
                    ));
                    assert!(engine.stack.execution.call_state.substring.is_none());
                    assert_eq!(engine.stack.execution.evaluation_stack.stack.len(), 3);
                    assert_eq!(
                        engine.stack.execution.frame_stack.current_frame().state.ip,
                        0
                    );
                })
            })
            .unwrap();
    }
}

#[cfg(all(feature = "bench-instrumentation", not(feature = "multithreading")))]
#[test]
fn measured_copy_work_survives_collection_and_stops_on_abort() {
    use dotnet_intrinsics_string::search::substring_metrics;
    for abort in [false, true] {
        let mut executor = executor();
        substring_metrics::reset();
        assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
        collect(&mut executor);
        if abort {
            executor.shared.abort_signal().request_abort();
            assert!(matches!(executor.run(), ExecutorResult::Error(_)));
        } else {
            assert_eq!(resume(&mut executor), StepResult::Yield);
            collect(&mut executor);
            assert_eq!(resume(&mut executor), StepResult::Continue);
        }
        let measured = substring_metrics::snapshot();
        assert_eq!(measured.copied_units, if abort { CHUNK } else { LENGTH });
        assert_eq!(measured.buffer_allocations, 1);
        assert_eq!(measured.buffer_reallocations, 0);
        eprintln!("substring forced_gc abort={abort} {measured:?}");
    }
}

/// Whole native copy latency, from admission through result publication. Input and
/// caller construction are excluded equally; collection time is included when requested.
#[cfg(not(feature = "multithreading"))]
#[test]
#[ignore = "manual timing"]
fn measure_whole_copy_with_and_without_collection() {
    for forced_gc in [false, true] {
        for sample in 0..30 {
            let mut executor = executor();
            let (result, initial) = timed_offer_len(&mut executor, true, LENGTH);
            assert_eq!(result, Some(StepResult::Yield));
            let start = std::time::Instant::now();
            if forced_gc {
                collect(&mut executor);
            }
            assert_eq!(resume(&mut executor), StepResult::Yield);
            if forced_gc {
                collect(&mut executor);
            }
            assert_eq!(resume(&mut executor), StepResult::Continue);
            let elapsed = initial + start.elapsed();
            eprintln!(
                "whole_native_copy forced_gc={forced_gc} sample={sample} ns={} units={LENGTH}",
                elapsed.as_nanos()
            );
        }
    }
}

// Manual timing of real forced-GC parked suffix latency, excluding setup and the initial chunk.
// This is not comparable to a whole-call throughput baseline.
#[cfg(not(feature = "multithreading"))]
#[test]
#[ignore = "manual timing"]
fn measure_forced_collection_copy() {
    use std::{mem::size_of, time::Instant};

    let mut samples = Vec::with_capacity(30);
    for sample in 0..30 {
        let mut executor = executor();
        assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
        let (initial_offset, initial_len, initial_ptr, capacity) = executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let work = engine
                        .stack
                        .execution
                        .call_state
                        .substring
                        .as_ref()
                        .unwrap();
                    (
                        work.offset,
                        work.buffer.len(),
                        work.buffer.as_ptr() as usize,
                        work.buffer.capacity(),
                    )
                })
            })
            .unwrap();
        let retained_bytes = capacity * size_of::<u16>()
            + size_of::<crate::stack::context::NativeSubstringActivation<'_>>();
        let start = Instant::now();
        collect(&mut executor);
        let middle_result = resume(&mut executor);
        let middle = snapshot(&mut executor);
        collect(&mut executor);
        let final_result = resume(&mut executor);
        let elapsed = start.elapsed().as_nanos();

        assert_eq!((initial_offset, initial_len), (CHUNK, CHUNK));
        assert_eq!(middle_result, StepResult::Yield);
        assert_eq!(middle, (CHUNK * 2, CHUNK * 2, initial_ptr));
        assert_eq!(final_result, StepResult::Continue);
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let execution = &engine.stack.execution;
                    assert!(execution.call_state.substring.is_none());
                    assert_eq!(execution.frame_stack.current_frame().state.ip, 1);
                    assert_eq!(execution.evaluation_stack.stack.len(), 1);
                    let StackValue::ObjectRef(result) = &execution.evaluation_stack.stack[0] else {
                        panic!("expected string")
                    };
                    let handle = result.0.unwrap();
                    let inner = handle.borrow();
                    let HeapStorage::Str(result) = &inner.storage else {
                        panic!("expected string")
                    };
                    assert_eq!(&result[..], &payload()[2..2 + LENGTH]);
                    assert!(
                        engine.stack.local.heap.gchandles.borrow()[0]
                            .as_ref()
                            .unwrap()
                            .0
                            .0
                            .is_some()
                    );
                })
            })
            .unwrap();
        eprintln!(
            "forced_gc_parked_suffix sample={sample} ns={elapsed} suffix_units={} collections=2 peak_native_owned_bytes={retained_bytes} rooted_sources=1",
            LENGTH - CHUNK
        );
        samples.push(elapsed);
    }
    samples.sort_unstable();
    let median = (samples[14] + samples[15]) / 2;
    eprintln!(
        "forced_gc_parked_suffix median_ns={median} samples=30 suffix_units={} collections=2 setup_and_initial_chunk_excluded",
        LENGTH - CHUNK
    );
}
