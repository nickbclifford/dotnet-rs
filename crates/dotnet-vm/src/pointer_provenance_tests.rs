//! Bounded IL-level pointer provenance probes on the real VM.
//!
//! Each `p*` test builds a small CIL method that forms, converts and dereferences pointers
//! (locals, `localloc`, heap arrays, statics) and checks the result. Controls must pass
//! natively and under pinned permissive Tree Borrows Miri; diagnostic-only probes are
//! `#[ignore]`d and run on demand. Run them through `scripts/pointer_provenance.sh`
//! (`list`, `controls`, `batch`, `diagnostic`), which owns the shard and batch tables that
//! the `batches` module must match. Probe classifications and recorded outcomes live in
//! `docs/pointer-provenance/INVENTORY.md`.

use crate::{
    StepResult,
    stack::ops::VmCallOps,
    state::SharedGlobalState,
    sync::Arc,
    test_utils::{
        new_test_arena, parameterless_i32_signature, should_break_after_step, test_gc_handle,
    },
};
use dotnet_assemblies::AssemblyLoader;
use dotnet_intrinsics_unsafe::unsafe_ptr::{
    intrinsic_unsafe_add_byte_offset, intrinsic_unsafe_are_same, intrinsic_unsafe_as_pointer,
    intrinsic_unsafe_byte_offset, intrinsic_unsafe_is_address_ordered,
    intrinsic_unsafe_subtract_byte_offset,
};
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    resolution::ResolutionS,
};
use dotnet_utils::ByteOffset;
use dotnet_value::{
    StackValue,
    layout::{ArrayLayoutManager, LayoutManager, Scalar},
    object::{HeapStorage, ObjectRef, Vector},
    pointer::{ManagedPtr, unmanaged_ptr_from_addr},
};
use dotnet_vm_ops::ops::EvalStackOps;
use dotnetdll::{
    binary::signature::kinds::CallingConvention,
    prelude::*,
    resolved::{
        members::Method,
        types::{BaseType, TypeDefinition},
    },
};

fn corlib_resolution(module: &'static str, assembly: &'static str) -> Resolution<'static> {
    let mut res = Resolution::new(Module::new(module));
    res.assembly = Some(Assembly::new(assembly));
    let mut obj = TypeDefinition::new(None, "Object");
    obj.namespace = Some("System".into());
    let obj_idx = res.push_type_definition(obj);
    let void_sig = MethodSignature {
        instance: true,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters: vec![],
        return_type: ReturnType(vec![], None),
        varargs: None,
    };
    res.push_method(
        obj_idx,
        Method::new(
            Accessibility::Public,
            void_sig.clone(),
            ".ctor",
            Some(body::Method {
                header: body::Header {
                    maximum_stack_size: 1,
                    local_variables: vec![],
                    initialize_locals: true,
                },
                instructions: vec![Instruction::Return],
                data_sections: vec![],
            }),
        ),
    );
    let mut exception = TypeDefinition::new(None, "Exception");
    exception.namespace = Some("System".into());
    let exception_idx = res.push_type_definition(exception);
    res.push_method(
        exception_idx,
        Method::new(
            Accessibility::Public,
            void_sig,
            ".ctor",
            Some(body::Method {
                header: body::Header {
                    maximum_stack_size: 1,
                    local_variables: vec![],
                    initialize_locals: true,
                },
                instructions: vec![Instruction::Return],
                data_sections: vec![],
            }),
        ),
    );
    let mut invalid_program = TypeDefinition::new(None, "InvalidProgramException");
    invalid_program.namespace = Some("System".into());
    invalid_program.extends = Some(exception_idx.into());
    res.push_type_definition(invalid_program);
    let mut vt = TypeDefinition::new(None, "ValueType");
    vt.namespace = Some("System".into());
    vt.extends = Some(obj_idx.into());
    let vt_idx = res.push_type_definition(vt);
    for name in [
        "Boolean",
        "SByte",
        "Byte",
        "Int16",
        "UInt16",
        "Int32",
        "UInt32",
        "Int64",
        "UInt64",
        "Single",
        "Double",
        "Char",
        "IntPtr",
        "UIntPtr",
        "Void",
        "Enum",
        "TypedReference",
    ] {
        let mut t = TypeDefinition::new(None, name);
        t.namespace = Some("System".into());
        t.extends = Some(vt_idx.into());
        res.push_type_definition(t);
    }
    res
}

#[allow(clippy::arc_with_non_send_sync)]
fn get_mock_loader() -> Arc<AssemblyLoader> {
    thread_local! {
        static MOCK_LOADER: Arc<AssemblyLoader> = {
            let loader = AssemblyLoader::new_bare("mock_root_provenance_probe".to_string())
                .expect("Failed to create mock AssemblyLoader");
            loader.register_owned_assembly(corlib_resolution("mscorlib.dll", "mscorlib"));
            loader.register_owned_assembly(corlib_resolution("System.Runtime.dll", "System.Runtime"));
            Arc::new(loader)
        };
    }
    MOCK_LOADER.with(|l| l.clone())
}

fn run_i32_entrypoint(shared: Arc<SharedGlobalState>, entrypoint: MethodDescription) -> i32 {
    let mut arena = new_test_arena(&shared);

    arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — engine is the live arena root and the handle stays within this mutation.
        let gc_handle = unsafe { test_gc_handle(gc, engine) };
        let info = shared
            .caches
            .get_method_info(entrypoint.clone(), &Default::default(), shared.clone())
            .expect("Failed to resolve entrypoint");
        engine
            .ves_context(gc_handle)
            .entrypoint_frame(info, Default::default(), vec![])
            .expect("Failed to set up entrypoint frame");
    });

    let mut final_int = None;
    for _ in 0..100_000 {
        let step_res = arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — engine is the live arena root and the handle stays within this mutation.
            let gc_handle = unsafe { test_gc_handle(gc, engine) };
            let res = engine.step(gc_handle);
            if let StepResult::Return = res {
                let val = engine.stack.execution.evaluation_stack.pop();
                match val {
                    StackValue::Int32(v) => final_int = Some(v),
                    other => panic!("Expected Int32 return, got {:?}", other),
                }
            }
            res
        });
        if should_break_after_step(step_res) {
            break;
        }
    }
    final_int.expect("entrypoint did not produce an Int32 return value")
}

enum Observe {
    Continue,
    Stop,
    Collect,
}

fn run_entrypoint_observed(
    shared: Arc<SharedGlobalState>,
    entrypoint: MethodDescription,
    mut observer: impl for<'gc> FnMut(
        &mut crate::dispatch::ExecutionEngine<'gc>,
        &StepResult,
    ) -> Observe,
) {
    let mut arena = new_test_arena(&shared);

    arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — engine is the live arena root and the handle stays within this mutation.
        let gc_handle = unsafe { test_gc_handle(gc, engine) };
        let info = shared
            .caches
            .get_method_info(entrypoint.clone(), &Default::default(), shared.clone())
            .expect("Failed to resolve entrypoint");
        engine
            .ves_context(gc_handle)
            .entrypoint_frame(info, Default::default(), vec![])
            .expect("Failed to set up entrypoint frame");
    });

    for _ in 0..100_000 {
        let (should_stop, should_collect) = arena.mutate_root(|gc, engine| {
            // SAFETY: F1.GcHandleRooted — engine is the live arena root and the handle stays within this mutation.
            let gc_handle = unsafe { test_gc_handle(gc, engine) };
            let step_result = engine.step(gc_handle);
            let action = observer(engine, &step_result);
            (
                matches!(action, Observe::Stop) || should_break_after_step(step_result),
                matches!(action, Observe::Collect),
            )
        });
        if should_collect {
            arena.finish_cycle();
        }
        if should_stop {
            break;
        }
    }

    drop(arena);
}

fn local(base: BaseType<MethodType>) -> LocalVariable {
    LocalVariable::Variable {
        custom_modifiers: vec![],
        pinned: false,
        by_ref: false,
        var_type: MethodType::Base(Box::new(base)),
    }
}

fn run_il(
    asm: &str,
    max_stack: usize,
    locals: Vec<LocalVariable>,
    instructions: Vec<Instruction>,
) -> i32 {
    let loader = get_mock_loader();
    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(SharedGlobalState::new(loader.clone()));
    let (mut res, _, type_idx) = make_test_assembly!(
        format!("{asm}.dll"),
        format!("{asm}Assembly"),
        format!("{asm}Type")
    );
    let body = make_test_method!(max_stack: max_stack, instructions: instructions, locals: locals);
    let method_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            parameterless_i32_signature(),
            "Run",
            Some(body),
        ),
    );
    let res_s = loader.register_owned_assembly(res);
    let method_def = &res_s.definition()[method_idx];
    let td = TypeDescription::new(res_s.clone(), type_idx);
    let method_index = td
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, method_def))
        .expect("method must exist in type definition");
    let entrypoint = MethodDescription::new(
        td,
        GenericLookup::default(),
        res_s,
        MethodMemberIndex::Method(method_index),
    );
    run_i32_entrypoint(shared, entrypoint)
}

fn stind(t: StoreType) -> Instruction {
    Instruction::StoreIndirect {
        param0: t,
        unaligned: None,
        volatile: false,
    }
}
fn ldind(t: LoadType) -> Instruction {
    Instruction::LoadIndirect {
        param0: t,
        unaligned: None,
        volatile: false,
    }
}
fn conv_u() -> Instruction {
    Instruction::Convert(ConversionType::UIntPtr)
}

// ---- Probe 0: harness smoke test (no pointers at all) ----------------------------------------
#[test]
fn p00_smoke() {
    let r = run_il(
        "P00",
        2,
        vec![local(BaseType::Int32)],
        vec![
            Instruction::LoadConstantInt32(41),
            Instruction::StoreLocal(0),
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt32(1),
            Instruction::Add,
            Instruction::Return,
        ],
    );
    assert_eq!(r, 42);
}

// ---- Claim 1: ldloca; conv.u; stind (write through a stack-slot address laundered via an int) ---
#[test]
fn p01_ldloca_convu_stind_i4() {
    let r = run_il(
        "P01",
        3,
        vec![local(BaseType::Int32)],
        vec![
            Instruction::LoadLocalAddress(0),
            conv_u(),
            Instruction::LoadConstantInt32(7),
            stind(StoreType::Int32),
            Instruction::LoadLocal(0),
            Instruction::Return,
        ],
    );
    assert_eq!(r, 7);
}

// Control for P01: no integer laundering. Takes the interpreter's "reference slot" shortcut.
#[test]
fn p02_ldloca_stind_i4_control() {
    let r = run_il(
        "P02",
        3,
        vec![local(BaseType::Int32)],
        vec![
            Instruction::LoadLocalAddress(0),
            Instruction::LoadConstantInt32(7),
            stind(StoreType::Int32),
            Instruction::LoadLocal(0),
            Instruction::Return,
        ],
    );
    assert_eq!(r, 7);
}

// Read-only variant of P01: ldloca; conv.u; ldind.i4.
#[test]
fn p03_ldloca_convu_ldind_i4() {
    let r = run_il(
        "P03",
        3,
        vec![local(BaseType::Int32)],
        vec![
            Instruction::LoadConstantInt32(9),
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            conv_u(),
            ldind(LoadType::Int32),
            Instruction::Return,
        ],
    );
    assert_eq!(r, 9);
}

// ---- A second `localloc` must not invalidate the first block's pointer: blocks are frame-owned and never move. ----
#[test]
fn p04_double_localloc_write_first() {
    let r = run_il(
        "P04",
        3,
        vec![local(BaseType::IntPtr), local(BaseType::IntPtr)],
        vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadConstantInt32(4096),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(1),
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt32(5),
            stind(StoreType::Int32),
            Instruction::LoadLocal(0),
            ldind(LoadType::Int32),
            Instruction::Return,
        ],
    );
    assert_eq!(r, 5);
}

// Control for P04: a single localloc round-trip (isolates the UnmanagedPtr -> usize -> exposed ptr path).
#[test]
fn p05_single_localloc_roundtrip() {
    let r = run_il(
        "P05",
        3,
        vec![local(BaseType::IntPtr)],
        vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt32(5),
            stind(StoreType::Int32),
            Instruction::LoadLocal(0),
            ldind(LoadType::Int32),
            Instruction::Return,
        ],
    );
    assert_eq!(r, 5);
}

// Regression: the former in-bounds `offset` was UB when subtraction left the allocation.
#[test]
fn p06_localloc_minus_one_then_plus_one() {
    // (p - 1) + 1 == p ; the intermediate pointer is outside the allocation.
    let r = run_il(
        "P06",
        3,
        vec![local(BaseType::IntPtr)],
        vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::LoadConstantInt32(1),
            Instruction::Subtract,
            Instruction::LoadConstantInt32(1),
            Instruction::Add,
            Instruction::StoreLocal(0),
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt32(3),
            stind(StoreType::Int32),
            Instruction::LoadLocal(0),
            ldind(LoadType::Int32),
            Instruction::Return,
        ],
    );
    assert_eq!(r, 3);
}

// One-past-the-end is legal in Rust and in C (loop end pointers): p + 16 must NOT be flagged.
#[test]
fn p07_localloc_one_past_end() {
    let r = run_il(
        "P07",
        3,
        vec![local(BaseType::IntPtr)],
        vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::LoadConstantInt32(16),
            Instruction::Add,
            Instruction::StoreLocal(0),
            Instruction::LoadConstantInt32(1),
            Instruction::Return,
        ],
    );
    assert_eq!(r, 1);
}

