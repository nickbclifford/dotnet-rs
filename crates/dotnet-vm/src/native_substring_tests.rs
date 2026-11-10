//! Executor controls for the async, finite substring copy.

use super::*;
use crate::{
    StepResult,
    stack::{BasePointer, ops::EvalStackOps},
    test_utils::{parameterless_i32_signature, test_gc_handle},
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_utils::gc::GCHandleType;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
};
use dotnet_vm_data::ExceptionState;
use dotnetdll::{prelude::*, resolved::members::Method};

const CHUNK: usize = 64 * 1024;
const LENGTH: usize = CHUNK * 2 + 17;

fn executor() -> Executor {
    let loader = AssemblyLoader::new_bare("native_substring_test".to_string()).unwrap();
    let (mut res, _, type_idx) =
        make_test_assembly!("NativeSubstring.dll", "NativeSubstring", "Caller");
    res[type_idx].flags.before_field_init = true;
    let method_idx = res.push_method(type_idx, Method::new(
        Accessibility::Public,
        parameterless_i32_signature(),
        "Caller",
        Some(make_test_method!(max_stack: 3, instructions: vec![Instruction::LoadConstantInt32(0), Instruction::Return])),
    ));
    let mut echo_signature = parameterless_i32_signature();
    let string_type = || ParameterType::Value(MethodType::Base(Box::new(BaseType::String)));
    echo_signature
        .parameters
        .push(Parameter(vec![], string_type()));
    echo_signature.return_type = ReturnType(vec![], Some(string_type()));
    res.push_method(type_idx, Method::new(
        Accessibility::Public,
        echo_signature,
        "Echo",
        Some(make_test_method!(max_stack: 3, instructions: vec![Instruction::LoadArgument(0), Instruction::Return])),
    ));
    let mut byref_signature = parameterless_i32_signature();
    byref_signature.parameters.push(Parameter(
        vec![],
        ParameterType::Ref(MethodType::Base(Box::new(BaseType::Int32))),
    ));
    res.push_method(type_idx, Method::new(
        Accessibility::Public,
        byref_signature,
        "ReadByRef",
        Some(make_test_method!(max_stack: 1, instructions: vec![Instruction::LoadConstantInt32(0), Instruction::Return])),
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

fn payload() -> Vec<u16> {
    payload_len(LENGTH)
}

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
    let method = substring_method(executor, true);
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
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(2));
                ctx.push(StackValue::Int32(length as i32));
                let start = std::time::Instant::now();
                let result = eligible.then(|| {
                    use crate::stack::ops::{UnifiedDispatchTarget, VmCallOps};
                    ctx.unified_dispatch(
                        UnifiedDispatchTarget::Resolved(method, GenericLookup::default()),
                        None,
                        None,
                    )
                });
                (result, start.elapsed())
            })
        })
        .unwrap()
}

fn collect(executor: &mut Executor) {
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| engine.stack.local.heap.needs_full_collect.set(true))
        })
        .unwrap();
    assert_eq!(executor.collection_check().unwrap(), (true, true));
    executor.perform_full_gc().unwrap();
}

fn resume(executor: &mut Executor) -> StepResult {
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root throughout this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.step(gc)
            })
        })
        .unwrap()
}

#[test]
fn rooted_progress_survives_real_collections_and_completes_once() {
    let mut executor = executor();
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Yield);
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let execution = &engine.stack.execution;
                assert!(execution.call_state.native.is_empty());
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

#[test]
fn unsupported_dispatch_preserves_operands_before_fallback() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    assert_eq!(offer_len(&mut executor, false, LENGTH), None);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let before = ctx.evaluation_stack.stack.clone();
                let result = ctx.start_native(
                    method,
                    NativeDispatchMode::Other,
                    &GenericLookup::default(),
                    Some(native_test_reject_selector),
                    native_test_fallback,
                    native_test_body,
                );
                assert_eq!(ctx.evaluation_stack.stack, before);
                result
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert_eq!(engine.stack.execution.evaluation_stack.stack.len(), 3);
            })
        })
        .unwrap();
}

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
            arena.mutate(|_, engine| assert!(engine.stack.execution.call_state.native.is_empty()))
        })
        .unwrap();
}

#[test]
fn abort_clears_parked_activation_without_more_copying() {
    let mut executor = executor();
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
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
#[test]
fn exhausted_budget_clears_parked_activation() {
    let mut executor = executor();
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    executor.instruction_budget = Some(0);
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
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

#[test]
fn incomplete_chunks_yield_without_collection_pressure() {
    let mut executor = executor();
    assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
    assert_eq!(resume(&mut executor), StepResult::Yield);
    assert_eq!(resume(&mut executor), StepResult::Continue);
}

/// Small real-arena control shared by the pinned Miri configurations.
#[test]
pub(crate) fn native_substring_gc_control() {
    let mut executor = executor();
    let result = offer_len(&mut executor, true, CHUNK + 1);
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
}

#[test]
fn pending_finalizer_runs_in_separate_segment_during_copy() {
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
                    engine.process_pending_finalizers(gc),
                    StepResult::FramePushed
                );
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert!(
                    engine
                        .stack
                        .execution
                        .frame_stack
                        .current_frame()
                        .is_finalizer
                );
                let caller = engine.stack.parked_finalizer_caller.as_ref().unwrap();
                assert_eq!(caller.frame_stack.len(), 1);
                assert_eq!(caller.frame_stack.current_frame().state.ip, 0);
                assert_eq!(caller.call_state.native.len(), 1);
            })
        })
        .unwrap();
    collect(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert_eq!(engine.step(gc), StepResult::Continue);
                assert!(matches!(
                    engine.stack.execution.evaluation_stack.stack.last(),
                    Some(StackValue::Int32(42))
                ));
                assert_eq!(engine.step(gc), StepResult::Continue);
                assert_eq!(engine.step(gc), StepResult::Continue);
                assert!(engine.stack.parked_finalizer_caller.is_none());
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert_eq!(
                    engine.stack.execution.frame_stack.current_frame().state.ip,
                    0
                );
                assert!(!engine.stack.local.heap.processing_finalizer.get());
            })
        })
        .unwrap();
    assert_eq!(resume(&mut executor), StepResult::Yield);
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Continue);
}

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

fn substring_method(executor: &Executor, explicit_length: bool) -> MethodDescription {
    use dotnetdll::{
        binary::signature::kinds::CallingConvention,
        resolved::{
            signature::{Parameter, ParameterType, ReturnType},
            types::TypeDefinition,
        },
    };
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
    MethodDescription::new(
        ty,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(index),
    )
}

#[test]
fn substring_selector_preserves_operands_and_selects_only_large_valid_ranges() {
    use crate::stack::VesContext;
    use dotnet_intrinsics_string::search::substring_select;
    use dotnet_vm_ops::effects::{NativeOperandView, NativeSelection};

    for explicit_length in [false, true] {
        let mut executor = executor();
        let method = substring_method(&executor, explicit_length);
        let cases = if explicit_length {
            vec![
                (0, Some(0), None),
                (0, Some(CHUNK as i32), None),
                (1, Some((CHUNK + 1) as i32), Some((1, CHUNK + 1))),
                (-1, Some(-1), None),
                (0, Some(-1), None),
                (2, Some(i32::MAX), None),
                (i32::MAX, Some(i32::MAX), None),
            ]
        } else {
            vec![
                ((CHUNK + 2) as i32, None, None),
                (2, None, None),
                (1, None, Some((1, CHUNK + 1))),
                (-1, None, None),
                (i32::MAX, None, None),
            ]
        };
        executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let source =
                        ObjectRef::new(gc, HeapStorage::Str(CLRString::new(vec![0x61; CHUNK + 2])));
                    let mut ctx = engine.ves_context(gc);
                    for (start, length, expected) in cases {
                        ctx.push(StackValue::ObjectRef(source));
                        ctx.push(StackValue::Int32(start));
                        if let Some(length) = length {
                            ctx.push(StackValue::Int32(length));
                        }
                        let before = ctx.evaluation_stack.stack.clone();
                        let selected = substring_select::<VesContext<'_, '_>>(
                            &ctx,
                            NativeOperandView::new(&ctx),
                            &method,
                            &GenericLookup::default(),
                        );
                        match (selected, expected) {
                            (NativeSelection::Async(invocation), Some((start, length))) => {
                                assert_eq!((invocation.start, invocation.length), (start, length));
                                assert_eq!(
                                    invocation.parameter_count,
                                    if explicit_length { 2 } else { 1 }
                                );
                            }
                            (NativeSelection::Synchronous(_), None) => {}
                            _ => panic!("unexpected substring selection"),
                        }
                        assert_eq!(ctx.evaluation_stack.stack, before);
                        ctx.drop_top(if explicit_length { 3 } else { 2 });
                    }
                    for receiver in [StackValue::ObjectRef(ObjectRef(None)), StackValue::Int32(9)] {
                        ctx.push(receiver);
                        ctx.push(StackValue::Int32(0));
                        if explicit_length {
                            ctx.push(StackValue::Int32((CHUNK + 1) as i32));
                        }
                        let before = ctx.evaluation_stack.stack.clone();
                        assert!(matches!(
                            substring_select::<VesContext<'_, '_>>(
                                &ctx,
                                NativeOperandView::new(&ctx),
                                &method,
                                &GenericLookup::default(),
                            ),
                            NativeSelection::Synchronous(_)
                        ));
                        assert_eq!(ctx.evaluation_stack.stack, before);
                        ctx.drop_top(if explicit_length { 3 } else { 2 });
                    }
                })
            })
            .unwrap();
    }
}

#[test]
fn substring_dispatch_admits_only_inline_calls() {
    use crate::stack::ops::{UnifiedDispatchTarget, VmCallOps};
    for explicit_length in [false, true] {
        for entry in ["inline", "tail", "engine_api"] {
            let mut executor = executor();
            let method = substring_method(&executor, explicit_length);
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
                        match entry {
                            "inline" => ctx.unified_dispatch(
                                UnifiedDispatchTarget::Resolved(method, GenericLookup::default()),
                                None,
                                None,
                            ),
                            "tail" => ctx.unified_dispatch_tail(
                                UnifiedDispatchTarget::Resolved(method, GenericLookup::default()),
                                None,
                                None,
                            ),
                            _ => ctx.dispatch_resolved_method_for_engine_api(
                                method,
                                GenericLookup::default(),
                            ),
                        }
                    })
                })
                .unwrap();
            if entry == "inline" {
                assert_eq!(result, StepResult::Yield);
                assert_eq!(resume(&mut executor), StepResult::Yield);
                assert_eq!(resume(&mut executor), StepResult::Continue);
            } else {
                assert_eq!(result, StepResult::Continue);
            }
            executor
                .with_arena(|arena| {
                    arena.mutate(|_, engine| {
                        assert!(engine.stack.execution.call_state.native.is_empty());
                        let [StackValue::ObjectRef(ObjectRef(Some(result)))] =
                            engine.stack.execution.evaluation_stack.stack.as_slice()
                        else {
                            panic!("expected string")
                        };
                        let inner = result.borrow();
                        let HeapStorage::Str(chars) = &inner.storage else {
                            panic!("expected string")
                        };
                        assert_eq!(&chars[..], &payload()[2..2 + LENGTH]);
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
                    assert!(engine.stack.execution.call_state.native.is_empty());
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

#[cfg(feature = "bench-instrumentation")]
#[test]
fn measured_copy_work_survives_collection_and_stops_on_abort() {
    use dotnet_intrinsics_string::search::substring_metrics;
    for abort in [false, true] {
        let mut executor = executor();
        substring_metrics::reset();
        assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
        assert_eq!(substring_metrics::snapshot().copied_units, CHUNK);
        collect(&mut executor);
        if abort {
            executor.shared.abort_signal().request_abort();
            assert!(matches!(executor.run(), ExecutorResult::Error(_)));
        } else {
            assert_eq!(resume(&mut executor), StepResult::Yield);
            assert_eq!(substring_metrics::snapshot().copied_units, CHUNK * 2);
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

// Manual timing excludes setup and the first copy chunk.
#[test]
#[ignore = "manual timing"]
fn measure_forced_collection_copy() {
    for sample in 0..30 {
        let mut executor = executor();
        assert_eq!(offer(&mut executor, true), Some(StepResult::Yield));
        let start = std::time::Instant::now();
        collect(&mut executor);
        assert_eq!(resume(&mut executor), StepResult::Yield);
        collect(&mut executor);
        assert_eq!(resume(&mut executor), StepResult::Continue);
        eprintln!(
            "forced_gc_parked_suffix sample={sample} ns={}",
            start.elapsed().as_nanos()
        );
    }
}

#[cfg(feature = "multithreading")]
#[test]
fn foreign_source_survives_coordinated_collections_between_chunks() {
    use crate::{
        gc::coordinator::GcCycleGuard,
        stack::ops::{UnifiedDispatchTarget, VmCallOps},
        test_utils::{
            foreign_reference::{decode_reference, encode_reference},
            worker::Worker,
        },
        threading::ThreadManagerOps,
    };
    use std::{sync::mpsc, thread, time::Duration};

    let mut executor = executor();
    let method = substring_method(&executor, true);
    let state = executor.shared.clone();
    let owner_state = state.clone();
    let (send, receive) = mpsc::channel();
    let (command, commands) = mpsc::channel::<()>();
    let (witness, observations) = mpsc::channel();
    let owner = thread::spawn(move || {
        let mut owner = Executor::new(owner_state.clone());
        let id = owner.thread_id;
        let storage = owner
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::new(payload())));
                    engine
                        .stack
                        .local
                        .heap
                        .gchandles
                        .borrow_mut()
                        .push(Some((source, GCHandleType::Weak)));
                    encode_reference(source)
                })
            })
            .unwrap();
        send.send((id, storage)).unwrap();
        loop {
            owner_state
                .thread_manager
                .safe_point(id, &owner_state.gc_coordinator);
            match commands.recv_timeout(Duration::from_millis(1)) {
                Ok(()) => {
                    let alive = owner
                        .with_arena(|arena| {
                            arena.mutate(|_, engine| {
                                engine.stack.local.heap.gchandles.borrow()[0]
                                    .as_ref()
                                    .unwrap()
                                    .0
                                    .0
                                    .is_some()
                            })
                        })
                        .unwrap();
                    witness.send(alive).unwrap();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    let owner = Worker::new(owner, command);
    let (owner_id, storage) = receive.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_ne!(owner_id, executor.thread_id);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = decode_reference(storage, gc);
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(2));
                ctx.push(StackValue::Int32(LENGTH as i32));
                ctx.unified_dispatch(
                    UnifiedDispatchTarget::Resolved(method, GenericLookup::default()),
                    None,
                    None,
                )
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Yield);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let work = engine.stack.execution.call_state.native.last().unwrap();
                assert!(work.operands.is_empty());
                assert!(work.outputs.is_empty());
                assert!(engine.stack.execution.evaluation_stack.stack.is_empty());
            })
        })
        .unwrap();
    for turn in 0..2 {
        let coordinator = state.gc_coordinator.clone();
        let session = coordinator.begin_collection().expect("idle coordinator");
        let stop = state.thread_manager.request_stop_the_world();
        let cycle = GcCycleGuard::new(session, stop);
        cycle.collect_all_arenas(executor.thread_id);
        drop(cycle);
        owner.resources().send(()).unwrap();
        let alive = observations.recv_timeout(Duration::from_secs(30)).unwrap();
        if !alive {
            executor
                .with_arena(|arena| {
                    arena.mutate_root(|_, engine| {
                        if let Some(native) = engine.stack.execution.call_state.native.pop() {
                            native.cleanup();
                        }
                    })
                })
                .unwrap();
            panic!("foreign source was not traced");
        }
        assert_eq!(
            resume(&mut executor),
            if turn == 0 {
                StepResult::Yield
            } else {
                StepResult::Continue
            }
        );
    }
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let [StackValue::ObjectRef(ObjectRef(Some(result)))] =
                    engine.stack.execution.evaluation_stack.stack.as_slice()
                else {
                    panic!("expected string")
                };
                let inner = result.borrow();
                let HeapStorage::Str(chars) = &inner.storage else {
                    panic!("expected string")
                };
                assert_eq!(&chars[..], &payload()[2..2 + LENGTH]);
            })
        })
        .unwrap();
    owner.finish().unwrap();
}

#[cfg(feature = "multithreading")]
fn native_foreign_byref_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let target = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let (_, mut arguments) = {
            let mut vm = fx.vm().await?;
            vm.prepare_multicast()?
        };
        let pointer = arguments.remove(0);
        drop(arguments);
        fx.yield_now().await?;
        let result = fx
            .call(dotnet_vm_ops::effects::NativeCall {
                method: target,
                generics: GenericLookup::default(),
                arguments: vec![pointer],
            })
            .await?;
        assert!(matches!(
            result,
            dotnet_vm_ops::effects::NativeValue::Value(_)
        ));
        let mut vm = fx.vm().await?;
        vm.push_new_string(vec![65])?;
        Ok(())
    })
}

#[cfg(feature = "multithreading")]
#[test]
fn native_foreign_byref_owner_survives_collection_and_child_call() {
    foreign_byref_owner_survives_collection_and_child_call(false);
}

#[cfg(feature = "multithreading")]
#[test]
fn native_cross_arena_byref_owner_survives_collection_and_child_call() {
    foreign_byref_owner_survives_collection_and_child_call(true);
}

