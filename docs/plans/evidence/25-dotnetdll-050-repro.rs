use dotnetdll::binary::signature::{
    encoded::{Param, ParamType, RetType, RetTypeType, Type},
    kinds::{
        CallingConvention, MethodDefSig, MethodRefSig, StandAloneCallingConvention,
        StandAloneMethodSig,
    },
};
use scroll::{Pread, Pwrite};
fn int_param() -> Param {
    Param(vec![], ParamType::Type(Type::Int32))
}
fn main() {
    let mut bytes = [0u8; 64];
    let sig = StandAloneMethodSig {
        has_this: false,
        explicit_this: false,
        calling_convention: StandAloneCallingConvention::Cdecl,
        ret_type: RetType(vec![], RetTypeType::Void),
        params: vec![int_param()],
        varargs: vec![int_param()],
    };
    let count = bytes.pwrite(sig, 0).unwrap();
    println!(
        "Cdecl with one fixed and one optional: {:02X?}",
        &bytes[..count]
    );
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let truncated_panicked = std::panic::catch_unwind(|| {
        let _ = (&bytes[..count]).pread::<StandAloneMethodSig>(0);
    })
    .is_err();
    std::panic::set_hook(hook);
    println!("Reading the emitted Cdecl signature panicked: {truncated_panicked}");
    let expected: &[u8] = &[0x01, 0x02, 0x01, 0x08, 0x41, 0x08];
    let decoded: StandAloneMethodSig = expected.pread(0).unwrap();
    println!(
        "Reading valid encoding: fixed={} optional={}",
        decoded.params.len(),
        decoded.varargs.len()
    );
    let mut bytes = [0u8; 64];
    let sig = MethodRefSig {
        method_def: MethodDefSig {
            has_this: false,
            explicit_this: false,
            calling_convention: CallingConvention::Vararg,
            ret_type: RetType(vec![], RetTypeType::Void),
            params: vec![int_param()],
        },
        varargs: vec![],
    };
    let count = bytes.pwrite(sig, 0).unwrap();
    println!(
        "Managed vararg method reference with zero optional: {:02X?}",
        &bytes[..count]
    );
    let mut bytes = [0u8; 64];
    let sig = StandAloneMethodSig {
        has_this: false,
        explicit_this: false,
        calling_convention: StandAloneCallingConvention::Vararg,
        ret_type: RetType(vec![], RetTypeType::Void),
        params: vec![int_param()],
        varargs: vec![int_param()],
    };
    let count = bytes.pwrite(sig, 0).unwrap();
    println!(
        "Managed vararg standalone with one optional: {:02X?}",
        &bytes[..count]
    );
    nested_fnptr_roundtrip();
    callvirt_no_roundtrip();
    long_branch_roundtrip();
    short_branch_roundtrip();
    exception_end_roundtrip();
    simple_exception_roundtrip();
    mixed_fat_handlers_roundtrip();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panicked = std::panic::catch_unwind(many_handlers_roundtrip).is_err();
    std::panic::set_hook(hook);
    if panicked {
        println!("21 exception clauses: reader panicked");
    }
}

fn nested_fnptr_roundtrip() {
    use dotnetdll::prelude::*;
    let mut res = Resolution::new(Module::new("fnptr.dll"));
    res.assembly = Some(Assembly::new("fnptr"));
    let owner = res.push_type_definition(TypeDefinition::new(None, "Subject"));
    let int: MethodType = BaseType::Int32.into();
    let fn_sig = MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: StandAloneCallingConvention::Vararg,
        parameters: vec![Parameter::value(int.clone())],
        return_type: ReturnType::VOID,
        varargs: Some(vec![Parameter::value(int)]),
    };
    let fn_type: MethodType = BaseType::FunctionPointer(fn_sig).into();
    let mut method_sig = msig! { static void () };
    method_sig.parameters.push(Parameter::value(fn_type));
    res.push_method(
        owner,
        Method::new(Accessibility::Public, method_sig, "Probe", None),
    );
    let bytes = res.write(WriteOptions::default()).unwrap();
    let loaded = Resolution::parse(&bytes, ReadOptions::default()).unwrap();
    println!(
        "Nested function pointer: {:?}",
        loaded[owner].methods[0].signature.parameters[0]
    );
}