// Overshoot by more than one-past-end (e.g. loop `p += stride` after the last element).
#[test]
fn p08_localloc_overshoot() {
    let r = run_il(
        "P08",
        3,
        vec![local(BaseType::IntPtr)],
        vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::LoadConstantInt32(32),
            Instruction::Add,
            Instruction::StoreLocal(0),
            Instruction::LoadConstantInt32(1),
            Instruction::Return,
        ],
    );
    assert_eq!(r, 1);
}

// =================================================================================================
// Batch 2: generic runner with helper methods / static fields / arrays
// =================================================================================================
use dotnetdll::resolved::{
    members::{Field, UserMethod},
    signature::{MethodSignature, ReturnType},
};

fn void_sig() -> MethodSignature<dotnetdll::binary::signature::kinds::CallingConvention, MethodType>
{
    MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: dotnetdll::binary::signature::kinds::CallingConvention::Default,
        parameters: vec![],
        return_type: ReturnType(vec![], None),
        varargs: None,
    }
}

fn new_static_field(name: &str, ty: BaseType<MemberType>) -> Field<'static> {
    Field {
        attributes: vec![],
        name: name.to_string().into(),
        type_modifiers: vec![],
        by_ref: false,
        return_type: MemberType::Base(Box::new(ty)),
        accessibility: dotnetdll::resolved::members::Accessibility::Access(
            dotnetdll::resolved::Accessibility::Public,
        ),
        static_member: true,
        init_only: false,
        literal: false,
        default: None,
        not_serialized: false,
        special_name: false,
        pinvoke: None,
        runtime_special_name: false,
        offset: None,
        marshal: None,
        initial_value: None,
    }
}

/// `setup` receives the resolution + type index and returns (max_stack, locals, instructions) for `Run`.
fn run_program(
    asm: &str,
    setup: impl FnOnce(
        &mut Resolution<'static>,
        dotnetdll::prelude::TypeIndex,
    ) -> (usize, Vec<LocalVariable>, Vec<Instruction>),
) -> i32 {
    run_program_with_collection(asm, setup, false)
}

fn run_program_with_collection(
    asm: &str,
    setup: impl FnOnce(
        &mut Resolution<'static>,
        dotnetdll::prelude::TypeIndex,
    ) -> (usize, Vec<LocalVariable>, Vec<Instruction>),
    collect_after_local_store: bool,
) -> i32 {
    let (shared, entrypoint) = build_program(asm, setup);
    if !collect_after_local_store {
        return run_i32_entrypoint(shared, entrypoint);
    }
    let mut collected = false;
    let mut result = None;
    run_entrypoint_observed(shared, entrypoint, |engine, step| {
        if matches!(step, StepResult::Return) {
            match engine.stack.execution.evaluation_stack.pop() {
                StackValue::Int32(value) => result = Some(value),
                other => panic!("Expected Int32 return, got {:?}", other),
            }
        }
        let frames = &engine.stack.execution.frame_stack;
        if !collected && frames.len() == 1 && frames.frames[0].state.ip == 3 {
            collected = true;
            return Observe::Collect;
        }
        Observe::Continue
    });
    assert!(collected, "collection point was not reached");
    result.expect("entrypoint did not produce an Int32 return value")
}

/// Registers a test assembly whose `Run` method is built by `setup` and returns its entrypoint.
fn build_program(
    asm: &str,
    setup: impl FnOnce(
        &mut Resolution<'static>,
        dotnetdll::prelude::TypeIndex,
    ) -> (usize, Vec<LocalVariable>, Vec<Instruction>),
) -> (Arc<SharedGlobalState>, MethodDescription) {
    let loader = get_mock_loader();
    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(SharedGlobalState::new(loader.clone()));
    let (mut res, _, type_idx) = make_test_assembly!(
        format!("{asm}.dll"),
        format!("{asm}Assembly"),
        format!("{asm}Type")
    );
    let (max_stack, locals, instructions) = setup(&mut res, type_idx);
    let body = make_test_method!(max_stack: max_stack, instructions: instructions, locals: locals);
    let method_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            parameterless_i32_signature(),
            "Run",
            Some(body),
        ),
    );
    let res_s = loader.register_owned_assembly(res);
    let method_def = &res_s.definition()[method_idx];
    let td = TypeDescription::new(res_s.clone(), type_idx);
    let method_index = td
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, method_def))
        .expect("method must exist in type definition");
    let entrypoint = MethodDescription::new(
        td,
        GenericLookup::default(),
        res_s,
        MethodMemberIndex::Method(method_index),
    );
    (shared, entrypoint)
}