#[cfg(feature = "multithreading")]
fn foreign_byref_owner_survives_collection_and_child_call(cross_origin: bool) {
    use crate::{
        gc::coordinator::GcCycleGuard,
        test_utils::{
            foreign_reference::{decode_reference, encode_reference},
            worker::Worker,
        },
        threading::ThreadManagerOps,
    };
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    use std::{sync::mpsc, thread, time::Duration};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    let state = executor.shared.clone();
    let owner_state = state.clone();
    let (send, receive) = mpsc::channel();
    let (command, commands) = mpsc::channel::<()>();
    let (witness, observations) = mpsc::channel();
    let owner = thread::spawn(move || {
        let mut owner = Executor::new(owner_state.clone());
        let id = owner.thread_id;
        let storage = owner
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("foreign")));
                    engine
                        .stack
                        .local
                        .heap
                        .gchandles
                        .borrow_mut()
                        .push(Some((source, GCHandleType::Weak)));
                    encode_reference(source)
                })
            })
            .unwrap();
        send.send((id, storage)).unwrap();
        loop {
            owner_state
                .thread_manager
                .safe_point(id, &owner_state.gc_coordinator);
            match commands.recv_timeout(Duration::from_millis(1)) {
                Ok(()) => {
                    let alive = owner
                        .with_arena(|arena| {
                            arena.mutate(|_, engine| {
                                engine.stack.local.heap.gchandles.borrow()[0]
                                    .as_ref()
                                    .unwrap()
                                    .0
                                    .0
                                    .is_some()
                            })
                        })
                        .unwrap();
                    witness.send(alive).unwrap();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    let owner = Worker::new(owner, command);
    let (owner_id, storage) = receive.recv_timeout(Duration::from_secs(30)).unwrap();
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = decode_reference(storage, gc);
                let parent = engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .parent
                    .clone();
                let index = parent
                    .definition()
                    .methods
                    .iter()
                    .position(|method| method.name == "ReadByRef")
                    .unwrap();
                let target = MethodDescription::new(
                    parent.clone(),
                    GenericLookup::default(),
                    parent.resolution.clone(),
                    MethodMemberIndex::Method(index),
                );
                CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
                let pointer = if cross_origin {
                    let lease = dotnet_utils::gc::try_acquire_lease(owner_id).expect("live owner");
                    // SAFETY: F1.ArenaGenerationMatch — the exact owner lease is held. The object is
                    // only weakly held by its owner, but the test requests no collection of the
                    // owner arena before the pointer is placed in the traced activation.
                    unsafe {
                        dotnet_value::ManagedPtr::new_cross_arena(
                            None,
                            TypeDescription::NULL,
                            source.as_ptr().expect("foreign object"),
                            &lease,
                            dotnet_utils::ByteOffset::new(0),
                        )
                    }
                } else {
                    dotnet_value::ManagedPtr::new(
                        None,
                        TypeDescription::NULL,
                        Some(source),
                        false,
                        Some(dotnet_utils::ByteOffset::new(0)),
                    )
                };
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::null());
                ctx.push(StackValue::ManagedPtr(pointer.into()));
                ctx.push(StackValue::Int32(0));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_foreign_byref_body,
                )
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Yield);
    let coordinator = state.gc_coordinator.clone();
    let session = coordinator.begin_collection().expect("idle coordinator");
    let stop = state.thread_manager.request_stop_the_world();
    let cycle = GcCycleGuard::new(session, stop);
    cycle.collect_all_arenas(executor.thread_id);
    drop(cycle);
    owner.resources().send(()).unwrap();
    assert!(observations.recv_timeout(Duration::from_secs(30)).unwrap());
    let mut done = false;
    for _ in 0..20 {
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    engine.step(gc)
                })
            })
            .unwrap();
        if executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| engine.stack.execution.call_state.native.is_empty())
            })
            .unwrap()
        {
            assert!(matches!(result, StepResult::Continue | StepResult::Yield));
            done = true;
            break;
        }
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
    }
    assert!(done, "native call did not finish");
    owner.finish().unwrap();
}

fn native_test_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let count = fx.invocation().parameter_count;
        let source = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(count)?;
            vm.pop_string()?
        };
        fx.yield_now().await?;
        let mut chars = Vec::new();
        let mut vm = fx.vm().await?;
        vm.string(&source).copy_chunk(0, 3, &mut chars)?;
        vm.push_new_string(chars)?;
        Ok(())
    })
}

fn native_test_fallback<'gc>(
    ctx: &mut crate::stack::VesContext<'_, 'gc>,
    _: MethodDescription,
    _: &GenericLookup,
) -> StepResult {
    assert_eq!(ctx.top_of_stack().as_usize(), 3);
    StepResult::Continue
}

fn native_test_reject_selector<'a, 'gc>(
    _: &crate::stack::VesContext<'a, 'gc>,
    _: dotnet_vm_ops::effects::NativeOperandView<'_, 'gc>,
    _: &MethodDescription,
    _: &GenericLookup,
) -> dotnet_vm_ops::effects::NativeSelection<crate::stack::VesContext<'a, 'gc>> {
    panic!("unsupported modes must not run the selector")
}

#[test]
fn native_activation_real_context_and_collection() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                assert_eq!(
                    ctx.start_native(
                        method.clone(),
                        NativeDispatchMode::Other,
                        &GenericLookup::default(),
                        Some(native_test_reject_selector),
                        native_test_fallback,
                        native_test_body
                    ),
                    StepResult::Continue
                );
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_test_body,
                )
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Yield);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(!engine.stack.execution.call_state.native.is_empty());
                assert_eq!(
                    engine
                        .stack
                        .execution
                        .evaluation_stack
                        .top_of_stack()
                        .as_usize(),
                    0
                );
                engine.stack.local.heap.needs_full_collect.set(true);
            })
        })
        .unwrap();
    for turn in 0..2 {
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    if turn == 0 {
                        engine.step(gc)
                    } else {
                        engine.run(gc)
                    }
                })
            })
            .unwrap();
        assert_eq!(result, StepResult::Yield);
    }
    assert_eq!(executor.collection_check().unwrap(), (true, true));
    executor.perform_full_gc().unwrap();
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.run(gc)
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let execution = &engine.stack.execution;
                assert!(execution.call_state.native.is_empty());
                assert_eq!(execution.frame_stack.current_frame().state.ip, 1);
                let [StackValue::ObjectRef(ObjectRef(Some(result)))] =
                    execution.evaluation_stack.stack.as_slice()
                else {
                    panic!("expected string result")
                };
                let inner = result.borrow();
                let HeapStorage::Str(chars) = &inner.storage else {
                    panic!("expected string")
                };
                assert_eq!(&chars[..], &[97, 98, 99]);
            })
        })
        .unwrap();
}

fn native_test_select_async<'a, 'gc>(
    _: &crate::stack::VesContext<'a, 'gc>,
    operands: dotnet_vm_ops::effects::NativeOperandView<'_, 'gc>,
    _: &MethodDescription,
    _: &GenericLookup,
) -> dotnet_vm_ops::effects::NativeSelection<crate::stack::VesContext<'a, 'gc>> {
    assert!(matches!(operands.peek_at(0), StackValue::Int32(3)));
    dotnet_vm_ops::effects::NativeSelection::Async(dotnet_vm_ops::effects::NativeInvocation {
        start: 0,
        length: 3,
        parameter_count: 2,
    })
}

fn native_test_select_sync<'a, 'gc>(
    _: &crate::stack::VesContext<'a, 'gc>,
    operands: dotnet_vm_ops::effects::NativeOperandView<'_, 'gc>,
    _: &MethodDescription,
    _: &GenericLookup,
) -> dotnet_vm_ops::effects::NativeSelection<crate::stack::VesContext<'a, 'gc>> {
    assert!(matches!(operands.peek_at(0), StackValue::Int32(3)));
    dotnet_vm_ops::effects::NativeSelection::Synchronous(native_test_fallback)
}

fn native_test_select_reject<'a, 'gc>(
    _: &crate::stack::VesContext<'a, 'gc>,
    operands: dotnet_vm_ops::effects::NativeOperandView<'_, 'gc>,
    _: &MethodDescription,
    _: &GenericLookup,
) -> dotnet_vm_ops::effects::NativeSelection<crate::stack::VesContext<'a, 'gc>> {
    assert!(matches!(operands.peek_at(0), StackValue::Int32(3)));
    dotnet_vm_ops::effects::NativeSelection::Reject(
        crate::error::ExecutionError::InternalError("test selector rejection".into()).into(),
    )
}

fn native_test_host_failure(
    _: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async {
        Err(dotnet_vm_ops::effects::NativeFailure::Host(
            crate::error::ExecutionError::InternalError("test host failure".into()).into(),
        ))
    })
}

#[test]
fn native_activation_selection_and_failure() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    for case in ["sync", "async", "host", "reject"] {
        let mut executor = executor();
        let method = substring_method(&executor, true);
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                    let mut ctx = engine.ves_context(gc);
                    ctx.push(StackValue::ObjectRef(source));
                    ctx.push(StackValue::Int32(0));
                    ctx.push(StackValue::Int32(3));
                    let selector = match case {
                        "sync" => native_test_select_sync,
                        "reject" => native_test_select_reject,
                        _ => native_test_select_async,
                    };
                    let entry = if case == "host" {
                        native_test_host_failure
                    } else {
                        native_test_body
                    };
                    ctx.start_native(
                        method,
                        NativeDispatchMode::InlineCall,
                        &GenericLookup::default(),
                        Some(selector),
                        native_test_fallback,
                        entry,
                    )
                })
            })
            .unwrap();
        match case {
            "sync" => assert_eq!(result, StepResult::Continue),
            "async" => assert_eq!(result, StepResult::Yield),
            _ => assert!(matches!(result, StepResult::Error(_))),
        }
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let execution = &engine.stack.execution;
                    assert_eq!(
                        execution.evaluation_stack.top_of_stack().as_usize(),
                        if case == "sync" || case == "reject" {
                            3
                        } else {
                            0
                        }
                    );
                    assert_eq!(!execution.call_state.native.is_empty(), case == "async");
                })
            })
            .unwrap();
    }
}

#[test]
fn native_activation_identity_exhaustion_preserves_operands() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                engine.stack.execution.call_state.last_native_id = u64::MAX;
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                let result = ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_test_body,
                );
                assert!(matches!(result, StepResult::Error(_)));
                assert!(ctx.call_state.native.is_empty());
                assert_eq!(ctx.frame_stack.current_frame().stack_height.as_usize(), 3);
                assert_eq!(ctx.evaluation_stack.stack.len(), 3);
                assert_eq!(ctx.call_state.last_native_id, u64::MAX);
            })
        })
        .unwrap();
}

#[test]
fn native_activation_abort_and_changed_caller_cleanup() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    for abort in [false, true] {
        let mut executor = executor();
        let method = substring_method(&executor, true);
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                    let mut ctx = engine.ves_context(gc);
                    ctx.push(StackValue::ObjectRef(source));
                    ctx.push(StackValue::Int32(0));
                    ctx.push(StackValue::Int32(3));
                    ctx.start_native(
                        method,
                        NativeDispatchMode::InlineCall,
                        &GenericLookup::default(),
                        None,
                        native_test_fallback,
                        native_test_body,
                    )
                })
            })
            .unwrap();
        assert_eq!(result, StepResult::Yield);
        if abort {
            executor.shared.abort_signal().request_abort();
            assert!(matches!(executor.run(), ExecutorResult::Error(_)));
        } else {
            let result = executor
                .with_arena(|arena| {
                    arena.mutate_root(|gc, engine| {
                        engine
                            .stack
                            .execution
                            .frame_stack
                            .current_frame_mut()
                            .state
                            .ip += 1;
                        // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                        let gc = unsafe { test_gc_handle(gc, engine) };
                        engine.step(gc)
                    })
                })
                .unwrap();
            assert!(matches!(result, StepResult::Error(_)));
        }
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    assert!(engine.stack.execution.call_state.native.is_empty());
                    assert!(engine.stack.execution.evaluation_stack.stack.is_empty());
                })
            })
            .unwrap();
    }
}

fn native_test_throw(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let exception = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            vm.pop_string()?
        };
        fx.yield_now().await?;
        Err(dotnet_vm_ops::effects::NativeFailure::Managed(exception))
    })
}

#[test]
fn native_activation_managed_throw_reaches_exception_search() {
    use dotnet_vm_ops::{
        MemoryOps,
        effects::{NativeDispatchMode, NativeIntrinsicHost},
    };
    let mut executor = executor();
    let mut core = Resolution::new(Module::new("System.Private.CoreLib.dll"));
    core.assembly = Some(Assembly::new("System.Private.CoreLib"));
    for name in [
        "Object",
        "Exception",
        "Enum",
        "ValueType",
        "Array",
        "String",
    ] {
        let mut ty = TypeDefinition::new(None, name);
        ty.namespace = Some("System".into());
        core.push_type_definition(ty);
    }
    executor.shared.loader.register_owned_assembly(core.clone());
    let mut compatibility = core;
    compatibility.assembly = Some(Assembly::new("mscorlib"));
    executor
        .shared
        .loader
        .register_owned_assembly(compatibility);
    let exception_type = executor
        .shared
        .loader
        .corlib_type("System.Exception")
        .unwrap();
    let method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let exception = ctx.new_object(exception_type).unwrap();
                let exception = ctx.alloc_obj_ref(gc, exception);
                ctx.push(StackValue::ObjectRef(exception));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_test_throw,
                )
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Yield);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| engine.stack.local.heap.needs_full_collect.set(true))
        })
        .unwrap();
    assert_eq!(executor.collection_check().unwrap(), (true, true));
    executor.perform_full_gc().unwrap();
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.step(gc)
            })
        })
        .unwrap();
    let StepResult::MethodThrew(exception) = result else {
        panic!("expected unhandled exception: {result:?}")
    };
    assert_eq!(exception.type_name, "System.Exception");
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
            })
        })
        .unwrap();
}

fn native_test_immediate(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let mut vm = fx.vm().await?;
        vm.discard_scalars(2)?;
        let _source = vm.pop_string()?;
        vm.push_new_string(vec![120])?;
        Ok(())
    })
}

fn native_test_invalid_pending(
    _: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(std::future::pending())
}

#[test]
fn native_activation_immediate_completion_and_bad_poll() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    for valid in [true, false] {
        let mut executor = executor();
        let method = substring_method(&executor, true);
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                    let mut ctx = engine.ves_context(gc);
                    ctx.push(StackValue::ObjectRef(source));
                    ctx.push(StackValue::Int32(0));
                    ctx.push(StackValue::Int32(3));
                    let entry = if valid {
                        native_test_immediate
                    } else {
                        native_test_invalid_pending
                    };
                    ctx.start_native(
                        method,
                        NativeDispatchMode::InlineCall,
                        &GenericLookup::default(),
                        None,
                        native_test_fallback,
                        entry,
                    )
                })
            })
            .unwrap();
        if valid {
            assert_eq!(result, StepResult::Jump(1));
        } else {
            assert!(matches!(result, StepResult::Error(_)));
        }
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    assert!(engine.stack.execution.call_state.native.is_empty());
                    assert_eq!(
                        engine.stack.execution.evaluation_stack.stack.len(),
                        usize::from(valid)
                    );
                    assert_eq!(
                        engine
                            .stack
                            .execution
                            .frame_stack
                            .current_frame()
                            .stack_height
                            .as_usize(),
                        usize::from(valid)
                    );
                })
            })
            .unwrap();
    }
}

fn native_test_forget_access(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let source = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            vm.pop_string()?
        };
        fx.yield_now().await?;
        std::mem::forget(fx.vm().await?);
        drop(source);
        Ok(())
    })
}

fn native_test_pending_without_effect(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let source = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            vm.pop_string()?
        };
        fx.yield_now().await?;
        std::future::pending::<()>().await;
        drop(source);
        Ok(())
    })
}

fn native_test_poll_panic(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    Box::pin(async move {
        let source = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            vm.pop_string()?
        };
        fx.yield_now().await?;
        let mut vm = fx.vm().await?;
        vm.with(|_| {
            let _keep_root_live = &source;
            panic!("native poll panic");
        })?;
        Ok(())
    })
}

fn start_native_with_weak_source(
    executor: &mut Executor,
    entry: fn(dotnet_vm_ops::effects::VmEffects) -> dotnet_vm_ops::effects::NativeFuture,
) -> StepResult {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let method = substring_method(executor, true);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                engine
                    .stack
                    .local
                    .heap
                    .gchandles
                    .borrow_mut()
                    .push(Some((source, GCHandleType::Weak)));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    entry,
                )
            })
        })
        .unwrap()
}

fn assert_native_source_released(executor: &mut Executor) {
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
            })
        })
        .unwrap();
    collect(executor);
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

#[test]
fn native_activation_forgotten_access_is_terminal() {
    let mut executor = executor();
    assert_eq!(
        start_native_with_weak_source(&mut executor, native_test_forget_access),
        StepResult::Yield
    );
    assert!(
        matches!(resume(&mut executor), StepResult::Error(crate::error::VmError::Execution(
        crate::error::ExecutionError::InternalError(ref message)
    )) if message.contains("invalid native poll result"))
    );
    assert_native_source_released(&mut executor);
}

#[test]
fn native_activation_unregistered_pending_is_terminal() {
    let mut executor = executor();
    assert_eq!(
        start_native_with_weak_source(&mut executor, native_test_pending_without_effect),
        StepResult::Yield
    );
    assert!(
        matches!(resume(&mut executor), StepResult::Error(crate::error::VmError::Execution(
        crate::error::ExecutionError::InternalError(ref message)
    )) if message.contains("invalid native poll result"))
    );
    assert_native_source_released(&mut executor);
}

#[test]
fn native_activation_poll_panic_unwinds_and_releases_roots() {
    let mut executor = executor();
    assert_eq!(
        start_native_with_weak_source(&mut executor, native_test_poll_panic),
        StepResult::Yield
    );
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.step(gc)))
                        .is_err()
                );
                assert!(engine.stack.execution.call_state.native.is_empty());
            })
        })
        .unwrap();
    assert_native_source_released(&mut executor);
}

#[test]
fn native_activation_rejects_each_changed_caller_component() {
    for component in ["depth", "ip", "base"] {
        let mut executor = executor();
        assert_eq!(
            start_native_with_weak_source(&mut executor, native_test_body),
            StepResult::Yield
        );
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    match component {
                        "depth" => {
                            let frames = &mut engine.stack.execution.frame_stack;
                            let info = frames.current_frame().state.info_handle.clone();
                            frames.push_frame(
                                BasePointer::default(),
                                info,
                                Default::default(),
                                Default::default(),
                            );
                        }
                        "ip" => {
                            engine
                                .stack
                                .execution
                                .frame_stack
                                .current_frame_mut()
                                .state
                                .ip += 1
                        }
                        "base" => engine.ves_context(gc).push(StackValue::Int32(7)),
                        _ => unreachable!(),
                    }
                    engine.step(gc)
                })
            })
            .unwrap();
        assert!(
            matches!(result, StepResult::Error(crate::error::VmError::Execution(
            crate::error::ExecutionError::InternalError(ref message)
        )) if message.contains("native caller changed")),
            "{component}: {result:?}"
        );
        assert_native_source_released(&mut executor);
    }
}

thread_local! {
    static CALL_TARGET: std::cell::RefCell<Option<MethodDescription>> = const { std::cell::RefCell::new(None) };
}

fn native_managed_call_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let method = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let source = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            vm.pop_string()?
        };
        for _ in 0..2 {
            let dotnet_vm_ops::effects::NativeValue::Value(value) = fx
                .call(dotnet_vm_ops::effects::NativeCall {
                    method: method.clone(),
                    generics: GenericLookup::default(),
                    arguments: Vec::new(),
                })
                .await?
            else {
                panic!("expected return value")
            };
            let mut vm = fx.vm().await?;
            assert!(vm.with(|region| {
                region
                    .fetch(&value)
                    .map(|v| matches!(v, StackValue::Int32(0)))
            })??);
        }
        let mut chars = Vec::new();
        let mut vm = fx.vm().await?;
        vm.string(&source).copy_chunk(0, 3, &mut chars)?;
        vm.push_new_string(chars)?;
        Ok(())
    })
}

