use crate::{Executor, state::SharedGlobalState, sync::Arc};
use arbitrary::Arbitrary;
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::{TypeDescription, members::MethodDescription, resolution::ResolutionS};
use dotnetdll::{
    binary::signature::kinds::CallingConvention,
    prelude::*,
    resolved::{
        self as resolved_mod,
        assembly::ExternalAssemblyReference,
        il::{ConversionType, LoadType, NumberSign, StoreType},
        members::{Field, FieldSource, Method, MethodSource, UserMethod},
        signature::{Parameter, ParameterType, ReturnType},
        types::{
            BaseType, ExternalTypeReference, LocalVariable, MemberType, MethodType,
            ResolutionScope, TypeDefinition,
        },
    },
};
use std::{path::PathBuf, sync::OnceLock};

static LOADER: OnceLock<&'static AssemblyLoader> = OnceLock::new();
#[cfg(test)]
static MOCK_LOADER: OnceLock<&'static AssemblyLoader> = OnceLock::new();

fn get_loader() -> &'static AssemblyLoader {
    LOADER.get_or_init(|| {
        let base = std::env::var("DOTNET_ROOT")
            .map(|p| PathBuf::from(p).join("shared/Microsoft.NETCore.App"))
            .unwrap_or_else(|_| PathBuf::from("/usr/share/dotnet/shared/Microsoft.NETCore.App"));

        let base = if !base.exists() {
            let alt_base = PathBuf::from("/usr/lib/dotnet/shared/Microsoft.NETCore.App");
            if alt_base.exists() { alt_base } else { base }
        } else {
            base
        };

        let mut entries: Vec<_> = std::fs::read_dir(base)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();

        entries.sort_by_key(|e| e.file_name());
        let path = entries
            .last()
            .expect("no versions found in .NET shared path")
            .path();

        let loader = AssemblyLoader::new(path.to_str().unwrap().to_string())
            .expect("Failed to create AssemblyLoader");
        Box::leak(Box::new(loader))
    })
}

#[cfg(test)]
fn get_mock_loader() -> &'static AssemblyLoader {
    MOCK_LOADER.get_or_init(|| {
        let loader = AssemblyLoader::new_bare("mock_root".to_string())
            .expect("Failed to create mock AssemblyLoader");

        // Register mock assemblies so that System.Object can be resolved
        let mut mscorlib = Resolution::new(Module::new("mscorlib.dll"));
        mscorlib.assembly = Some(Assembly::new("mscorlib"));
        let mut obj = TypeDefinition::new(None, "Object");
        obj.namespace = Some("System".into());
        mscorlib.push_type_definition(obj);

        let mscorlib_res = ResolutionS::new(Box::leak(Box::new(mscorlib)));
        loader.register_assembly(mscorlib_res);

        let mut system_runtime = Resolution::new(Module::new("System.Runtime.dll"));
        system_runtime.assembly = Some(Assembly::new("System.Runtime"));

        let mut obj2 = TypeDefinition::new(None, "Object");
        obj2.namespace = Some("System".into());
        system_runtime.push_type_definition(obj2);

        let mut type_type = TypeDefinition::new(None, "Type");
        type_type.namespace = Some("System".into());
        system_runtime.push_type_definition(type_type);

        let mut array_type = TypeDefinition::new(None, "Array");
        array_type.namespace = Some("System".into());
        system_runtime.push_type_definition(array_type);

        let mut string_type = TypeDefinition::new(None, "String");
        string_type.namespace = Some("System".into());
        system_runtime.push_type_definition(string_type);

        let system_runtime_res = ResolutionS::new(Box::leak(Box::new(system_runtime)));
        loader.register_assembly(system_runtime_res);

        Box::leak(Box::new(loader))
    })
}

#[derive(Debug, Arbitrary, Clone)]
pub enum FuzzInstruction {
    // Stack operations
    Dup,
    Pop,
    Ldnull,
    LdcI4(i32),
    LdcI8(i64),
    LdcR4(f32),
    LdcR8(f64),

    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Neg,
    Not,

