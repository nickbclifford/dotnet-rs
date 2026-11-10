use super::*;
use dotnet_vm_ops::effects::{NativeDispatchMode, NativeIntrinsicHost, NativeValue, VmEffects};

#[derive(Clone, Copy, Debug)]
enum ReturnCase {
    Int32,
    Int64,
    NativeInt,
    Float32,
    Float64,
    Enum,
    Aggregate,
    ByRef,
    StringInvoke,
    DelegateString,
    DelegateOtherString,
}

thread_local! {
    static CASE: std::cell::Cell<ReturnCase> = const { std::cell::Cell::new(ReturnCase::Int32) };
    static ENTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn method(executor: &Executor, case: ReturnCase) -> MethodDescription {
    let mut res = Resolution::new(Module::new("NativeReturns.dll"));
    res.assembly = Some(Assembly::new("NativeReturns"));
    let mut enum_base = TypeDefinition::new(None, "Enum");
    enum_base.namespace = Some("System".into());
    let enum_base = res.push_type_definition(enum_base);
    let mut enumeration = TypeDefinition::new(None, "Number");
    enumeration.extends = Some(enum_base.into());
    let enumeration = res.push_type_definition(enumeration);
    res.push_field(
        enumeration,
        Field {
            attributes: vec![],
            name: "value__".into(),
            type_modifiers: vec![],
            by_ref: false,
            return_type: MemberType::Base(Box::new(BaseType::Int64)),
            accessibility: dotnetdll::resolved::members::Accessibility::Access(
                dotnetdll::resolved::Accessibility::Public,
            ),
            static_member: false,
            init_only: false,
            literal: false,
            default: None,
            not_serialized: false,
            special_name: true,
            pinvoke: None,
            runtime_special_name: true,
            offset: None,
            marshal: None,
            initial_value: None,
        },
    );
    let aggregate = res.push_type_definition(TypeDefinition::new(None, "Aggregate"));
    let mut delegate = TypeDefinition::new(None, "MulticastDelegate");
    delegate.namespace = Some("System".into());
    let delegate = res.push_type_definition(delegate);
    let mut owner = TypeDefinition::new(None, "IntrinsicOwner");
    if matches!(
        case,
        ReturnCase::DelegateString | ReturnCase::DelegateOtherString
    ) {
        owner.extends = Some(delegate.into());
    }
    let owner = res.push_type_definition(owner);
    let ty = match case {
        ReturnCase::Int32 | ReturnCase::ByRef => BaseType::Int32,
        ReturnCase::Int64 => BaseType::Int64,
        ReturnCase::NativeInt => BaseType::IntPtr,
        ReturnCase::Float32 => BaseType::Float32,
        ReturnCase::Float64 => BaseType::Float64,
        ReturnCase::Enum => BaseType::Type {
            source: TypeSource::User(UserType::Definition(enumeration)),
            value_kind: Some(ValueKind::ValueType),
        },
        ReturnCase::Aggregate => BaseType::Type {
            source: TypeSource::User(UserType::Definition(aggregate)),
            value_kind: Some(ValueKind::ValueType),
        },
        ReturnCase::StringInvoke | ReturnCase::DelegateString | ReturnCase::DelegateOtherString => {
            BaseType::String
        }
    };
    let ty = MethodType::Base(Box::new(ty));
    let mut sig = parameterless_i32_signature();
    sig.parameters.push(Parameter(
        vec![],
        ParameterType::Value(MethodType::Base(Box::new(BaseType::Int32))),
    ));
    sig.return_type = ReturnType(
        vec![],
        Some(if matches!(case, ReturnCase::ByRef) {
            ParameterType::Ref(ty)
        } else {
            ParameterType::Value(ty)
        }),
    );
    let name = if matches!(case, ReturnCase::StringInvoke | ReturnCase::DelegateString) {
        "Invoke"
    } else {
        "Native"
    };
    res.push_method(owner, Method::new(Accessibility::Public, sig, name, None));
    let res = executor.shared.loader.register_owned_assembly(res);
    MethodDescription::new(
        TypeDescription::new(res.clone(), owner),
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(0),
    )
}

fn return_body(mut fx: VmEffects) -> dotnet_vm_ops::effects::NativeFuture {
    ENTERED.with(|entered| entered.set(true));
    Box::pin(async move {
        let mut vm = fx.vm().await?;
        vm.discard_scalars(1)?;
        let value = vm.with(|region| {
            let value = CASE.with(|case| match case.get() {
                ReturnCase::Int32 => StackValue::Int32(17),
                ReturnCase::Int64 | ReturnCase::Enum => StackValue::Int64(42),
                ReturnCase::NativeInt => StackValue::NativeInt(19),
                ReturnCase::Float32 | ReturnCase::Float64 => StackValue::NativeFloat(1.5),
                ReturnCase::StringInvoke
                | ReturnCase::DelegateString
                | ReturnCase::DelegateOtherString => StackValue::null(),
                _ => StackValue::Int32(0),
            });
            region.root(value)
        })??;
        vm.push_value(NativeValue::Value(value))
    })
}

fn run(case: ReturnCase) -> (StepResult, bool, Vec<StackValue<'static>>, usize) {
    let mut executor = executor();
    let method = method(&executor, case);
    CASE.with(|slot| slot.set(case));
    ENTERED.with(|slot| slot.set(false));
    executor
        .with_arena(|arena| {
            arena.mutate_root(|gc, engine| {
                // SAFETY: F1.GcHandleRooted — the engine remains the live arena root throughout this mutation.
                let gc = unsafe { test_gc_handle(gc, engine) };
                let mut ctx = engine.ves_context(gc);
                ctx.push(StackValue::Int32(99));
                let result = ctx.start_native(
                    method,
                    NativeDispatchMode::InlineCall,
                    &GenericLookup::default(),
                    None,
                    Some(native_test_fallback),
                    return_body,
                );
                let values = ctx
                    .evaluation_stack
                    .stack
                    .iter()
                    .map(|value| match value {
                        StackValue::Int32(v) => StackValue::Int32(*v),
                        StackValue::Int64(v) => StackValue::Int64(*v),
                        StackValue::NativeInt(v) => StackValue::NativeInt(*v),
                        StackValue::NativeFloat(v) => StackValue::NativeFloat(*v),
                        StackValue::ObjectRef(ObjectRef(None)) => StackValue::null(),
                        _ => panic!("unexpected output"),
                    })
                    .collect();
                (
                    result,
                    ENTERED.with(|slot| slot.get()),
                    values,
                    ctx.frame_stack.current_frame().stack_height.as_usize(),
                )
            })
        })
        .unwrap()
}

#[test]
fn native_activation_scalar_and_enum_returns() {
    for (case, expected) in [
        (ReturnCase::Int32, StackValue::Int32(17)),
        (ReturnCase::Int64, StackValue::Int64(42)),
        (ReturnCase::NativeInt, StackValue::NativeInt(19)),
        (ReturnCase::Float32, StackValue::NativeFloat(1.5)),
        (ReturnCase::Float64, StackValue::NativeFloat(1.5)),
        (ReturnCase::Enum, StackValue::Int64(42)),
    ] {
        let (result, entered, stack, height) = run(case);
        assert_eq!(result, StepResult::Jump(1), "{case:?}");
        assert!(entered, "{case:?}");
        assert_eq!(stack, vec![expected], "{case:?}");
        assert_eq!(height, 1, "{case:?}");
    }
}

#[test]
fn native_activation_unsupported_returns_preserve_operands() {
    for case in [ReturnCase::Aggregate, ReturnCase::ByRef] {
        let (result, entered, stack, height) = run(case);
        assert!(matches!(result, StepResult::Error(_)), "{case:?}");
        assert!(!entered, "{case:?}");
        assert_eq!(stack, vec![StackValue::Int32(99)], "{case:?}");
        assert_eq!(height, 1, "{case:?}");
    }
}

#[test]
fn native_activation_delegate_string_output_is_not_selected_by_method_name() {
    let (result, _, _, _) = run(ReturnCase::StringInvoke);
    assert!(matches!(result, StepResult::Error(_)));
    for case in [ReturnCase::DelegateString, ReturnCase::DelegateOtherString] {
        let (result, _, values, _) = run(case);
        assert_eq!(result, StepResult::Jump(1), "{case:?}");
        assert_eq!(values, vec![StackValue::null()], "{case:?}");
    }
}