#[test]
fn native_activation_managed_calls_through_frames() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    let first = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let target = engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .clone();
                CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_managed_call_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(first, StepResult::NativeSuspended(_)));
    executor.perform_full_gc().unwrap();
    let mut generations = Vec::new();
    for turn in 0usize..12 {
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    if let Some(destination) = engine
                        .stack
                        .execution
                        .frame_stack
                        .current_frame()
                        .native_return
                        && generations.last() != Some(&destination.generation)
                    {
                        generations.push(destination.generation);
                    }
                })
            })
            .unwrap();
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    if turn.is_multiple_of(2) {
                        engine.run(gc)
                    } else {
                        engine.step(gc)
                    }
                })
            })
            .unwrap();
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
        if executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| engine.stack.execution.call_state.native.is_empty())
            })
            .unwrap()
        {
            break;
        }
    }
    assert_eq!(generations, [1, 2]);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert_eq!(
                    engine.stack.execution.frame_stack.current_frame().state.ip,
                    1
                );
                assert!(matches!(
                    engine.stack.execution.evaluation_stack.stack.as_slice(),
                    [StackValue::ObjectRef(ObjectRef(Some(_)))]
                ));
            })
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

thread_local! {
    static INLINE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn native_inline_calls_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let method = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let (source, start, length) = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            let source = vm.pop_string()?;
            let start = vm.with(|region| region.root(StackValue::Int32(0)))??;
            let length = vm.with(|region| region.root(StackValue::Int32(3)))??;
            (source, start, length)
        };
        for _ in 0..40 {
            let dotnet_vm_ops::effects::NativeValue::Value(value) = fx
                .call(dotnet_vm_ops::effects::NativeCall {
                    method: method.clone(),
                    generics: GenericLookup::default(),
                    arguments: vec![source.clone(), start.clone(), length.clone()],
                })
                .await?
            else {
                panic!("substring returns a string")
            };
            INLINE_CALLS.with(|count| count.set(count.get() + 1));
            drop(value);
        }
        let mut chars = Vec::new();
        let mut vm = fx.vm().await?;
        vm.string(&source).copy_chunk(0, 3, &mut chars)?;
        vm.push_new_string(chars)?;
        Ok(())
    })
}

#[test]
fn native_activation_inline_call_budget() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(method.clone()));
    INLINE_CALLS.with(|count| count.set(0));
    let first = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_inline_calls_body,
                )
            })
        })
        .unwrap();
    assert_eq!(first, StepResult::Yield);
    INLINE_CALLS.with(|count| assert_eq!(count.get(), 31));
    let second = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.step(gc)
            })
        })
        .unwrap();
    assert_eq!(second, StepResult::Continue);
    INLINE_CALLS.with(|count| assert_eq!(count.get(), 40));
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_activation_nested_abort_retires_managed_children() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    let first = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let target = engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .clone();
                CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method.clone(),
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_managed_call_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(first, StepResult::NativeSuspended(_)));
    let second = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("def")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_test_body,
                )
            })
        })
        .unwrap();
    assert_eq!(second, StepResult::Yield);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let execution = &engine.stack.execution;
                assert_eq!(execution.call_state.native.len(), 2);
                assert!(execution.call_state.native[0].id.0 < execution.call_state.native[1].id.0);
                assert_eq!(execution.frame_stack.len(), 2);
            })
        })
        .unwrap();
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert!(engine.stack.execution.evaluation_stack.stack.is_empty());
            })
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

fn native_echo_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let method = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let source = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            vm.pop_string()?
        };
        let dotnet_vm_ops::effects::NativeValue::Value(result) = fx
            .call(dotnet_vm_ops::effects::NativeCall {
                method,
                generics: GenericLookup::default(),
                arguments: vec![source],
            })
            .await?
        else {
            panic!("expected string")
        };
        let mut chars = Vec::new();
        let mut vm = fx.vm().await?;
        vm.string(&result).copy_chunk(0, 3, &mut chars)?;
        vm.push_new_string(chars)?;
        Ok(())
    })
}

#[test]
fn native_activation_nested_return_is_copied() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost, NativeValue};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    let first = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source_method = engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .clone();
                let index = source_method
                    .parent
                    .definition()
                    .methods
                    .iter()
                    .position(|m| m.name == "Echo")
                    .unwrap();
                let target = MethodDescription::new(
                    source_method.parent.clone(),
                    GenericLookup::default(),
                    source_method.method_resolution.clone(),
                    MethodMemberIndex::Method(index),
                );
                CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method.clone(),
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_echo_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(first, StepResult::NativeSuspended(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let execution = &engine.stack.execution;
                let destination = execution.frame_stack.current_frame().native_return.unwrap();
                assert_eq!(destination.generation, 1);
                let control = &execution.call_state.native[0].owner.control;
                assert!(
                    control
                        .reply_call(destination.generation + 1, NativeValue::Void)
                        .is_err()
                );
            })
        })
        .unwrap();
    let inner = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("def")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_test_body,
                )
            })
        })
        .unwrap();
    assert_eq!(inner, StepResult::Yield);
    executor.perform_full_gc().unwrap();
    for _ in 0..8 {
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    engine.step(gc)
                })
            })
            .unwrap();
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
        if executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| engine.stack.execution.call_state.native.is_empty())
            })
            .unwrap()
        {
            break;
        }
    }
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                let [StackValue::ObjectRef(ObjectRef(Some(result)))] =
                    engine.stack.execution.evaluation_stack.stack.as_slice()
                else {
                    panic!("expected string")
                };
                let inner = result.borrow();
                let HeapStorage::Str(chars) = &inner.storage else {
                    panic!("expected string")
                };
                assert_eq!(&chars[..], &[100, 101, 102]);
            })
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

fn native_call_with_operands_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let method = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let _ = fx
            .call(dotnet_vm_ops::effects::NativeCall {
                method,
                generics: GenericLookup::default(),
                arguments: Vec::new(),
            })
            .await?;
        Ok(())
    })
}

#[test]
fn native_activation_rejects_call_with_unconsumed_operands() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let target = engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .clone();
                CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_call_with_operands_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(result, StepResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert!(engine.stack.execution.evaluation_stack.stack.is_empty());
            })
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

fn native_nested_intrinsic_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let method = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let (source, start, length) = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            let source = vm.pop_string()?;
            let start = vm.with(|region| region.root(StackValue::Int32(0)))??;
            let length = vm.with(|region| region.root(StackValue::Int32(LENGTH as i32)))??;
            (source, start, length)
        };
        let dotnet_vm_ops::effects::NativeValue::Value(result) = fx
            .call(dotnet_vm_ops::effects::NativeCall {
                method,
                generics: GenericLookup::default(),
                arguments: vec![source, start, length],
            })
            .await?
        else {
            panic!("expected string")
        };
        let mut chars = Vec::new();
        let mut vm = fx.vm().await?;
        vm.string(&result).copy_chunk(0, 3, &mut chars)?;
        vm.push_new_string(chars)?;
        Ok(())
    })
}

#[test]
fn native_activation_calls_suspending_intrinsic() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(method.clone()));
    let first = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::new(payload())));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_nested_intrinsic_body,
                )
            })
        })
        .unwrap();
    assert_eq!(first, StepResult::Yield);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert_eq!(engine.stack.execution.call_state.native.len(), 2);
                assert!(
                    engine.stack.execution.call_state.native[0].id.0
                        < engine.stack.execution.call_state.native[1].id.0
                );
            })
        })
        .unwrap();
    executor.perform_full_gc().unwrap();
    for _ in 0..8 {
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    engine.step(gc)
                })
            })
            .unwrap();
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
        if executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| engine.stack.execution.call_state.native.is_empty())
            })
            .unwrap()
        {
            break;
        }
    }
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                let [StackValue::ObjectRef(ObjectRef(Some(result)))] =
                    engine.stack.execution.evaluation_stack.stack.as_slice()
                else {
                    panic!("expected string")
                };
                let inner = result.borrow();
                let HeapStorage::Str(chars) = &inner.storage else {
                    panic!("expected string")
                };
                assert_eq!(&chars[..], &[0, 1, 2]);
            })
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

fn native_initialized_target_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let method = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    let initializer = method.parent.static_initializer().unwrap();
    let void_index = method
        .parent
        .definition()
        .methods
        .iter()
        .position(|m| m.name == "VoidTarget")
        .unwrap();
    let void_method = MethodDescription::new(
        method.parent.clone(),
        GenericLookup::default(),
        method.method_resolution.clone(),
        MethodMemberIndex::Method(void_index),
    );
    Box::pin(async move {
        let (source, arg) = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            let source = vm.pop_string()?;
            let arg = vm.with(|region| region.root(StackValue::Int32(42)))??;
            (source, arg)
        };
        let dotnet_vm_ops::effects::NativeValue::Value(value) = fx
            .call(dotnet_vm_ops::effects::NativeCall {
                method,
                generics: GenericLookup::default(),
                arguments: vec![arg],
            })
            .await?
        else {
            panic!("expected integer")
        };
        assert!(matches!(
            fx.call(dotnet_vm_ops::effects::NativeCall {
                method: void_method,
                generics: GenericLookup::default(),
                arguments: Vec::new(),
            })
            .await?,
            dotnet_vm_ops::effects::NativeValue::Void
        ));
        assert!(matches!(
            fx.call(dotnet_vm_ops::effects::NativeCall {
                method: initializer,
                generics: GenericLookup::default(),
                arguments: Vec::new(),
            })
            .await?,
            dotnet_vm_ops::effects::NativeValue::Void
        ));
        let mut vm = fx.vm().await?;
        assert!(vm.with(|region| {
            region
                .fetch(&value)
                .map(|v| matches!(v, StackValue::Int32(42)))
        })??);
        let mut chars = Vec::new();
        vm.string(&source).copy_chunk(0, 3, &mut chars)?;
        vm.push_new_string(chars)?;
        Ok(())
    })
}

#[test]
fn native_activation_retries_target_after_initializer() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    use dotnetdll::resolved::types::TypeDefinition;
    let mut executor = executor();
    let loader = &executor.shared.loader;
    let mut corlib = Resolution::new(Module::new("mscorlib.dll"));
    corlib.assembly = Some(Assembly::new("mscorlib"));
    for name in ["Object", "ValueType", "Enum"] {
        let mut ty = TypeDefinition::new(None, name);
        ty.namespace = Some("System".into());
        corlib.push_type_definition(ty);
    }
    loader.register_owned_assembly(corlib);
    let mut res = Resolution::new(Module::new("InitTarget.dll"));
    res.assembly = Some(Assembly::new("InitTarget"));
    let ty = res.push_type_definition(TypeDefinition::new(None, "InitTarget"));
    let mut sig = parameterless_i32_signature();
    sig.return_type = ReturnType(vec![], None);
    let mut cctor = Method::new(
        Accessibility::Private,
        sig,
        ".cctor",
        Some(make_test_method!(max_stack: 0, instructions: vec![Instruction::Return])),
    );
    cctor.runtime_special_name = true;
    res.push_method(ty, cctor);
    let mut void_sig = parameterless_i32_signature();
    void_sig.return_type = ReturnType(vec![], None);
    res.push_method(
        ty,
        Method::new(
            Accessibility::Public,
            void_sig,
            "VoidTarget",
            Some(make_test_method!(max_stack: 0, instructions: vec![Instruction::Return])),
        ),
    );
    let mut sig = parameterless_i32_signature();
    sig.parameters.push(Parameter(
        vec![],
        ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32))),
    ));
    res.push_method(ty, Method::new(Accessibility::Public, sig, "Target", Some(make_test_method!(max_stack: 1, instructions: vec![Instruction::LoadArgument(0), Instruction::Return]))));
    let res = loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res.clone(), ty);
    let index = ty
        .definition()
        .methods
        .iter()
        .position(|m| m.name == "Target")
        .unwrap();
    let target = MethodDescription::new(
        ty,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(index),
    );
    CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
    let method = substring_method(&executor, true);
    let first = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_initialized_target_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(first, StepResult::NativeSuspended(_)), "{first:?}");
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(
                    engine
                        .stack
                        .execution
                        .frame_stack
                        .current_frame()
                        .state
                        .info_handle
                        .is_cctor
                );
                assert!(
                    engine
                        .stack
                        .execution
                        .frame_stack
                        .current_frame()
                        .native_return
                        .is_none()
                );
            })
        })
        .unwrap();
    for _ in 0..16 {
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    engine.step(gc)
                })
            })
            .unwrap();
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
        if executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| engine.stack.execution.call_state.native.is_empty())
            })
            .unwrap()
        {
            break;
        }
    }
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert_eq!(engine.stack.execution.evaluation_stack.stack.len(), 1);
            })
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

fn native_ensure_initialized_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let target = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let source = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            vm.pop_string()?
        };
        fx.ensure_initialized(target.parent, GenericLookup::default())
            .await?;
        let mut vm = fx.vm().await?;
        let mut chars = Vec::new();
        vm.string(&source).copy_chunk(0, 3, &mut chars)?;
        vm.push_new_string(chars)?;
        Ok(())
    })
}

#[test]
fn native_initializer_claim_executes_and_publishes_success() {
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "SuccessSubject");
    assert!(matches!(
        start_initializer_test(&mut executor),
        StepResult::NativeSuspended(_)
    ));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let frame = engine.stack.execution.frame_stack.current_frame();
                assert!(frame.state.info_handle.is_cctor);
                assert!(frame.native_return.is_some());
                assert!(matches!(
                    engine
                        .stack
                        .shared
                        .statics
                        .get_init_state(subject.clone(), &GenericLookup::default()),
                    crate::statics::InitState::Initializing(_)
                ));
            })
        })
        .unwrap();
    for _ in 0..16 {
        let result = resume(&mut executor);
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
        if executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| engine.stack.execution.call_state.native.is_empty())
            })
            .unwrap()
        {
            break;
        }
    }
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert_eq!(engine.stack.execution.evaluation_stack.stack.len(), 1);
            })
        })
        .unwrap();
    assert_eq!(
        executor
            .shared
            .statics
            .get_init_state(subject, &GenericLookup::default()),
        crate::statics::InitState::Initialized
    );
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_activation_defers_nested_entry_at_limit() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let mut executor = executor();
    let method = substring_method(&executor, true);
    CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(method.clone()));
    let first = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.stack.execution.call_state.native_entry_depth =
                    Some(std::rc::Rc::new(std::cell::Cell::new(32)));
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::new(payload())));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_nested_intrinsic_body,
                )
            })
        })
        .unwrap();
    assert_eq!(first, StepResult::Yield);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let work = &engine.stack.execution.call_state;
                assert_eq!(work.native.len(), 2);
                assert_eq!(work.native.last().unwrap().operands.len(), 3);
                work.native_entry_depth.as_ref().unwrap().set(0);
            })
        })
        .unwrap();
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| assert!(engine.stack.execution.call_state.native.is_empty()))
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_activation_rejects_reply_from_wrong_root_set() {
    use dotnet_vm_ops::effects::{
        NativeBackend, NativeDispatchMode, NativeIntrinsicHost, NativeRegion, NativeValue,
    };
    struct EmptyBackend<'gc>(Vec<StackValue<'gc>>);
    impl<'gc> NativeBackend<'gc> for EmptyBackend<'gc> {
        fn stop_requested(&self) -> bool {
            false
        }
        fn active_scope_depth(&self) -> usize {
            0
        }
        fn operands(&mut self) -> &mut Vec<StackValue<'gc>> {
            &mut self.0
        }
        fn push_new_string(&mut self, _: Vec<u16>) -> Result<(), crate::error::VmError> {
            unreachable!()
        }
    }
    let mut executor = executor();
    let method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let handle = unsafe { test_gc_handle(gc, engine) };
                let target = engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .clone();
                CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
                let source = ObjectRef::new(handle, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(handle);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                let first = ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_managed_call_body,
                );
                assert!(matches!(first, StepResult::NativeSuspended(_)));
                let wrong_set = gc_arena::DynamicRootSet::new(gc);
                let foreign = ObjectRef::new(handle, HeapStorage::Str(CLRString::from("wrong")));
                let mut backend = EmptyBackend(Vec::new());
                let wrong = NativeRegion::new(gc, wrong_set, &mut backend)
                    .root(StackValue::ObjectRef(foreign))
                    .unwrap();
                let execution = &mut engine.stack.execution;
                let child = execution.frame_stack.pop().unwrap();
                let destination = child.native_return.unwrap();
                execution.evaluation_stack.truncate(child.base.arguments);
                execution.frame_stack.recycle_frame(child);
                let work = execution.call_state.native.last_mut().unwrap();
                work.owner
                    .control
                    .reply_call(destination.generation, NativeValue::Value(wrong))
                    .unwrap();
                work.pending = None;
                engine.step(handle)
            })
        })
        .unwrap();
    assert!(matches!(result, StepResult::Error(_)));
    assert!(format!("{result:?}").contains("mismatched native root set"));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| assert!(engine.stack.execution.call_state.native.is_empty()))
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(feature = "multithreading")]
#[test]
fn native_activation_foreign_child_return_survives_collection() {
    use crate::{
        test_utils::{
            foreign_reference::{decode_reference, encode_reference},
            worker::Worker,
        },
        threading::ThreadManagerOps,
    };
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    use std::{sync::mpsc, thread, time::Duration};

    let mut executor = executor();
    let method = substring_method(&executor, true);
    let shared = executor.shared.clone();
    let owner_shared = shared.clone();
    let (send, receive) = mpsc::channel();
    let (command, commands) = mpsc::channel::<()>();
    let (witness, observations) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut owner = Executor::new(owner_shared.clone());
        let id = owner.thread_id;
        let encoded = owner
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                    engine
                        .stack
                        .local
                        .heap
                        .gchandles
                        .borrow_mut()
                        .push(Some((source, GCHandleType::Weak)));
                    encode_reference(source)
                })
            })
            .unwrap();
        send.send(encoded).unwrap();
        loop {
            owner_shared
                .thread_manager
                .safe_point(id, &owner_shared.gc_coordinator);
            match commands.recv_timeout(Duration::from_millis(1)) {
                Ok(()) => {
                    let alive = owner
                        .with_arena(|arena| {
                            arena.mutate(|_, engine| {
                                engine.stack.local.heap.gchandles.borrow()[0]
                                    .as_ref()
                                    .unwrap()
                                    .0
                                    .0
                                    .is_some()
                            })
                        })
                        .unwrap();
                    witness.send(alive).unwrap();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    let owner = Worker::new(worker, command);
    let encoded = receive.recv_timeout(Duration::from_secs(30)).unwrap();
    let first = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source_method = engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .clone();
                let index = source_method
                    .parent
                    .definition()
                    .methods
                    .iter()
                    .position(|m| m.name == "Echo")
                    .unwrap();
                let target = MethodDescription::new(
                    source_method.parent.clone(),
                    GenericLookup::default(),
                    source_method.method_resolution.clone(),
                    MethodMemberIndex::Method(index),
                );
                CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
                let source = decode_reference(encoded, gc);
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_echo_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(first, StepResult::NativeSuspended(_)));
    for _ in 0..2 {
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    engine.step(gc)
                })
            })
            .unwrap();
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
    }
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                let work = engine.stack.execution.call_state.native.last().unwrap();
                assert!(work.operands.is_empty());
                assert!(work.pending.is_none());
                assert!(work.outputs.is_empty());
                assert!(engine.stack.execution.evaluation_stack.stack.is_empty());
            })
        })
        .unwrap();
    executor.perform_full_gc().unwrap();
    owner.resources().send(()).unwrap();
    assert!(observations.recv_timeout(Duration::from_secs(30)).unwrap());
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.step(gc)
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                let [StackValue::ObjectRef(ObjectRef(Some(result)))] =
                    engine.stack.execution.evaluation_stack.stack.as_slice()
                else {
                    panic!("expected string")
                };
                let inner = result.borrow();
                let HeapStorage::Str(chars) = &inner.storage else {
                    panic!("expected string")
                };
                assert_eq!(&chars[..], &[97, 98, 99]);
            })
        })
        .unwrap();
    executor.perform_full_gc().unwrap();
    owner.resources().send(()).unwrap();
    assert!(!observations.recv_timeout(Duration::from_secs(30)).unwrap());
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
    owner.finish().unwrap();
}