// Stale-address probe: `ldloca; conv.u` then a callee needing a big evaluation stack forces the
// shared Vec<StackValue> to reallocate; the integer address held by the caller dangles.
#[test]
fn p09_ldloca_convu_survives_callee_stack_growth() {
    let r = run_program("P09", |res, type_idx| {
        let helper_body = make_test_method!(
            max_stack: 4096,
            instructions: vec![Instruction::Return],
        );
        let helper_idx = res.push_method(
            type_idx,
            Method::new(Accessibility::Public, void_sig(), "Grow", Some(helper_body)),
        );
        (
            3,
            vec![local(BaseType::Int32), local(BaseType::IntPtr)],
            vec![
                Instruction::LoadLocalAddress(0),
                conv_u(),
                Instruction::StoreLocal(1),
                Instruction::Call {
                    tail_call: false,
                    param0: MethodSource::User(UserMethod::Definition(helper_idx)),
                },
                Instruction::LoadLocal(1),
                Instruction::LoadConstantInt32(7),
                stind(StoreType::Int32),
                Instruction::LoadLocal(0),
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 7);
}

// Heap array element address laundered through an integer: `fixed (int* p = &arr[1]) *p = 7`.
#[test]
fn p10_ldelema_convu_stind_heap_array() {
    let r = run_program("P10", |_res, _t| {
        (
            4,
            vec![local(BaseType::IntPtr)],
            vec![
                Instruction::LoadConstantInt32(4),
                Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
                Instruction::StoreLocal(0),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(1),
                Instruction::LoadElementAddress {
                    skip_type_check: false,
                    skip_range_check: false,
                    skip_null_check: false,
                    param0: MethodType::Base(Box::new(BaseType::Int32)),
                },
                conv_u(),
                Instruction::LoadConstantInt32(7),
                stind(StoreType::Int32),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(1),
                Instruction::LoadElementPrimitive {
                    skip_range_check: false,
                    skip_null_check: false,
                    param0: LoadType::Int32,
                },
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 7);
}

// Control for P10: same array write through the managed byref (no integer laundering).
#[test]
fn p11_ldelema_stind_heap_array_control() {
    let r = run_program("P11", |_res, _t| {
        (
            4,
            vec![local(BaseType::IntPtr)],
            vec![
                Instruction::LoadConstantInt32(4),
                Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
                Instruction::StoreLocal(0),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(1),
                Instruction::LoadElementAddress {
                    skip_type_check: false,
                    skip_range_check: false,
                    skip_null_check: false,
                    param0: MethodType::Base(Box::new(BaseType::Int32)),
                },
                Instruction::LoadConstantInt32(7),
                stind(StoreType::Int32),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(1),
                Instruction::LoadElementPrimitive {
                    skip_range_check: false,
                    skip_null_check: false,
                    param0: LoadType::Int32,
                },
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 7);
}

// Static field address laundered through an integer.
#[test]
#[ignore = "known or suspected host UB: static field addresses are not yet stable/aligned (docs/plans/README.md); run only via scripts/pointer_provenance.sh diagnostic"]
fn p12_ldsflda_convu_stind_static() {
    let r = run_program("P12", |res, type_idx| {
        let f = FieldSource::Definition(res.push_field(
            type_idx,
            new_static_field("Slot", BaseType::<MemberType>::Int32),
        ));
        (
            3,
            vec![],
            vec![
                Instruction::LoadStaticFieldAddress(f),
                conv_u(),
                Instruction::LoadConstantInt32(7),
                stind(StoreType::Int32),
                Instruction::LoadStaticField {
                    param0: f,
                    volatile: false,
                },
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 7);
}

// The static address is stored as a native integer before a full arena collection.
#[test]
fn p36_static_address_survives_forced_collection() {
    let r = run_program_with_collection(
        "P36",
        |res, type_idx| {
            let f = FieldSource::Definition(res.push_field(
                type_idx,
                new_static_field("Slot", BaseType::<MemberType>::UInt8),
            ));
            (
                2,
                vec![local(BaseType::UIntPtr)],
                vec![
                    Instruction::LoadStaticFieldAddress(f),
                    conv_u(),
                    Instruction::StoreLocal(0),
                    Instruction::LoadLocal(0),
                    Instruction::LoadConstantInt32(7),
                    stind(StoreType::Int8),
                    Instruction::LoadStaticField {
                        param0: f,
                        volatile: false,
                    },
                    Instruction::Return,
                ],
            )
        },
        true,
    );
    assert_eq!(r, 7);
}

// `ldflda` on a stack-resident value type followed by `ldind.i4`: the field address must keep
// its owning slot alive and readable.
#[test]
fn p35_ldflda_on_stack_value_type_then_ldind() {
    let r = run_program("P35", |res, _type_idx| {
        // The mock loader registers two corlib copies; `ValueType` must resolve through the one
        // `corlib_wkt` uses (mscorlib) or `is_value_type` is false and the local initialises null.
        let system_runtime = res.push_assembly_reference(
            dotnetdll::resolved::assembly::ExternalAssemblyReference::new("mscorlib"),
        );
        let value_type_ref =
            res.push_type_reference(dotnetdll::resolved::types::ExternalTypeReference::new(
                Some("System".into()),
                "ValueType",
                dotnetdll::resolved::types::ResolutionScope::Assembly(system_runtime),
            ));
        let mut struct_def = TypeDefinition::new(None, "P35Struct");
        struct_def.extends = Some(value_type_ref.into());
        let struct_idx = res.push_type_definition(struct_def);
        let field = FieldSource::Definition(res.push_field(
            struct_idx,
            Field {
                static_member: false,
                ..new_static_field("F", BaseType::<MemberType>::Int32)
            },
        ));
        let struct_ty = BaseType::Type {
            source: TypeSource::User(UserType::Definition(struct_idx)),
            value_kind: Some(ValueKind::ValueType),
        };
        (
            2,
            vec![local(struct_ty)],
            vec![
                Instruction::LoadLocal(0),
                Instruction::LoadFieldAddress(field),
                ldind(LoadType::Int32),
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 0);
}

/// Defines a value type `name` with one instance `Int32` field `F` (a `ref int` field when
/// `ref_field`), marked byref-like through an embedded `IsByRefLikeAttribute` when `byref_like`.
fn push_test_value_type(
    res: &mut Resolution<'static>,
    name: &str,
    byref_like: bool,
    ref_field: bool,
) -> (TypeIndex, BaseType<MethodType>, FieldSource) {
    let corlib = res.push_assembly_reference(
        dotnetdll::resolved::assembly::ExternalAssemblyReference::new("mscorlib"),
    );
    let value_type_ref =
        res.push_type_reference(dotnetdll::resolved::types::ExternalTypeReference::new(
            Some("System".into()),
            "ValueType",
            dotnetdll::resolved::types::ResolutionScope::Assembly(corlib),
        ));
    let mut struct_def = TypeDefinition::new(None, name.to_string());
    struct_def.extends = Some(value_type_ref.into());
    if byref_like {
        let mut attr_def = TypeDefinition::new(None, "IsByRefLikeAttribute");
        attr_def.namespace = Some("System.Runtime.CompilerServices".into());
        let attr_idx = res.push_type_definition(attr_def);
        let ctor_sig = MethodSignature {
            instance: true,
            ..void_sig()
        };
        let ctor = res.push_method(
            attr_idx,
            Method::new(
                Accessibility::Public,
                ctor_sig,
                ".ctor",
                Some(make_test_method!(max_stack: 1, instructions: vec![Instruction::Return])),
            ),
        );
        struct_def.attributes.push(Attribute::new(
            UserMethod::Definition(ctor),
            CustomAttributeData {
                constructor_args: vec![],
                named_args: vec![],
            },
        ));
    }
    let struct_idx = res.push_type_definition(struct_def);
    let field = res.push_field(
        struct_idx,
        Field {
            static_member: false,
            by_ref: ref_field,
            ..new_static_field("F", BaseType::<MemberType>::Int32)
        },
    );
    let struct_ty = BaseType::Type {
        source: TypeSource::User(UserType::Definition(struct_idx)),
        value_kind: Some(ValueKind::ValueType),
    };
    (struct_idx, struct_ty, FieldSource::Definition(field))
}

fn instance_layout_of_test_value_type(
    byref_like: bool,
) -> Result<dotnet_value::layout::FieldLayoutManager, dotnet_types::error::TypeResolutionError> {
    let mut struct_idx = None;
    let asm = if byref_like {
        "RefFieldByRefLike"
    } else {
        "RefFieldPlain"
    };
    let (shared, entrypoint) = build_program(asm, |res, _| {
        struct_idx = Some(push_test_value_type(res, "S", byref_like, true).0);
        (
            1,
            vec![],
            vec![Instruction::LoadConstantInt32(0), Instruction::Return],
        )
    });
    let td = TypeDescription::new(entrypoint.resolution(), struct_idx.unwrap());
    crate::resolver::VmResolverService::new(shared)
        .instance_fields_with_lookup(td, &GenericLookup::default())
}

// ECMA-335 §II.14.4.2: a `ref` field may be declared only by a byref-like type.
#[test]
fn ref_field_layout_requires_byref_like_declaring_type() {
    let layout = instance_layout_of_test_value_type(true).expect("byref-like layout");
    let field = layout.fields.values().next().expect("one field");
    assert!(matches!(
        &*field.layout,
        LayoutManager::Scalar(Scalar::ManagedPtr)
    ));
    assert!(matches!(
        instance_layout_of_test_value_type(false),
        Err(dotnet_types::error::TypeResolutionError::InvalidLayout(_))
    ));
}

fn box_test_value_type(byref_like: bool) -> Result<i32, String> {
    let asm = if byref_like {
        "BoxByRefLike"
    } else {
        "BoxPlain"
    };
    let (shared, entrypoint) = build_program(asm, |res, _| {
        let (_, struct_ty, _) = push_test_value_type(res, "S", byref_like, false);
        (
            1,
            vec![local(struct_ty.clone())],
            vec![
                Instruction::LoadLocal(0),
                Instruction::BoxValue(MethodType::Base(Box::new(struct_ty))),
                Instruction::Pop,
                Instruction::LoadConstantInt32(1),
                Instruction::Return,
            ],
        )
    });
    let mut outcome = None;
    run_entrypoint_observed(shared, entrypoint, |engine, step| match step {
        StepResult::Return => {
            outcome = Some(Ok(engine.stack.execution.evaluation_stack.pop().as_i32()));
            Observe::Stop
        }
        StepResult::MethodThrew(exception) => {
            outcome = Some(Err(exception.type_name.clone()));
            Observe::Stop
        }
        _ => Observe::Continue,
    });
    outcome.expect("entrypoint neither returned nor threw")
}

// ECMA-335 §II.14.4.2: a byref-like value can never be boxed.
#[test]
fn box_of_byref_like_value_throws_invalid_program() {
    assert_eq!(box_test_value_type(false), Ok(1));
    assert_eq!(
        box_test_value_type(true),
        Err("System.InvalidProgramException".to_string())
    );
}

// A byref to a value-type local must keep addressing that local after `stloc` assigns it a new
// value: its address, taken after the store, writes the local seen by the next `ldloc`.
#[test]
fn p37_ldloca_value_type_survives_stloc() {
    let r = run_program("P37", |res, _type_idx| {
        let (_, struct_ty, field) = push_test_value_type(res, "P37Struct", false, false);
        (
            3,
            vec![local(struct_ty.clone()), local(struct_ty)],
            vec![
                Instruction::LoadLocalAddress(0),
                Instruction::LoadLocal(1),
                Instruction::StoreLocal(0),
                conv_u(),
                Instruction::LoadConstantInt32(7),
                stind(StoreType::Int32),
                Instruction::LoadLocal(0),
                Instruction::LoadField {
                    param0: field,
                    unaligned: None,
                    volatile: false,
                },
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 7);
}

// An object stored in an addressed local lives in the frame's home block; the frame must trace
// it across a full collection.
#[test]
fn homed_object_local_survives_collection() {
    let r = run_program_with_collection(
        "HomedObjectLocal",
        |_res, _type_idx| {
            (
                2,
                vec![local(BaseType::Object)],
                vec![
                    Instruction::LoadConstantInt32(4),
                    Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
                    Instruction::StoreLocal(0),
                    Instruction::LoadLocalAddress(0),
                    ldind(LoadType::Object),
                    Instruction::LoadLength,
                    Instruction::Convert(ConversionType::Int32),
                    Instruction::Return,
                ],
            )
        },
        true,
    );
    assert_eq!(r, 4);
}

// A byref to a value-type argument must keep addressing that argument after `starg` assigns it a
// new value: its address, converted after the store, writes the argument seen by the next `ldarg`.
#[test]
fn p38_ldarga_value_type_survives_starg() {
    let r = run_program("P38", |res, type_idx| {
        let (_, struct_ty, field) = push_test_value_type(res, "P38Struct", false, false);
        let callee_body = make_test_method!(
            max_stack: 3,
            instructions: vec![
                Instruction::LoadArgumentAddress(0),
                Instruction::LoadLocal(0),
                Instruction::StoreArgument(0),
                conv_u(),
                Instruction::LoadConstantInt32(7),
                stind(StoreType::Int32),
                Instruction::LoadArgument(0),
                Instruction::LoadField {
                    param0: field,
                    unaligned: None,
                    volatile: false,
                },
                Instruction::Return,
            ],
            locals: vec![local(struct_ty.clone())],
        );
        let callee_sig = MethodSignature {
            parameters: vec![Parameter(
                vec![],
                ParameterType::Value(MethodType::Base(Box::new(struct_ty.clone()))),
            )],
            ..parameterless_i32_signature()
        };
        let callee = res.push_method(
            type_idx,
            Method::new(
                Accessibility::Public,
                callee_sig,
                "Callee",
                Some(callee_body),
            ),
        );
        (
            2,
            vec![local(struct_ty)],
            vec![
                Instruction::LoadLocal(0),
                Instruction::Call {
                    tail_call: false,
                    param0: MethodSource::User(UserMethod::Definition(callee)),
                },
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 7);
}

/// Pushes `Leak`, a method that stores 5 in its homed local and returns a byref to it
/// (`ref int Leak() { int x = 5; return ref Unsafe.AsRef(ref x); }`), and returns its index.
fn push_leaking_method(res: &mut Resolution<'static>, type_idx: TypeIndex) -> MethodIndex {
    let body = make_test_method!(
        max_stack: 1,
        instructions: vec![
            Instruction::LoadConstantInt32(5),
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            Instruction::Return,
        ],
        locals: vec![local(BaseType::Int32)],
    );
    let signature = MethodSignature {
        return_type: ReturnType(
            vec![],
            Some(ParameterType::Ref(MethodType::Base(Box::new(
                BaseType::Int32,
            )))),
        ),
        ..parameterless_i32_signature()
    };
    res.push_method(
        type_idx,
        Method::new(Accessibility::Public, signature, "Leak", Some(body)),
    )
}

/// Runs `Run` and returns the host error it stops with.
fn run_expecting_error(
    asm: &str,
    setup: impl FnOnce(
        &mut Resolution<'static>,
        TypeIndex,
    ) -> (usize, Vec<LocalVariable>, Vec<Instruction>),
) -> crate::error::VmError {
    let (shared, entrypoint) = build_program(asm, setup);
    let mut error = None;
    run_entrypoint_observed(shared, entrypoint, |_engine, step| match step {
        StepResult::Error(e) => {
            error = Some(e.clone());
            Observe::Stop
        }
        StepResult::Return => panic!("{asm} returned instead of failing"),
        _ => Observe::Continue,
    });
    error.unwrap_or_else(|| panic!("{asm} did not stop with an error"))
}

fn assert_stale_frame_home(error: crate::error::VmError) {
    assert!(
        matches!(
            error,
            crate::error::VmError::MemoryAccess(crate::error::MemoryAccessError::InvalidOrigin)
        ),
        "expected a stale frame-home error, got {error:?}"
    );
}

// A byref to a returned frame's home names a frame that no longer exists: reading through it is
// a host error, never a read of the released block.
#[test]
fn stale_frame_home_byref_read_is_rejected() {
    let error = run_expecting_error("StaleHomeRead", |res, type_idx| {
        let leak = push_leaking_method(res, type_idx);
        (
            2,
            vec![],
            vec![
                Instruction::Call {
                    tail_call: false,
                    param0: MethodSource::User(UserMethod::Definition(leak)),
                },
                ldind(LoadType::Int32),
                Instruction::Return,
            ],
        )
    });
    assert_stale_frame_home(error);
}

// As above for a write, after another frame with a home block has run at the same depth: the
// stale byref must not resolve into the newer frame.
#[test]
fn stale_frame_home_byref_write_is_rejected_after_frame_reuse() {
    let error = run_expecting_error("StaleHomeWrite", |res, type_idx| {
        let leak = push_leaking_method(res, type_idx);
        let other = push_leaking_method(res, type_idx);
        (
            3,
            vec![LocalVariable::Variable {
                custom_modifiers: vec![],
                pinned: false,
                by_ref: true,
                var_type: MethodType::Base(Box::new(BaseType::Int32)),
            }],
            vec![
                Instruction::Call {
                    tail_call: false,
                    param0: MethodSource::User(UserMethod::Definition(leak)),
                },
                Instruction::StoreLocal(0),
                Instruction::Call {
                    tail_call: false,
                    param0: MethodSource::User(UserMethod::Definition(other)),
                },
                Instruction::Pop,
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(7),
                stind(StoreType::Int32),
                Instruction::LoadConstantInt32(0),
                Instruction::Return,
            ],
        )
    });
    assert_stale_frame_home(error);
}

// A byref to a caller's homed local resolves through the caller's live frame while a callee with
// its own home block runs.
#[test]
fn frame_home_byref_resolves_into_caller_frame() {
    let r = run_program("CallerHome", |res, type_idx| {
        let callee_body = make_test_method!(
            max_stack: 2,
            instructions: vec![
                Instruction::LoadLocalAddress(0),
                Instruction::Pop,
                Instruction::LoadArgument(0),
                Instruction::LoadConstantInt32(7),
                stind(StoreType::Int32),
                Instruction::Return,
            ],
            locals: vec![local(BaseType::Int32)],
        );
        let callee_sig = MethodSignature {
            parameters: vec![Parameter(
                vec![],
                ParameterType::Ref(MethodType::Base(Box::new(BaseType::Int32))),
            )],
            ..void_sig()
        };
        let callee = res.push_method(
            type_idx,
            Method::new(
                Accessibility::Public,
                callee_sig,
                "Store",
                Some(callee_body),
            ),
        );
        (
            2,
            vec![local(BaseType::Int32)],
            vec![
                Instruction::LoadLocalAddress(0),
                Instruction::Call {
                    tail_call: false,
                    param0: MethodSource::User(UserMethod::Definition(callee)),
                },
                Instruction::LoadLocal(0),
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 7);
}

// ldelema + managed pointer arithmetic past the end of the array (one-past-end then beyond):
// `Unsafe.Add(ref arr[0], 100)` style. Never dereferenced; address formation must use wrapping arithmetic.
#[test]
#[cfg_attr(
    feature = "memory-validation",
    ignore = "p13 deliberately forms an out-of-bounds managed address; memory-validation's debug bound trap rejects it by design"
)]
fn p13_ldelema_managed_add_far_out_of_bounds_then_conv_u() {
    let r = run_program("P13", |_res, _t| {
        (
            4,
            vec![local(BaseType::IntPtr)],
            vec![
                Instruction::LoadConstantInt32(4),
                Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
                Instruction::StoreLocal(0),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(0),
                Instruction::LoadElementAddress {
                    skip_type_check: false,
                    skip_range_check: false,
                    skip_null_check: false,
                    param0: MethodType::Base(Box::new(BaseType::Int32)),
                },
                Instruction::LoadConstantInt32(4096),
                Instruction::Add,
                conv_u(),
                Instruction::Pop,
                Instruction::LoadConstantInt32(1),
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 1);
}

// Regression for the separate conv.u8 implementation: the same heap element address
// must be observed by conv.u8 and conv.u, without deriving either from a payload slice.
#[test]
fn conv_u8_heap_ldelema_matches_conv_u() {
    let r = run_program("ConvU8AddressMatch", |_res, _t| {
        (
            4,
            vec![local(BaseType::IntPtr), local(BaseType::IntPtr)],
            vec![
                Instruction::LoadConstantInt32(4),
                Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
                Instruction::StoreLocal(0),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(1),
                ldelema_i4(),
                Instruction::Convert(ConversionType::UInt64),
                Instruction::Convert(ConversionType::UIntPtr),
                Instruction::StoreLocal(1),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(1),
                ldelema_i4(),
                conv_u(),
                Instruction::LoadLocal(1),
                Instruction::CompareEqual,
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 1);
}

// Address observation after managed pointer arithmetic is allowed to form a far-OOB
// numeric address; this regression never dereferences the resulting integer.
#[test]
#[cfg_attr(
    feature = "memory-validation",
    ignore = "conv.u8 regression deliberately forms an out-of-bounds managed address; memory-validation's debug bound trap rejects it by design"
)]
fn conv_u8_after_far_oob_managed_add_does_not_panic() {
    let r = run_program("ConvU8FarOob", |_res, _t| {
        (
            4,
            vec![local(BaseType::IntPtr)],
            vec![
                Instruction::LoadConstantInt32(4),
                Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
                Instruction::StoreLocal(0),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(0),
                ldelema_i4(),
                Instruction::LoadConstantInt32(4096),
                Instruction::Add,
                Instruction::Convert(ConversionType::UInt64),
                Instruction::Pop,
                Instruction::LoadConstantInt32(1),
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 1);
}

#[test]
fn unsafe_as_pointer_heap_array_element_round_trips_raw_write_read() {
    let loader = get_mock_loader();
    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(SharedGlobalState::new(loader));
    let mut arena = new_test_arena(&shared);

    arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — the test arena root remains live for this mutation.
        let gc_handle = unsafe { test_gc_handle(gc, engine) };
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::UInt8),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::UInt8)),
                length: 4,
            },
            vec![0; 4],
            vec![4, 1],
        );
        let owner = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
        let element = ManagedPtr::new(
            owner.pointer(),
            TypeDescription::NULL,
            Some(owner),
            false,
            Some(ByteOffset::new(1)),
        );

        let address = {
            let mut ctx = engine.ves_context(gc_handle);
            ctx.push(StackValue::ManagedPtr(element.into()));
            let ignored_method = MethodDescription::new(
                TypeDescription::NULL,
                GenericLookup::default(),
                ResolutionS::NULL,
                MethodMemberIndex::Method(0),
            );
            assert!(matches!(
                intrinsic_unsafe_as_pointer(&mut ctx, ignored_method, &GenericLookup::default()),
                StepResult::Continue
            ));
            let StackValue::NativeInt(address) = ctx.pop() else {
                panic!("Unsafe.AsPointer must return a native integer pointer value");
            };
            address
        };

        // SAFETY: F10.RawMemoryAccessValid — the returned exposed address points to
        // element 1 of the live, rooted four-byte array allocated above.
        let raw = unsafe { unmanaged_ptr_from_addr(address as usize) };
        // SAFETY: F10.RawMemoryAccessValid — the owner remains rooted and the
        // element address is within the array payload.
        unsafe { raw.write(0x5A) };
        assert_eq!(owner.with_data(|data| data[1]), 0x5A);
    });
}

#[test]
fn unmanaged_stind_ldind_round_trips_localloc_native_int() {
    let r = run_program("LocallocNativeIntIndirectRoundTrip", |_res, _t| {
        (
            3,
            vec![local(BaseType::IntPtr)],
            vec![
                Instruction::LoadConstantInt32(4),
                Instruction::LocalMemoryAllocate,
                conv_u(),
                Instruction::StoreLocal(0),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(0x5A),
                stind(StoreType::Int8),
                Instruction::LoadLocal(0),
                ldind(LoadType::UInt8),
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 0x5A);
}

// =================================================================================================
// Batch 4: systematic "managed -> raw via StackValue::as_ptr()" write paths (initblk / cpblk)
// =================================================================================================
fn ldelema_i4() -> Instruction {
    Instruction::LoadElementAddress {
        skip_type_check: false,
        skip_range_check: false,
        skip_null_check: false,
        param0: MethodType::Base(Box::new(BaseType::Int32)),
    }
}

// initblk(&arr[0], 0x01, 16): the destination is a ManagedPtr(Heap) converted with as_ptr() then ptr::write_bytes.
#[test]
fn p14_initblk_heap_array_via_managed_ptr() {
    let r = run_program("P14", |_res, _t| {
        (
            4,
            vec![local(BaseType::IntPtr)],
            vec![
                Instruction::LoadConstantInt32(4),
                Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
                Instruction::StoreLocal(0),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(0),
                ldelema_i4(),
                Instruction::LoadConstantInt32(1),
                Instruction::LoadConstantInt32(16),
                Instruction::InitializeMemoryBlock {
                    unaligned: None,
                    volatile: false,
                },
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(3),
                Instruction::LoadElementPrimitive {
                    skip_range_check: false,
                    skip_null_check: false,
                    param0: LoadType::Int32,
                },
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 0x01010101);
}

// initblk(&local, 0x01, 4): destination is ManagedPtr(Stack).
#[test]
#[ignore = "known or suspected host UB until frame homes land (docs/plans/01-stable-frame-homes.md); run only via scripts/pointer_provenance.sh diagnostic"]
fn p15_initblk_stack_local_via_managed_ptr() {
    let r = run_program("P15", |_res, _t| {
        (
            4,
            vec![local(BaseType::Int32)],
            vec![
                Instruction::LoadLocalAddress(0),
                Instruction::LoadConstantInt32(1),
                Instruction::LoadConstantInt32(4),
                Instruction::InitializeMemoryBlock {
                    unaligned: None,
                    volatile: false,
                },
                Instruction::LoadLocal(0),
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 0x01010101);
}

// cpblk(&dst[0], &src[0], 16): both ManagedPtr(Heap); the write uses the observed heap address.
#[test]
fn p16_cpblk_heap_arrays_via_managed_ptr() {
    let r = run_program("P16", |_res, _t| {
        (
            5,
            vec![local(BaseType::IntPtr), local(BaseType::IntPtr)],
            vec![
                Instruction::LoadConstantInt32(4),
                Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
                Instruction::StoreLocal(0),
                Instruction::LoadConstantInt32(4),
                Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
                Instruction::StoreLocal(1),
                Instruction::LoadLocal(1),
                Instruction::LoadConstantInt32(0),
                ldelema_i4(),
                Instruction::LoadConstantInt32(0x01010101),
                Instruction::LoadConstantInt32(16),
                Instruction::InitializeMemoryBlock {
                    unaligned: None,
                    volatile: false,
                },
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(0),
                ldelema_i4(),
                Instruction::LoadLocal(1),
                Instruction::LoadConstantInt32(0),
                ldelema_i4(),
                Instruction::LoadConstantInt32(16),
                Instruction::CopyMemoryBlock {
                    unaligned: None,
                    volatile: false,
                },
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(3),
                Instruction::LoadElementPrimitive {
                    skip_range_check: false,
                    skip_null_check: false,
                    param0: LoadType::Int32,
                },
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 0x01010101);
}

// =================================================================================================
// Batch 5: comparison controls and a diagnostic-only negative-index panic probe.
// =================================================================================================
// Identical localloc pointers must compare equal under `ceq`.
#[test]
fn p17_localloc_ptr_ceq_self() {
    let r = run_il(
        "P17",
        3,
        vec![local(BaseType::IntPtr)],
        vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocal(0),
            Instruction::LoadLocal(0),
            Instruction::CompareEqual,
            Instruction::Return,
        ],
    );
    assert_eq!(r, 1, "p == p must be true");
}

// `clt` on localloc pointers must order them by address, so pointer loops terminate.
#[test]
fn p18_localloc_ptr_clt() {
    let r = run_il(
        "P18",
        4,
        vec![local(BaseType::IntPtr)],
        vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocal(0),
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt32(8),
            Instruction::Add,
            Instruction::CompareLess(NumberSign::Unsigned),
            Instruction::Return,
        ],
    );
    assert_eq!(r, 1, "p < p+8 must be true");
}

// Agent claim: negative array index -> host overflow panic instead of IndexOutOfRangeException.
#[test]
#[ignore = "reproduces a host panic/abort on a negative array index; run only via scripts/pointer_provenance.sh diagnostic"]
fn p19_ldelem_negative_index() {
    let r = run_il(
        "P19",
        4,
        vec![local(BaseType::IntPtr)],
        vec![
            Instruction::LoadConstantInt32(4),
            Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
            Instruction::LoadConstantInt32(-1),
            Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: LoadType::Int32,
            },
            Instruction::Return,
        ],
    );
    let _ = r;
}

// Interleaving robustness: take the integer address, do an ordinary typed VM write to the same
// array (stelem), then store through the integer. Expect 7. (Lab E12b predicts a reference-derived
// exposure dies here; a root-derived exposure must survive.)
#[test]
fn p20_heap_convu_then_typed_write_then_raw_store() {
    let r = run_program("P20", |_res, _t| {
        (
            5,
            vec![local(BaseType::IntPtr), local(BaseType::IntPtr)],
            vec![
                Instruction::LoadConstantInt32(4),
                Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
                Instruction::StoreLocal(0),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(1),
                ldelema_i4(),
                conv_u(),
                Instruction::StoreLocal(1),
                // ordinary typed write to the same element
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(1),
                Instruction::LoadConstantInt32(5),
                Instruction::StoreElementPrimitive {
                    skip_type_check: false,
                    skip_range_check: false,
                    skip_null_check: false,
                    param0: StoreType::Int32,
                },
                // raw store through the earlier integer address
                Instruction::LoadLocal(1),
                Instruction::LoadConstantInt32(7),
                stind(StoreType::Int32),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(1),
                Instruction::LoadElementPrimitive {
                    skip_range_check: false,
                    skip_null_check: false,
                    param0: LoadType::Int32,
                },
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 7);
}

// Extreme signed offsets cancel back to the allocation before any dereference.
#[test]
fn p21_localloc_signed_extreme_offsets_roundtrip() {
    let r = run_il(
        "P21",
        4,
        vec![local(BaseType::IntPtr), local(BaseType::IntPtr)],
        vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            // p - i32::MIN + i32::MIN == p.
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt32(i32::MIN),
            Instruction::Subtract,
            Instruction::LoadConstantInt32(i32::MIN),
            Instruction::Add,
            Instruction::StoreLocal(1),
            Instruction::LoadLocal(1),
            Instruction::LoadConstantInt32(3),
            stind(StoreType::Int32),
            // p + isize::MIN + isize::MIN == p (on the native 64-bit lane).
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt64(i64::MIN),
            Instruction::Convert(ConversionType::IntPtr),
            Instruction::Add,
            Instruction::LoadConstantInt64(i64::MIN),
            Instruction::Convert(ConversionType::IntPtr),
            Instruction::Add,
            Instruction::StoreLocal(1),
            Instruction::LoadLocal(1),
            Instruction::LoadLocal(1),
            ldind(LoadType::Int32),
            Instruction::LoadConstantInt32(4),
            Instruction::Add,
            stind(StoreType::Int32),
            // The arithmetic state change is complete before the final read.
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt32(-1),
            Instruction::Add,
            Instruction::LoadConstantInt32(1),
            Instruction::Add,
            ldind(LoadType::Int32),
            Instruction::Return,
        ],
    );
    assert_eq!(r, 7);
}

// Subtracting a pointer's exposed integer address produces explicit null; adding it back
// must reconstruct an address that still permits a real write and read.
#[test]
fn p22_localloc_wrap_to_zero_roundtrip() {
    let r = run_il(
        "P22",
        5,
        vec![
            local(BaseType::IntPtr),
            local(BaseType::IntPtr),
            local(BaseType::IntPtr),
            local(BaseType::IntPtr),
            local(BaseType::Int32),
        ],
        vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            // a = conv.u p; z = p - a (the wrapped address is zero).
            Instruction::LoadLocal(0),
            conv_u(),
            Instruction::StoreLocal(1),
            Instruction::LoadLocal(0),
            Instruction::LoadLocal(1),
            Instruction::Subtract,
            Instruction::StoreLocal(2),
            // Record that zero compares equal to conv.i 0.
            Instruction::LoadLocal(2),
            Instruction::LoadConstantInt32(0),
            Instruction::Convert(ConversionType::IntPtr),
            Instruction::CompareEqual,
            Instruction::StoreLocal(4),
            // q = z + a; mutate through q, then read through the original p.
            Instruction::LoadLocal(2),
            Instruction::LoadLocal(1),
            Instruction::Add,
            Instruction::StoreLocal(3),
            Instruction::LoadLocal(3),
            Instruction::LoadConstantInt32(9),
            stind(StoreType::Int32),
            Instruction::LoadLocal(0),
            ldind(LoadType::Int32),
            Instruction::LoadLocal(4),
            Instruction::LoadConstantInt32(100),
            Instruction::Multiply,
            Instruction::Add,
            Instruction::Return,
        ],
    );
    assert_eq!(r, 109);
}

// Exercise unsigned pointer branches over a byte buffer. Each loop compares and dereferences
// pointers after the prior arithmetic update; the second loop's result is the read-back evidence.
#[test]
fn p23_localloc_pointer_loop_unsigned_branch_compare() {
    let r = run_il(
        "P23",
        4,
        vec![
            local(BaseType::IntPtr),
            local(BaseType::IntPtr),
            local(BaseType::Int32),
        ],
        vec![
            Instruction::LoadConstantInt32(8),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt32(8),
            Instruction::Add,
            Instruction::StoreLocal(1),
            Instruction::LoadConstantInt32(0),
            Instruction::StoreLocal(2),
            // If comparisons are incomparable, the paired blt check safely exits rather than
            // attempting a write beyond the localloc buffer on this diagnostic baseline.
            Instruction::LoadLocal(0), // 9: first-loop condition
            Instruction::LoadLocal(1),
            Instruction::BranchGreaterOrEqual(NumberSign::Unsigned, 24),
            Instruction::LoadLocal(0),
            Instruction::LoadLocal(1),
            Instruction::BranchLess(NumberSign::Unsigned, 16),
            Instruction::Branch(24),
            Instruction::LoadLocal(0), // 16: first-loop body
            Instruction::LoadConstantInt32(5),
            stind(StoreType::Int8),
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt32(1),
            Instruction::Add,
            Instruction::StoreLocal(0),
            Instruction::Branch(9),
            // Reset p = end - 8 and sum = 0 before reading the bytes back.
            Instruction::LoadLocal(1), // 24: first loop exit
            Instruction::LoadConstantInt32(8),
            Instruction::Subtract,
            Instruction::StoreLocal(0),
            Instruction::LoadConstantInt32(0),
            Instruction::StoreLocal(2),
            // The read loop uses blt.un to enter and bge.un to exit.
            Instruction::LoadLocal(0), // 30: second-loop condition
            Instruction::LoadLocal(1),
            Instruction::BranchGreaterOrEqual(NumberSign::Unsigned, 47),
            Instruction::LoadLocal(0),
            Instruction::LoadLocal(1),
            Instruction::BranchLess(NumberSign::Unsigned, 37),
            Instruction::Branch(47),
            Instruction::LoadLocal(2), // 37: read-loop body
            Instruction::LoadLocal(0),
            ldind(LoadType::UInt8),
            Instruction::Add,
            Instruction::StoreLocal(2),
            Instruction::LoadLocal(0),
            Instruction::LoadConstantInt32(1),
            Instruction::Add,
            Instruction::StoreLocal(0),
            Instruction::Branch(30),
            Instruction::LoadLocal(2), // 47: second loop exit
            Instruction::Return,
        ],
    );
    assert_eq!(r, 40);
}

#[test]
fn p24_unsafe_byte_offset_direction_and_deref() {
    let loader = get_mock_loader();
    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(SharedGlobalState::new(loader.clone()));

    let (mut method_res, _, method_type_idx) = make_test_assembly!(
        "UnsafeAddressOrder.dll".to_string(),
        "UnsafeAddressOrderAssembly".to_string(),
        "UnsafeAddressOrderType".to_string()
    );
    let method_idx = method_res.push_method(
        method_type_idx,
        Method::new(
            Accessibility::Public,
            parameterless_i32_signature(),
            "IsAddressLessThan",
            None,
        ),
    );
    let method_res = loader.register_owned_assembly(method_res);
    let method_def = &method_res.definition()[method_idx];
    let method_type = TypeDescription::new(method_res.clone(), method_type_idx);
    let method_member_index = method_type
        .definition()
        .methods
        .iter()
        .position(|method| std::ptr::eq(method, method_def))
        .expect("test method exists in its type");
    let is_address_less_than = MethodDescription::new(
        method_type,
        GenericLookup::default(),
        method_res,
        MethodMemberIndex::Method(method_member_index),
    );
    let add_generics = GenericLookup::from_arcs(
        Arc::new([]),
        vec![ConcreteType::new(ResolutionS::NULL, BaseType::Int32)].into(),
    );

    let mut arena = new_test_arena(&shared);
    arena.mutate_root(|gc, engine| {
        // SAFETY: F1.GcHandleRooted — the test arena root remains live for this mutation.
        let gc_handle = unsafe { test_gc_handle(gc, engine) };
        let mut payload = vec![0; 16];
        payload[8..12].copy_from_slice(&77_i32.to_ne_bytes());
        let vector = Vector::new(
            ConcreteType::new(ResolutionS::NULL, BaseType::Int32),
            ArrayLayoutManager {
                element_layout: Arc::new(LayoutManager::Scalar(Scalar::Int32)),
                length: 4,
            },
            payload,
            vec![4, 1],
        );
        let owner = ObjectRef::new(gc_handle, HeapStorage::Vec(Box::new(vector)));
        let origin = ManagedPtr::new(
            owner.pointer(),
            TypeDescription::NULL,
            Some(owner),
            false,
            Some(ByteOffset::new(0)),
        );
        let target = ManagedPtr::new(
            owner.pointer(),
            TypeDescription::NULL,
            Some(owner),
            false,
            Some(ByteOffset::new(8)),
        );

        // AddByteOffset must form element 2, and the dereference observes its initialized value.
        let mut ctx = engine.ves_context(gc_handle);
        ctx.push(StackValue::ManagedPtr(origin.clone().into()));
        ctx.push(StackValue::NativeInt(8));
        assert!(matches!(
            intrinsic_unsafe_add_byte_offset(&mut ctx, is_address_less_than.clone(), &add_generics),
            StepResult::Continue
        ));
        let StackValue::ManagedPtr(at_element_two) = ctx.pop() else {
            panic!("Unsafe.AddByteOffset must return a managed pointer");
        };
        // SAFETY: F10.RawMemoryAccessValid — this pointer is element 2 of the rooted Int32[4].
        let value = unsafe {
            at_element_two.with_data(4, |data| {
                i32::from_ne_bytes(data[..4].try_into().expect("four-byte Int32 payload"))
            })
        };
        assert_eq!(value, 77);

        // SubtractByteOffset returns to element 0; AreSame provides a value-bearing comparison.
        ctx.push(StackValue::ManagedPtr(at_element_two));
        ctx.push(StackValue::NativeInt(8));
        assert!(matches!(
            intrinsic_unsafe_subtract_byte_offset(
                &mut ctx,
                is_address_less_than.clone(),
                &GenericLookup::default()
            ),
            StepResult::Continue
        ));
        let StackValue::ManagedPtr(back_at_origin) = ctx.pop() else {
            panic!("Unsafe.SubtractByteOffset must return a managed pointer");
        };
        ctx.push(StackValue::ManagedPtr(origin.clone().into()));
        ctx.push(StackValue::ManagedPtr(back_at_origin));
        assert!(matches!(
            intrinsic_unsafe_are_same(
                &mut ctx,
                is_address_less_than.clone(),
                &GenericLookup::default()
            ),
            StepResult::Continue
        ));
        assert!(matches!(ctx.pop(), StackValue::Int32(1)));

        // Address ordering uses unsigned addresses: 0 precedes the all-ones address.
        ctx.push(StackValue::NativeInt(0));
        ctx.push(StackValue::NativeInt(-1));
        assert!(matches!(
            intrinsic_unsafe_is_address_ordered(
                &mut ctx,
                is_address_less_than.clone(),
                &GenericLookup::default()
            ),
            StepResult::Continue
        ));
        assert!(matches!(ctx.pop(), StackValue::Int32(1)));

        // Native-integer operands cover direction at zero and the signed-width boundary.
        macro_rules! assert_byte_offset {
            ($origin:expr, $target:expr, $expected:expr) => {{
                ctx.push($origin);
                ctx.push($target);
                assert!(matches!(
                    intrinsic_unsafe_byte_offset(
                        &mut ctx,
                        is_address_less_than.clone(),
                        &GenericLookup::default()
                    ),
                    StepResult::Continue
                ));
                let StackValue::NativeInt(actual) = ctx.pop() else {
                    panic!("Unsafe.ByteOffset must return a native integer");
                };
                assert_eq!(actual, $expected);
            }};
        }
        assert_byte_offset!(StackValue::NativeInt(-1), StackValue::NativeInt(0), 1);
        assert_byte_offset!(StackValue::NativeInt(0), StackValue::NativeInt(-1), -1);
        assert_byte_offset!(
            StackValue::NativeInt(isize::MIN),
            StackValue::NativeInt(isize::MAX),
            -1
        );

        // ByteOffset is target - origin; the reverse managed-pointer call is also checked.
        assert_byte_offset!(
            StackValue::ManagedPtr(origin.clone().into()),
            StackValue::ManagedPtr(target.clone().into()),
            8
        );
        assert_byte_offset!(
            StackValue::ManagedPtr(target.into()),
            StackValue::ManagedPtr(origin.into()),
            -8
        );
    });
}

use Instruction as I;

// These controls each use one fresh localloc
// and never converts or dereferences its pointer before the pointer-valued store.
#[test]
fn p27_stind_i_fresh_pointer_through_local_slot_dereferences() {
    let r = run_il(
        "StindPointerLocal",
        3,
        vec![local(BaseType::IntPtr)],
        vec![
            // The local's managed address takes the stack-slot branch in stind.
            I::LoadLocalAddress(0),
            I::LoadConstantInt32(16),
            I::LocalMemoryAllocate,
            stind(StoreType::IntPtr),
            // The loaded native integer must reconstruct the allocation for both accesses.
            I::LoadLocal(0),
            I::LoadConstantInt32(41),
            stind(StoreType::Int32),
            I::LoadLocal(0),
            ldind(LoadType::Int32),
            I::Return,
        ],
    );
    assert_eq!(r, 41);
}

#[test]
fn p28_stind_i_fresh_pointer_through_heap_memory_dereferences() {
    let element_address = || I::LoadElementAddress {
        skip_type_check: false,
        skip_range_check: false,
        skip_null_check: false,
        param0: MethodType::Base(Box::new(BaseType::IntPtr)),
    };
    let r = run_il(
        "StindPointerMemory",
        3,
        vec![local(BaseType::IntPtr)],
        vec![
            // The array element is a separate heap destination, not the localloc source.
            I::LoadConstantInt32(1),
            I::NewArray(MethodType::Base(Box::new(BaseType::IntPtr))),
            I::StoreLocal(0),
            I::LoadLocal(0),
            I::LoadConstantInt32(0),
            element_address(),
            I::LoadConstantInt32(16),
            I::LocalMemoryAllocate,
            stind(StoreType::IntPtr),
            I::LoadLocal(0),
            I::LoadConstantInt32(0),
            element_address(),
            ldind(LoadType::IntPtr),
            I::LoadConstantInt32(73),
            stind(StoreType::Int32),
            I::LoadLocal(0),
            I::LoadConstantInt32(0),
            element_address(),
            ldind(LoadType::IntPtr),
            ldind(LoadType::Int32),
            I::Return,
        ],
    );
    assert_eq!(r, 73);
}

// Cover unmanaged `conv.i`/`conv.i8` acceptance, pointer-valued `stind.i`, and a managed
// array-element address converted with `conv.i`. Every transition is followed by a real read.
#[test]
fn p25_convi_and_stind_i_pointer_roundtrip_deref() {
    let r = run_il(
        "P25",
        4,
        vec![
            local(BaseType::IntPtr),
            local(BaseType::IntPtr),
            local(BaseType::Int32),
            local(BaseType::IntPtr),
        ],
        vec![
            I::LoadConstantInt32(32),
            I::LocalMemoryAllocate,
            I::StoreLocal(0),
            // A: conv.i of the localloc pointer, then read through the original pointer.
            I::LoadLocal(0),
            I::Convert(ConversionType::IntPtr),
            I::StoreLocal(1),
            I::LoadLocal(1),
            I::LoadConstantInt32(11),
            stind(StoreType::Int32),
            I::LoadLocal(0),
            ldind(LoadType::Int32),
            I::StoreLocal(2),
            // B: both conversions retain the address; the following dereference updates p[0].
            I::LoadLocal(0),
            I::Convert(ConversionType::Int64),
            I::Convert(ConversionType::UIntPtr),
            I::LoadConstantInt32(13),
            stind(StoreType::Int32),
            I::LoadLocal(2),
            I::LoadLocal(0),
            ldind(LoadType::Int32),
            I::Add,
            I::StoreLocal(2),
            // C: stind.i receives the unmanaged pointer itself, without an integer conversion.
            I::LoadLocal(0),
            I::LoadConstantInt32(16),
            I::Add,
            I::LoadLocal(0),
            stind(StoreType::IntPtr),
            // Load the stored native integer and use it as the destination of a real write.
            I::LoadLocal(0),
            I::LoadConstantInt32(16),
            I::Add,
            ldind(LoadType::IntPtr),
            I::LoadConstantInt32(100),
            stind(StoreType::Int32),
            I::LoadLocal(2),
            I::LoadLocal(0),
            ldind(LoadType::Int32),
            I::Add,
            I::StoreLocal(2),
            // D: convert an array element's managed address and confirm the array observes it.
            I::LoadConstantInt32(4),
            I::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
            I::StoreLocal(3),
            I::LoadLocal(3),
            I::LoadConstantInt32(1),
            ldelema_i4(),
            I::Convert(ConversionType::IntPtr),
            I::StoreLocal(1),
            I::LoadLocal(1),
            I::LoadConstantInt32(7),
            stind(StoreType::Int32),
            I::LoadLocal(2),
            I::LoadLocal(3),
            I::LoadConstantInt32(1),
            I::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: LoadType::Int32,
            },
            I::Add,
            I::StoreLocal(2),
            I::LoadLocal(2),
            I::Return,
        ],
    );
    // A: r=11; B: r=11+13=24; C: r=24+100=124; D: r=124+7=131.
    assert_eq!(r, 131);
}

// Round-trip native-width signed/unsigned boundaries through stind.i and both native load widths.
// Each comparison contributes one bit only when its expected result holds; the returned mask
// therefore contains every asserted bit only when every conversion, comparison, and dereference
// below behaves as specified.
#[test]
fn p26_native_int_boundary_store_load_roundtrip() {
    let mut instructions = vec![
        I::LoadConstantInt32(32),
        I::LocalMemoryAllocate,
        I::StoreLocal(0),
        I::LoadConstantInt32(0),
        I::StoreLocal(1),
        // conv.i sign-extends i32::MIN; conv.u zero-extends the same 32-bit input.
        I::LoadLocal(0),
        I::LoadConstantInt32(i32::MIN),
        I::Convert(ConversionType::IntPtr),
        stind(StoreType::IntPtr),
        I::LoadLocal(0),
        I::LoadConstantInt32(8),
        I::Add,
        I::LoadConstantInt32(i32::MIN),
        I::Convert(ConversionType::UIntPtr),
        stind(StoreType::IntPtr),
    ];
    let mut bit_index = 0;
    macro_rules! record_bit {
        ($expected:expr; $($condition:expr),+ $(,)?) => {{
            instructions.extend([$($condition),+]);
            if !$expected {
                instructions.extend([I::LoadConstantInt32(0), I::CompareEqual]);
            }
            instructions.push(I::LoadConstantInt32(1 << bit_index));
            instructions.push(I::Multiply);
            instructions.extend([I::LoadLocal(1), I::Add, I::StoreLocal(1)]);
            bit_index += 1;
        }};
    }
    macro_rules! load_slot {
        ($offset:expr, $load_type:expr, $convert_native:expr) => {{
            instructions.extend([
                I::LoadLocal(0),
                I::LoadConstantInt32($offset),
                I::Add,
                I::LoadIndirect {
                    param0: $load_type,
                    unaligned: None,
                    volatile: false,
                },
            ]);
            if $convert_native {
                instructions.push(I::Convert(ConversionType::Int64));
            }
        }};
    }
    macro_rules! check_min_value {
        ($offset:expr, $load_type:expr, $convert_native:expr, $expected:expr, $is_negative:expr) => {{
            load_slot!($offset, $load_type, $convert_native);
            record_bit!(true; I::LoadConstantInt64($expected), I::CompareEqual);
            load_slot!($offset, $load_type, $convert_native);
            record_bit!($is_negative; I::LoadConstantInt64(0), I::CompareLess(NumberSign::Signed));
            load_slot!($offset, $load_type, $convert_native);
            record_bit!(!$is_negative; I::LoadConstantInt64(0), I::CompareGreater(NumberSign::Signed));
            load_slot!($offset, $load_type, $convert_native);
            record_bit!(true; I::LoadConstantInt64(0), I::CompareGreater(NumberSign::Unsigned));
        }};
    }
    macro_rules! check_roundtrip {
        ($store:expr, $expected:expr) => {{
            instructions.extend([
                I::LoadLocal(0),
                I::LoadConstantInt32(16),
                I::Add,
                I::LoadConstantInt64($store),
                I::Convert(ConversionType::IntPtr),
                stind(StoreType::IntPtr),
            ]);
            load_slot!(16, LoadType::Int64, false);
            record_bit!(true; I::LoadConstantInt64($expected), I::CompareEqual);
            load_slot!(16, LoadType::IntPtr, true);
            record_bit!(true; I::LoadConstantInt64($expected), I::CompareEqual);
        }};
    }

    // The i8 and native loads each check exact value plus signed/unsigned zero comparisons.
    check_min_value!(0, LoadType::Int64, false, -2_147_483_648, true);
    check_min_value!(0, LoadType::IntPtr, true, -2_147_483_648, true);
    check_min_value!(8, LoadType::Int64, false, 2_147_483_648, false);
    check_min_value!(8, LoadType::IntPtr, true, 2_147_483_648, false);

    // isize::MIN, isize::MAX, and usize::MAX (all 64-bit on the supported Miri host).
    check_roundtrip!(isize::MIN as i64, isize::MIN as i64);
    check_roundtrip!(isize::MAX as i64, isize::MAX as i64);
    instructions.extend([
        I::LoadLocal(0),
        I::LoadConstantInt32(16),
        I::Add,
        I::LoadConstantInt64(-1),
        I::Convert(ConversionType::UIntPtr),
        stind(StoreType::IntPtr),
    ]);
    load_slot!(16, LoadType::Int64, false);
    record_bit!(true; I::LoadConstantInt64(-1), I::CompareEqual);
    load_slot!(16, LoadType::IntPtr, true);
    record_bit!(true; I::LoadConstantInt64(-1), I::CompareEqual);

    // Expose/store the buffer's own pointer, reload it with ldind.i, and prove it can dereference.
    instructions.extend([
        I::LoadLocal(0),
        I::LoadConstantInt32(24),
        I::Add,
        I::LoadLocal(0),
        conv_u(),
        stind(StoreType::IntPtr),
        I::LoadLocal(0),
        I::LoadConstantInt32(24),
        I::Add,
        ldind(LoadType::IntPtr),
        I::LoadConstantInt32(0x1357),
        stind(StoreType::Int32),
        I::LoadLocal(0),
        ldind(LoadType::Int32),
    ]);
    record_bit!(true; I::LoadConstantInt32(0x1357), I::CompareEqual);
    instructions.extend([I::LoadLocal(1), I::Return]);

    let expected_mask = (1_i32 << bit_index) - 1;
    let r = run_il(
        "P26",
        4,
        vec![local(BaseType::IntPtr), local(BaseType::Int32)],
        instructions,
    );
    assert_eq!(r, expected_mask);
}

fn ptr_local() -> LocalVariable {
    local(BaseType::ValuePointer(vec![], None))
}

/// C#: byte* p = stackalloc byte[16]; byte** pp = &p; *pp = null; return p == null ? 1 : 0;
#[test]
fn p_null_through_pointer_local_address() {
    let r = run_il(
        "Pass2Null",
        8,
        vec![ptr_local()],
        vec![
            I::LoadConstantInt32(16),
            I::LocalMemoryAllocate,
            I::StoreLocal(0),
            I::LoadLocalAddress(0),
            I::LoadConstantInt32(0),
            I::Convert(ConversionType::IntPtr),
            stind(StoreType::IntPtr),
            I::LoadLocal(0),
            I::LoadConstantInt32(0),
            I::Convert(ConversionType::IntPtr),
            I::CompareEqual,
            I::Return,
        ],
    );
    assert_eq!(r, 1);
}

/// C#: int x = 0; int y = 0x55; *(long*)&x = -1; return y;   (unverifiable but legal IL type pun)
#[test]
#[ignore = "outcome not yet verified; run only via scripts/pointer_provenance.sh diagnostic"]
fn p_wide_store_into_int_local() {
    let r = run_il(
        "Pass2Wide",
        8,
        vec![local(BaseType::Int32), local(BaseType::Int32)],
        vec![
            I::LoadConstantInt32(0x55),
            I::StoreLocal(1),
            I::LoadLocalAddress(0),
            I::LoadConstantInt64(-1),
            stind(StoreType::Int64),
            I::LoadLocal(1),
            I::Return,
        ],
    );
    assert_eq!(r, 0x55);
}

#[test]
fn p29_localloc_blocks_survive_growth_and_are_16_byte_aligned() {
    const SIZES: [i32; 7] = [1, 16, 17, 0, 4096, 65536, 15];

    let mut instructions = Vec::new();
    for (index, &size) in SIZES.iter().enumerate() {
        let local_index = index as u16;
        instructions.extend([
            I::LoadConstantInt32(size),
            I::LocalMemoryAllocate,
            I::StoreLocal(local_index),
            I::LoadLocal(local_index),
            conv_u(),
            I::LoadConstantInt32(15),
            I::And,
            I::LoadConstantInt32(0),
            I::CompareEqual,
        ]);
        if index != 0 {
            instructions.push(I::And);
        }

        let first_value = 0x20 + index as i32;
        instructions.extend([
            I::LoadLocal(local_index),
            I::LoadConstantInt32(first_value),
            stind(StoreType::Int8),
        ]);
        if size > 1 {
            let last_value = 0x90 + index as i32;
            instructions.extend([
                I::LoadLocal(local_index),
                I::LoadConstantInt32(size - 1),
                I::Add,
                I::LoadConstantInt32(last_value),
                stind(StoreType::Int8),
            ]);
        }
        // For size 0 the guaranteed one-byte backing block, like a one-byte request,
        // has the same address for its first and last byte.
    }

    for (index, &size) in SIZES.iter().enumerate() {
        let local_index = index as u16;
        let first_value = 0x20 + index as i32;
        instructions.extend([
            I::LoadLocal(local_index),
            ldind(LoadType::UInt8),
            I::LoadConstantInt32(first_value),
            I::CompareEqual,
            I::And,
        ]);

        let last_offset = if size > 1 { size - 1 } else { 0 };
        let last_value = if size > 1 {
            0x90 + index as i32
        } else {
            first_value
        };
        instructions.extend([
            I::LoadLocal(local_index),
            I::LoadConstantInt32(last_offset),
            I::Add,
            ldind(LoadType::UInt8),
            I::LoadConstantInt32(last_value),
            I::CompareEqual,
            I::And,
        ]);
    }
    instructions.push(I::Return);

    let r = run_il(
        "LocallocBlocksGrowthAlignment",
        4,
        (0..SIZES.len()).map(|_| local(BaseType::IntPtr)).collect(),
        instructions,
    );
    assert_eq!(r, 1);
}

#[test]
fn p30_localloc_block_survives_nested_callee_localloc() {
    let r = run_program("LocallocNestedCallee", |res, type_idx| {
        let callee_body = make_test_method!(
            max_stack: 3,
            instructions: vec![
                Instruction::LoadConstantInt32(8192),
                Instruction::LocalMemoryAllocate,
                Instruction::StoreLocal(0),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(8191),
                Instruction::Add,
                Instruction::LoadConstantInt32(0x5a),
                stind(StoreType::Int8),
                Instruction::Return,
            ],
            locals: vec![local(BaseType::IntPtr)],
        );
        let callee_idx = res.push_method(
            type_idx,
            Method::new(
                Accessibility::Public,
                void_sig(),
                "Callee",
                Some(callee_body),
            ),
        );

        (
            3,
            vec![local(BaseType::IntPtr)],
            vec![
                Instruction::LoadConstantInt32(16),
                Instruction::LocalMemoryAllocate,
                Instruction::StoreLocal(0),
                Instruction::LoadLocal(0),
                Instruction::LoadConstantInt32(0x2a),
                stind(StoreType::Int8),
                Instruction::Call {
                    tail_call: false,
                    param0: MethodSource::User(UserMethod::Definition(callee_idx)),
                },
                Instruction::LoadLocal(0),
                ldind(LoadType::UInt8),
                Instruction::Return,
            ],
        )
    });
    assert_eq!(r, 0x2a);
}

#[test]
fn p31_localloc_blocks_released_on_return_and_pooling() {
    let baseline = dotnet_utils::raw_block::live_blocks();
    let loader = get_mock_loader();
    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(SharedGlobalState::new(loader.clone()));
    let (mut res, _, type_idx) = make_test_assembly!(
        "LocallocReturnPooling.dll".to_string(),
        "LocallocReturnPoolingAssembly".to_string(),
        "LocallocReturnPoolingType".to_string()
    );

    let callee_body = make_test_method!(
        max_stack: 2,
        instructions: vec![
            Instruction::LoadConstantInt32(32),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            Instruction::Pop,
            Instruction::Return,
        ],
        locals: vec![local(BaseType::IntPtr)],
    );
    let callee_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            void_sig(),
            "Callee",
            Some(callee_body),
        ),
    );

    let caller_body = make_test_method!(
        max_stack: 2,
        instructions: vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            Instruction::Pop,
            Instruction::Call {
                tail_call: false,
                param0: MethodSource::User(UserMethod::Definition(callee_idx)),
            },
            Instruction::LoadConstantInt32(1),
            Instruction::Return,
        ],
        locals: vec![local(BaseType::IntPtr)],
    );
    let caller_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            parameterless_i32_signature(),
            "Run",
            Some(caller_body),
        ),
    );

    let res_s = loader.register_owned_assembly(res);
    let method_def = &res_s.definition()[caller_idx];
    let td = TypeDescription::new(res_s.clone(), type_idx);
    let method_index = td
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, method_def))
        .expect("method must exist in type definition");
    let entrypoint = MethodDescription::new(
        td,
        GenericLookup::default(),
        res_s,
        MethodMemberIndex::Method(method_index),
    );

    let mut previous_depth = 1;
    let mut caller_allocated = false;
    let mut callee_allocated = false;
    let mut callee_returned = false;
    let mut caller_returned = false;
    run_entrypoint_observed(shared, entrypoint, |engine, step_result| {
        let frame_stack = &engine.stack.execution.frame_stack;
        let depth = frame_stack.len();
        let homes = frame_stack
            .frames
            .iter()
            .chain(frame_stack.suspended_frames.iter())
            .filter(|frame| !frame.homes.is_empty())
            .count();
        assert!(
            frame_stack
                .pooled_frames
                .iter()
                .all(|frame| frame.homes.is_empty())
        );
        for frame in &frame_stack.frames {
            if !frame.state.allocations.is_empty() {
                assert!(
                    !frame.homes.is_empty(),
                    "addressed local must have a live home"
                );
            }
        }
        let blocks = dotnet_utils::raw_block::live_blocks() - homes;
        assert!(depth <= 2, "unexpected frame depth: {depth}");
        match depth {
            0 => assert_eq!(blocks, baseline),
            1 => assert!(blocks == baseline || blocks == baseline + 1),
            2 => assert!(blocks == baseline + 1 || blocks == baseline + 2),
            _ => unreachable!(),
        }

        if depth == 1 && blocks == baseline + 1 {
            assert_eq!(frame_stack.frames[0].state.allocations.len(), 1);
            caller_allocated = true;
        }
        if depth == 2 {
            assert!(caller_allocated, "callee ran before caller localloc");
            if blocks == baseline + 2 {
                assert_eq!(frame_stack.frames[0].state.allocations.len(), 1);
                assert_eq!(frame_stack.frames[1].state.allocations.len(), 1);
                callee_allocated = true;
            } else {
                assert_eq!(blocks, baseline + 1);
            }
        }

        if previous_depth == 2 && depth == 1 {
            assert!(callee_allocated, "callee returned before its localloc");
            assert_eq!(blocks, baseline + 1);
            assert_eq!(frame_stack.frames[0].state.allocations.len(), 1);
            assert!(
                frame_stack
                    .pooled_frames
                    .iter()
                    .all(|frame| frame.state.allocations.is_empty()),
                "returned callee's pooled frame retained localloc blocks"
            );
            callee_returned = true;
        }

        if depth == 0 {
            assert!(callee_returned, "caller returned before callee");
            assert_eq!(blocks, baseline);
            assert!(matches!(step_result, StepResult::Return));
            assert!(frame_stack.pooled_frames.len() >= 2);
            assert!(
                frame_stack
                    .pooled_frames
                    .iter()
                    .all(|frame| frame.state.allocations.is_empty()),
                "returned pooled frames retained localloc blocks"
            );
            caller_returned = true;
        }

        previous_depth = depth;
        if caller_returned {
            Observe::Stop
        } else {
            Observe::Continue
        }
    });

    assert!(caller_allocated, "caller localloc was not observed");
    assert!(callee_allocated, "callee localloc was not observed");
    assert!(callee_returned, "callee return was not observed");
    assert!(caller_returned, "caller return was not observed");
    assert_eq!(dotnet_utils::raw_block::live_blocks(), baseline);
}

#[test]
fn p32_localloc_blocks_released_on_tail_call_replacement() {
    let baseline = dotnet_utils::raw_block::live_blocks();
    let loader = get_mock_loader();
    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(SharedGlobalState::new(loader.clone()));
    let (mut res, _, type_idx) = make_test_assembly!(
        "LocallocTailReplacement.dll".to_string(),
        "LocallocTailReplacementAssembly".to_string(),
        "LocallocTailReplacementType".to_string()
    );

    let callee_body = make_test_method!(
        max_stack: 1,
        instructions: vec![
            Instruction::LoadConstantInt32(32),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            Instruction::Pop,
            Instruction::Return,
        ],
        locals: vec![local(BaseType::IntPtr)],
    );
    let callee_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            void_sig(),
            "Callee",
            Some(callee_body),
        ),
    );

    let caller_body = make_test_method!(
        max_stack: 1,
        instructions: vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            Instruction::Pop,
            Instruction::Call {
                tail_call: true,
                param0: MethodSource::User(UserMethod::Definition(callee_idx)),
            },
            Instruction::Return,
        ],
        locals: vec![local(BaseType::IntPtr)],
    );
    let caller_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            void_sig(),
            "Caller",
            Some(caller_body),
        ),
    );

    let res_s = loader.register_owned_assembly(res);
    let method_def = &res_s.definition()[caller_idx];
    let td = TypeDescription::new(res_s.clone(), type_idx);
    let method_index = td
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, method_def))
        .expect("method must exist in type definition");
    let entrypoint = MethodDescription::new(
        td,
        GenericLookup::default(),
        res_s,
        MethodMemberIndex::Method(method_index),
    );

    let mut caller_allocated = false;
    let mut tail_replaced = false;
    let mut callee_allocated = false;
    let mut callee_returned = false;
    let mut max_blocks = baseline;
    run_entrypoint_observed(shared, entrypoint, |engine, step_result| {
        let frame_stack = &engine.stack.execution.frame_stack;
        let depth = frame_stack.len();
        let homes = frame_stack
            .frames
            .iter()
            .chain(frame_stack.suspended_frames.iter())
            .filter(|frame| !frame.homes.is_empty())
            .count();
        assert!(
            frame_stack
                .pooled_frames
                .iter()
                .all(|frame| frame.homes.is_empty())
        );
        for frame in &frame_stack.frames {
            if !frame.state.allocations.is_empty() {
                assert!(
                    !frame.homes.is_empty(),
                    "addressed local must have a live home"
                );
            }
        }
        let blocks = dotnet_utils::raw_block::live_blocks() - homes;
        max_blocks = max_blocks.max(blocks);
        assert!(
            blocks <= baseline + 1,
            "tail-call replacement retained overlapping caller and callee blocks: {blocks} blocks"
        );
        assert!(depth <= 1, "valid tail call grew frame depth to {depth}");

        if depth == 1 {
            let allocations = frame_stack.frames[0].state.allocations.len();
            if !caller_allocated {
                if allocations == 1 {
                    assert_eq!(blocks, baseline + 1);
                    caller_allocated = true;
                } else {
                    assert_eq!(allocations, 0);
                    assert_eq!(blocks, baseline);
                }
            } else if !tail_replaced {
                if allocations == 0 && matches!(step_result, StepResult::FramePushed) {
                    assert_eq!(blocks, baseline, "caller block survived tail replacement");
                    assert!(
                        frame_stack
                            .pooled_frames
                            .iter()
                            .all(|frame| frame.state.allocations.is_empty()),
                        "replaced caller's pooled frame retained localloc blocks"
                    );
                    tail_replaced = true;
                } else {
                    assert_eq!(allocations, 1);
                    assert_eq!(blocks, baseline + 1);
                }
            } else if allocations == 1 {
                assert_eq!(blocks, baseline + 1);
                callee_allocated = true;
            } else {
                assert_eq!(allocations, 0);
                assert_eq!(blocks, baseline);
            }
        } else {
            assert!(
                tail_replaced,
                "entrypoint ended before tail-call replacement"
            );
            assert!(callee_allocated, "callee returned before its localloc");
            assert_eq!(blocks, baseline);
            assert!(matches!(step_result, StepResult::Return));
            callee_returned = true;
        }

        if callee_returned {
            Observe::Stop
        } else {
            Observe::Continue
        }
    });

    assert!(caller_allocated, "caller localloc was not observed");
    assert!(tail_replaced, "valid tail call was not honored");
    assert!(callee_allocated, "callee localloc was not observed");
    assert!(callee_returned, "callee return was not observed");
    assert!(max_blocks <= baseline + 1);
    assert_eq!(dotnet_utils::raw_block::live_blocks(), baseline);
}

#[test]
fn p33_localloc_blocks_released_on_exception_unwind() {
    frame_blocks_exception_lifecycle(None);
}

#[test]
fn p39_frame_homes_survive_filter_suspension() {
    frame_blocks_exception_lifecycle(Some(0));
    frame_blocks_exception_lifecycle(Some(1));
}

fn frame_blocks_exception_lifecycle(filter_result: Option<i32>) {
    let baseline = dotnet_utils::raw_block::live_blocks();
    let loader = get_mock_loader();
    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(SharedGlobalState::new(loader.clone()));
    let (mut res, system_runtime, type_idx) = make_test_assembly!(
        "LocallocExceptionUnwind.dll".to_string(),
        "LocallocExceptionUnwindAssembly".to_string(),
        "LocallocExceptionUnwindType".to_string()
    );

    let exception_type_ref =
        res.push_type_reference(dotnetdll::resolved::types::ExternalTypeReference::new(
            Some("System".into()),
            "Exception",
            dotnetdll::resolved::types::ResolutionScope::Assembly(system_runtime),
        ));
    let exception_ctor =
        res.push_method_reference(dotnetdll::resolved::members::ExternalMethodReference {
            parent: dotnetdll::resolved::members::MethodReferenceParent::Type(MethodType::Base(
                Box::new(BaseType::class(exception_type_ref)),
            )),
            name: ".ctor".into(),
            signature: MethodSignature {
                instance: true,
                explicit_this: false,
                calling_convention: CallingConvention::Default,
                parameters: vec![],
                return_type: ReturnType(vec![], None),
                varargs: None,
            },
            attributes: vec![],
        });

    let thrower_body = make_test_method!(
        max_stack: 1,
        instructions: vec![
            Instruction::LoadConstantInt32(64),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            Instruction::Pop,
            Instruction::LoadConstantInt32(4),
            Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32))),
            Instruction::StoreLocal(1),
            Instruction::LoadLocalAddress(1),
            Instruction::Pop,
            Instruction::NewObject(UserMethod::Reference(exception_ctor)),
            Instruction::Throw,
            Instruction::Return,
        ],
        locals: vec![local(BaseType::IntPtr), local(BaseType::Object)],
    );
    let thrower_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            void_sig(),
            "Thrower",
            Some(thrower_body),
        ),
    );

    let callee_body = make_test_method!(
        max_stack: 1,
        instructions: vec![
            Instruction::LoadConstantInt32(32),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            Instruction::Pop,
            Instruction::Call {
                tail_call: false,
                param0: MethodSource::User(UserMethod::Definition(thrower_idx)),
            },
            Instruction::Return,
        ],
        locals: vec![local(BaseType::IntPtr)],
    );
    let callee_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            void_sig(),
            "Callee",
            Some(callee_body),
        ),
    );

    let mut caller_body = make_test_method!(
        max_stack: 2,
        instructions: vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            Instruction::Pop,
            Instruction::Call {
                tail_call: false,
                param0: MethodSource::User(UserMethod::Definition(callee_idx)),
            },
            Instruction::LoadConstantInt32(0),
            Instruction::Return,
            Instruction::Pop,
            Instruction::LoadConstantInt32(42),
            Instruction::Return,
        ],
        locals: vec![local(BaseType::IntPtr)],
        data_sections: vec![body::DataSection::ExceptionHandlers(vec![body::Exception {
            kind: body::ExceptionKind::TypedException(MethodType::Base(Box::new(
                BaseType::class(exception_type_ref),
            ))),
            try_offset: 5,
            try_length: 1,
            handler_offset: 8,
            handler_length: 3,
        }])],
    );
    if let Some(result) = filter_result {
        caller_body.instructions.splice(
            8..8,
            [
                Instruction::Pop,
                Instruction::LoadConstantInt32(result),
                Instruction::EndFilter,
            ],
        );
        let body::DataSection::ExceptionHandlers(handlers) = &mut caller_body.data_sections[0]
        else {
            unreachable!()
        };
        handlers[0].handler_offset = 11;
        let mut filter = handlers[0].clone();
        filter.kind = body::ExceptionKind::Filter { offset: 8 };
        handlers.insert(0, filter);
    }
    let caller_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            parameterless_i32_signature(),
            "Run",
            Some(caller_body),
        ),
    );

    let res_s = loader.register_owned_assembly(res);
    let method_def = &res_s.definition()[caller_idx];
    let td = TypeDescription::new(res_s.clone(), type_idx);
    let method_index = td
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, method_def))
        .expect("method must exist in type definition");
    let entrypoint = MethodDescription::new(
        td,
        GenericLookup::default(),
        res_s,
        MethodMemberIndex::Method(method_index),
    );

    let mut caller_allocated = false;
    let mut callee_allocated = false;
    let mut thrower_allocated = false;
    let mut handler_started = false;
    let mut caller_returned = false;
    let mut max_suspended_frames = 0;
    struct HomeResolver<'a, 'gc>(&'a dotnet_vm_data::FrameStack<'gc>);
    impl<'gc> dotnet_value::pointer::ManagedPtrResolver<'gc> for HomeResolver<'_, 'gc> {
        fn stack_slot_base(&self, _: crate::StackSlotIndex) -> Option<std::ptr::NonNull<u8>> {
            None
        }
        fn static_storage_base(
            &self,
            _: &dotnet_value::pointer::StaticMetadata,
        ) -> Option<std::ptr::NonNull<u8>> {
            None
        }
        fn frame_home_block(
            &self,
            serial: dotnet_value::pointer::FrameHomeSerial,
        ) -> Option<(std::ptr::NonNull<u8>, usize)> {
            self.0.home_block(serial)
        }
    }
    let mut suspended_home = None;
    let mut collected_suspended = false;
    run_entrypoint_observed(shared, entrypoint, |engine, step_result| {
        let frame_stack = &engine.stack.execution.frame_stack;
        let depth = frame_stack.len();
        let homes = frame_stack
            .frames
            .iter()
            .chain(frame_stack.suspended_frames.iter())
            .filter(|frame| !frame.homes.is_empty())
            .count();
        assert!(
            frame_stack
                .pooled_frames
                .iter()
                .all(|frame| frame.homes.is_empty())
        );
        for frame in &frame_stack.frames {
            if !frame.state.allocations.is_empty() {
                assert!(
                    !frame.homes.is_empty(),
                    "addressed local must have a live home"
                );
            }
        }
        let blocks = dotnet_utils::raw_block::live_blocks() - homes;
        max_suspended_frames = max_suspended_frames.max(frame_stack.suspended_frames.len());
        if filter_result.is_none() {
            assert!(frame_stack.suspended_frames.is_empty());
        }
        for frame in &frame_stack.suspended_frames {
            if let Some(slot) = frame.homes.local(1) {
                let checked = dotnet_value::pointer::frame_home_window(
                    &HomeResolver(frame_stack),
                    slot.serial,
                    slot.home.offset,
                    ObjectRef::SIZE,
                )
                .unwrap();
                // SAFETY: F10.FrameHomeChecked — checked covers the initialized object home.
                let bytes =
                    unsafe { std::slice::from_raw_parts(checked.as_ptr(), ObjectRef::SIZE) };
                // SAFETY: F1.GcHandleRooted — guest stloc serialized this same arena's array;
                // the suspended frame is rooted and traces its home. The decoded handle stays
                // in this observer and is dropped before collection resumes.
                let object = unsafe { ObjectRef::read_unchecked(bytes) };
                object.as_heap_storage(|storage| {
                    let HeapStorage::Vec(vector) = storage else {
                        panic!("expected suspended array")
                    };
                    assert_eq!(&*vector.dims, &[4]);
                });
            }
            if let Some(slot) = frame.homes.local(0) {
                let block = frame_stack
                    .home_block(slot.serial)
                    .expect("suspended home must resolve");
                assert_eq!(block.0, slot.address);
                // SAFETY: F10.FrameHomeChecked — lookup found this live suspended frame;
                // its IntPtr home is initialized and pointer-sized.
                let checked = dotnet_value::pointer::frame_home_window(
                    &HomeResolver(frame_stack),
                    slot.serial,
                    slot.home.offset,
                    std::mem::size_of::<usize>(),
                )
                .expect("suspended home must pass range and liveness checks");
                // SAFETY: F10.FrameHomeChecked — the checked window covers the initialized IntPtr home.
                let value = unsafe { checked.as_ptr().cast::<usize>().read_unaligned() };
                // SAFETY: F10.FrameHomeChecked — the checked window is live and writable;
                // instruction execution is paused during this observer.
                unsafe { checked.as_ptr().cast::<usize>().write_unaligned(value) };
                assert!(
                    dotnet_value::pointer::frame_home_window(
                        &HomeResolver(frame_stack),
                        slot.serial,
                        block.1,
                        1,
                    )
                    .is_err()
                );
                assert_ne!(value, 0);
                suspended_home = Some((slot.serial, block.0, value));
            }
        }
        if let Some((serial, address, value)) = suspended_home
            && let Some((base, _)) = frame_stack.home_block(serial)
        {
            assert_eq!(base, address);
            // SAFETY: F10.FrameHomeChecked — the serial still resolves to a live initialized IntPtr home.
            let actual = unsafe { base.as_ptr().cast::<usize>().read_unaligned() };
            assert_eq!(actual, value);
        }
        assert!(depth <= 4, "unexpected exception frame depth: {depth}");

        let active_allocations = frame_stack
            .frames
            .iter()
            .chain(frame_stack.suspended_frames.iter())
            .map(|frame| frame.state.allocations.len())
            .sum::<usize>();
        assert_eq!(blocks, baseline + active_allocations);

        if depth > 0 && frame_stack.frames[0].state.allocations.len() == 1 {
            caller_allocated = true;
        }
        if depth > 1 && frame_stack.frames[1].state.allocations.len() == 1 {
            assert!(caller_allocated, "callee allocated before caller");
            callee_allocated = true;
        }
        if depth > 2 && frame_stack.frames[2].state.allocations.len() == 1 {
            assert!(callee_allocated, "thrower allocated before callee");
            thrower_allocated = true;
        }

        if depth == 1
            && matches!(step_result, StepResult::Exception)
            && frame_stack.frames[0].state.ip == if filter_result.is_some() { 11 } else { 8 }
        {
            if let Some((serial, _, _)) = suspended_home {
                assert!(
                    frame_stack.home_block(serial).is_none(),
                    "unwound home must stop resolving"
                );
            }
            assert!(caller_allocated, "handler began before caller localloc");
            assert!(callee_allocated, "handler began before callee localloc");
            assert!(thrower_allocated, "handler began before thrower localloc");
            assert_eq!(blocks, baseline + 1, "unwound blocks survived into catch");
            assert_eq!(frame_stack.frames[0].state.allocations.len(), 1);
            assert!(
                frame_stack
                    .pooled_frames
                    .iter()
                    .all(|frame| frame.state.allocations.is_empty()),
                "unwound pooled frames retained localloc blocks"
            );
            handler_started = true;
        }

        if depth == 0 {
            assert!(
                handler_started,
                "entrypoint returned without entering catch"
            );
            assert_eq!(blocks, baseline);
            assert!(matches!(step_result, StepResult::Return));
            caller_returned = true;
        }

        if caller_returned {
            Observe::Stop
        } else if !frame_stack.suspended_frames.is_empty() && !collected_suspended {
            collected_suspended = true;
            Observe::Collect
        } else {
            Observe::Continue
        }
    });

    assert!(caller_allocated, "caller localloc was not observed");
    assert!(callee_allocated, "callee localloc was not observed");
    assert!(thrower_allocated, "thrower localloc was not observed");
    assert!(handler_started, "catch handler was not observed");
    assert!(caller_returned, "caller return was not observed");
    if filter_result.is_some() {
        assert!(max_suspended_frames >= 2);
        assert!(collected_suspended);
        assert!(suspended_home.is_some());
    } else {
        assert_eq!(max_suspended_frames, 0);
    }
    assert_eq!(dotnet_utils::raw_block::live_blocks(), baseline);
}

#[test]
fn p34_localloc_blocks_released_on_executor_teardown() {
    let baseline = dotnet_utils::raw_block::live_blocks();
    let loader = get_mock_loader();
    #[allow(clippy::arc_with_non_send_sync)]
    let shared = Arc::new(SharedGlobalState::new(loader.clone()));
    let (mut res, _, type_idx) = make_test_assembly!(
        "LocallocExecutorTeardown.dll".to_string(),
        "LocallocExecutorTeardownAssembly".to_string(),
        "LocallocExecutorTeardownType".to_string()
    );

    let callee_body = make_test_method!(
        max_stack: 1,
        instructions: vec![
            Instruction::LoadConstantInt32(32),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            Instruction::Pop,
            Instruction::Return,
        ],
        locals: vec![local(BaseType::IntPtr)],
    );
    let callee_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            void_sig(),
            "Callee",
            Some(callee_body),
        ),
    );

    let caller_body = make_test_method!(
        max_stack: 1,
        instructions: vec![
            Instruction::LoadConstantInt32(16),
            Instruction::LocalMemoryAllocate,
            Instruction::StoreLocal(0),
            Instruction::LoadLocalAddress(0),
            Instruction::Pop,
            Instruction::Call {
                tail_call: false,
                param0: MethodSource::User(UserMethod::Definition(callee_idx)),
            },
            Instruction::LoadConstantInt32(0),
            Instruction::Return,
        ],
        locals: vec![local(BaseType::IntPtr)],
    );
    let caller_idx = res.push_method(
        type_idx,
        Method::new(
            Accessibility::Public,
            parameterless_i32_signature(),
            "Run",
            Some(caller_body),
        ),
    );

    let res_s = loader.register_owned_assembly(res);
    let method_def = &res_s.definition()[caller_idx];
    let td = TypeDescription::new(res_s.clone(), type_idx);
    let method_index = td
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, method_def))
        .expect("method must exist in type definition");
    let entrypoint = MethodDescription::new(
        td,
        GenericLookup::default(),
        res_s,
        MethodMemberIndex::Method(method_index),
    );

    let mut observed_two_live_blocks = false;
    run_entrypoint_observed(shared, entrypoint, |engine, _step_result| {
        let frame_stack = &engine.stack.execution.frame_stack;
        let homes = frame_stack
            .frames
            .iter()
            .chain(frame_stack.suspended_frames.iter())
            .filter(|frame| !frame.homes.is_empty())
            .count();
        assert!(
            frame_stack
                .pooled_frames
                .iter()
                .all(|frame| frame.homes.is_empty())
        );
        for frame in &frame_stack.frames {
            if !frame.state.allocations.is_empty() {
                assert!(
                    !frame.homes.is_empty(),
                    "addressed local must have a live home"
                );
            }
        }
        let blocks = dotnet_utils::raw_block::live_blocks() - homes;
        assert!(
            blocks <= baseline + 2,
            "unexpected localloc block count: {blocks}"
        );

        if blocks == baseline + 2 {
            assert_eq!(
                frame_stack.len(),
                2,
                "expected caller and callee to remain active"
            );
            let active_allocations = frame_stack
                .frames
                .iter()
                .map(|frame| frame.state.allocations.len())
                .sum::<usize>();
            assert_eq!(active_allocations, 2);
            observed_two_live_blocks = true;
            Observe::Stop
        } else {
            Observe::Continue
        }
    });

    assert!(
        observed_two_live_blocks,
        "two active localloc blocks were not observed"
    );
    assert_eq!(dotnet_utils::raw_block::live_blocks(), baseline);
}

#[test]
fn p40_jmp_forwards_modified_home_argument() {
    for value_type in [false, true] {
        let (shared, entrypoint) = build_program(
            if value_type {
                "JmpStructHome"
            } else {
                "JmpHome"
            },
            |res, type_idx| {
                let (arg_type, field) = if value_type {
                    let (_, ty, field) = push_test_value_type(res, "JmpStruct", false, false);
                    (ty, Some(field))
                } else {
                    (BaseType::Int32, None)
                };
                let sig = MethodSignature {
                    parameters: vec![Parameter(
                        vec![],
                        ParameterType::Value(MethodType::Base(Box::new(arg_type.clone()))),
                    )],
                    ..parameterless_i32_signature()
                };
                let mut target_instructions = vec![
                    Instruction::LoadArgumentAddress(0),
                    Instruction::Pop,
                    Instruction::LoadArgument(0),
                ];
                if let Some(field) = field {
                    target_instructions.push(Instruction::LoadField {
                        param0: field,
                        unaligned: None,
                        volatile: false,
                    });
                }
                target_instructions.push(Instruction::Return);
                let target = res.push_method(
                    type_idx,
                    Method::new(
                        Accessibility::Public,
                        sig.clone(),
                        "Target",
                        Some(make_test_method!(max_stack: 1, instructions: target_instructions)),
                    ),
                );
                let jumper = res.push_method(type_idx, Method::new(
                Accessibility::Public, sig, "Jumper",
                Some(make_test_method!(max_stack: 2, instructions: vec![
                    Instruction::LoadLocalAddress(0), Instruction::LoadConstantInt32(50),
                    stind(StoreType::Int32), Instruction::LoadLocal(0), Instruction::StoreArgument(0),
                    Instruction::LoadArgumentAddress(0), Instruction::LoadConstantInt32(73),
                    stind(StoreType::Int32),
                    Instruction::Jump(MethodSource::User(UserMethod::Definition(target))),
                ], locals: vec![local(arg_type.clone())])),
            ));
                (
                    1,
                    vec![local(arg_type)],
                    vec![
                        Instruction::LoadLocal(0),
                        Instruction::Call {
                            tail_call: false,
                            param0: MethodSource::User(UserMethod::Definition(jumper)),
                        },
                        Instruction::Return,
                    ],
                )
            },
        );
        let mut source_serial = None;
        let mut saw_target = false;
        let mut result = None;
        run_entrypoint_observed(shared, entrypoint, |engine, step| {
            let frames = &engine.stack.execution.frame_stack;
            if let Some(frame) = frames.frames.last() {
                match frame.state.info_handle.source.method().name.as_ref() {
                    "Jumper" => source_serial = Some(frame.homes.argument(0).unwrap().serial),
                    "Target" => {
                        let old = source_serial.expect("jumper must run before target");
                        assert!(frames.home_block(old).is_none());
                        assert_ne!(frame.homes.argument(0).unwrap().serial, old);
                        assert!(frames.pooled_frames.iter().all(|f| f.homes.is_empty()));
                        saw_target = true;
                    }
                    _ => {}
                }
            }
            if matches!(step, StepResult::Return) {
                result = Some(engine.stack.execution.evaluation_stack.pop().as_i32());
                Observe::Stop
            } else {
                Observe::Continue
            }
        });
        assert!(saw_target);
        assert_eq!(result, Some(73));
    }
}

// ---- Batched execution of the reviewed controls ------------------------------------------------
// CI runs the controls under Miri either as one process per control (nightly full sweep, and the
// attribution re-run after a failed batch) or as a few batches that call the same control
// functions sequentially on one thread. `get_mock_loader` is thread-local, so each batch parses
// the embedded support library once instead of once per control. Membership is authored in
// scripts/pointer_provenance.sh (`expected_controls`, third column) and checked against the
// invocations below by `pointer_provenance.sh verify-batches`; edit both together.
// The module is named `batches` so batch tests are not under the `pointer_provenance_tests::p`
// namespace that the control discovery guard enumerates. Batches only call enabled controls,
// never the ignored diagnostic probes.
mod batches {
    /// Defines one batch test. Each control runs in order on the calling thread; a panic in a
    /// control fails the whole batch, and the `start`/`ok` lines let the runner name the control
    /// that was in flight. Attributes on an entry apply to that control's run (p13 is skipped
    /// under `memory-validation` for the same reason it is ignored there).
    macro_rules! control_batch {
        ($batch:ident: [$($(#[$attr:meta])* $control:ident),+ $(,)?]) => {
            #[test]
            fn $batch() {
                $(
                    $(#[$attr])*
                    {
                        println!(
                            "batch-control start pointer_provenance_tests::{}",
                            stringify!($control)
                        );
                        super::$control();
                        println!(
                            "batch-control ok pointer_provenance_tests::{}",
                            stringify!($control)
                        );
                    }
                )+
            }
        };
    }

    control_batch!(batch_1: [
        p00_smoke,
        p07_localloc_one_past_end,
        #[cfg(not(feature = "memory-validation"))]
        p13_ldelema_managed_add_far_out_of_bounds_then_conv_u,
        p06_localloc_minus_one_then_plus_one,
        p21_localloc_signed_extreme_offsets_roundtrip,
        p25_convi_and_stind_i_pointer_roundtrip_deref,
        p04_double_localloc_write_first,
        p32_localloc_blocks_released_on_tail_call_replacement,
        p36_static_address_survives_forced_collection,
        p38_ldarga_value_type_survives_starg,
    ]);

    control_batch!(batch_2: [
        p02_ldloca_stind_i4_control,
        p11_ldelema_stind_heap_array_control,
        p14_initblk_heap_array_via_managed_ptr,
        p08_localloc_overshoot,
        p22_localloc_wrap_to_zero_roundtrip,
        p26_native_int_boundary_store_load_roundtrip,
        p29_localloc_blocks_survive_growth_and_are_16_byte_aligned,
        p33_localloc_blocks_released_on_exception_unwind,
        p01_ldloca_convu_stind_i4,
        p40_jmp_forwards_modified_home_argument,
    ]);

    control_batch!(batch_3: [
        p03_ldloca_convu_ldind_i4,
        p10_ldelema_convu_stind_heap_array,
        p16_cpblk_heap_arrays_via_managed_ptr,
        p17_localloc_ptr_ceq_self,
        p23_localloc_pointer_loop_unsigned_branch_compare,
        p27_stind_i_fresh_pointer_through_local_slot_dereferences,
        p30_localloc_block_survives_nested_callee_localloc,
        p34_localloc_blocks_released_on_executor_teardown,
        p37_ldloca_value_type_survives_stloc,
    ]);

    control_batch!(batch_4: [
        p05_single_localloc_roundtrip,
        p20_heap_convu_then_typed_write_then_raw_store,
        p_null_through_pointer_local_address,
        p18_localloc_ptr_clt,
        p24_unsafe_byte_offset_direction_and_deref,
        p28_stind_i_fresh_pointer_through_heap_memory_dereferences,
        p31_localloc_blocks_released_on_return_and_pooling,
        p35_ldflda_on_stack_value_type_then_ldind,
        p09_ldloca_convu_survives_callee_stack_growth,
        p39_frame_homes_survive_filter_suspension,
    ]);
}