    // Memory (modeled via generic load/store indirect)
    LdindI1,
    LdindI2,
    LdindI4,
    LdindI8,
    LdindU1,
    LdindU2,
    LdindU4,
    LdindR4,
    LdindR8,
    LdindI,
    StindI1,
    StindI2,
    StindI4,
    StindI8,
    StindR4,
    StindR8,
    StindI,
    Cpblk,
    Initblk,
    Localloc,

    ConvI,
    ConvU,

    Br(i8),
    Brtrue(i8),
    Brfalse(i8),
    Ret,

    // Locals and Arguments (6.3.1)
    Ldloc(u8),
    Ldloca(u8),
    Stloc(u8),
    Ldarg(u8),
    Ldarga(u8),
    Starg(u8),

    // Object operations (6.3.2)
    Newobj(u8),
    Ldfld(u8),
    Stfld(u8),
    Ldflda(u8),

    // Array operations (6.3.3)
    Newarr,
    LdelemI1,
    LdelemI2,
    LdelemI4,
    LdelemI8,
    LdelemU1,
    LdelemU2,
    LdelemU4,
    LdelemR4,
    LdelemR8,
    LdelemI,
    LdelemRef,
    Ldelema,
    StelemI1,
    StelemI2,
    StelemI4,
    StelemI8,
    StelemR4,
    StelemR8,
    StelemI,
    StelemRef,
    Ldlen,

    // Method calls (6.3.4)
    Call(u8),
    Callvirt(u8),
    Ldftn(u8),

    // Exception Handling (6.3.5)
    Throw,
    Rethrow,
    Leave(i8),
    Endfinally,
}

#[derive(Debug, Arbitrary, Clone)]
pub struct FuzzProgram {
    pub instructions: Vec<FuzzInstruction>,
    #[arbitrary(with = |u: &mut arbitrary::Unstructured| u.int_in_range(0..=10))]
    pub num_locals: u8,
    #[arbitrary(with = |u: &mut arbitrary::Unstructured| u.int_in_range(1..=10))]
    pub num_args: u8,
}