thread_local! {
    static BOUNDARY_DROPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BOUNDARY_RESUMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BOUNDARY_NEXT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BOUNDARY_ORDER: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn boundary_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    struct Witness(usize);
    impl Drop for Witness {
        fn drop(&mut self) {
            BOUNDARY_DROPS.with(|count| count.set(count.get() + 1));
            BOUNDARY_ORDER.with(|order| order.borrow_mut().push(self.0));
        }
    }
    let target = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let id = BOUNDARY_NEXT.with(|next| {
            let id = next.get() + 1;
            next.set(id);
            id
        });
        let _witness = Witness(id);
        let source = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            vm.pop_string()?
        };
        fx.call(dotnet_vm_ops::effects::NativeCall {
            method: target,
            generics: GenericLookup::default(),
            arguments: Vec::new(),
        })
        .await?;
        BOUNDARY_RESUMES.with(|count| count.set(count.get() + 1));
        let mut chars = Vec::new();
        let mut vm = fx.vm().await?;
        vm.string(&source).copy_chunk(0, 3, &mut chars)?;
        vm.push_new_string(chars)?;
        Ok(())
    })
}

fn boundary_executor(with_trace: bool) -> (Executor, TypeDescription) {
    boundary_executor_with_initializer(with_trace, false)
}

fn boundary_executor_with_initializer(
    with_trace: bool,
    with_initializer: bool,
) -> (Executor, TypeDescription) {
    let executor = executor();
    let mut core = Resolution::new(Module::new("System.Private.CoreLib.dll"));
    core.assembly = Some(Assembly::new("System.Private.CoreLib"));
    for name in [
        "Object",
        "Exception",
        "ArgumentException",
        "ArgumentOutOfRangeException",
        "Enum",
        "ValueType",
        "Byte",
        "Array",
        "String",
    ] {
        let mut ty = TypeDefinition::new(None, name);
        ty.namespace = Some("System".into());
        let index = core.push_type_definition(ty);
        if name == "Object" {
            let mut signature = parameterless_i32_signature();
            signature.instance = true;
            signature.return_type = ReturnType(vec![], None);
            let mut finalizer = Method::new(
                Accessibility::Public,
                signature,
                "Finalize",
                Some(make_test_method!(max_stack: 1, instructions: vec![Instruction::Return])),
            );
            finalizer.virtual_member = true;
            core.push_method(index, finalizer);
        }
        if name == "Exception" && with_trace {
            core.push_field(
                index,
                dotnetdll::resolved::members::Field::instance(
                    Accessibility::Public,
                    "_stackTraceString",
                    MemberType::Base(Box::new(BaseType::String)),
                ),
            );
        }
    }
    if with_initializer {
        let exception = core
            .enumerate_type_definitions()
            .find(|(_, ty)| ty.name == "Exception")
            .unwrap()
            .0;
        core.push_field(
            exception,
            dotnetdll::resolved::members::Field::instance(
                Accessibility::Public,
                "_innerException",
                MemberType::Base(Box::new(BaseType::Object)),
            ),
        );
        let mut wrapper = TypeDefinition::new(None, "TypeInitializationException");
        wrapper.namespace = Some("System".into());
        wrapper.extends = Some(exception.into());
        core.push_type_definition(wrapper);
    }
    executor.shared.loader.register_owned_assembly(core.clone());
    core.assembly = Some(Assembly::new("mscorlib"));
    executor.shared.loader.register_owned_assembly(core);
    let ty = executor
        .shared
        .loader
        .corlib_type("System.Exception")
        .unwrap();
    BOUNDARY_DROPS.with(|count| count.set(0));
    BOUNDARY_RESUMES.with(|count| count.set(0));
    BOUNDARY_NEXT.with(|count| count.set(0));
    BOUNDARY_ORDER.with(|order| order.borrow_mut().clear());
    (executor, ty)
}

#[test]
fn handler_unwind_stack_resumes_and_discards_exited_handlers() {
    use crate::stack::ops::VmStackOps;
    use dotnet_vm_data::{HandlerAddress, UnwindState, UnwindTarget, exceptions::HandlerKind};
    let (mut executor, _) = boundary_executor(false);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                ctx.current_frame_mut().state.info_handle.exceptions = vec![
                    Arc::new(boundary_section(HandlerKind::Finally, 0..1, 1..20)),
                    Arc::new(boundary_section(HandlerKind::Finally, 2..3, 3..10)),
                ];
                let state = |section_index| UnwindState {
                    exception: None,
                    target: UnwindTarget::Instruction(30 + section_index),
                    cursor: HandlerAddress {
                        frame_index: 0,
                        section_index,
                        handler_index: 0,
                    },
                    nested_filter: None,
                };
                ctx.current_frame_mut().state.ip = 2;
                ctx.push_handler_unwind(state(1));
                ctx.current_frame_mut().state.ip = 4;
                ctx.push_handler_unwind(state(2));
                assert_eq!(ctx.resume_handler_unwind(Some((0, 5))), Some(state(2)));
                assert_eq!(ctx.resume_handler_unwind(Some((0, 15))), Some(state(1)));
                ctx.current_frame_mut().state.ip = 2;
                ctx.push_handler_unwind(state(1));
                ctx.current_frame_mut().state.ip = 4;
                ctx.push_handler_unwind(state(2));
                assert_eq!(ctx.resume_handler_unwind(Some((0, 15))), Some(state(1)));
                assert_eq!(ctx.resume_handler_unwind(Some((0, 15))), None);
                ctx.current_frame_mut().state.ip = 2;
                ctx.push_handler_unwind(state(1));
                assert_eq!(ctx.resume_handler_unwind(Some((0, 25))), None);
            })
        })
        .unwrap();
}

fn boundary_start(executor: &mut Executor) {
    boundary_start_with(executor, boundary_body);
}

fn boundary_start_with(
    executor: &mut Executor,
    body: fn(dotnet_vm_ops::effects::VmEffects) -> dotnet_vm_ops::effects::NativeFuture,
) {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let method = substring_method(executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let target = engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .clone();
                CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
                let mut ctx = engine.ves_context(gc);
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    body,
                )
            })
        })
        .unwrap();
    assert!(
        matches!(result, StepResult::NativeSuspended(_)),
        "{result:?}"
    );
}

fn boundary_throw(
    executor: &mut Executor,
    exception_type: TypeDescription,
    child: Vec<dotnet_vm_data::ProtectedSection>,
    parent: Vec<dotnet_vm_data::ProtectedSection>,
) -> StepResult {
    boundary_throw_inner(executor, exception_type, child, parent, false)
}

fn boundary_throw_inner(
    executor: &mut Executor,
    exception_type: TypeDescription,
    child: Vec<dotnet_vm_data::ProtectedSection>,
    parent: Vec<dotnet_vm_data::ProtectedSection>,
    weak: bool,
) -> StepResult {
    use crate::stack::ops::ExceptionOps;
    use dotnet_vm_ops::MemoryOps;
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                ctx.frame_stack.frames[0].state.info_handle.exceptions =
                    parent.into_iter().map(Arc::new).collect();
                ctx.frame_stack
                    .current_frame_mut()
                    .state
                    .info_handle
                    .exceptions = child.into_iter().map(Arc::new).collect();
                ctx.frame_stack
                    .current_frame_mut()
                    .state
                    .info_handle
                    .instructions = &[
                    Instruction::LoadConstantInt32(0),
                    Instruction::EndFinally,
                    Instruction::EndFinally,
                ];
                let exception = ctx.new_object(exception_type).unwrap();
                let exception = ctx.alloc_obj_ref(gc, exception);
                if weak {
                    ctx.local
                        .heap
                        .gchandles
                        .borrow_mut()
                        .push(Some((exception, GCHandleType::Weak)));
                }
                ctx.throw(exception)
            })
        })
        .unwrap()
}

fn boundary_section(
    kind: dotnet_vm_data::exceptions::HandlerKind,
    instructions: std::ops::Range<usize>,
    handler: std::ops::Range<usize>,
) -> dotnet_vm_data::ProtectedSection {
    dotnet_vm_data::ProtectedSection {
        instructions,
        handlers: vec![dotnet_vm_data::exceptions::Handler {
            instructions: handler,
            kind,
        }],
    }
}

fn boundary_handle(executor: &mut Executor, endfinally: bool) -> StepResult {
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                if endfinally {
                    engine.step(gc)
                } else {
                    engine.ves_context(gc).handle_exception()
                }
            })
        })
        .unwrap()
}

fn capturing_boundary_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let target = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            let _source = vm.pop_string()?;
        }
        let result = fx
            .try_call(dotnet_vm_ops::effects::NativeCall {
                method: target,
                generics: GenericLookup::default(),
                arguments: Vec::new(),
            })
            .await;
        let Err(dotnet_vm_ops::effects::NativeFailure::Managed(exception)) = result else {
            panic!("expected captured exception");
        };
        BOUNDARY_RESUMES.with(|count| count.set(count.get() + 1));
        fx.yield_now().await?;
        Err(dotnet_vm_ops::effects::NativeFailure::Managed(exception))
    })
}

#[test]
fn native_capture_filters_and_child_cleanup_precede_outer_search() {
    use crate::stack::ops::ExceptionOps;
    use dotnet_vm_data::exceptions::HandlerKind;
    let (mut executor, ty) = boundary_executor(false);
    boundary_start_with(&mut executor, capturing_boundary_body);
    let child = vec![
        boundary_section(HandlerKind::Filter { clause_offset: 1 }, 0..1, 1..2),
        boundary_section(HandlerKind::Finally, 0..1, 1..2),
        boundary_section(HandlerKind::Fault, 0..2, 2..3),
    ];
    let parent = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw_inner(&mut executor, ty, child, parent, true),
        StepResult::Exception
    );
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    dotnet_vm_data::ExceptionState::Filtering(_)
                ));
                assert_eq!(engine.stack.execution.frame_stack.len(), 2);
                assert_eq!(engine.ves_context(gc).endfilter(0), StepResult::Exception);
            })
        })
        .unwrap();
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    for ip in [1, 2] {
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    assert_eq!(
                        engine.stack.execution.frame_stack.current_frame().state.ip,
                        ip
                    );
                    assert_eq!(BOUNDARY_RESUMES.with(|count| count.get()), 0);
                })
            })
            .unwrap();
        let result = boundary_handle(&mut executor, true);
        assert_eq!(
            result,
            if ip == 1 {
                StepResult::Exception
            } else {
                StepResult::Yield
            }
        );
    }
    executor.perform_full_gc().unwrap();
    assert_eq!(resume(&mut executor), StepResult::Yield);
    assert_eq!(BOUNDARY_RESUMES.with(|count| count.get()), 1);
    executor.perform_full_gc().unwrap();
    assert_eq!(resume(&mut executor), StepResult::Exception);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    dotnet_vm_data::ExceptionState::Filtering(_)
                ));
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                let original = engine.stack.local.heap.gchandles.borrow()[0]
                    .as_ref()
                    .unwrap()
                    .0;
                assert!(original.0.is_some(), "captured exception was collected");
                assert_eq!(
                    engine
                        .stack
                        .execution
                        .frame_stack
                        .current_frame()
                        .exception_stack
                        .last(),
                    Some(&original)
                );
                assert_eq!(engine.ves_context(gc).endfilter(0), StepResult::Exception);
            })
        })
        .unwrap();
    let result = boundary_handle(&mut executor, false);
    assert!(matches!(result, StepResult::MethodThrew(_)), "{result:?}");
}

fn initializer_test_subject(executor: &Executor, name: &str) -> TypeDescription {
    let mut assembly = Resolution::new(Module::new(format!("{name}.dll")));
    assembly.assembly = Some(Assembly::new(name.to_string()));
    let index = assembly.push_type_definition(TypeDefinition::new(None, name.to_string()));
    let mut signature = parameterless_i32_signature();
    signature.return_type = ReturnType(vec![], None);
    let mut cctor = Method::new(
        Accessibility::Private,
        signature,
        ".cctor",
        Some(make_test_method!(max_stack: 0, instructions: vec![Instruction::Return])),
    );
    cctor.runtime_special_name = true;
    assembly.push_method(index, cctor);
    let assembly = executor.shared.loader.register_owned_assembly(assembly);
    let ty = TypeDescription::new(assembly, index);
    CALL_TARGET.with(|slot| *slot.borrow_mut() = ty.static_initializer());
    ty
}

fn start_initializer_test(executor: &mut Executor) -> StepResult {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let method = substring_method(executor, true);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_ensure_initialized_body,
                )
            })
        })
        .unwrap()
}

#[test]
fn static_store_operand_survives_initializer_and_collection() {
    static_store_operand_collection(true);
}

#[test]
fn static_store_operand_released_after_abandonment() {
    static_store_operand_collection(false);
}

fn static_store_operand_collection(complete: bool) {
    use crate::stack::ops::{StaticFieldAction, StaticFieldKind, VmStaticsOps};
    use dotnet_types::members::FieldDescription;
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let mut assembly = Resolution::new(Module::new("StaticStore.dll"));
    assembly.assembly = Some(Assembly::new("StaticStore"));
    let index = assembly.push_type_definition(TypeDefinition::new(None, "Subject"));
    assembly.push_field(
        index,
        dotnetdll::resolved::members::Field::static_member(
            Accessibility::Public,
            "Value",
            MemberType::Base(Box::new(BaseType::Object)),
        ),
    );
    let mut signature = parameterless_i32_signature();
    signature.return_type = ReturnType(vec![], None);
    let mut cctor = Method::new(
        Accessibility::Private,
        signature,
        ".cctor",
        Some(make_test_method!(max_stack: 0, instructions: vec![Instruction::Return])),
    );
    cctor.runtime_special_name = true;
    assembly.push_method(index, cctor);
    let assembly = executor.shared.loader.register_owned_assembly(assembly);
    let subject = TypeDescription::new(assembly.clone(), index);
    let field = FieldDescription::new(subject, assembly, 0);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the arena root throughout this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let object = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("retained")));
                engine
                    .stack
                    .local
                    .heap
                    .gchandles
                    .borrow_mut()
                    .push(Some((object, GCHandleType::Weak)));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(object));
                ctx.start_static_field(StaticFieldAction {
                    field,
                    generics: GenericLookup::default(),
                    kind: StaticFieldKind::Store { volatile: false },
                })
            })
        })
        .unwrap();
    assert!(
        matches!(
            result,
            StepResult::NativeSuspended(_) | StepResult::FramePushed
        ),
        "{result:?}"
    );
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
                        .is_some()
                );
                let work = engine.stack.execution.call_state.native.last().unwrap();
                assert!(matches!(
                    work.instruction.as_ref().unwrap().operand,
                    Some(StackValue::ObjectRef(_))
                ));
                assert_eq!(work.caller_ip, 0);
            })
        })
        .unwrap();
    if !complete {
        executor
            .with_arena(|arena| {
                arena.mutate_root(|_, engine| {
                    let execution = &mut engine.stack.execution;
                    execution.call_state.cleanup_native(
                        &mut execution.frame_stack,
                        &mut execution.evaluation_stack,
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
        return;
    }
    for _ in 0..16 {
        let result = resume(&mut executor);
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
        if executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| engine.stack.execution.call_state.native.is_empty())
            })
            .unwrap()
        {
            break;
        }
    }
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert_eq!(
                    engine.stack.execution.frame_stack.current_frame().state.ip,
                    1
                );
                assert!(engine.stack.execution.evaluation_stack.stack.is_empty());
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
                        .is_some()
                );
            })
        })
        .unwrap();
}

