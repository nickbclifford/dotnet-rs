//! Bounded IL-level pointer provenance probes on the real VM.
//! The six controls below are enabled after Phase 2 revalidation in both configurations; all other
//! probes remain ignored with their final diagnostic-only reasons.
//!
//! Classification is summarized in docs/pointer-provenance/INVENTORY.md; the six controls passed in both
//! configurations, while the other expected outcomes remain review evidence:
//! | Probe | Finding | Work item | Lane | Expected outcome |
//! |---|---|---|---|---|
//! | `p00_smoke` | harness control | M0 | blocking-native + blocking-permissive-TB | pass (Phase 2 revalidated in both configs) |
//! | `p01_ldloca_convu_stind_i4` | slot/byref | M3 | diagnostic-permissive-TB | unknown-needs-run (suspected Miri UB) |
//! | `p02_ldloca_stind_i4_control` | slot/byref control | M3 | blocking-native + blocking-permissive-TB | pass (Phase 2 revalidated in both configs) |
//! | `p03_ldloca_convu_ldind_i4` | slot/byref read control | M3 | blocking-native + blocking-permissive-TB | pass (Phase 2 revalidated in both configs) |
//! | `p04_double_localloc_write_first` | localloc/arithmetic | M1c | diagnostic-permissive-TB | Miri UB reported group; exact current outcome unverified |
//! | `p05_single_localloc_roundtrip` | serde/transport control; localloc | M1c/M1e/M3 | blocking-native + blocking-permissive-TB | pass (Phase 2 revalidated in both configs) |
//! | `p06_localloc_minus_one_then_plus_one` | localloc/arithmetic | M1b | diagnostic-permissive-TB | Miri UB reported group; exact current outcome unverified |
//! | `p07_localloc_one_past_end` | localloc/arithmetic control | M1b | blocking-native + blocking-permissive-TB | pass (Phase 2 revalidated in both configs) |
//! | `p08_localloc_overshoot` | localloc/arithmetic | M1b | diagnostic-permissive-TB | Miri UB reported group; exact current outcome unverified |
//! | `p09_ldloca_convu_survives_callee_stack_growth` | slot/byref | M3 | diagnostic-permissive-TB | Miri UB reported group; exact current outcome unverified |
//! | `p10_ldelema_convu_stind_heap_array` | heap slice/address | M1a/M3 | diagnostic-permissive-TB | Miri UB reported group; exact current outcome unverified |
//! | `p11_ldelema_stind_heap_array_control` | heap slice/address control | M1a/M3 | blocking-native + blocking-permissive-TB | pass (Phase 2 revalidated in both configs) |
//! | `p12_ldsflda_convu_stind_static` | slot/static address; guards | M3/M1d | diagnostic-permissive-TB | unknown-needs-run (suspected Miri UB) |
//! | `p13_ldelema_managed_add_far_out_of_bounds_then_conv_u` | heap slice/address | M1a/M3 | diagnostic-permissive-TB | Miri UB reported group; exact current outcome unverified |
//! | `p14_initblk_heap_array_via_managed_ptr` | heap slice/address; block | M1a/M1f/M5 | diagnostic-permissive-TB | Miri UB reported group; exact current outcome unverified |
//! | `p15_initblk_stack_local_via_managed_ptr` | slot/byref; block | M3/M1f/M5 | diagnostic-permissive-TB | unknown-needs-run (suspected Miri UB) |
//! | `p16_cpblk_heap_arrays_via_managed_ptr` | heap slice/address; block | M1a/M1f/M5 | diagnostic-permissive-TB | Miri UB reported group; exact current outcome unverified |
//! | `p17_localloc_ptr_ceq_self` | localloc/arithmetic comparison | M1b | diagnostic-native (explicit opt-in) or diagnostic-permissive-TB | wrong result (predicted, not rerun) |
//! | `p18_localloc_ptr_clt` | localloc/arithmetic comparison | M1b | diagnostic-native (explicit opt-in) or diagnostic-permissive-TB | wrong result (predicted, not rerun) |
//! | `p19_ldelem_negative_index` | negative index/native-int | M1f | diagnostic-native or diagnostic-permissive-TB (isolated process) | native panic (reported; not rerun) |
//! | `p20_heap_convu_then_typed_write_then_raw_store` | heap slice/address interleaving | M1a/M3 | diagnostic-permissive-TB | unknown-needs-run |
//! | `p_null_through_pointer_local_address` | localloc/arithmetic null; native-int store | M1b/M3 | diagnostic-permissive-TB | unknown-needs-run |
//! | `p_wide_store_into_int_local` | slot/byref width; native-int stores | M3 | diagnostic-permissive-TB | unknown-needs-run |

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
use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
use dotnet_value::StackValue;
use dotnetdll::{
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
#[ignore = "M3: p01 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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

// ---- Claim 2: double localloc -> first pointer dangles after pool Vec reallocation -------------
#[test]
#[ignore = "M1c: p04 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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

// ---- Claim 3: UnmanagedPtr - 1 uses `offset` (UB when it leaves the allocation) -----------------
#[test]
#[ignore = "M1b: p06 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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
#[ignore = "M1b: p08 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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
    run_i32_entrypoint(shared, entrypoint)
}

// Stale-address probe: `ldloca; conv.u` then a callee needing a big evaluation stack forces the
// shared Vec<StackValue> to reallocate; the integer address held by the caller dangles.
#[test]
#[ignore = "M3: p09 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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
#[ignore = "M1a/M3: p10 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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
#[ignore = "M3/M1d: p12 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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

// ldelema + managed pointer arithmetic past the end of the array (one-past-end then beyond):
// `Unsafe.Add(ref arr[0], 100)` style. Never dereferenced; Rust-UB only if the VM forms a raw ptr.
#[test]
#[ignore = "M1a/M3: p13 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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
#[ignore = "M1a/M1f/M5: p14 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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
#[ignore = "M3/M1f/M5: p15 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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

// cpblk(&dst[0], &src[0], 16): both ManagedPtr(Heap); the write side goes through the read-only-derived pointer.
#[test]
#[ignore = "M1a/M1f/M5: p16 known or suspected host UB; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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
// Batch 5: semantic and panic probes; native execution requires explicit diagnostic selection.
// =================================================================================================
// Agent claim: ceq on two identical localloc pointers is false (no UnmanagedPtr arm in PartialEq).
#[test]
#[ignore = "M1b: p17 predicted semantic failure; explicit diagnostic only via scripts/pointer_provenance.sh diagnostic"]
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

// Agent claim: clt on stackalloc pointers is false both ways; `for (p = buf; p < end; p++)` never runs.
#[test]
#[ignore = "M1b: p18 predicted semantic failure; explicit diagnostic only via scripts/pointer_provenance.sh diagnostic"]
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
#[ignore = "M1f: p19 negative index host panic/abort reproducer; explicit diagnostic only via scripts/pointer_provenance.sh diagnostic"]
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
#[ignore = "M1a/M3: p20 unverified on baseline; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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

use Instruction as I;

fn ptr_local() -> LocalVariable {
    local(BaseType::ValuePointer(vec![], None))
}

/// C#: byte* p = stackalloc byte[16]; byte** pp = &p; *pp = null; return p == null ? 1 : 0;
#[test]
#[ignore = "M1b/M3: p_null_through_pointer_local_address unverified on baseline; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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
#[ignore = "M3: p_wide_store_into_int_local unverified on baseline; diagnostic-permissive-TB only via scripts/pointer_provenance.sh diagnostic"]
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