impl FuzzInstruction {
    pub fn to_dotnetdll(
        &self,
        current_ip: usize,
        total_len: usize,
        num_locals: u8,
        num_args: u8,
        fields: &[FieldSource],
        methods: &[MethodSource],
    ) -> Instruction {
        match self {
            FuzzInstruction::Dup => Instruction::Duplicate,
            FuzzInstruction::Pop => Instruction::Pop,
            FuzzInstruction::Ldnull => Instruction::LoadNull,
            FuzzInstruction::LdcI4(v) => Instruction::LoadConstantInt32(*v),
            FuzzInstruction::LdcI8(v) => Instruction::LoadConstantInt64(*v),
            FuzzInstruction::LdcR4(v) => Instruction::LoadConstantFloat32(*v),
            FuzzInstruction::LdcR8(v) => Instruction::LoadConstantFloat64(*v),

            FuzzInstruction::Add => Instruction::Add,
            FuzzInstruction::Sub => Instruction::Subtract,
            FuzzInstruction::Mul => Instruction::Multiply,
            FuzzInstruction::Div => Instruction::Divide(NumberSign::Signed), // signed by default for fuzzing
            FuzzInstruction::Rem => Instruction::Remainder(NumberSign::Signed),
            FuzzInstruction::And => Instruction::And,
            FuzzInstruction::Or => Instruction::Or,
            FuzzInstruction::Xor => Instruction::Xor,
            FuzzInstruction::Shl => Instruction::ShiftLeft,
            FuzzInstruction::Shr => Instruction::ShiftRight(NumberSign::Signed),
            FuzzInstruction::Neg => Instruction::Negate,
            FuzzInstruction::Not => Instruction::Not,

            FuzzInstruction::LdindI1 => Instruction::LoadIndirect {
                unaligned: None,
                volatile: false,
                param0: LoadType::Int8,
            },
            FuzzInstruction::LdindI2 => Instruction::LoadIndirect {
                unaligned: None,
                volatile: false,
                param0: LoadType::Int16,
            },
            FuzzInstruction::LdindI4 => Instruction::LoadIndirect {
                unaligned: None,
                volatile: false,
                param0: LoadType::Int32,
            },
            FuzzInstruction::LdindI8 => Instruction::LoadIndirect {
                unaligned: None,
                volatile: false,
                param0: LoadType::Int64,
            },
            FuzzInstruction::LdindU1 => Instruction::LoadIndirect {
                unaligned: None,
                volatile: false,
                param0: LoadType::UInt8,
            },
            FuzzInstruction::LdindU2 => Instruction::LoadIndirect {
                unaligned: None,
                volatile: false,
                param0: LoadType::UInt16,
            },
            FuzzInstruction::LdindU4 => Instruction::LoadIndirect {
                unaligned: None,
                volatile: false,
                param0: LoadType::UInt32,
            },
            FuzzInstruction::LdindR4 => Instruction::LoadIndirect {
                unaligned: None,
                volatile: false,
                param0: LoadType::Float32,
            },
            FuzzInstruction::LdindR8 => Instruction::LoadIndirect {
                unaligned: None,
                volatile: false,
                param0: LoadType::Float64,
            },
            FuzzInstruction::LdindI => Instruction::LoadIndirect {
                unaligned: None,
                volatile: false,
                param0: LoadType::IntPtr,
            },
            FuzzInstruction::StindI1 => Instruction::StoreIndirect {
                unaligned: None,
                volatile: false,
                param0: StoreType::Int8,
            },
            FuzzInstruction::StindI2 => Instruction::StoreIndirect {
                unaligned: None,
                volatile: false,
                param0: StoreType::Int16,
            },
            FuzzInstruction::StindI4 => Instruction::StoreIndirect {
                unaligned: None,
                volatile: false,
                param0: StoreType::Int32,
            },
            FuzzInstruction::StindI8 => Instruction::StoreIndirect {
                unaligned: None,
                volatile: false,
                param0: StoreType::Int64,
            },
            FuzzInstruction::StindR4 => Instruction::StoreIndirect {
                unaligned: None,
                volatile: false,
                param0: StoreType::Float32,
            },
            FuzzInstruction::StindR8 => Instruction::StoreIndirect {
                unaligned: None,
                volatile: false,
                param0: StoreType::Float64,
            },
            FuzzInstruction::StindI => Instruction::StoreIndirect {
                unaligned: None,
                volatile: false,
                param0: StoreType::IntPtr,
            },
            FuzzInstruction::Cpblk => Instruction::CopyMemoryBlock {
                unaligned: None,
                volatile: false,
            },
            FuzzInstruction::Initblk => Instruction::InitializeMemoryBlock {
                unaligned: None,
                volatile: false,
            },
            FuzzInstruction::Localloc => Instruction::LocalMemoryAllocate,

            FuzzInstruction::ConvI => Instruction::Convert(ConversionType::IntPtr),
            FuzzInstruction::ConvU => Instruction::Convert(ConversionType::UIntPtr),

            FuzzInstruction::Br(offset) => {
                let max_target = (total_len as isize - 1).max(0);
                let target =
                    (current_ip as isize + 1 + *offset as isize).clamp(0, max_target) as usize;
                Instruction::Branch(target)
            }
            FuzzInstruction::Brtrue(offset) => {
                let max_target = (total_len as isize - 1).max(0);
                let target =
                    (current_ip as isize + 1 + *offset as isize).clamp(0, max_target) as usize;
                Instruction::BranchTruthy(target)
            }
            FuzzInstruction::Brfalse(offset) => {
                let max_target = (total_len as isize - 1).max(0);
                let target =
                    (current_ip as isize + 1 + *offset as isize).clamp(0, max_target) as usize;
                Instruction::BranchFalsy(target)
            }
            FuzzInstruction::Ret => Instruction::Return,

            // Locals and Arguments
            FuzzInstruction::Ldloc(idx) => {
                Instruction::LoadLocal((*idx % num_locals.max(1)) as u16)
            }
            FuzzInstruction::Ldloca(idx) => {
                Instruction::LoadLocalAddress((*idx % num_locals.max(1)) as u16)
            }
            FuzzInstruction::Stloc(idx) => {
                Instruction::StoreLocal((*idx % num_locals.max(1)) as u16)
            }
            FuzzInstruction::Ldarg(idx) => {
                Instruction::LoadArgument((*idx % num_args.max(1)) as u16)
            }
            FuzzInstruction::Ldarga(idx) => {
                Instruction::LoadArgumentAddress((*idx % num_args.max(1)) as u16)
            }
            FuzzInstruction::Starg(idx) => {
                Instruction::StoreArgument((*idx % num_args.max(1)) as u16)
            }

            // Object operations
            FuzzInstruction::Newobj(idx) => {
                let method_source = methods[*idx as usize % methods.len()].clone();
                if let MethodSource::User(UserMethod::Definition(idx)) = method_source {
                    Instruction::NewObject(UserMethod::Definition(idx))
                } else {
                    Instruction::LoadNull // Should not happen with current fuzzing logic
                }
            }
            FuzzInstruction::Ldfld(idx) => Instruction::LoadField {
                volatile: false,
                unaligned: None,
                param0: fields[*idx as usize % fields.len()],
            },
            FuzzInstruction::Stfld(idx) => Instruction::StoreField {
                volatile: false,
                unaligned: None,
                param0: fields[*idx as usize % fields.len()],
            },
            FuzzInstruction::Ldflda(idx) => {
                Instruction::LoadFieldAddress(fields[*idx as usize % fields.len()])
            }

            // Arrays
            FuzzInstruction::Newarr => {
                Instruction::NewArray(MethodType::Base(Box::new(BaseType::Int32)))
            }
            FuzzInstruction::LdelemI1 => Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::LoadType::Int8,
            },
            FuzzInstruction::LdelemI2 => Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::LoadType::Int16,
            },
            FuzzInstruction::LdelemI4 => Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::LoadType::Int32,
            },
            FuzzInstruction::LdelemI8 => Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::LoadType::Int64,
            },
            FuzzInstruction::LdelemU1 => Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::LoadType::UInt8,
            },
            FuzzInstruction::LdelemU2 => Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::LoadType::UInt16,
            },
            FuzzInstruction::LdelemU4 => Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::LoadType::UInt32,
            },
            FuzzInstruction::LdelemR4 => Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::LoadType::Float32,
            },
            FuzzInstruction::LdelemR8 => Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::LoadType::Float64,
            },
            FuzzInstruction::LdelemI => Instruction::LoadElementPrimitive {
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::LoadType::IntPtr,
            },
            FuzzInstruction::LdelemRef => Instruction::LoadElement {
                skip_range_check: false,
                skip_null_check: false,
                param0: MethodType::Base(Box::new(BaseType::Object)),
            },
            FuzzInstruction::Ldelema => Instruction::LoadElementAddress {
                skip_type_check: false,
                skip_range_check: false,
                skip_null_check: false,
                param0: MethodType::Base(Box::new(BaseType::Int32)),
            },
            FuzzInstruction::StelemI1 => Instruction::StoreElementPrimitive {
                skip_type_check: false,
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::StoreType::Int8,
            },
            FuzzInstruction::StelemI2 => Instruction::StoreElementPrimitive {
                skip_type_check: false,
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::StoreType::Int16,
            },
            FuzzInstruction::StelemI4 => Instruction::StoreElementPrimitive {
                skip_type_check: false,
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::StoreType::Int32,
            },
            FuzzInstruction::StelemI8 => Instruction::StoreElementPrimitive {
                skip_type_check: false,
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::StoreType::Int64,
            },
            FuzzInstruction::StelemR4 => Instruction::StoreElementPrimitive {
                skip_type_check: false,
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::StoreType::Float32,
            },
            FuzzInstruction::StelemR8 => Instruction::StoreElementPrimitive {
                skip_type_check: false,
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::StoreType::Float64,
            },
            FuzzInstruction::StelemI => Instruction::StoreElementPrimitive {
                skip_type_check: false,
                skip_range_check: false,
                skip_null_check: false,
                param0: dotnetdll::resolved::il::StoreType::IntPtr,
            },
            FuzzInstruction::StelemRef => Instruction::StoreElement {
                skip_type_check: false,
                skip_range_check: false,
                skip_null_check: false,
                param0: MethodType::Base(Box::new(BaseType::Object)),
            },
            FuzzInstruction::Ldlen => Instruction::LoadLength,

            // Method calls
            FuzzInstruction::Call(idx) => Instruction::Call {
                tail_call: false,
                param0: methods[*idx as usize % methods.len()].clone(),
            },
            FuzzInstruction::Callvirt(idx) => Instruction::CallVirtual {
                skip_null_check: false,
                param0: methods[*idx as usize % methods.len()].clone(),
            },
            FuzzInstruction::Ldftn(idx) => {
                Instruction::LoadMethodPointer(methods[*idx as usize % methods.len()].clone())
            }

            // Exception Handling
            FuzzInstruction::Throw => Instruction::Throw,
            FuzzInstruction::Rethrow => Instruction::Rethrow,
            FuzzInstruction::Leave(offset) => {
                let max_target = (total_len as isize - 1).max(0);
                let target =
                    (current_ip as isize + 1 + *offset as isize).clamp(0, max_target) as usize;
                Instruction::Leave(target)
            }
            FuzzInstruction::Endfinally => Instruction::EndFinally,
        }
    }
}