#[test]
fn native_initializer_failure_is_published_with_original_inner() {
    let (mut executor, exception_type) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "InitializerSubject");
    assert!(matches!(
        start_initializer_test(&mut executor),
        StepResult::NativeSuspended(_)
    ));
    assert!(matches!(
        executor
            .shared
            .statics
            .get_init_state(subject.clone(), &GenericLookup::default()),
        crate::statics::InitState::Initializing(_)
    ));
    assert_eq!(
        boundary_throw_inner(&mut executor, exception_type.clone(), vec![], vec![], true),
        StepResult::Yield
    );
    executor.perform_full_gc().unwrap();
    let result = resume(&mut executor);
    assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
    executor.perform_full_gc().unwrap();
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let cached = engine
                    .stack
                    .shared
                    .statics
                    .failed_exception(subject.clone(), &GenericLookup::default(), gc)
                    .expect("cached failure");
                let inner = cached.as_object(|object| {
                    object
                        .instance_storage
                        .field::<ObjectRef>(exception_type, "_innerException")
                        .unwrap()
                        .read()
                });
                let original = engine.stack.local.heap.gchandles.borrow()[0]
                    .as_ref()
                    .unwrap()
                    .0;
                assert!(original.0.is_some());
                assert_eq!(inner, original);
                assert_eq!(
                    engine
                        .stack
                        .shared
                        .statics
                        .get_init_state(subject, &GenericLookup::default()),
                    crate::statics::InitState::Failed
                );
            })
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_initializer_abandoned_claim_fails_on_cleanup() {
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "InitializerSubject");
    assert!(matches!(
        start_initializer_test(&mut executor),
        StepResult::NativeSuspended(_)
    ));
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                let execution = &mut engine.stack.execution;
                execution
                    .call_state
                    .cleanup_native(&mut execution.frame_stack, &mut execution.evaluation_stack);
            })
        })
        .unwrap();
    assert_eq!(
        executor
            .shared
            .statics
            .get_init_state(subject, &GenericLookup::default()),
        crate::statics::InitState::Failed
    );
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_initializer_observes_legacy_failure_with_original_inner() {
    use dotnet_vm_ops::{MemoryOps, StaticsOps};
    let (mut executor, exception_type) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "LegacyFailureSubject");
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                assert_eq!(
                    ctx.initialize_static_storage(subject.clone(), GenericLookup::default()),
                    StepResult::FramePushed
                );
                let inner = ctx.new_object(exception_type.clone()).unwrap();
                let inner = ctx.alloc_obj_ref(gc, inner);
                ctx.local
                    .heap
                    .gchandles
                    .borrow_mut()
                    .push(Some((inner, GCHandleType::Weak)));
                *ctx.exception_mode = ExceptionState::Throwing(inner, false);
                ctx.unwind_frame();
                *ctx.exception_mode = ExceptionState::None;
            })
        })
        .unwrap();
    executor.perform_full_gc().unwrap();
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let wrapper = engine
                    .stack
                    .shared
                    .statics
                    .failed_exception(subject.clone(), &GenericLookup::default(), gc)
                    .unwrap();
                let original = engine.stack.local.heap.gchandles.borrow()[0]
                    .as_ref()
                    .unwrap()
                    .0;
                let inner = wrapper.as_object(|object| {
                    object
                        .instance_storage
                        .field::<ObjectRef>(exception_type, "_innerException")
                        .unwrap()
                        .read()
                });
                assert_eq!(inner, original);
                assert!(original.0.is_some());
            })
        })
        .unwrap();
    let result = start_initializer_test(&mut executor);
    assert!(
        matches!(result, StepResult::MethodThrew(ref exception) if exception.type_name == "System.TypeInitializationException"),
        "{result:?}"
    );
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_initializer_recursion_does_not_wait_for_same_thread() {
    use crate::stack::ops::VmResolutionOps;
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "RecursiveSubject");
    assert!(matches!(
        start_initializer_test(&mut executor),
        StepResult::NativeSuspended(_)
    ));
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let ctx = engine.ves_context(gc);
                assert!(matches!(
                    ctx.shared
                        .statics
                        .init(
                            subject,
                            &ctx.with_generics(&GenericLookup::default()),
                            ctx.thread_id.get()
                        )
                        .unwrap(),
                    crate::statics::StaticInitResult::Recursive
                ));
            })
        })
        .unwrap();
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_initializer_abort_publishes_failure() {
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "AbortSubject");
    assert!(matches!(
        start_initializer_test(&mut executor),
        StepResult::NativeSuspended(_)
    ));
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    assert_eq!(
        executor
            .shared
            .statics
            .get_init_state(subject, &GenericLookup::default()),
        crate::statics::InitState::Failed
    );
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_initializer_host_error_publishes_failure() {
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "ErrorSubject");
    assert!(matches!(
        start_initializer_test(&mut executor),
        StepResult::NativeSuspended(_)
    ));
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                let frames = &mut engine.stack.execution.frame_stack;
                let child = frames.pop().unwrap();
                frames.recycle_frame(child);
                frames.current_frame_mut().state.ip += 1;
            })
        })
        .unwrap();
    assert!(matches!(resume(&mut executor), StepResult::Error(_)));
    assert_eq!(
        executor
            .shared
            .statics
            .get_init_state(subject, &GenericLookup::default()),
        crate::statics::InitState::Failed
    );
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_initializer_child_cleanup_precedes_outer_filter() {
    use crate::stack::ops::ExceptionOps;
    use dotnet_vm_data::exceptions::HandlerKind;
    let (mut executor, exception_type) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "FilterSubject");
    assert!(matches!(
        start_initializer_test(&mut executor),
        StepResult::NativeSuspended(_)
    ));
    let child = vec![boundary_section(HandlerKind::Finally, 0..1, 1..2)];
    let outer = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw_inner(&mut executor, exception_type, child, outer, true),
        StepResult::Exception
    );
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert_eq!(engine.stack.execution.frame_stack.len(), 2);
                assert_eq!(
                    engine.stack.execution.frame_stack.current_frame().state.ip,
                    1
                );
                assert!(matches!(
                    engine
                        .stack
                        .shared
                        .statics
                        .get_init_state(subject.clone(), &GenericLookup::default()),
                    crate::statics::InitState::Initializing(_)
                ));
            })
        })
        .unwrap();
    assert_eq!(boundary_handle(&mut executor, true), StepResult::Yield);
    assert_eq!(resume(&mut executor), StepResult::Exception);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    ExceptionState::Filtering(_)
                ));
                let exception = *engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .exception_stack
                    .last()
                    .unwrap();
                assert_eq!(
                    exception.as_object(|object| object.description.type_name()),
                    "System.TypeInitializationException"
                );
                assert_eq!(engine.ves_context(gc).endfilter(0), StepResult::Exception);
            })
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(feature = "multithreading")]
fn claim_initializer_for(
    executor: &mut Executor,
    ty: TypeDescription,
    owner: dotnet_utils::ArenaId,
) -> crate::statics::StaticInitResult {
    use crate::stack::ops::VmResolutionOps;
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let ctx = engine.ves_context(gc);
                ctx.shared
                    .statics
                    .init(ty, &ctx.with_generics(&GenericLookup::default()), owner)
                    .unwrap()
            })
        })
        .unwrap()
}

#[cfg(feature = "multithreading")]
#[test]
fn native_initializer_wait_rechecks_cycle_after_real_collection() {
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let first = initializer_test_subject(&executor, "FirstSubject");
    let second = initializer_test_subject(&executor, "SecondSubject");
    CALL_TARGET.with(|slot| *slot.borrow_mut() = first.static_initializer());
    let owner = dotnet_utils::ArenaId::new(9999);
    let thread = executor.thread_id;
    let claim_1 = claim_initializer_for(&mut executor, first.clone(), owner);
    assert!(matches!(
        claim_1,
        crate::statics::StaticInitResult::Execute(..)
    ));
    let claim_2 = claim_initializer_for(&mut executor, second.clone(), thread);
    assert!(matches!(
        claim_2,
        crate::statics::StaticInitResult::Execute(..)
    ));
    assert_eq!(start_initializer_test(&mut executor), StepResult::Yield);
    executor.perform_full_gc().unwrap();
    let reverse = claim_initializer_for(&mut executor, second.clone(), owner);
    assert!(
        matches!(reverse, crate::statics::StaticInitResult::Waiting(_)),
        "{reverse:?}"
    );
    assert_eq!(resume(&mut executor), StepResult::Continue);
    assert!(matches!(
        executor
            .shared
            .statics
            .get_init_state(first.clone(), &GenericLookup::default()),
        crate::statics::InitState::Initializing(_)
    ));
    drop(reverse);
    executor
        .shared
        .statics
        .mark_initialized(first, &GenericLookup::default());
    executor
        .shared
        .statics
        .mark_initialized(second, &GenericLookup::default());
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(feature = "multithreading")]
#[test]
fn native_initializer_wait_replies_before_collection_when_owner_is_busy() {
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "CollectionWaitSubject");
    let owner = dotnet_utils::ArenaId::new(9999);
    let claim = claim_initializer_for(&mut executor, subject.clone(), owner);
    assert!(matches!(
        claim,
        crate::statics::StaticInitResult::Execute(..)
    ));
    assert_eq!(start_initializer_test(&mut executor), StepResult::Yield);
    assert!(executor.shared.statics.has_initializer_waits());
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| engine.stack.local.heap.needs_full_collect.set(true))
        })
        .unwrap();
    assert_eq!(resume(&mut executor), StepResult::Yield);
    assert!(
        !executor.shared.statics.has_initializer_waits(),
        "collection must release the wait before resumption"
    );
    assert_eq!(executor.collection_check().unwrap(), (true, true));
    executor.perform_full_gc().unwrap();
    executor
        .shared
        .statics
        .mark_initialized(subject, &GenericLookup::default());
    assert_eq!(resume(&mut executor), StepResult::Continue);
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(feature = "multithreading")]
#[test]
fn native_initializer_wait_observes_published_success() {
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "WaitSubject");
    let owner = dotnet_utils::ArenaId::new(9999);
    let claim_3 = claim_initializer_for(&mut executor, subject.clone(), owner);
    assert!(matches!(
        claim_3,
        crate::statics::StaticInitResult::Execute(..)
    ));
    assert_eq!(start_initializer_test(&mut executor), StepResult::Yield);
    executor
        .shared
        .statics
        .mark_initialized(subject, &GenericLookup::default());
    assert_eq!(resume(&mut executor), StepResult::Continue);
    assert!(
        !executor.shared.statics.has_initializer_waits(),
        "completed initializer wait retained its graph edge"
    );
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(feature = "multithreading")]
#[test]
fn native_initializer_wait_abort_removes_graph_edge() {
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let first = initializer_test_subject(&executor, "WaitAbortFirst");
    let second = initializer_test_subject(&executor, "WaitAbortSecond");
    CALL_TARGET.with(|slot| *slot.borrow_mut() = first.static_initializer());
    let owner = dotnet_utils::ArenaId::new(9999);
    let thread = executor.thread_id;
    let claim_4 = claim_initializer_for(&mut executor, first.clone(), owner);
    assert!(matches!(
        claim_4,
        crate::statics::StaticInitResult::Execute(..)
    ));
    let claim_5 = claim_initializer_for(&mut executor, second.clone(), thread);
    assert!(matches!(
        claim_5,
        crate::statics::StaticInitResult::Execute(..)
    ));
    assert_eq!(start_initializer_test(&mut executor), StepResult::Yield);
    assert!(matches!(
        claim_initializer_for(&mut executor, second.clone(), owner),
        crate::statics::StaticInitResult::Recursive
    ));
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    let reverse = claim_initializer_for(&mut executor, second.clone(), owner);
    assert!(
        matches!(reverse, crate::statics::StaticInitResult::Waiting(_)),
        "{reverse:?}"
    );
    drop(reverse);
    executor
        .shared
        .statics
        .mark_failed(first, &GenericLookup::default());
    executor
        .shared
        .statics
        .mark_failed(second, &GenericLookup::default());
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(feature = "multithreading")]
#[test]
fn native_initializer_executor_wait_wakes_after_registration() {
    use crate::test_utils::worker::Worker;
    use std::{sync::mpsc, time::Duration};
    struct ClearHook;
    impl Drop for ClearHook {
        fn drop(&mut self) {
            super::WAIT_TEST_HOOK.with(|hook| *hook.borrow_mut() = None);
        }
    }
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "WakeSubject");
    let owner = dotnet_utils::ArenaId::new(9999);
    let claim_6 = claim_initializer_for(&mut executor, subject.clone(), owner);
    assert!(matches!(
        claim_6,
        crate::statics::StaticInitResult::Execute(..)
    ));
    assert_eq!(start_initializer_test(&mut executor), StepResult::Yield);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame_mut()
                    .state
                    .info_handle
                    .instructions = &[
                    Instruction::NoOperation,
                    Instruction::Pop,
                    Instruction::LoadConstantInt32(42),
                    Instruction::Return,
                ];
            })
        })
        .unwrap();
    let (send, receive) = mpsc::channel();
    super::WAIT_TEST_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let _ = send.send(());
        }))
    });
    let manager = executor.shared.statics.clone();
    let worker = Worker::new(
        std::thread::spawn(move || {
            if receive.recv_timeout(Duration::from_secs(10)).is_ok() {
                manager.mark_initialized(subject, &GenericLookup::default());
                true
            } else {
                false
            }
        }),
        ClearHook,
    );
    let result = executor.run();
    let signalled = worker.finish().unwrap();
    assert!(signalled, "executor did not park the registered wait");
    assert!(matches!(result, ExecutorResult::Exited(42)), "{result:?}");
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(feature = "multithreading")]
#[test]
fn native_initializer_failure_survives_owner_retirement_and_collection() {
    use crate::test_utils::foreign_reference::{decode_reference, encode_reference};
    use std::time::{Duration, Instant};
    struct Shutdown(Arc<SharedGlobalState>);
    impl Drop for Shutdown {
        fn drop(&mut self) {
            self.0.shutdown_workers();
        }
    }
    let (initial, exception_type) = boundary_executor_with_initializer(false, true);
    let shared = initial.shared.clone();
    let _shutdown = Shutdown(shared.clone());
    let mut executor = initial;
    let subject = initializer_test_subject(&executor, "RetiredFailureSubject");
    let caller = executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .clone()
            })
        })
        .unwrap();
    let owner_state = shared.clone();
    let owner_subject = subject.clone();
    let owner_exception = exception_type.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let (ready, started) = std::sync::mpsc::channel();
    let (release, wait_for_release) = std::sync::mpsc::channel();
    let completion = shared.workers.spawn_owner(move |running| {
        use crate::threading::ThreadManagerOps;
        let mut owner = Executor::new(owner_state.clone());
        owner
            .install_thread_start(caller, GenericLookup::default())
            .unwrap();
        CALL_TARGET.with(|slot| *slot.borrow_mut() = owner_subject.static_initializer());
        assert!(matches!(
            start_initializer_test(&mut owner),
            StepResult::NativeSuspended(_)
        ));
        ready.send(()).unwrap();
        loop {
            owner_state
                .thread_manager
                .safe_point(owner.thread_id, &owner_state.gc_coordinator);
            match wait_for_release.recv_timeout(Duration::from_millis(1)) {
                Ok(()) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    owner
                        .with_arena(|arena| {
                            arena.mutate_root(|_, engine| {
                                let execution = &mut engine.stack.execution;
                                execution.call_state.cleanup_native(
                                    &mut execution.frame_stack,
                                    &mut execution.evaluation_stack,
                                );
                                *execution = crate::stack::context::ThreadContext::default();
                            })
                        })
                        .unwrap();
                    owner.serve_retired(running.complete(ExecutorResult::Exited(0)));
                    return;
                }
            }
        }
        assert_eq!(
            boundary_throw_inner(&mut owner, owner_exception, vec![], vec![], true),
            StepResult::Yield
        );
        let result = resume(&mut owner);
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
        let reference = owner
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    let original = engine.stack.local.heap.gchandles.borrow()[0]
                        .as_ref()
                        .unwrap()
                        .0;
                    encode_reference(original)
                })
            })
            .unwrap();
        send.send(reference).unwrap();
        owner
            .with_arena(|arena| {
                arena.mutate_root(|_, engine| {
                    let execution = &mut engine.stack.execution;
                    execution.call_state.cleanup_native(
                        &mut execution.frame_stack,
                        &mut execution.evaluation_stack,
                    );
                    *execution = crate::stack::context::ThreadContext::default();
                })
            })
            .unwrap();
        owner.serve_retired(running.complete(ExecutorResult::Exited(0)));
    });
    started.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(start_initializer_test(&mut executor), StepResult::Yield);
    release.send(()).unwrap();
    let reference = receive.recv_timeout(Duration::from_secs(30)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while completion.phase() != "retired" {
        assert!(Instant::now() < deadline, "worker failed to retire");
        std::thread::yield_now();
    }
    executor.perform_full_gc().unwrap();
    executor.perform_full_gc().unwrap();
    let result = resume(&mut executor);
    assert!(
        matches!(result, StepResult::MethodThrew(ref exception) if exception.type_name == "System.TypeInitializationException"),
        "{result:?}"
    );
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let original = decode_reference(reference, gc);
                let cached = engine
                    .stack
                    .shared
                    .statics
                    .failed_exception(subject.clone(), &GenericLookup::default(), gc)
                    .unwrap();
                let inner = cached.as_object(|object| {
                    object
                        .instance_storage
                        .field::<ObjectRef>(exception_type, "_innerException")
                        .unwrap()
                        .read()
                });
                assert_eq!(inner, original);
            })
        })
        .unwrap();
}

#[test]
fn native_capture_initializer_does_not_wrap_before_reply() {
    let (mut executor, ty) = boundary_executor(false);
    boundary_start_with(&mut executor, capturing_boundary_body);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame_mut()
                    .state
                    .info_handle
                    .is_cctor = true;
            })
        })
        .unwrap();
    assert_eq!(
        boundary_throw(&mut executor, ty, vec![], vec![]),
        StepResult::Yield
    );
    assert_eq!(resume(&mut executor), StepResult::Yield);
    assert_eq!(BOUNDARY_RESUMES.with(|count| count.get()), 1);
    let StepResult::MethodThrew(exception) = resume(&mut executor) else {
        panic!("expected the captured exception");
    };
    assert_eq!(exception.type_name, "System.Exception");
}

fn capturing_success_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let target = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let source = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            vm.pop_string()?
        };
        let result = fx
            .try_call(dotnet_vm_ops::effects::NativeCall {
                method: target,
                generics: GenericLookup::default(),
                arguments: Vec::new(),
            })
            .await?;
        assert!(matches!(
            result,
            dotnet_vm_ops::effects::NativeValue::Value(_)
        ));
        BOUNDARY_RESUMES.with(|count| count.set(count.get() + 1));
        let mut chars = Vec::new();
        let mut vm = fx.vm().await?;
        vm.string(&source).copy_chunk(0, 3, &mut chars)?;
        vm.push_new_string(chars)?;
        Ok(())
    })
}

#[test]
fn native_capture_child_handler_returns_normally() {
    use dotnet_vm_data::exceptions::HandlerKind;
    let (mut executor, ty) = boundary_executor(false);
    boundary_start_with(&mut executor, capturing_success_body);
    let child = vec![boundary_section(
        HandlerKind::Catch(ty.clone().into()),
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty, child, vec![]),
        StepResult::Exception
    );
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                assert_eq!(ctx.frame_stack.len(), 2);
                ctx.pop();
                ctx.push(StackValue::Int32(0));
                assert_eq!(ctx.handle_return(), StepResult::Yield);
            })
        })
        .unwrap();
    assert_eq!(resume(&mut executor), StepResult::Continue);
    assert_eq!(BOUNDARY_RESUMES.with(|count| count.get()), 1);
}