fn callvirt_no_roundtrip() {
    use dotnetdll::prelude::*;
    let mut res = Resolution::new(Module::new("callvirt.dll"));
    res.assembly = Some(Assembly::new("callvirt"));
    let core = res.push_assembly_reference(ExternalAssemblyReference::new("mscorlib"));
    let object = res.push_type_reference(type_ref! { System.Object in #core });
    let object_type: MethodType = BaseType::class(object).into();
    let to_string = res.push_method_reference(method_ref! { string #object_type::ToString() });
    let owner = res.push_type_definition(TypeDefinition::new(None, "Subject"));
    res.push_method(
        owner,
        Method::new(
            Accessibility::Public,
            msig! { static void () },
            "Probe",
            Some(body::Method::new(vec![
                Instruction::CallVirtual {
                    skip_null_check: true,
                    param0: to_string.into(),
                },
                Instruction::Return,
            ])),
        ),
    );
    let bytes = res.write(WriteOptions::default()).unwrap();
    let loaded = Resolution::parse(&bytes, ReadOptions::default()).unwrap();
    println!(
        "Callvirt skip null roundtrip: {:?}",
        loaded[owner].methods[0].body.as_ref().unwrap().instructions[0]
    );
}

fn long_branch_roundtrip() {
    use dotnetdll::prelude::*;
    let mut res = Resolution::new(Module::new("long-branch.dll"));
    res.assembly = Some(Assembly::new("long-branch"));
    let owner = res.push_type_definition(TypeDefinition::new(None, "Subject"));
    let mut instrs = vec![Instruction::Branch(121)];
    for _ in 0..60 {
        instrs.push(Instruction::LoadConstantInt64(1));
        instrs.push(Instruction::Pop);
    }
    instrs.push(Instruction::Return);
    res.push_method(
        owner,
        Method::new(
            Accessibility::Public,
            msig! { static void () },
            "Probe",
            Some(body::Method::new(instrs)),
        ),
    );
    let bytes = res.write(WriteOptions::default()).unwrap();
    match Resolution::parse(&bytes, ReadOptions::default()) {
        Ok(loaded) => println!(
            "Long branch roundtrip: {:?}",
            loaded[owner].methods[0].body.as_ref().unwrap().instructions[0]
        ),
        Err(e) => println!("Long branch roundtrip error: {e}"),
    }
}

fn short_branch_roundtrip() {
    use dotnetdll::prelude::*;
    let mut res = Resolution::new(Module::new("short-branch.dll"));
    res.assembly = Some(Assembly::new("short-branch"));
    let owner = res.push_type_definition(TypeDefinition::new(None, "Subject"));
    let mut instrs = vec![Instruction::Branch(203)];
    instrs.extend((0..200).map(|_| Instruction::NoOperation));
    instrs.push(Instruction::Branch(200));
    instrs.push(Instruction::NoOperation);
    instrs.push(Instruction::Return);
    res.push_method(
        owner,
        Method::new(
            Accessibility::Public,
            msig! { static void () },
            "Probe",
            Some(body::Method::new(instrs)),
        ),
    );
    let bytes = res.write(WriteOptions::default()).unwrap();
    match Resolution::parse(&bytes, ReadOptions::default()) {
        Ok(loaded) => println!(
            "Short branch roundtrip: {:?}",
            loaded[owner].methods[0].body.as_ref().unwrap().instructions[0]
        ),
        Err(e) => println!("Short branch roundtrip error: {e}"),
    }
}

fn exception_end_roundtrip() {
    use dotnetdll::prelude::*;
    use dotnetdll::resolved::body::{DataSection, Exception, ExceptionKind};
    let mut res = Resolution::new(Module::new("handler-end.dll"));
    res.assembly = Some(Assembly::new("handler-end"));
    let core = res.push_assembly_reference(ExternalAssemblyReference::new("mscorlib"));
    let exception = res.push_type_reference(type_ref! { System.Exception in #core });
    let owner = res.push_type_definition(TypeDefinition::new(None, "Subject"));
    let mut body = body::Method::new(vec![
        Instruction::LoadNull,
        Instruction::Throw,
        Instruction::Pop,
        Instruction::Rethrow,
    ]);
    body.data_sections
        .push(DataSection::ExceptionHandlers(vec![Exception {
            kind: ExceptionKind::TypedException(BaseType::class(exception).into()),
            try_offset: 0,
            try_length: 2,
            handler_offset: 2,
            handler_length: 2,
        }]));
    res.push_method(
        owner,
        Method::new(
            Accessibility::Public,
            msig! { static void () },
            "Probe",
            Some(body),
        ),
    );
    let bytes = res.write(WriteOptions::default()).unwrap();
    match Resolution::parse(&bytes, ReadOptions::default()) {
        Ok(_) => println!("Exception handler ending after 2-byte rethrow: success"),
        Err(e) => println!("Exception handler ending after 2-byte rethrow: {e}"),
    }
}

fn many_handlers_roundtrip() {
    use dotnetdll::prelude::*;
    use dotnetdll::resolved::body::{DataSection, Exception, ExceptionKind};
    let mut res = Resolution::new(Module::new("many-handlers.dll"));
    res.assembly = Some(Assembly::new("many-handlers"));
    let core = res.push_assembly_reference(ExternalAssemblyReference::new("mscorlib"));
    let exception = res.push_type_reference(type_ref! { System.Exception in #core });
    let owner = res.push_type_definition(TypeDefinition::new(None, "Subject"));
    let mut instrs = vec![Instruction::LoadNull, Instruction::Throw];
    let mut clauses = vec![];
    for _ in 0..21 {
        let start = instrs.len();
        instrs.push(Instruction::Pop);
        instrs.push(Instruction::Rethrow);
        clauses.push(Exception {
            kind: ExceptionKind::TypedException(BaseType::class(exception).into()),
            try_offset: 0,
            try_length: 2,
            handler_offset: start,
            handler_length: 2,
        });
    }
    instrs.push(Instruction::Return);
    let mut body = body::Method::new(instrs);
    body.data_sections
        .push(DataSection::ExceptionHandlers(clauses));
    res.push_method(
        owner,
        Method::new(
            Accessibility::Public,
            msig! { static void () },
            "Probe",
            Some(body),
        ),
    );
    let bytes = res.write(WriteOptions::default()).unwrap();
    match Resolution::parse(&bytes, ReadOptions::default()) {
        Ok(loaded) => println!(
            "21 exception clauses: {:?}",
            loaded[owner].methods[0]
                .body
                .as_ref()
                .unwrap()
                .data_sections
        ),
        Err(e) => println!("21 exception clauses: {e}"),
    }
}

fn mixed_fat_handlers_roundtrip() {
    use dotnetdll::prelude::*;
    use dotnetdll::resolved::body::{DataSection, Exception, ExceptionKind};
    let mut res = Resolution::new(Module::new("mixed-handlers.dll"));
    res.assembly = Some(Assembly::new("mixed-handlers"));
    let core = res.push_assembly_reference(ExternalAssemblyReference::new("mscorlib"));
    let exception = res.push_type_reference(type_ref! { System.Exception in #core });
    let owner = res.push_type_definition(TypeDefinition::new(None, "Subject"));
    let mut instrs = vec![
        Instruction::LoadNull,
        Instruction::Throw,
        Instruction::Pop,
        Instruction::Rethrow,
    ];
    instrs.extend((0..256).map(|_| Instruction::NoOperation));
    instrs.push(Instruction::Pop);
    instrs.push(Instruction::Rethrow);
    instrs.push(Instruction::Return);
    let mut body = body::Method::new(instrs);
    body.data_sections.push(DataSection::ExceptionHandlers(vec![
        Exception {
            kind: ExceptionKind::TypedException(BaseType::class(exception).into()),
            try_offset: 0,
            try_length: 2,
            handler_offset: 2,
            handler_length: 2,
        },
        Exception {
            kind: ExceptionKind::TypedException(BaseType::class(exception).into()),
            try_offset: 0,
            try_length: 2,
            handler_offset: 4,
            handler_length: 258,
        },
    ]));
    res.push_method(
        owner,
        Method::new(
            Accessibility::Public,
            msig! { static void () },
            "Probe",
            Some(body),
        ),
    );
    let bytes = res.write(WriteOptions::default()).unwrap();
    match Resolution::parse(&bytes, ReadOptions::default()) {
        Ok(_) => println!("Mixed fat and small handler lengths: success"),
        Err(e) => println!("Mixed fat and small handler lengths: {e}"),
    }
}

fn simple_exception_roundtrip() {
    use dotnetdll::prelude::*;
    use dotnetdll::resolved::body::{DataSection, Exception, ExceptionKind};
    for filter in [false, true] {
        let mut res = Resolution::new(Module::new("exception.dll"));
        res.assembly = Some(Assembly::new("exception"));
        let owner = res.push_type_definition(TypeDefinition::new(None, "Subject"));
        let (instrs, clause) = if filter {
            (
                vec![
                    Instruction::LoadNull,
                    Instruction::Throw,
                    Instruction::Pop,
                    Instruction::LoadConstantInt32(1),
                    Instruction::EndFilter,
                    Instruction::Pop,
                    Instruction::Leave(7),
                    Instruction::Return,
                ],
                Exception {
                    kind: ExceptionKind::Filter { offset: 2 },
                    try_offset: 0,
                    try_length: 2,
                    handler_offset: 5,
                    handler_length: 2,
                },
            )
        } else {
            (
                vec![
                    Instruction::LoadNull,
                    Instruction::Throw,
                    Instruction::NoOperation,
                    Instruction::EndFinally,
                    Instruction::Return,
                ],
                Exception {
                    kind: ExceptionKind::Fault,
                    try_offset: 0,
                    try_length: 2,
                    handler_offset: 2,
                    handler_length: 2,
                },
            )
        };
        let mut body = body::Method::new(instrs);
        body.data_sections
            .push(DataSection::ExceptionHandlers(vec![clause]));
        res.push_method(
            owner,
            Method::new(
                Accessibility::Public,
                msig! { static void () },
                "Probe",
                Some(body),
            ),
        );
        let bytes = res.write(WriteOptions::default()).unwrap();
        match Resolution::parse(&bytes, ReadOptions::default()) {
            Ok(loaded) => println!(
                "Simple {} roundtrip: {:?}",
                if filter { "filter" } else { "fault" },
                loaded[owner].methods[0]
                    .body
                    .as_ref()
                    .unwrap()
                    .data_sections
            ),
            Err(e) => println!(
                "Simple {} roundtrip error: {e}",
                if filter { "filter" } else { "fault" }
            ),
        }
    }
}