pub fn execute_cil_program(program: FuzzProgram) {
    execute_cil_program_with_loader(program, get_loader());
}

pub fn execute_cil_program_with_loader(program: FuzzProgram, loader: &'static AssemblyLoader) {
    let shared = Arc::new(SharedGlobalState::new(loader));

    // Build a minimal Resolution with one type and one static void method
    let mut res = Resolution::new(Module::new("Fuzz.dll"));
    res.assembly = Some(Assembly::new("FuzzAssembly"));

    // Add references for inheritance and basic types
    let system_runtime =
        res.push_assembly_reference(ExternalAssemblyReference::new("System.Runtime"));
    let object_type_ref = res.push_type_reference(ExternalTypeReference::new(
        Some("System".into()),
        "Object",
        ResolutionScope::Assembly(system_runtime),
    ));

    let mut type_def = TypeDefinition::new(None, "FuzzType");
    type_def.extends = Some(object_type_ref.into());
    let type_idx = res.push_type_definition(type_def);

    let mut fields = Vec::new();
    let mut methods = Vec::new();

    // Add some fields to FuzzType
    fields.push(FieldSource::Definition(res.push_field(
        type_idx,
        Field {
            attributes: vec![],
            name: "FStaticI32".into(),
            type_modifiers: vec![],
            by_ref: false,
            return_type: MemberType::Base(Box::new(BaseType::Int32)),
            accessibility: resolved_mod::members::Accessibility::Access(
                resolved_mod::Accessibility::Public,
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
        },
    )));

    fields.push(FieldSource::Definition(res.push_field(
        type_idx,
        Field {
            attributes: vec![],
            name: "FInstanceI32".into(),
            type_modifiers: vec![],
            by_ref: false,
            return_type: MemberType::Base(Box::new(BaseType::Int32)),
            accessibility: resolved_mod::members::Accessibility::Access(
                resolved_mod::Accessibility::Public,
            ),
            static_member: false,
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
        },
    )));

    // Add a constructor
    let ctor_sig = MethodSignature {
        instance: true,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters: vec![],
        return_type: ReturnType(vec![], None),
        varargs: None,
    };
    let ctor_body = body::Method {
        header: body::Header {
            maximum_stack_size: 8,
            local_variables: vec![],
            initialize_locals: true,
        },
        instructions: vec![Instruction::LoadArgument(0), Instruction::Return],
        data_sections: vec![],
    };
    methods.push(MethodSource::User(UserMethod::Definition(res.push_method(
        type_idx,
        Method::new(
            resolved_mod::Accessibility::Public,
            ctor_sig,
            ".ctor",
            Some(ctor_body),
        ),
    ))));

    let local_types = vec![
        MethodType::Base(Box::new(BaseType::Int32)),
        MethodType::Base(Box::new(BaseType::Int64)),
        MethodType::Base(Box::new(BaseType::Float32)),
        MethodType::Base(Box::new(BaseType::Float64)),
        MethodType::Base(Box::new(BaseType::Object)),
        MethodType::Base(Box::new(BaseType::String)),
        MethodType::Base(Box::new(BaseType::IntPtr)),
        MethodType::Base(Box::new(BaseType::Int8)),
        MethodType::Base(Box::new(BaseType::Int16)),
        MethodType::Base(Box::new(BaseType::UInt8)),
    ];

    let mut parameters = Vec::new();
    for i in 0..program.num_args {
        let base = local_types[i as usize % local_types.len()].clone();
        parameters.push(Parameter(vec![], ParameterType::Value(base)));
    }

    let sig = MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters,
        return_type: ReturnType(vec![], None),
        varargs: None,
    };

    let instructions: Vec<_> = program
        .instructions
        .iter()
        .enumerate()
        .map(|(i, ins)| {
            ins.to_dotnetdll(
                i,
                program.instructions.len(),
                program.num_locals,
                program.num_args,
                &fields,
                &methods,
            )
        })
        .collect();

    let local_variables = (0..program.num_locals)
        .map(|i| LocalVariable::Variable {
            custom_modifiers: vec![],
            pinned: false,
            by_ref: false,
            var_type: local_types[i as usize % local_types.len()].clone(),
        })
        .collect();

    let body = body::Method {
        header: body::Header {
            maximum_stack_size: 64, // reasonable for fuzzing
            local_variables,
            initialize_locals: true,
        },
        instructions,
        data_sections: vec![],
    };

    let method = Method::new(
        resolved_mod::Accessibility::Public,
        sig,
        "FuzzMain",
        Some(body),
    );
    let method_idx = res.push_method(type_idx, method);
    methods.push(MethodSource::User(UserMethod::Definition(method_idx)));

    // Add a few more dummy methods
    for i in 0..2 {
        let dummy_sig = MethodSignature {
            instance: false,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![],
            return_type: ReturnType(vec![], None),
            varargs: None,
        };
        let dummy_body = body::Method {
            header: body::Header {
                maximum_stack_size: 8,
                local_variables: vec![],
                initialize_locals: true,
            },
            instructions: vec![Instruction::Return],
            data_sections: vec![],
        };
        methods.push(MethodSource::User(UserMethod::Definition(res.push_method(
            type_idx,
            Method::new(
                resolved_mod::Accessibility::Public,
                dummy_sig,
                format!("DummyMethod{}", i),
                Some(dummy_body),
            ),
        ))));
    }

    // Leak the resolution to 'static as required by ResolutionS wrapper
    let resolution = Box::leak(Box::new(res));
    let res_s = ResolutionS::new(resolution);

    // Construct MethodDescription and TypeDescription for executor
    let typedef = &resolution[type_idx];
    let method_def = &resolution[method_idx];

    let entrypoint = MethodDescription {
        parent: TypeDescription::new(res_s, typedef, type_idx),
        method_resolution: res_s,
        method: method_def,
    };

    let mut executor = Executor::new(shared);
    executor.instruction_budget = Some(10000); // Prevent infinite loops
    executor.entrypoint(entrypoint);

    let _result = executor.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build and execute a FuzzProgram with given instructions.
    fn run_program(instructions: Vec<FuzzInstruction>, num_locals: u8, num_args: u8) {
        let program = FuzzProgram {
            instructions,
            num_locals,
            num_args,
        };

        if cfg!(miri) {
            execute_cil_program_with_loader(program, get_mock_loader());
        } else {
            execute_cil_program(program);
        }
    }

    /// Simple arithmetic: push two i32s, add, return.
    /// Exercises stack push/pop and arithmetic under miri.
    #[test]
    fn miri_arithmetic_add() {
        run_program(
            vec![
                FuzzInstruction::LdcI4(10),
                FuzzInstruction::LdcI4(20),
                FuzzInstruction::Add,
                FuzzInstruction::Pop,
                FuzzInstruction::Ret,
            ],
            0,
            1,
        );
    }

    /// Division by zero should not cause UB — must be caught as a managed exception.
    #[test]
    fn miri_div_by_zero() {
        run_program(
            vec![
                FuzzInstruction::LdcI4(42),
                FuzzInstruction::LdcI4(0),
                FuzzInstruction::Div,
                FuzzInstruction::Pop,
                FuzzInstruction::Ret,
            ],
            0,
            1,
        );
    }

    /// Stack underflow: pop from empty stack should not cause UB.
    #[test]
    fn miri_stack_underflow() {
        run_program(vec![FuzzInstruction::Pop, FuzzInstruction::Ret], 0, 1);
    }

    /// Load and store locals with various types.
    #[test]
    fn miri_locals() {
        run_program(
            vec![
                FuzzInstruction::LdcI4(99),
                FuzzInstruction::Stloc(0),
                FuzzInstruction::Ldloc(0),
                FuzzInstruction::Pop,
                FuzzInstruction::LdcI8(123456),
                FuzzInstruction::Stloc(1),
                FuzzInstruction::Ldloc(1),
                FuzzInstruction::Pop,
                FuzzInstruction::Ret,
            ],
            3,
            1,
        );
    }

    /// Load null and throw — exception path should not trigger UB.
    #[test]
    fn miri_null_throw() {
        run_program(vec![FuzzInstruction::Ldnull, FuzzInstruction::Throw], 0, 1);
    }

    /// Branch forward past end — should be clamped safely.
    #[test]
    fn miri_branch_out_of_bounds() {
        run_program(vec![FuzzInstruction::Br(127), FuzzInstruction::Ret], 0, 1);
    }

    /// Dup and arithmetic on floats.
    #[test]
    fn miri_float_ops() {
        run_program(
            vec![
                FuzzInstruction::LdcR8(std::f64::consts::PI),
                FuzzInstruction::Dup,
                FuzzInstruction::Mul,
                FuzzInstruction::Pop,
                FuzzInstruction::Ret,
            ],
            0,
            1,
        );
    }

    /// Load and store via IntPtr local to exercise native-int paths.
    #[test]
    fn miri_native_int_local() {
        // Local index 6 maps to IntPtr in the local_types array
        run_program(
            vec![
                FuzzInstruction::LdcI4(0),
                FuzzInstruction::Stloc(6),
                FuzzInstruction::Ldloc(6),
                FuzzInstruction::Pop,
                FuzzInstruction::Ret,
            ],
            7,
            1,
        );
    }

    /// Empty program — just return.
    #[test]
    fn miri_empty_program() {
        run_program(vec![FuzzInstruction::Ret], 0, 1);
    }

    /// Bitwise operations should not trigger UB.
    #[test]
    fn miri_bitwise_ops() {
        run_program(
            vec![
                FuzzInstruction::LdcI4(0xFF),
                FuzzInstruction::LdcI4(0x0F),
                FuzzInstruction::And,
                FuzzInstruction::LdcI4(0xF0),
                FuzzInstruction::Or,
                FuzzInstruction::Not,
                FuzzInstruction::Pop,
                FuzzInstruction::Ret,
            ],
            0,
            1,
        );
    }

    /// Test localloc and indirect memory access.
    #[test]
    fn miri_localloc_indirect() {
        run_program(
            vec![
                FuzzInstruction::LdcI4(4),
                FuzzInstruction::ConvI,
                FuzzInstruction::Localloc,
                FuzzInstruction::Dup,
                FuzzInstruction::LdcI4(1234),
                FuzzInstruction::StindI4,
                FuzzInstruction::LdindI4,
                FuzzInstruction::Pop,
                FuzzInstruction::Ret,
            ],
            0,
            1,
        );
    }

    /// Test array creation and element access.
    #[test]
    fn miri_array_ops() {
        run_program(
            vec![
                FuzzInstruction::LdcI4(10),
                FuzzInstruction::Newarr,
                FuzzInstruction::Dup,
                FuzzInstruction::LdcI4(5),
                FuzzInstruction::LdcI4(42),
                FuzzInstruction::StelemI4,
                FuzzInstruction::LdcI4(5),
                FuzzInstruction::LdelemI4,
                FuzzInstruction::Pop,
                FuzzInstruction::Ret,
            ],
            0,
            1,
        );
    }
}