#[test]
fn native_capture_child_catch_preserves_interrupted_handler() {
    use dotnet_vm_data::{HandlerAddress, UnwindState, UnwindTarget, exceptions::HandlerKind};
    let (mut executor, ty) = boundary_executor(false);
    fn ambient<'gc>() -> UnwindState<'gc> {
        UnwindState {
            exception: None,
            target: UnwindTarget::Instruction(2),
            cursor: HandlerAddress {
                frame_index: 0,
                section_index: 0,
                handler_index: 0,
            },
            nested_filter: None,
        }
    }
    executor
        .with_arena(|arena| {
            arena.mutate_root(|_, engine| {
                engine.stack.execution.exception_mode = ExceptionState::ExecutingHandler(ambient());
                engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame_mut()
                    .state
                    .ip = 1;
            })
        })
        .unwrap();
    boundary_start_with(&mut executor, capturing_success_body);
    let child = vec![boundary_section(
        HandlerKind::Catch(ty.clone().into()),
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(
            &mut executor,
            ty,
            child,
            vec![boundary_section(HandlerKind::Finally, 0..1, 1..3)],
        ),
        StepResult::Exception
    );
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                ctx.pop();
                ctx.push(StackValue::Int32(0));
                assert_eq!(ctx.handle_return(), StepResult::Yield);
                assert_eq!(
                    *ctx.exception_mode,
                    ExceptionState::ExecutingHandler(ambient())
                );
            })
        })
        .unwrap();
    assert_eq!(resume(&mut executor), StepResult::Continue);
}

fn capturing_synchronous_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let method = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let (source, start, length) = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            let source = vm.pop_string()?;
            let start = vm.with(|region| region.root(StackValue::Int32(-1)))??;
            let length = vm.with(|region| region.root(StackValue::Int32(1)))??;
            (source, start, length)
        };
        let Err(dotnet_vm_ops::effects::NativeFailure::Managed(exception)) = fx
            .try_call(dotnet_vm_ops::effects::NativeCall {
                method,
                generics: GenericLookup::default(),
                arguments: vec![source, start, length],
            })
            .await
        else {
            panic!("expected captured synchronous exception")
        };
        BOUNDARY_RESUMES.with(|count| count.set(count.get() + 1));
        Err(dotnet_vm_ops::effects::NativeFailure::Managed(exception))
    })
}

#[test]
fn native_capture_synchronous_throw_precedes_outer_filter() {
    use dotnet_vm_data::exceptions::HandlerKind;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let (mut executor, _) = boundary_executor(false);
    let method = substring_method(&executor, true);
    CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(method.clone()));
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                ctx.frame_stack
                    .current_frame_mut()
                    .state
                    .info_handle
                    .exceptions = vec![Arc::new(boundary_section(
                    HandlerKind::Filter { clause_offset: 1 },
                    0..1,
                    1..2,
                ))];
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    capturing_synchronous_body,
                )
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Exception);
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Yield);
    assert_eq!(BOUNDARY_RESUMES.with(|count| count.get()), 0);
    executor.perform_full_gc().unwrap();
    assert_eq!(resume(&mut executor), StepResult::Exception);
    assert_eq!(BOUNDARY_RESUMES.with(|count| count.get()), 1);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    dotnet_vm_data::ExceptionState::Filtering(_)
                ));
            })
        })
        .unwrap();
}

fn capturing_handled_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    let target = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let source = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            vm.pop_string()?
        };
        assert!(matches!(
            fx.try_call(dotnet_vm_ops::effects::NativeCall {
                method: target,
                generics: GenericLookup::default(),
                arguments: Vec::new(),
            })
            .await,
            Err(dotnet_vm_ops::effects::NativeFailure::Managed(_))
        ));
        BOUNDARY_RESUMES.with(|count| count.set(count.get() + 1));
        let mut chars = Vec::new();
        let mut vm = fx.vm().await?;
        vm.string(&source).copy_chunk(0, 3, &mut chars)?;
        vm.push_new_string(chars)?;
        Ok(())
    })
}

#[test]
fn native_capture_inside_filter_restores_outer_exception() {
    use crate::stack::ops::ExceptionOps;
    use dotnet_vm_data::exceptions::HandlerKind;
    use dotnet_vm_ops::{
        MemoryOps,
        effects::{NativeDispatchMode, NativeIntrinsicHost},
    };
    let (mut executor, ty) = boundary_executor(false);
    boundary_start(&mut executor);
    let parent = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty.clone(), vec![], parent),
        StepResult::Exception
    );
    let method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                assert!(matches!(*ctx.exception_mode, ExceptionState::Filtering(_)));
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    capturing_handled_body,
                )
            })
        })
        .unwrap();
    assert!(
        matches!(result, StepResult::NativeSuspended(_)),
        "{result:?}"
    );
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let exception = ctx.new_object(ty).unwrap();
                let exception = ctx.alloc_obj_ref(gc, exception);
                ctx.throw(exception)
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Yield);
    executor.perform_full_gc().unwrap();
    assert_eq!(resume(&mut executor), StepResult::Continue);
    assert_eq!(BOUNDARY_RESUMES.with(|count| count.get()), 1);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                assert!(matches!(*ctx.exception_mode, ExceptionState::Filtering(_)));
                ctx.pop();
                assert_eq!(ctx.endfilter(1), StepResult::Exception);
            })
        })
        .unwrap();
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    assert_eq!(BOUNDARY_DROPS.with(|count| count.get()), 1);
}

#[test]
fn native_boundary_unhandled_runs_child_cleanup() {
    use dotnet_vm_data::exceptions::HandlerKind;
    let (mut executor, ty) = boundary_executor(true);
    boundary_start(&mut executor);
    let sections = vec![
        boundary_section(HandlerKind::Finally, 0..1, 1..2),
        boundary_section(HandlerKind::Fault, 0..2, 2..3),
    ];
    assert_eq!(
        boundary_throw(&mut executor, ty, sections, vec![]),
        StepResult::Exception
    );
    for ip in [1, 2] {
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    assert!(matches!(
                        engine.stack.execution.exception_mode,
                        dotnet_vm_data::ExceptionState::ExecutingHandler(_)
                    ));
                    assert_eq!(
                        engine.stack.execution.frame_stack.current_frame().state.ip,
                        ip
                    );
                    assert_eq!(engine.stack.execution.call_state.native.len(), 1);
                })
            })
            .unwrap();
        assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
        let result = boundary_handle(&mut executor, true);
        if ip == 1 {
            assert_eq!(result, StepResult::Exception);
        } else {
            let StepResult::MethodThrew(exception) = result else {
                panic!("{result:?}")
            };
            let trace = exception.stack_trace.unwrap();
            assert_eq!(trace.matches("[Native 1]").count(), 1, "{trace}");
            assert!(
                trace.contains("Substring") && trace.contains("in IP 0"),
                "{trace}"
            );
        }
    }
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
    assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 0);
}

#[test]
fn native_boundary_outer_handler_runs_after_child_cleanup() {
    use dotnet_vm_data::exceptions::HandlerKind;
    let (mut executor, ty) = boundary_executor(true);
    boundary_start(&mut executor);
    let child = vec![
        boundary_section(HandlerKind::Finally, 0..1, 1..2),
        boundary_section(HandlerKind::Fault, 0..2, 2..3),
    ];
    let parent = vec![boundary_section(
        HandlerKind::Catch(ty.clone().into()),
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty, child, parent),
        StepResult::Exception
    );
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    for ip in [1, 2] {
        executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| {
                    assert_eq!(engine.stack.execution.frame_stack.len(), 2);
                    assert_eq!(
                        engine.stack.execution.frame_stack.current_frame().state.ip,
                        ip
                    );
                })
            })
            .unwrap();
        assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
        assert_eq!(boundary_handle(&mut executor, true), StepResult::Exception);
    }
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert_eq!(
                    engine.stack.execution.frame_stack.current_frame().state.ip,
                    1
                );
                assert!(engine.stack.execution.call_state.native.is_empty());
            })
        })
        .unwrap();
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
    assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 0);
}

#[test]
fn native_boundary_inner_handler_keeps_parent_alive() {
    use dotnet_vm_data::exceptions::HandlerKind;
    let (mut executor, ty) = boundary_executor(true);
    boundary_start(&mut executor);
    let child = vec![boundary_section(
        HandlerKind::Catch(ty.clone().into()),
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty, child, vec![]),
        StepResult::Exception
    );
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                assert_eq!(ctx.frame_stack.len(), 2);
                assert_eq!(ctx.call_state.native.len(), 1);
                ctx.pop();
                ctx.push(StackValue::Int32(0));
                assert_eq!(ctx.handle_return(), StepResult::Yield);
            })
        })
        .unwrap();
    assert_eq!(resume(&mut executor), StepResult::Continue);
    assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 1);
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
}

#[test]
fn native_boundary_filter_search_precedes_cleanup() {
    use crate::stack::ops::ExceptionOps;
    use dotnet_vm_data::exceptions::HandlerKind;
    let (mut executor, ty) = boundary_executor(true);
    boundary_start(&mut executor);
    let child = vec![boundary_section(HandlerKind::Finally, 0..1, 1..2)];
    let parent = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty, child, parent),
        StepResult::Exception
    );
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    dotnet_vm_data::ExceptionState::Filtering(_)
                ));
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
                assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 0);
                assert_eq!(engine.ves_context(gc).endfilter(1), StepResult::Exception);
            })
        })
        .unwrap();
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
    assert_eq!(boundary_handle(&mut executor, true), StepResult::Exception);
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
    assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 0);
}

#[test]
fn native_boundary_stack_trace_and_cleanup_for_nested_calls() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let (mut executor, ty) = boundary_executor(true);
    boundary_start(&mut executor);
    let method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    boundary_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(result, StepResult::NativeSuspended(_)));
    let result = boundary_throw(&mut executor, ty, vec![], vec![]);
    let StepResult::MethodThrew(exception) = result else {
        panic!("{result:?}")
    };
    let trace = exception.stack_trace.unwrap();
    let first = trace.find("[Native 2]").unwrap();
    let second = trace.find("[Native 1]").unwrap();
    assert!(first < second, "{trace}");
    assert_eq!(trace.matches("[Native ").count(), 2, "{trace}");
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 2);
    assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 0);
}

#[test]
fn native_boundary_survives_exception_in_outer_filter() {
    use crate::stack::ops::ExceptionOps;
    use dotnet_vm_data::exceptions::{Handler, HandlerKind, ProtectedSection};
    use dotnet_vm_ops::MemoryOps;
    let (mut executor, ty) = boundary_executor(true);
    boundary_start(&mut executor);
    let parent = vec![ProtectedSection {
        instructions: 0..1,
        handlers: vec![
            Handler {
                instructions: 1..2,
                kind: HandlerKind::Filter { clause_offset: 1 },
            },
            Handler {
                instructions: 1..2,
                kind: HandlerKind::Catch(ty.clone().into()),
            },
        ],
    }];
    assert_eq!(
        boundary_throw(&mut executor, ty.clone(), vec![], parent),
        StepResult::Exception
    );
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let mut info = ctx.frame_stack.current_frame().state.info_handle.clone();
                info.exceptions.clear();
                let top = ctx.top_of_stack();
                ctx.frame_stack.push_frame(
                    BasePointer {
                        arguments: top,
                        locals: top,
                        stack: top,
                    },
                    info,
                    Default::default(),
                    Default::default(),
                );
                let exception = ctx.new_object(ty).unwrap();
                let exception = ctx.alloc_obj_ref(gc, exception);
                ctx.throw(exception)
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Exception);
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
    assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 0);
}

#[test]
fn native_boundary_cleanup_survives_collection() {
    use dotnet_vm_data::exceptions::HandlerKind;
    let (mut executor, ty) = boundary_executor(false);
    boundary_start(&mut executor);
    let child = vec![boundary_section(HandlerKind::Finally, 0..1, 1..2)];
    let parent = vec![boundary_section(
        HandlerKind::Catch(ty.clone().into()),
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty, child, parent),
        StepResult::Exception
    );
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    executor.perform_full_gc().unwrap();
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
    assert_eq!(boundary_handle(&mut executor, true), StepResult::Exception);
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
    assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 0);
}

fn boundary_synchronous_throw_body(
    mut fx: dotnet_vm_ops::effects::VmEffects,
) -> dotnet_vm_ops::effects::NativeFuture {
    struct Witness;
    impl Drop for Witness {
        fn drop(&mut self) {
            BOUNDARY_DROPS.with(|count| count.set(count.get() + 1));
        }
    }
    let method = CALL_TARGET.with(|slot| slot.borrow().as_ref().unwrap().clone());
    Box::pin(async move {
        let _witness = Witness;
        let (source, start, length) = {
            let mut vm = fx.vm().await?;
            vm.discard_scalars(2)?;
            let source = vm.pop_string()?;
            let start = vm.with(|region| region.root(StackValue::Int32(-1)))??;
            let length = vm.with(|region| region.root(StackValue::Int32(1)))??;
            (source, start, length)
        };
        fx.call(dotnet_vm_ops::effects::NativeCall {
            method,
            generics: GenericLookup::default(),
            arguments: vec![source, start, length],
        })
        .await?;
        BOUNDARY_RESUMES.with(|count| count.set(count.get() + 1));
        Ok(())
    })
}

#[test]
fn native_boundary_synchronous_throw_reaches_caller_handler() {
    use dotnet_vm_data::exceptions::HandlerKind;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let (mut executor, _) = boundary_executor(false);
    let ty = executor
        .shared
        .loader
        .corlib_type("System.ArgumentOutOfRangeException")
        .unwrap();
    let method = substring_method(&executor, true);
    CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(method.clone()));
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                ctx.frame_stack
                    .current_frame_mut()
                    .state
                    .info_handle
                    .exceptions = vec![Arc::new(boundary_section(
                    HandlerKind::Catch(ty.into()),
                    0..1,
                    1..2,
                ))];
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    boundary_synchronous_throw_body,
                )
            })
        })
        .unwrap();
    assert_eq!(result, StepResult::Exception);
    for _ in 0..4 {
        let result = resume(&mut executor);
        assert_eq!(result, StepResult::Exception);
        if BOUNDARY_DROPS.with(|x| x.get()) == 1 {
            break;
        }
    }
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
    assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 0);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert_eq!(
                    engine.stack.execution.frame_stack.current_frame().state.ip,
                    1
                );
                assert!(engine.stack.execution.call_state.native.is_empty());
            })
        })
        .unwrap();
}

#[test]
fn native_call_in_filter_does_not_reply_to_suspended_call() {
    use dotnet_vm_data::exceptions::HandlerKind;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let (mut executor, ty) = boundary_executor(true);
    boundary_start(&mut executor);
    let parent = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty, vec![], parent),
        StepResult::Exception
    );
    let method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_managed_call_body,
                )
            })
        })
        .unwrap();
    assert!(
        matches!(result, StepResult::NativeSuspended(_)),
        "{result:?}"
    );
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let active = &engine.stack.execution.call_state.native;
                assert_eq!(active.len(), 1);
                assert!(active[0].parent_destination.is_none());
                assert_eq!(engine.stack.execution.call_state.suspended_native.len(), 1);
            })
        })
        .unwrap();
    collect(&mut executor);
    for _ in 0..16 {
        let result = resume(&mut executor);
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
        if executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| engine.stack.execution.call_state.native.is_empty())
            })
            .unwrap()
        {
            break;
        }
    }
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                use crate::stack::ops::ExceptionOps;
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert_eq!(engine.stack.execution.call_state.suspended_native.len(), 1);
                assert_eq!(engine.ves_context(gc).endfilter(1), StepResult::Exception);
                assert_eq!(engine.stack.execution.call_state.native.len(), 1);
                assert!(
                    engine
                        .stack
                        .execution
                        .call_state
                        .suspended_native
                        .is_empty()
                );
            })
        })
        .unwrap();
    assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 0);
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
}

fn native_replacement_target(executor: &Executor, tail: bool) -> MethodDescription {
    use dotnetdll::resolved::members::{MethodSource, UserMethod};
    let (mut res, _, ty) = make_test_assembly!("NativeTransfer.dll", "NativeTransfer", "Transfer");
    res[ty].flags.before_field_init = true;
    let sig = parameterless_i32_signature();
    let target = res.push_method(ty, Method::new(Accessibility::Public, sig.clone(), "Target", Some(make_test_method!(max_stack: 1, instructions: vec![Instruction::LoadConstantInt32(0), Instruction::Return]))));
    let source = MethodSource::User(UserMethod::Definition(target));
    let instructions = if tail {
        vec![
            Instruction::Call {
                tail_call: true,
                param0: source,
            },
            Instruction::Return,
        ]
    } else {
        vec![Instruction::Jump(source)]
    };
    let method = res.push_method(
        ty,
        Method::new(
            Accessibility::Public,
            sig,
            "Transfer",
            Some(make_test_method!(max_stack: 1, instructions: instructions)),
        ),
    );
    let res = executor.shared.loader.register_owned_assembly(res);
    let desc = TypeDescription::new(res.clone(), ty);
    let index = desc
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &res.definition()[method]))
        .unwrap();
    MethodDescription::new(
        desc,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(index),
    )
}

fn native_replacement_check(tail: bool) {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let (mut executor, _) = boundary_executor(false);
    let target = native_replacement_target(&executor, tail);
    CALL_TARGET.with(|slot| *slot.borrow_mut() = Some(target));
    let method = substring_method(&executor, true);
    let first = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_managed_call_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(first, StepResult::NativeSuspended(_)), "{first:?}");
    for _ in 0..24 {
        let result = executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    engine.step(gc)
                })
            })
            .unwrap();
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
        if executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| engine.stack.execution.call_state.native.is_empty())
            })
            .unwrap()
        {
            break;
        }
    }
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.execution.call_state.native.is_empty());
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert!(matches!(
                    engine.stack.execution.evaluation_stack.stack.as_slice(),
                    [StackValue::ObjectRef(ObjectRef(Some(_)))]
                ));
            })
        })
        .unwrap();
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_child_tail_call_returns_to_native() {
    native_replacement_check(true);
}
#[test]
fn native_child_jmp_returns_to_native() {
    native_replacement_check(false);
}

fn enqueue_test_finalizer(executor: &mut Executor) {
    use dotnet_vm_ops::MemoryOps;
    let ty = executor.shared.loader.corlib_type("System.Object").unwrap();
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let ctx = engine.ves_context(gc);
                let object = ctx.new_object(ty).unwrap();
                let object = ctx.alloc_obj_ref(gc, object);
                engine
                    .stack
                    .local
                    .heap
                    .pending_finalization
                    .borrow_mut()
                    .push(object);
            })
        })
        .unwrap();
}

