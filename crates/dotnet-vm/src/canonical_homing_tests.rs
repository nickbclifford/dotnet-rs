//! Canonical allocation service: owner-arena homing and shutdown.

use super::*;
use dotnet_assemblies::AssemblyLoader;
use dotnet_vm_ops::effects::{CanonicalKey, NativeInitWait};
use std::time::{Duration, Instant};

fn shared() -> Arc<SharedGlobalState> {
    Arc::new(SharedGlobalState::new(Arc::new(
        AssemblyLoader::new_bare("canonical_homing".to_string()).unwrap(),
    )))
}

struct ShutdownOnDrop(Arc<SharedGlobalState>);

impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        self.0.shutdown_workers();
    }
}

fn literal(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Every requester misses in its own arena, yet the object is allocated in the service arena.
#[test]
fn canonical_homing_owner_arena() {
    let state = shared();
    let _shutdown = ShutdownOnDrop(state.clone());
    let main = Executor::new(state.clone());
    main.start_finalizer_thread();

    let owner = state
        .canonical
        .owner()
        .expect("service opened before start returns");
    assert_ne!(
        owner, main.thread_id,
        "service owner is the finalizer arena"
    );
    assert!(state.canonical.is_active());

    // The empty string is installed before admission, in the owner's arena.
    let empty = *state
        .interned_strings
        .get(&[][..])
        .expect("empty installed at startup");
    assert_eq!(empty.owner_id(), owner);

    // A request from a non-owner is served by the owner: the arena is not the requester's.
    let key = CanonicalKey::String(literal("canonical-homing literal"));
    let wait = state.canonical.request(&key).expect("admitted");
    let sleeper = wait.sleeper();
    wait_until("service completion", || {
        if !wait.ready() {
            sleeper.wait(Duration::from_millis(5));
        }
        wait.ready()
    });
    let homed = *state
        .interned_strings
        .get(literal("canonical-homing literal").as_slice())
        .expect("published before completion");
    assert_eq!(homed.owner_id(), owner);
    assert_ne!(homed.owner_id(), main.thread_id);

    // Simultaneous misses share one object.
    let a = state
        .canonical
        .request(&CanonicalKey::String(literal("shared")))
        .unwrap();
    let b = state
        .canonical
        .request(&CanonicalKey::String(literal("shared")))
        .unwrap();
    wait_until("shared request", || a.ready() && b.ready());
    assert_eq!(
        state
            .interned_strings
            .get(literal("shared").as_slice())
            .unwrap()
            .owner_id(),
        owner
    );
}

/// Assembly wrappers and their reverse metadata mapping are published by the owner.
#[test]
fn canonical_homing_assembly_owner_and_race() {
    use dotnetdll::prelude::{Assembly, Module, Resolution, TypeDefinition, UserType};

    let loader = AssemblyLoader::new_bare("canonical_homing".to_string()).unwrap();
    let mut core = Resolution::new(Module::new("System.Private.CoreLib.dll"));
    core.assembly = Some(Assembly::new("System.Private.CoreLib"));
    let mut object = TypeDefinition::new(None, "Object");
    object.namespace = Some("System".into());
    let object = core.push_type_definition(object);
    for name in ["ValueType", "Enum", "Array", "String"] {
        let mut ty = TypeDefinition::new(None, name);
        ty.namespace = Some("System".into());
        ty.extends = Some(UserType::Definition(object).into());
        core.push_type_definition(ty);
    }
    let mut assembly = TypeDefinition::new(None, "RuntimeAssembly");
    assembly.namespace = Some("System.Reflection".into());
    assembly.extends = Some(UserType::Definition(object).into());
    core.push_type_definition(assembly);
    let resolution = loader.register_owned_assembly(core);
    let state = Arc::new(SharedGlobalState::new(Arc::new(loader)));
    let _shutdown = ShutdownOnDrop(state.clone());
    let main = Executor::new(state.clone());
    main.start_finalizer_thread();
    let owner = state.canonical.owner().expect("service owner");
    let key = CanonicalKey::Assembly(resolution.clone());
    let a = state.canonical.request(&key).expect("first request");
    let b = state.canonical.request(&key).expect("second request");
    wait_until("assembly requests", || a.ready() && b.ready());
    let object = *state
        .reflection_registry
        .runtime_assembly_objects
        .get(&resolution)
        .expect("assembly published");
    assert_eq!(object.owner_id(), owner);
    assert_ne!(object.owner_id(), main.thread_id);
    assert_eq!(
        *state
            .reflection_registry
            .runtime_assembly_resolutions
            .get(&object)
            .expect("reverse mapping published"),
        resolution
    );
}

/// A failed materialization wakes the requester without publishing a partial object.
#[test]
fn canonical_homing_assembly_failure() {
    let state = shared();
    let _shutdown = ShutdownOnDrop(state.clone());
    let main = Executor::new(state.clone());
    main.start_finalizer_thread();
    let resolution = dotnet_types::resolution::ResolutionS::NULL;
    let key = CanonicalKey::Assembly(resolution.clone());
    let first = state.canonical.request(&key).expect("request admitted");
    wait_until("failed assembly request", || first.ready());
    assert!(
        !state
            .reflection_registry
            .runtime_assembly_objects
            .contains_key(&resolution)
    );
    assert!(matches!(
        state.canonical.request(&key),
        Err(crate::threading::canonical::CanonicalError::Materialization(_))
    ));
}

/// Shutdown closes admission and releases anything that was still waiting.
#[test]
fn canonical_homing_shutdown_closes_admission() {
    let state = shared();
    let main = Executor::new(state.clone());
    main.start_finalizer_thread();
    assert!(state.canonical.is_active());
    state.shutdown_workers();
    assert!(
        !state.canonical.is_active(),
        "closed with the finalizer owner"
    );
    assert!(
        state
            .canonical
            .request(&CanonicalKey::String(literal("late")))
            .is_err()
    );
    // Without an active service a miss falls back to local allocation instead of waiting.
    assert!(!state.canonical.must_queue(main.thread_id));
}

/// Each reflection key is materialized in the service arena and visible to all requesters.
#[test]
fn canonical_homing_token_objects_owner_and_race() {
    use crate::stack::BasePointer;
    use dotnet_types::{
        TypeDescription,
        generics::GenericLookup,
        members::{FieldDescription, MethodDescription},
        runtime::RuntimeType,
    };
    use dotnetdll::prelude::*;

    let loader = AssemblyLoader::new_bare("canonical_homing".to_string()).unwrap();
    let mut core = Resolution::new(Module::new("System.Private.CoreLib.dll"));
    core.assembly = Some(Assembly::new("System.Private.CoreLib"));
    let mut object = TypeDefinition::new(None, "Object");
    object.namespace = Some("System".into());
    let object = core.push_type_definition(object);
    for (namespace, name) in [
        ("System", "ValueType"),
        ("System", "Enum"),
        ("System", "Array"),
        ("System", "String"),
        ("System", "Type"),
        ("System.Reflection", "MethodInfo"),
        ("System.Reflection", "ConstructorInfo"),
        ("System.Reflection", "FieldInfo"),
    ] {
        let mut ty = TypeDefinition::new(None, name);
        ty.namespace = Some(namespace.into());
        ty.extends = Some(UserType::Definition(object).into());
        core.push_type_definition(ty);
    }
    let subject = core.push_type_definition(TypeDefinition::new(None, "Subject"));
    core[subject].extends = Some(UserType::Definition(object).into());
    let member = core.push_method(
        subject,
        Method::new(
            Accessibility::Public,
            crate::test_utils::parameterless_i32_signature(),
            "Test",
            None,
        ),
    );
    let field_index = core.push_field(
        subject,
        Field::static_member(
            Accessibility::Public,
            "Value",
            MemberType::Base(Box::new(BaseType::Int32)),
        ),
    );
    let race = core.push_type_definition(TypeDefinition::new(None, "RaceSubject"));
    core[race].extends = Some(UserType::Definition(object).into());
    let runner = core.push_type_definition(TypeDefinition::new(None, "Runner"));
    core[runner].extends = Some(UserType::Definition(object).into());
    let body = make_test_method!(max_stack: 1, instructions: vec![
        Instruction::LoadTokenType(MethodType::Base(Box::new(BaseType::Type { source: TypeSource::User(UserType::Definition(subject)), value_kind: None }))),
        Instruction::Pop,
        Instruction::LoadTokenMethod(MethodSource::User(UserMethod::Definition(member))),
        Instruction::Pop,
        Instruction::LoadTokenField(FieldSource::Definition(field_index)),
        Instruction::Pop,
        Instruction::LoadConstantInt32(42),
        Instruction::Return,
    ]);
    core.push_method(
        runner,
        Method::new(
            Accessibility::Public,
            crate::test_utils::parameterless_i32_signature(),
            "Run",
            Some(body),
        ),
    );
    let mut mscorlib = core.clone();
    mscorlib.assembly = Some(Assembly::new("mscorlib"));
    loader.register_owned_assembly(mscorlib);
    let mut system_runtime = core.clone();
    system_runtime.assembly = Some(Assembly::new("System.Runtime"));
    loader.register_owned_assembly(system_runtime);
    let res = loader.register_owned_assembly(core);
    let ty = TypeDescription::new(res.clone(), subject);
    let race_type = RuntimeType::Type(TypeDescription::new(res.clone(), race));
    let run_method = MethodDescription::new(
        TypeDescription::new(res.clone(), runner),
        GenericLookup::default(),
        res.clone(),
        MethodMemberIndex::Method(0),
    );
    let lookup = GenericLookup::default();
    let method = MethodDescription::new(
        ty.clone(),
        lookup.clone(),
        res.clone(),
        MethodMemberIndex::Method(0),
    );
    let field = FieldDescription::new(ty.clone(), res, 0);
    let runtime_type = RuntimeType::Type(ty);
    let state = Arc::new(SharedGlobalState::new(Arc::new(loader)));
    let _shutdown = ShutdownOnDrop(state.clone());
    let info = state
        .caches
        .get_method_info(run_method, &GenericLookup::default(), state.clone())
        .unwrap();
    let mut main = Executor::new(state.clone());
    main.with_arena(|arena| {
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
    assert!(matches!(main.run_entrypoint(), ExecutorResult::Exited(42)));
    let owner = state.canonical.owner().unwrap();
    for key in [
        CanonicalKey::Type(runtime_type.clone()),
        CanonicalKey::Method(method.clone(), lookup.clone()),
        CanonicalKey::Field(field.clone(), lookup.clone()),
    ] {
        let object = match &key {
            CanonicalKey::Type(ty) => *state
                .reflection_registry
                .runtime_type_objects
                .get(ty)
                .expect("type instruction published"),
            CanonicalKey::Method(method, lookup) => *state
                .reflection_registry
                .runtime_method_objects
                .get(&(method.clone(), lookup.clone()))
                .expect("method instruction published"),
            CanonicalKey::Field(field, lookup) => *state
                .reflection_registry
                .runtime_field_objects
                .get(&(field.clone(), lookup.clone()))
                .expect("field instruction published"),
            _ => unreachable!(),
        };
        assert_eq!(object.owner_id(), owner);
        assert_ne!(object.owner_id(), main.thread_id);
    }
    let key = CanonicalKey::Type(race_type.clone());
    let a = state.canonical.request(&key).unwrap();
    let b = state.canonical.request(&key).unwrap();
    wait_until("reflection requests", || a.ready() && b.ready());
    let object = *state
        .reflection_registry
        .runtime_type_objects
        .get(&race_type)
        .expect("race type published");
    assert_eq!(object.owner_id(), owner);
    assert_ne!(object.owner_id(), main.thread_id);
}

/// A failed runtime type request wakes its waiters without publishing a candidate.
#[test]
fn canonical_homing_runtime_type_failure() {
    use dotnet_types::runtime::RuntimeType;

    let state = shared();
    let _shutdown = ShutdownOnDrop(state.clone());
    let main = Executor::new(state.clone());
    main.start_finalizer_thread();
    let key = CanonicalKey::Type(RuntimeType::Void);
    let wait = state.canonical.request(&key).expect("request admitted");
    wait_until("failed type request", || wait.ready());
    assert!(
        !state
            .reflection_registry
            .runtime_type_objects
            .contains_key(&RuntimeType::Void)
    );
    assert!(matches!(
        state.canonical.request(&key),
        Err(crate::threading::canonical::CanonicalError::Materialization(_))
    ));
}

/// Missing wrapper ancestry is a materialization error rather than an owner-thread panic.
#[test]
fn canonical_homing_runtime_type_missing_ancestor() {
    use dotnet_types::runtime::RuntimeType;
    use dotnetdll::prelude::{Assembly, Module, Resolution, TypeDefinition};

    let loader = AssemblyLoader::new_bare("canonical_failure".to_string()).unwrap();
    let mut core = Resolution::new(Module::new("System.Private.CoreLib.dll"));
    core.assembly = Some(Assembly::new("System.Private.CoreLib"));
    let mut object = TypeDefinition::new(None, "Object");
    object.namespace = Some("System".into());
    core.push_type_definition(object);
    loader.register_owned_assembly(core);
    let state = Arc::new(SharedGlobalState::new(Arc::new(loader)));
    let _shutdown = ShutdownOnDrop(state.clone());
    let main = Executor::new(state.clone());
    main.start_finalizer_thread();
    let key = CanonicalKey::Type(RuntimeType::Void);
    let wait = state.canonical.request(&key).expect("request admitted");
    wait_until("failed type ancestry", || wait.ready());
    assert!(
        !state
            .reflection_registry
            .runtime_type_objects
            .contains_key(&RuntimeType::Void)
    );
    assert!(matches!(
        state.canonical.request(&key),
        Err(crate::threading::canonical::CanonicalError::Materialization(_))
    ));
}

/// A field type requested by workers is published in the service arena.
#[test]
fn canonical_homing_field_type_accessor_owner() {
    use dotnet_types::{
        TypeDescription, generics::GenericLookup, members::MethodDescription, runtime::RuntimeType,
    };
    use dotnetdll::prelude::{EntryPoint, MethodMemberIndex};
    use std::{path::Path, process::Command};

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dotnet-cli");
    let output =
        std::env::temp_dir().join(format!("dotnet-canonical-field-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    let source = root.join("tests/fixtures/threading/canonical_field_access_mt_42.cs");
    let build = Command::new("dotnet")
        .arg("build")
        .arg(root.join("tests/SingleFile.csproj"))
        .arg(format!("-p:TestFile={}", source.display()))
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
    let res = loader
        .load_resolution_from_file(output.join("SingleFile.dll"))
        .unwrap();
    let EntryPoint::Method(entry) = res.entry_point.expect("entrypoint") else {
        panic!("method entrypoint expected")
    };
    let ty = TypeDescription::new(res.clone(), entry.parent_type());
    let index = ty
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, &res.definition()[entry]))
        .unwrap();
    let method = MethodDescription::new(
        ty,
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(index),
    );
    let state = Arc::new(SharedGlobalState::new(Arc::new(loader)));
    let _shutdown = ShutdownOnDrop(state.clone());
    let mut executor = Executor::new(state.clone());
    executor.entrypoint(method).unwrap();
    assert!(matches!(
        executor.run_entrypoint(),
        ExecutorResult::Exited(42)
    ));
    executor.wait_for_foreground_workers();
    let owner = state.canonical.owner().expect("service owner");
    let entry = state
        .reflection_registry
        .runtime_type_objects
        .iter()
        .find(
            |entry| matches!(entry.key(), RuntimeType::Type(ty) if ty.type_name() == "FieldValue"),
        )
        .expect("field type published");
    assert_eq!(entry.value().owner_id(), owner);
    assert_ne!(entry.value().owner_id(), executor.thread_id);
    drop(entry);
    let declaring = state
        .reflection_registry
        .runtime_type_objects
        .iter()
        .find(|entry| {
            matches!(entry.key(), RuntimeType::Type(ty) if ty.type_name() == "GenericOwner`1")
        })
        .expect("declaring type published");
    assert_eq!(declaring.value().owner_id(), owner);
    assert_ne!(declaring.value().owner_id(), executor.thread_id);
    std::fs::remove_dir_all(output).unwrap();
}

fn compile_threading_fixture(
    name: &str,
) -> (
    AssemblyLoader,
    dotnet_types::members::MethodDescription,
    std::path::PathBuf,
) {
    use dotnetdll::prelude::EntryPoint;
    use std::{path::Path, process::Command};

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dotnet-cli");
    let output = std::env::temp_dir().join(format!(
        "dotnet-canonical-{}-{}",
        std::process::id(),
        name.trim_end_matches(".cs")
    ));
    std::fs::create_dir_all(&output).unwrap();
    let source = root.join("tests/fixtures/threading").join(name);
    let build = Command::new("dotnet")
        .arg("build")
        .arg(root.join("tests/SingleFile.csproj"))
        .arg(format!("-p:TestFile={}", source.display()))
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
        "fixture build failed: {}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
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
    let EntryPoint::Method(entry) = resolution.entry_point.expect("fixture entrypoint") else {
        panic!("method entrypoint expected")
    };
    let parent = dotnet_types::TypeDescription::new(resolution.clone(), entry.parent_type());
    let index = parent
        .definition()
        .methods
        .iter()
        .position(|candidate| std::ptr::eq(candidate, &resolution.definition()[entry]))
        .unwrap();
    let method = dotnet_types::members::MethodDescription::new(
        parent,
        dotnet_types::generics::GenericLookup::default(),
        resolution,
        dotnetdll::prelude::MethodMemberIndex::Method(index),
    );
    (loader, method, output)
}

/// Reflection producers, including nested property construction, hydrate one coherent set of
/// forward/reverse registries whose newly materialized objects all belong to the service arena.
#[test]
fn canonical_homing_reflection_owner_and_reverse_caches() {
    let (loader, entry, output) = compile_threading_fixture("canonical_reflection_mt_42.cs");
    let state = Arc::new(SharedGlobalState::new(Arc::new(loader)));
    let _shutdown = ShutdownOnDrop(state.clone());
    let mut main = Executor::new(state.clone());
    main.entrypoint(entry).unwrap();
    let result = main.run_entrypoint();
    assert!(
        matches!(result, ExecutorResult::Exited(42)),
        "fixture result: {result:?}"
    );
    main.wait_for_foreground_workers();
    let owner = state.canonical.owner().expect("service owner");

    for object in state.reflection_registry.runtime_type_objects.iter() {
        assert_eq!(
            object.value().owner_id(),
            owner,
            "runtime type {:?}",
            object.key()
        );
        let index = *state
            .reflection_registry
            .runtime_types
            .get(object.key())
            .expect("forward type index")
            .value();
        assert_eq!(
            state
                .reflection_registry
                .runtime_types_rev
                .get(&index)
                .expect("reverse type index")
                .value(),
            object.key()
        );
    }
    for object in state.reflection_registry.runtime_method_objects.iter() {
        assert_eq!(
            object.value().owner_id(),
            owner,
            "runtime method {:?}",
            object.key()
        );
        let index = *state
            .reflection_registry
            .runtime_methods
            .get(object.key())
            .expect("forward method index")
            .value();
        assert_eq!(
            state
                .reflection_registry
                .runtime_methods_rev
                .get(&index)
                .expect("reverse method index")
                .value(),
            object.key()
        );
    }
    for object in state.reflection_registry.runtime_field_objects.iter() {
        assert_eq!(
            object.value().owner_id(),
            owner,
            "runtime field {:?}",
            object.key()
        );
        let index = *state
            .reflection_registry
            .runtime_fields
            .get(object.key())
            .expect("forward field index")
            .value();
        assert_eq!(
            state
                .reflection_registry
                .runtime_fields_rev
                .get(&index)
                .expect("reverse field index")
                .value(),
            object.key()
        );
    }
    for object in state.reflection_registry.runtime_property_objects.iter() {
        assert_eq!(
            object.value().owner_id(),
            owner,
            "runtime property {:?}",
            object.key()
        );
    }
    for object in state.reflection_registry.runtime_assembly_objects.iter() {
        assert_eq!(
            object.value().owner_id(),
            owner,
            "runtime assembly {:?}",
            object.key()
        );
        assert_eq!(
            state
                .reflection_registry
                .runtime_assembly_resolutions
                .get(object.value())
                .expect("reverse assembly resolution")
                .value(),
            object.key()
        );
    }
    drop(main);
    std::fs::remove_dir_all(output).unwrap();
}

/// The main-thread Join race and a guest finalizer parked joining the requesting worker both
/// continue servicing owned reflection requests while their execution frame is parked.
#[test]
fn canonical_homing_finalizer_wait_and_join_race() {
    let (loader, entry, output) = compile_threading_fixture("canonical_join_race_mt_42.cs");
    let state = Arc::new(SharedGlobalState::new(Arc::new(loader)));
    let _shutdown = ShutdownOnDrop(state.clone());
    let mut main = Executor::new(state.clone());
    main.entrypoint(entry).unwrap();
    assert!(matches!(main.run_entrypoint(), ExecutorResult::Exited(42)));
    main.wait_for_foreground_workers();
    let owner = state.canonical.owner().expect("service owner");
    for entry in state.reflection_registry.runtime_property_objects.iter() {
        assert_eq!(
            entry.value().owner_id(),
            owner,
            "runtime property {:?} not homed to service",
            entry.key()
        );
    }
    for entry in state.reflection_registry.runtime_method_objects.iter() {
        assert_eq!(
            entry.value().owner_id(),
            owner,
            "runtime method {:?} not homed to service",
            entry.key()
        );
    }
    drop(main);
    std::fs::remove_dir_all(output).unwrap();
}

/// A property whose accessor does not belong to the requested owner fails before publishing a
/// parent entry; no caller can observe a partially materialized property wrapper.
#[test]
fn canonical_homing_nested_failure() {
    use dotnet_types::{TypeDescription, generics::GenericLookup, members::MethodDescription};
    use dotnet_vm_ops::effects::CanonicalKey;
    use dotnetdll::prelude::*;

    // Property materialization must not publish its parent when a nested canonical method
    // dependency is missing. This bare loader has Object but deliberately no MethodInfo wrapper.
    let loader = AssemblyLoader::new_bare("canonical_nested_failure".to_string()).unwrap();
    let mut core = Resolution::new(Module::new("System.Private.CoreLib.dll"));
    core.assembly = Some(Assembly::new("System.Private.CoreLib"));
    let mut object = TypeDefinition::new(None, "Object");
    object.namespace = Some("System".into());
    let object = core.push_type_definition(object);
    let runner = core.push_type_definition(TypeDefinition::new(None, "Runner"));
    core[runner].extends = Some(UserType::Definition(object).into());
    let subject = core.push_type_definition(TypeDefinition::new(None, "PropertyOwner"));
    core[subject].extends = Some(UserType::Definition(object).into());
    let mut signature = crate::test_utils::parameterless_i32_signature();
    signature.instance = true;
    core.push_method(
        subject,
        Method::new(Accessibility::Public, signature, "get_Value", None),
    );
    let body = make_test_method!(max_stack: 1, instructions: vec![
        Instruction::LoadConstantInt32(42),
        Instruction::Return,
    ]);
    core.push_method(
        runner,
        Method::new(
            Accessibility::Public,
            crate::test_utils::parameterless_i32_signature(),
            "Run",
            Some(body),
        ),
    );
    let mut mscorlib = core.clone();
    mscorlib.assembly = Some(Assembly::new("mscorlib"));
    loader.register_owned_assembly(mscorlib);
    let mut system_runtime = core.clone();
    system_runtime.assembly = Some(Assembly::new("System.Runtime"));
    loader.register_owned_assembly(system_runtime);
    let resolution = loader.register_owned_assembly(core);
    let owner = TypeDescription::new(resolution.clone(), subject);
    let accessor = MethodDescription::new(
        owner,
        GenericLookup::default(),
        resolution,
        MethodMemberIndex::Method(0),
    );
    let entry = MethodDescription::new(
        TypeDescription::new(accessor.resolution().clone(), runner),
        GenericLookup::default(),
        accessor.resolution().clone(),
        MethodMemberIndex::Method(0),
    );
    let state = Arc::new(SharedGlobalState::new(Arc::new(loader)));
    let _shutdown = ShutdownOnDrop(state.clone());
    let mut main = Executor::new(state.clone());
    main.entrypoint(entry).unwrap();
    assert!(matches!(main.run_entrypoint(), ExecutorResult::Exited(42)));
    main.wait_for_foreground_workers();

    let key = CanonicalKey::Property(accessor.clone(), GenericLookup::default());
    let wait = state
        .canonical
        .request(&key)
        .expect("nested request admitted");
    let sleeper = wait.sleeper();
    wait_until("failed nested method materialization", || {
        if !wait.ready() {
            sleeper.wait(Duration::from_millis(5));
        }
        wait.ready()
    });
    assert!(matches!(
        state.canonical.request(&key),
        Err(crate::threading::canonical::CanonicalError::Materialization(_))
    ));
    assert!(
        !state
            .reflection_registry
            .runtime_property_objects
            .contains_key(&(accessor.clone(), GenericLookup::default())),
        "a failing nested method must not publish the parent property"
    );
}