#[test]
fn finalizer_preserves_filter_and_suspended_native_context() {
    use dotnet_vm_data::exceptions::HandlerKind;
    let (mut executor, ty) = boundary_executor(false);
    boundary_start(&mut executor);
    let parent = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty, vec![], parent),
        StepResult::Exception
    );
    enqueue_test_finalizer(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    ExceptionState::Filtering(_)
                ));
                assert_eq!(
                    engine.process_pending_finalizers(gc),
                    StepResult::FramePushed
                );
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    ExceptionState::None
                ));
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                let caller = engine.stack.parked_finalizer_caller.as_ref().unwrap();
                assert!(matches!(
                    caller.exception_mode,
                    ExceptionState::Filtering(_)
                ));
                assert_eq!(caller.call_state.suspended_native.len(), 1);
            })
        })
        .unwrap();
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                use crate::stack::ops::ExceptionOps;
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    ExceptionState::Filtering(_)
                ));
                assert_eq!(engine.ves_context(gc).endfilter(1), StepResult::Exception);
            })
        })
        .unwrap();
    assert_eq!(boundary_handle(&mut executor, false), StepResult::Exception);
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
}

#[test]
fn finalizer_exception_cannot_reach_interrupted_handler() {
    use dotnet_vm_data::exceptions::HandlerKind;
    use dotnet_vm_ops::MemoryOps;
    let (mut executor, ty) = boundary_executor(false);
    boundary_start(&mut executor);
    let child = vec![boundary_section(HandlerKind::Finally, 0..1, 1..2)];
    assert_eq!(
        boundary_throw(&mut executor, ty.clone(), child, vec![]),
        StepResult::Exception
    );
    enqueue_test_finalizer(&mut executor);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    ExceptionState::ExecutingHandler(_)
                ));
                assert_eq!(
                    engine.process_pending_finalizers(gc),
                    StepResult::FramePushed
                );
                let mut ctx = engine.ves_context(gc);
                let exception = ctx.new_object(ty).unwrap();
                let exception = ctx.alloc_obj_ref(gc, exception);
                ctx.push(StackValue::ObjectRef(exception));
                ctx.frame_stack
                    .current_frame_mut()
                    .state
                    .info_handle
                    .instructions = &[Instruction::Throw];
                engine.step(gc)
            })
        })
        .unwrap();
    assert!(matches!(result, StepResult::MethodThrew(_)), "{result:?}");
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.parked_finalizer_caller.is_none());
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    ExceptionState::ExecutingHandler(_)
                ));
                assert_eq!(engine.stack.execution.frame_stack.len(), 2);
                assert_eq!(engine.stack.execution.call_state.native.len(), 1);
                assert!(!engine.stack.local.heap.processing_finalizer.get());
            })
        })
        .unwrap();
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
}

#[test]
fn finalizer_segment_resolves_parked_homes_and_preserves_serials() {
    use dotnet_value::pointer::{ManagedPtrResolver, frame_home_window};
    let (mut executor, _) = boundary_executor(false);
    enqueue_test_finalizer(&mut executor);
    let serial = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let frames = &mut engine.stack.execution.frame_stack;
                let serial = frames.issue_home_serial();
                let block = dotnet_value::layout::FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 8,
                    alignment: 1,
                    gc_desc: Default::default(),
                    has_ref_fields: false,
                };
                let layout = dotnet_vm_data::frame_homes::FrameHomeLayout::new(
                    block,
                    Box::new([]),
                    Box::new([]),
                );
                frames.current_frame_mut().homes =
                    dotnet_vm_data::frame_homes::FrameHomes::allocate(Arc::new(layout), serial)
                        .unwrap();
                assert_eq!(
                    engine.process_pending_finalizers(gc),
                    StepResult::FramePushed
                );
                let next = engine.stack.execution.frame_stack.issue_home_serial();
                assert!(next.get() > serial.get());
                let ctx = engine.ves_context(gc);
                assert!(ctx.frame_home_block(serial).is_some());
                assert!(frame_home_window(&ctx, serial, 0, 8).is_ok());
                assert!(frame_home_window(&ctx, serial, 8, 1).is_err());
                (serial, next)
            })
        })
        .unwrap();
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let next = engine.stack.execution.frame_stack.issue_home_serial();
                assert!(next.get() > serial.1.get());
                assert!(engine.ves_context(gc).frame_home_block(serial.0).is_some());
            })
        })
        .unwrap();
}

#[test]
fn native_call_in_handler_preserves_ambient_unwind() {
    use dotnet_vm_data::exceptions::HandlerKind;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let (mut executor, ty) = boundary_executor(false);
    boundary_start(&mut executor);
    let child = vec![boundary_section(HandlerKind::Finally, 0..1, 1..2)];
    assert_eq!(
        boundary_throw(&mut executor, ty, child, vec![]),
        StepResult::Exception
    );
    let method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    ExceptionState::ExecutingHandler(_)
                ));
                let mut ctx = engine.ves_context(gc);
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    native_managed_call_body,
                )
            })
        })
        .unwrap();
    assert!(
        matches!(result, StepResult::NativeSuspended(_)),
        "{result:?}"
    );
    collect(&mut executor);
    for _ in 0..16 {
        let result = resume(&mut executor);
        assert!(!matches!(result, StepResult::Error(_)), "{result:?}");
        if executor
            .with_arena(|arena| {
                arena.mutate(|_, engine| engine.stack.execution.call_state.native.len() == 1)
            })
            .unwrap()
        {
            break;
        }
    }
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                use crate::stack::ops::ExceptionOps;
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    ExceptionState::ExecutingHandler(_)
                ));
                assert_eq!(engine.stack.execution.call_state.native.len(), 1);
                assert_eq!(
                    engine.ves_context(gc).endfinally(),
                    StepResult::MethodThrew(dotnet_vm_data::exceptions::ManagedException {
                        type_name: "System.Exception".into(),
                        message: None,
                        stack_trace: None
                    })
                );
            })
        })
        .unwrap();
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
}

#[test]
fn native_activations_follow_nested_filter_segments() {
    use crate::stack::ops::ExceptionOps;
    use dotnet_vm_data::exceptions::HandlerKind;
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let (mut executor, ty) = boundary_executor(false);
    boundary_start(&mut executor);
    let outer = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty.clone(), vec![], outer),
        StepResult::Exception
    );
    let method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    boundary_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(result, StepResult::NativeSuspended(_)));
    // A filter cannot contain protected regions (ECMA-335 I.12.4.2.7), so the nested filter
    // belongs to the method the first filter called, and a second native call below it throws.
    let nested = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let ctx = engine.ves_context(gc);
                let frames = &mut ctx.frame_stack.frames;
                assert_eq!(frames.len(), 2);
                frames[1].state.info_handle.exceptions = nested.into_iter().map(Arc::new).collect();
            })
        })
        .unwrap();
    let inner_method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    inner_method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    boundary_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(result, StepResult::NativeSuspended(_)));
    let outer = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty, vec![], outer),
        StepResult::Exception
    );
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let calls = &engine.stack.execution.call_state;
                // The first filter parked A; the nested filter parks the call below its owner (C).
                // B, the call the first filter made, stays active.
                assert_eq!(calls.suspended_native.len(), 2);
                assert_eq!(calls.native.len(), 1);
                assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
            })
        })
        .unwrap();
    collect(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert_eq!(engine.ves_context(gc).endfilter(0), StepResult::Exception);
                assert_eq!(engine.stack.execution.call_state.suspended_native.len(), 1);
                assert_eq!(engine.stack.execution.call_state.native.len(), 2);
                assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
            })
        })
        .unwrap();
    assert!(matches!(
        boundary_handle(&mut executor, false),
        StepResult::MethodThrew(_)
    ));
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 3);
    assert_eq!(BOUNDARY_RESUMES.with(|x| x.get()), 0);
}

#[test]
fn abort_retires_suspended_native_frames() {
    use dotnet_vm_data::exceptions::HandlerKind;
    let (mut executor, ty) = boundary_executor(false);
    boundary_start(&mut executor);
    let parent = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty, vec![], parent),
        StepResult::Exception
    );
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let execution = &engine.stack.execution;
                assert!(execution.call_state.suspended_native.is_empty());
                assert!(execution.frame_stack.suspended_frames.is_empty());
                assert!(execution.frame_stack.active_filters.is_empty());
                assert!(execution.evaluation_stack.suspended_stack.is_empty());
            })
        })
        .unwrap();
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
}

#[test]
fn finalizer_native_ids_remain_monotonic_across_segments() {
    use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost};
    let (mut executor, _) = boundary_executor(false);
    boundary_start(&mut executor);
    enqueue_test_finalizer(&mut executor);
    let method = substring_method(&executor, true);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert_eq!(
                    engine.process_pending_finalizers(gc),
                    StepResult::FramePushed
                );
                let mut ctx = engine.ves_context(gc);
                ctx.frame_stack
                    .current_frame_mut()
                    .state
                    .info_handle
                    .instructions = &[Instruction::NoOperation, Instruction::Return];
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                assert_eq!(
                    ctx.start_native(
                        method,
                        NativeDispatchMode::InlineCall,
                        &GenericLookup::default(),
                        None,
                        native_test_fallback,
                        native_test_body
                    ),
                    StepResult::Yield
                );
                assert_eq!(ctx.call_state.last_native_id, 2);
            })
        })
        .unwrap();
    collect(&mut executor);
    assert_eq!(resume(&mut executor), StepResult::Continue);
    // The test body returns a string, so discard it before the void finalizer returns.
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.ves_context(gc).pop();
            })
        })
        .unwrap();
    assert_eq!(resume(&mut executor), StepResult::Continue);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.parked_finalizer_caller.is_none());
                assert_eq!(engine.stack.execution.call_state.last_native_id, 2);
                assert_eq!(engine.stack.execution.call_state.native.len(), 1);
            })
        })
        .unwrap();
}

#[test]
fn finalizer_exception_cannot_enter_interrupted_filter() {
    use dotnet_vm_data::exceptions::HandlerKind;
    use dotnet_vm_ops::MemoryOps;
    let (mut executor, ty) = boundary_executor(false);
    boundary_start(&mut executor);
    let parent = vec![boundary_section(
        HandlerKind::Filter { clause_offset: 1 },
        0..1,
        1..2,
    )];
    assert_eq!(
        boundary_throw(&mut executor, ty.clone(), vec![], parent),
        StepResult::Exception
    );
    enqueue_test_finalizer(&mut executor);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert_eq!(
                    engine.process_pending_finalizers(gc),
                    StepResult::FramePushed
                );
                let mut ctx = engine.ves_context(gc);
                let exception = ctx.new_object(ty).unwrap();
                let exception = ctx.alloc_obj_ref(gc, exception);
                ctx.push(StackValue::ObjectRef(exception));
                ctx.frame_stack
                    .current_frame_mut()
                    .state
                    .info_handle
                    .instructions = &[Instruction::Throw];
                engine.step(gc)
            })
        })
        .unwrap();
    assert!(matches!(result, StepResult::MethodThrew(_)), "{result:?}");
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(matches!(
                    engine.stack.execution.exception_mode,
                    ExceptionState::Filtering(_)
                ));
                assert_eq!(engine.stack.execution.call_state.suspended_native.len(), 1);
                assert_eq!(engine.stack.execution.frame_stack.suspended_frames.len(), 1);
                assert!(!engine.stack.local.heap.processing_finalizer.get());
            })
        })
        .unwrap();
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 0);
}

#[test]
fn finalizer_after_entrypoint_preserves_exit_value() {
    let (mut executor, _) = boundary_executor(false);
    enqueue_test_finalizer(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.stack.execution.frame_stack.clear();
                engine
                    .stack
                    .execution
                    .evaluation_stack
                    .push(StackValue::Int32(42));
                assert_eq!(
                    engine.process_pending_finalizers(gc),
                    StepResult::FramePushed
                );
                assert_eq!(engine.step(gc), StepResult::Continue);
                assert!(engine.stack.execution.frame_stack.is_empty());
                assert!(matches!(
                    engine.stack.execution.evaluation_stack.stack.as_slice(),
                    [StackValue::Int32(42)]
                ));
                assert_eq!(engine.step(gc), StepResult::Return);
            })
        })
        .unwrap();
}

#[test]
fn abort_drops_suspended_children_before_active_parent() {
    use crate::stack::ops::ExceptionOps;
    use dotnet_vm_data::exceptions::HandlerKind;
    use dotnet_vm_ops::{
        MemoryOps,
        effects::{NativeDispatchMode, NativeIntrinsicHost},
    };
    let (mut executor, ty) = boundary_executor(false);
    boundary_start(&mut executor);
    let method = substring_method(&executor, true);
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let source = ObjectRef::new(gc, HeapStorage::Str(CLRString::from("abc")));
                ctx.push(StackValue::ObjectRef(source));
                ctx.push(StackValue::Int32(0));
                ctx.push(StackValue::Int32(3));
                ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    native_test_fallback,
                    boundary_body,
                )
            })
        })
        .unwrap();
    assert!(matches!(result, StepResult::NativeSuspended(_)));
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                ctx.frame_stack.frames[1].state.info_handle.exceptions = vec![Arc::new(
                    boundary_section(HandlerKind::Filter { clause_offset: 1 }, 0..1, 1..2),
                )];
                let exception = ctx.new_object(ty).unwrap();
                let exception = ctx.alloc_obj_ref(gc, exception);
                assert_eq!(ctx.throw(exception), StepResult::Exception);
                assert_eq!(ctx.call_state.native.len(), 1);
                assert_eq!(ctx.call_state.suspended_native.len(), 1);
            })
        })
        .unwrap();
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    BOUNDARY_ORDER.with(|order| assert_eq!(&*order.borrow(), &[2, 1]));
}

#[test]
fn abort_cleans_finalizer_and_parked_caller() {
    let (mut executor, _) = boundary_executor(false);
    boundary_start(&mut executor);
    enqueue_test_finalizer(&mut executor);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains rooted for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                assert_eq!(
                    engine.process_pending_finalizers(gc),
                    StepResult::FramePushed
                );
                assert!(engine.stack.local.heap.processing_finalizer.get());
            })
        })
        .unwrap();
    executor.shared.abort_signal().request_abort();
    assert!(matches!(executor.run(), ExecutorResult::Error(_)));
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                assert!(engine.stack.parked_finalizer_caller.is_none());
                assert!(!engine.stack.local.heap.processing_finalizer.get());
                assert!(engine.stack.execution.call_state.native.is_empty());
            })
        })
        .unwrap();
    assert_eq!(BOUNDARY_DROPS.with(|x| x.get()), 1);
}

#[path = "condition_wait_tests.rs"]
mod condition_wait_tests;

#[cfg(feature = "multithreading")]
#[test]
fn managed_join_returns_before_native_worker_exits() {
    use dotnet_intrinsics_threading::ManagedThreadHost;
    use std::time::{Duration, Instant};

    let mut executor = executor();
    let shared = executor.shared.clone();
    let (release, held) = std::sync::mpsc::channel();
    let (ready, published) = std::sync::mpsc::channel();
    let completion = shared.workers.spawn_with_completion_hook(
        || ExecutorResult::Exited(0),
        move || {
            ready.send(()).unwrap();
            let _ = held.recv_timeout(Duration::from_secs(5));
        },
    );
    published.recv_timeout(Duration::from_secs(5)).unwrap();
    let start = Instant::now();
    let result = executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — this engine is the live root throughout the mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let thread = ObjectRef::new(gc, HeapStorage::Str(CLRString::new(vec![])));
                let ptr = thread.as_ptr().unwrap();
                engine.stack.shared.workers.register_managed_thread(
                    ptr,
                    crate::threading::workers::ManagedThreadMetadata {
                        id: 1,
                        delegate: None,
                        parameterized: false,
                        started: true,
                        background: false,
                        completion_index: Some(completion.index()),
                        retained: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
                    },
                );
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::ObjectRef(thread));
                ctx.threading_join_managed_thread(None)
            })
        })
        .unwrap();
    let elapsed = start.elapsed();
    let released = release.send(()).is_ok();
    drop(executor);
    shared.shutdown_workers();
    assert!(matches!(result, StepResult::Continue));
    assert!(released, "native worker exited before Join returned");
    assert!(
        elapsed < Duration::from_secs(4),
        "Join waited for the native worker"
    );
}

#[path = "native_return_tests.rs"]
mod native_return_tests;

#[test]
fn native_error_retires_initializer_frame_before_future_drop() {
    use dotnet_utils::ByteOffset;
    use dotnet_value::{
        ManagedPtr,
        layout::{FieldLayoutManager, GcDesc},
        object::Object as ObjectInstance,
        storage::FieldStorage,
    };
    use std::{cell::Cell, rc::Rc};
    struct Witness {
        inner: dotnet_vm_ops::effects::NativeFuture,
        layout: Arc<FieldLayoutManager>,
        retired: Rc<Cell<Option<bool>>>,
    }
    impl std::future::Future for Witness {
        type Output = dotnet_vm_ops::effects::NativeResult;
        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            self.inner.as_mut().poll(cx)
        }
    }
    impl Drop for Witness {
        fn drop(&mut self) {
            // The child's transient stack value holds the only other layout reference.
            self.retired.set(Some(Arc::strong_count(&self.layout) == 1));
        }
    }
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let subject = initializer_test_subject(&executor, "RetireErrorSubject");
    assert!(matches!(
        start_initializer_test(&mut executor),
        StepResult::NativeSuspended(_)
    ));
    let retired = Rc::new(Cell::new(None));
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root during this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let execution = &mut engine.stack.execution;
                assert!(
                    execution
                        .frame_stack
                        .current_frame()
                        .state
                        .info_handle
                        .is_cctor
                );
                let layout = Arc::new(FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 1,
                    alignment: 1,
                    gc_desc: GcDesc::default(),
                    has_ref_fields: false,
                });
                let obj = ObjectInstance::new(
                    TypeDescription::NULL,
                    GenericLookup::default(),
                    FieldStorage::new(layout.clone(), vec![0]),
                );
                let base =
                    obj.with_data(|data| std::ptr::NonNull::new(data.as_ptr().cast_mut()).unwrap());
                execution.evaluation_stack.push(StackValue::ManagedPtr(
                    ManagedPtr::new_transient(
                        Some(base),
                        TypeDescription::NULL,
                        obj,
                        ByteOffset::new(0),
                    )
                    .into(),
                ));
                let work = execution.call_state.native.last_mut().unwrap();
                let inner = work.owner.future.take().unwrap();
                work.owner.future = Some(Box::pin(Witness {
                    inner,
                    layout,
                    retired: retired.clone(),
                }));
                assert!(matches!(
                    engine.ves_context(gc).advance_native_effect(),
                    StepResult::Error(_)
                ));
                assert_eq!(
                    retired.get(),
                    Some(true),
                    "child-owned stack value must be released before future drop"
                );
                assert_eq!(engine.stack.execution.frame_stack.len(), 1);
                assert!(engine.stack.execution.call_state.native.is_empty());
            })
        })
        .unwrap();
    assert_eq!(
        executor
            .shared
            .statics
            .get_init_state(subject, &GenericLookup::default()),
        crate::statics::InitState::Failed
    );
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn native_future_owner_revokes_control_before_drop() {
    use dotnet_vm_ops::effects::{NativeControl, NativeInvocation, VmEffects};
    use std::{
        cell::Cell,
        future::Future,
        rc::Rc,
        task::{Context, Poll, Waker},
    };
    struct Probe {
        effects: VmEffects,
        rejected: Rc<Cell<bool>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            let request = self.effects.yield_now();
            let mut request = std::pin::pin!(request);
            let mut cx = Context::from_waker(Waker::noop());
            self.rejected.set(matches!(
                request.as_mut().poll(&mut cx),
                Poll::Ready(Err(_))
            ));
        }
    }
    let control = NativeControl::new();
    let rejected = Rc::new(Cell::new(false));
    let probe = Probe {
        effects: control
            .effects(NativeInvocation {
                start: 0,
                length: 0,
                parameter_count: 0,
            })
            .unwrap(),
        rejected: rejected.clone(),
    };
    let future = Box::pin(async move {
        let _probe = probe;
        std::future::pending::<()>().await;
        Ok(())
    });
    let owner = crate::stack::context::NativeFutureOwner {
        control,
        future: Some(future),
    };
    drop(owner);
    assert!(rejected.get(), "future destructor must see revoked control");
}

#[cfg(feature = "multithreading")]
#[test]
fn worker_teardown_cleans_native_before_retiring_caller() {
    use dotnet_utils::ByteOffset;
    use dotnet_value::{
        ManagedPtr,
        layout::{FieldLayoutManager, GcDesc},
        object::Object as ObjectInstance,
        storage::FieldStorage,
    };
    use std::{
        sync::{
            Arc as StdArc,
            atomic::{AtomicUsize, Ordering as AtomicOrdering},
        },
        time::Duration,
    };
    struct Witness {
        inner: dotnet_vm_ops::effects::NativeFuture,
        layout: Arc<FieldLayoutManager>,
        result: StdArc<AtomicUsize>,
    }
    impl std::future::Future for Witness {
        type Output = dotnet_vm_ops::effects::NativeResult;
        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            self.inner.as_mut().poll(cx)
        }
    }
    impl Drop for Witness {
        fn drop(&mut self) {
            // The caller's transient stack value must outlive native cleanup.
            self.result.store(
                if Arc::strong_count(&self.layout) == 2 {
                    2
                } else {
                    1
                },
                AtomicOrdering::SeqCst,
            );
        }
    }
    let (executor, _) = boundary_executor_with_initializer(false, true);
    let method = executor
        .with_arena_ref(|arena| {
            arena.mutate(|_, engine| {
                engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .clone()
            })
        })
        .unwrap();
    let state = executor.shared.clone();
    drop(executor);
    let result = StdArc::new(AtomicUsize::new(0));
    let worker_result = result.clone();
    let worker_state = state.clone();
    let completion = state.workers.spawn_owner(move |running| {
        WORKER_START_TEST_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move |executor| {
                initializer_test_subject(executor, "WorkerTeardownSubject");
                let layout = Arc::new(FieldLayoutManager {
                    fields: Default::default(),
                    total_size: 1,
                    alignment: 1,
                    gc_desc: GcDesc::default(),
                    has_ref_fields: false,
                });
                executor
                    .with_arena(|arena| {
                        arena.mutate_root(|gc, engine| {
                            // SAFETY: F1.GcHandleRooted — the engine is rooted throughout this mutation.
                            let gc = unsafe { test_gc_handle(gc, engine) };
                            let obj = ObjectInstance::new(
                                TypeDescription::NULL,
                                GenericLookup::default(),
                                FieldStorage::new(layout.clone(), vec![0]),
                            );
                            let base = obj.with_data(|data| {
                                std::ptr::NonNull::new(data.as_ptr().cast_mut()).unwrap()
                            });
                            engine.ves_context(gc).push(StackValue::ManagedPtr(
                                ManagedPtr::new_transient(
                                    Some(base),
                                    TypeDescription::NULL,
                                    obj,
                                    ByteOffset::new(0),
                                )
                                .into(),
                            ));
                        })
                    })
                    .unwrap();
                assert!(matches!(
                    start_initializer_test(executor),
                    StepResult::NativeSuspended(_)
                ));
                executor
                    .with_arena(|arena| {
                        arena.mutate_root(|_, engine| {
                            let execution = &mut engine.stack.execution;
                            assert!(
                                execution
                                    .frame_stack
                                    .current_frame()
                                    .state
                                    .info_handle
                                    .is_cctor
                            );
                            let work = execution.call_state.native.last_mut().unwrap();
                            let inner = work.owner.future.take().unwrap();
                            work.owner.future = Some(Box::pin(Witness {
                                inner,
                                layout,
                                result: worker_result,
                            }));
                        })
                    })
                    .unwrap();
                ExecutorResult::Error(
                    dotnet_types::error::ExecutionError::InternalError(
                        "injected worker failure".into(),
                    )
                    .into(),
                )
            }))
        });
        Executor::run_thread_start(worker_state, method, GenericLookup::default(), running);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !completion.is_finished() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let finished = completion.is_finished();
    let outcome = completion.take_outcome();
    state.shutdown_workers();
    assert!(finished, "worker did not finish");
    assert!(matches!(outcome, Some(Ok(ExecutorResult::Error(_)))));
    assert_eq!(
        result.load(AtomicOrdering::SeqCst),
        2,
        "caller roots must survive until native future cleanup"
    );
}

#[cfg(feature = "multithreading")]
#[test]
fn legacy_initializer_host_error_releases_waiter() {
    use dotnet_vm_ops::StaticsOps;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    struct OwnerCleanup {
        release: Option<mpsc::Sender<()>>,
        state: Arc<SharedGlobalState>,
    }
    impl Drop for OwnerCleanup {
        fn drop(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            self.state.shutdown_workers();
        }
    }
    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let method = executor
        .with_arena_ref(|arena| {
            arena.mutate(|_, engine| {
                engine
                    .stack
                    .execution
                    .frame_stack
                    .current_frame()
                    .state
                    .info_handle
                    .source
                    .clone()
            })
        })
        .unwrap();
    let state = executor.shared.clone();
    let worker_state = state.clone();
    let (send, receive) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let completion = state.workers.spawn_owner(move |running| {
        WORKER_START_TEST_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move |executor| {
                let subject = initializer_test_subject(executor, "LegacyFailureSubject");
                executor
                    .with_arena(|arena| {
                        arena.mutate_root(|gc, engine| {
                            // SAFETY: F1.GcHandleRooted — the engine is rooted throughout this mutation.
                            let gc = unsafe { test_gc_handle(gc, engine) };
                            let result = engine.ves_context(gc).initialize_static_storage(
                                subject.clone(),
                                GenericLookup::default(),
                            );
                            assert!(!matches!(
                                result,
                                StepResult::Error(_) | StepResult::Exception
                            ));
                            assert!(
                                engine
                                    .stack
                                    .execution
                                    .frame_stack
                                    .current_frame()
                                    .state
                                    .info_handle
                                    .is_cctor
                            );
                            assert!(engine.stack.execution.call_state.native.is_empty());
                        })
                    })
                    .unwrap();
                send.send(subject).unwrap();
                released.recv_timeout(Duration::from_secs(30)).unwrap();
                ExecutorResult::Error(
                    dotnet_types::error::ExecutionError::InternalError(
                        "injected worker failure".into(),
                    )
                    .into(),
                )
            }));
        });
        Executor::run_thread_start(worker_state, method, GenericLookup::default(), running);
    });
    let mut owner_cleanup = OwnerCleanup {
        release: Some(release),
        state: state.clone(),
    };
    let subject = receive.recv_timeout(Duration::from_secs(30));
    let waiting = subject.as_ref().ok().map(|subject| {
        CALL_TARGET.with(|slot| *slot.borrow_mut() = subject.static_initializer());
        start_initializer_test(&mut executor)
    });
    // Always release and join the owner before asserting, including setup failure paths.
    let _ = owner_cleanup.release.take().unwrap().send(());
    let deadline = Instant::now() + Duration::from_secs(30);
    while !completion.is_finished() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let finished = completion.is_finished();
    let outcome = completion.take_outcome();
    drop(owner_cleanup);
    assert!(finished, "initializer owner did not finish");
    assert!(matches!(outcome, Some(Ok(ExecutorResult::Error(_)))));
    assert_eq!(waiting, Some(StepResult::Yield));
    assert_eq!(
        state
            .statics
            .get_init_state(subject.unwrap(), &GenericLookup::default()),
        crate::statics::InitState::Failed
    );
    assert!(
        matches!(resume(&mut executor), StepResult::MethodThrew(ref exception) if exception.type_name == "System.TypeInitializationException")
    );
    CALL_TARGET.with(|slot| *slot.borrow_mut() = None);
}

#[test]
fn legacy_initializer_frame_retirement_fails_claim() {
    use dotnet_vm_ops::StaticsOps;
    for pool in [false, true] {
        let (mut executor, _) = boundary_executor_with_initializer(false, true);
        let subject = initializer_test_subject(&executor, "RetiredLegacySubject");
        executor
            .with_arena(|arena| {
                arena.mutate_root(|gc, engine| {
                    // SAFETY: F1.GcHandleRooted — the engine is rooted throughout this mutation.
                    let gc = unsafe { test_gc_handle(gc, engine) };
                    let result = engine
                        .ves_context(gc)
                        .initialize_static_storage(subject.clone(), GenericLookup::default());
                    assert!(!matches!(
                        result,
                        StepResult::Error(_) | StepResult::Exception
                    ));
                    let mut frame = engine.stack.execution.frame_stack.pop().unwrap();
                    assert!(frame.initializer_claim.is_some());
                    if pool {
                        frame.prepare_for_pool();
                    }
                    drop(frame);
                })
            })
            .unwrap();
        assert_eq!(
            executor
                .shared
                .statics
                .get_init_state(subject, &GenericLookup::default()),
            crate::statics::InitState::Failed
        );
    }
}

#[test]
fn initialized_static_fields_complete_without_activation() {
    static_fields_complete_without_activation(false);
}

#[test]
fn recursive_static_fields_complete_without_activation() {
    static_fields_complete_without_activation(true);
}

fn static_fields_complete_without_activation(recursive: bool) {
    use crate::stack::ops::{StaticFieldAction, StaticFieldKind, VmResolutionOps, VmStaticsOps};
    use crate::statics::StaticInitResult;
    use dotnet_types::members::FieldDescription;
    use dotnet_vm_data::stack::InitializerClaim;
    use dotnet_vm_ops::VesInternals;

    let (mut executor, _) = boundary_executor_with_initializer(false, true);
    let mut assembly = Resolution::new(Module::new("StaticFields.dll"));
    assembly.assembly = Some(Assembly::new("StaticFields"));
    let index = assembly.push_type_definition(TypeDefinition::new(None, "Subject"));
    assembly.push_field(
        index,
        dotnetdll::resolved::members::Field::static_member(
            Accessibility::Public,
            "Value",
            MemberType::Base(Box::new(BaseType::UInt8)),
        ),
    );
    let mut signature = parameterless_i32_signature();
    signature.return_type = ReturnType(vec![], None);
    let mut cctor = Method::new(
        Accessibility::Private,
        signature,
        ".cctor",
        Some(make_test_method!(max_stack: 0, instructions: vec![Instruction::Return])),
    );
    cctor.runtime_special_name = true;
    assembly.push_method(index, cctor);
    let assembly = executor.shared.loader.register_owned_assembly(assembly);
    let subject = TypeDescription::new(assembly.clone(), index);
    let field = FieldDescription::new(subject.clone(), assembly, 0);
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the arena root throughout this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                let generics = GenericLookup::default();
                let StaticInitResult::Execute(_, mut claim) = ctx
                    .shared
                    .statics
                    .init(
                        subject.clone(),
                        &ctx.with_generics(&generics),
                        ctx.thread_id.get(),
                    )
                    .unwrap()
                else {
                    panic!("initializer must be newly claimed")
                };
                if !recursive {
                    claim.succeed();
                }
                let native_id = ctx.call_state.last_native_id;
                let ip = ctx.state().ip;
                for volatile in [false, true] {
                    ctx.push(StackValue::Int32(42));
                    for kind in [
                        StaticFieldKind::Store { volatile },
                        StaticFieldKind::Load { volatile },
                        StaticFieldKind::Address,
                    ] {
                        assert_eq!(
                            ctx.start_static_field(StaticFieldAction {
                                field: field.clone(),
                                generics: generics.clone(),
                                kind: kind.clone(),
                            }),
                            StepResult::Continue
                        );
                        match kind {
                            StaticFieldKind::Store { .. } => {}
                            StaticFieldKind::Load { .. } => {
                                assert!(matches!(ctx.pop(), StackValue::Int32(42)))
                            }
                            StaticFieldKind::Address => {
                                assert!(matches!(ctx.pop(), StackValue::ManagedPtr(_)))
                            }
                        }
                        assert_eq!(ctx.call_state.last_native_id, native_id);
                        assert!(ctx.call_state.native.is_empty());
                        assert_eq!(
                            ctx.state().ip,
                            ip,
                            "ordinary instruction dispatch owns IP advancement"
                        );
                    }
                }
                claim.succeed();
            });
        })
        .unwrap();
}

#[test]
fn reflection_retry_completed_collection_does_not_retry() {
    let mut executor = executor();
    let mut res = Resolution::new(Module::new("mscorlib.dll"));
    res.assembly = Some(Assembly::new("mscorlib"));
    let mut gc_type = TypeDefinition::new(None, "GC");
    gc_type.namespace = Some("System".into());
    gc_type.flags.before_field_init = true;
    let gc_index = res.push_type_definition(gc_type);
    let mut signature = parameterless_i32_signature();
    signature.return_type = ReturnType(vec![], None);
    let method_index = res.push_method(
        gc_index,
        Method::new(Accessibility::Public, signature, "Collect", None),
    );
    let res = executor.shared.loader.register_owned_assembly(res);
    let ty = TypeDescription::new(res.clone(), gc_index);
    let method = ty
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &res.definition()[method_index]))
        .unwrap();
    let method = MethodDescription::new(
        ty,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(method),
    );
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine is the live arena root for this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                engine.ves_context(gc).push(StackValue::Int32(0));
                let frame = engine.stack.execution.frame_stack.current_frame_mut();
                frame.reflection_pending =
                    Some(Box::new(dotnet_vm_data::stack::ReflectionPending {
                        method,
                        generics: GenericLookup::default(),
                        arguments: vec![],
                        base: frame.stack_height,
                    }));
            })
        })
        .unwrap();
    assert_eq!(resume(&mut executor), StepResult::Yield);
    executor
        .with_arena(|arena| {
            arena.mutate(|_, engine| {
                let frame = engine.stack.execution.frame_stack.current_frame();
                assert!(
                    frame.reflection_pending.is_none(),
                    "completed collection was queued for retry"
                );
                assert_eq!(frame.state.ip, 1);
                assert!(engine.stack.local.heap.needs_full_collect.get());
            })
        })
        .unwrap();
    executor.perform_full_gc().unwrap();
    assert!(matches!(resume(&mut executor), StepResult::Return));
}

#[test]
fn multicast_fallback_completes_collection_targets() {
    use crate::stack::native_effects::MULTICAST_FALLBACK_TEST;
    use std::{path::Path, process::Command, sync::mpsc, time::Duration};
    struct Watchdog(Option<std::thread::JoinHandle<()>>, mpsc::Sender<()>);
    impl Drop for Watchdog {
        fn drop(&mut self) {
            let _ = self.1.send(());
            if let Some(thread) = self.0.take() {
                let _ = thread.join();
            }
        }
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            MULTICAST_FALLBACK_TEST.with(|slot| slot.set(None));
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dotnet-cli");
    let fixture = std::fs::canonicalize(
        root.join("tests/fixtures/delegates/delegate_multicast_gc_collect_42.cs"),
    )
    .unwrap();
    let output =
        std::env::temp_dir().join(format!("dotnet-multicast-collect-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    let build = Command::new("dotnet")
        .arg("build")
        .arg(root.join("tests/SingleFile.csproj"))
        .arg(format!("-p:TestFile={}", fixture.display()))
        .arg(format!(
            "-p:BaseIntermediateOutputPath={}/",
            output.join("obj").display()
        ))
        .args(["-v:q", "--nologo", "-o"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "fixture build failed: {}",
        String::from_utf8_lossy(&build.stdout)
    );
    let loader = AssemblyLoader::new(
        dotnet_assemblies::find_dotnet_app_path()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    let resolution = loader
        .load_resolution_from_file(output.join("SingleFile.dll"))
        .unwrap();
    let EntryPoint::Method(entry) = resolution.entry_point.expect("entrypoint") else {
        panic!("method entrypoint expected")
    };
    let ty = TypeDescription::new(resolution.clone(), entry.parent_type());
    let index = ty
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &resolution.definition()[entry]))
        .unwrap();
    let method = MethodDescription::new(
        ty,
        GenericLookup::default(),
        resolution,
        MethodMemberIndex::Method(index),
    );
    #[allow(
        clippy::arc_with_non_send_sync,
        reason = "the single-threaded test owns its loader"
    )]
    let shared = Arc::new(SharedGlobalState::new(Arc::new(loader)));
    let mut executor = Executor::new(shared.clone());
    executor.entrypoint(method).unwrap();
    let signal = shared.abort_signal();
    let (done_tx, done_rx) = mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(Duration::from_secs(15)).is_err() {
            signal.request_abort();
        }
    });
    let watchdog = Watchdog(Some(watchdog), done_tx);
    MULTICAST_FALLBACK_TEST.with(|slot| slot.set(Some(0)));
    let reset = Reset;
    let result = executor.run();
    let calls = MULTICAST_FALLBACK_TEST.with(|slot| slot.get().unwrap());
    drop(reset);
    drop(watchdog);
    drop(executor);
    shared.shutdown_workers();
    std::fs::remove_dir_all(output).unwrap();
    assert!(calls > 0, "fixture did not select fallback multicast");
    assert!(
        matches!(result, ExecutorResult::Exited(42)),
        "fallback collection did not complete: {result:?}"
    );
    assert!(calls >= 3, "fixture did not complete all multicast calls");
}
