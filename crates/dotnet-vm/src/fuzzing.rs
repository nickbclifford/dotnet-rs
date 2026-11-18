use crate::{Executor, ExecutorResult, GenericLookup, state::SharedGlobalState, sync::Arc};
use arbitrary::Arbitrary;
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::{TypeDescription, members::MethodDescription};
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
use std::path::PathBuf;

#[allow(
    clippy::arc_with_non_send_sync,
    reason = "the fuzz loader is confined to this thread-local corpus execution context"
)]
fn get_loader() -> Arc<AssemblyLoader> {
    thread_local! {
        static LOADER: Arc<AssemblyLoader> = {
            let base = std::env::var("DOTNET_ROOT")
                .map(|p| PathBuf::from(p).join("shared/Microsoft.NETCore.App"))
                .unwrap_or_else(|_| {
                    PathBuf::from("/usr/share/dotnet/shared/Microsoft.NETCore.App")
                });

            let base = if !base.exists() {
                let alt_base = PathBuf::from("/usr/lib/dotnet/shared/Microsoft.NETCore.App");
                if alt_base.exists() { alt_base } else { base }
            } else {
                base
            };

            let mut entries: Vec<_> = std::fs::read_dir(&base)
                .unwrap_or_else(|e| {
                    panic!(
                        "could not read .NET shared framework directory {}: {e}",
                        base.display()
                    )
                })
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .collect();

            entries.sort_by_key(|e| e.file_name());
            let path = entries
                .last()
                .expect("no versions found in .NET shared path")
                .path();

            let loader = AssemblyLoader::new(
                path.to_str()
                    .expect("shared framework version directory names are ASCII")
                    .to_string(),
            )
            .expect("Failed to create AssemblyLoader");
            Arc::new(loader)
        };
    }
    LOADER.with(|l| l.clone())
}

#[cfg(test)]
#[allow(
    clippy::arc_with_non_send_sync,
    reason = "the mock fuzz loader is confined to this thread-local test fixture"
)]
fn get_mock_loader() -> Arc<AssemblyLoader> {
    thread_local! {
        static MOCK_LOADER: Arc<AssemblyLoader> = {
            let loader = AssemblyLoader::new_bare("mock_root".to_string())
                .expect("Failed to create mock AssemblyLoader");

            // Register mock assemblies so that System.Object can be resolved
            let mut mscorlib = Resolution::new(Module::new("mscorlib.dll"));
            mscorlib.assembly = Some(Assembly::new("mscorlib"));
            let mut obj = TypeDefinition::new(None, "Object");
            obj.namespace = Some("System".into());
            let object = mscorlib.push_type_definition(obj);

            let mut value_type = TypeDefinition::new(None, "ValueType");
            value_type.namespace = Some("System".into());
            value_type.extends = Some(object.into());
            let value_type = mscorlib.push_type_definition(value_type);

            let mut enum_type = TypeDefinition::new(None, "Enum");
            enum_type.namespace = Some("System".into());
            enum_type.extends = Some(value_type.into());
            mscorlib.push_type_definition(enum_type);

            loader.register_owned_assembly(mscorlib);

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

            loader.register_owned_assembly(system_runtime);

            Arc::new(loader)
        };
    }
    MOCK_LOADER.with(|l| l.clone())
}

#[derive(Debug, Arbitrary, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct FuzzProgram {
    pub instructions: Vec<FuzzInstruction>,
    pub num_locals: u8,
    pub num_args: u8,
}

#[derive(Debug, Arbitrary)]
struct ArbitraryFuzzProgram {
    instructions: Vec<FuzzInstruction>,
    #[arbitrary(with = |u: &mut arbitrary::Unstructured| u.int_in_range(0..=10))]
    num_locals: u8,
    #[arbitrary(with = |u: &mut arbitrary::Unstructured| u.int_in_range(1..=10))]
    num_args: u8,
}

/// Named, decodable program seeds for the executor fuzzer.
///
/// `ArithmeticOverflow` exercises unchecked `i32` addition at the signed boundary; this
/// instruction set has no checked-add opcode, so the seed does not claim to test overflow
/// exceptions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticFuzzSeed {
    ArithmeticAdd,
    ArithmeticDivideByZero,
    ArithmeticOverflow,
    MemoryLocalloc,
    MemoryNullDereference,
    MemoryCpblkNull,
    ControlBranch,
    ControlInfiniteLoop,
    StackDeep,
    StackUnderflow,
    StackDup,
}

impl SemanticFuzzSeed {
    pub const ALL: [Self; 11] = [
        Self::ArithmeticAdd,
        Self::ArithmeticDivideByZero,
        Self::ArithmeticOverflow,
        Self::MemoryLocalloc,
        Self::MemoryNullDereference,
        Self::MemoryCpblkNull,
        Self::ControlBranch,
        Self::ControlInfiniteLoop,
        Self::StackDeep,
        Self::StackUnderflow,
        Self::StackDup,
    ];

    const fn id(self) -> u8 {
        match self {
            Self::ArithmeticAdd => 0,
            Self::ArithmeticDivideByZero => 1,
            Self::ArithmeticOverflow => 2,
            Self::MemoryLocalloc => 3,
            Self::MemoryNullDereference => 4,
            Self::MemoryCpblkNull => 5,
            Self::ControlBranch => 6,
            Self::ControlInfiniteLoop => 7,
            Self::StackDeep => 8,
            Self::StackUnderflow => 9,
            Self::StackDup => 10,
        }
    }

    fn from_id(id: u8) -> Option<Self> {
        Self::ALL.get(id as usize).copied()
    }

    pub const fn file_name(self) -> &'static str {
        match self {
            Self::ArithmeticAdd => "seed_program_arithmetic_add",
            Self::ArithmeticDivideByZero => "seed_program_arithmetic_div_zero",
            Self::ArithmeticOverflow => "seed_program_arithmetic_overflow",
            Self::MemoryLocalloc => "seed_program_memory_localloc",
            Self::MemoryNullDereference => "seed_program_memory_null_deref",
            Self::MemoryCpblkNull => "seed_program_memory_cpblk_null",
            Self::ControlBranch => "seed_program_control_branch",
            Self::ControlInfiniteLoop => "seed_program_control_infinite_loop",
            Self::StackDeep => "seed_program_stack_deep",
            Self::StackUnderflow => "seed_program_stack_underflow",
            Self::StackDup => "seed_program_stack_dup",
        }
    }

    pub fn encode(self) -> Vec<u8> {
        let mut bytes = b"DVMSEED1".to_vec();
        bytes.push(self.id());
        bytes
    }

    pub fn program(self) -> FuzzProgram {
        use FuzzInstruction as I;
        let instructions = match self {
            Self::ArithmeticAdd => vec![I::LdcI4(42), I::LdcI4(1), I::Add, I::Pop, I::Ret],
            Self::ArithmeticDivideByZero => {
                vec![I::LdcI4(10), I::LdcI4(0), I::Div, I::Pop, I::Ret]
            }
            Self::ArithmeticOverflow => {
                vec![I::LdcI4(i32::MAX), I::LdcI4(1), I::Add, I::Pop, I::Ret]
            }
            Self::MemoryLocalloc => vec![
                I::LdcI4(4),
                I::ConvI,
                I::Localloc,
                I::Dup,
                I::LdcI4(1234),
                I::StindI4,
                I::LdindI4,
                I::Pop,
                I::Ret,
            ],
            Self::MemoryNullDereference => vec![I::Ldnull, I::LdindI4, I::Ret],
            Self::MemoryCpblkNull => {
                vec![I::Ldnull, I::Ldnull, I::LdcI4(10), I::Cpblk, I::Ret]
            }
            Self::ControlBranch => {
                vec![I::LdcI4(1), I::Brtrue(1), I::Pop, I::Ret]
            }
            Self::ControlInfiniteLoop => vec![I::Br(-1)],
            Self::StackDeep => {
                let mut instructions = Vec::with_capacity(101);
                for _ in 0..50 {
                    instructions.push(I::LdcI4(0));
                }
                instructions.extend(std::iter::repeat_n(I::Pop, 50));
                instructions.push(I::Ret);
                instructions
            }
            Self::StackUnderflow => vec![I::Pop, I::Ret],
            Self::StackDup => vec![I::LdcI4(42), I::Dup, I::Add, I::Pop, I::Ret],
        };
        FuzzProgram {
            instructions,
            num_locals: 0,
            num_args: 1,
        }
    }
}

impl<'a> Arbitrary<'a> for FuzzProgram {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        if u.peek_bytes(8) == Some(&b"DVMSEED1"[..]) {
            let _ = u.bytes(8)?;
            let id = u.arbitrary::<u8>()?;
            return SemanticFuzzSeed::from_id(id)
                .map(SemanticFuzzSeed::program)
                .ok_or(arbitrary::Error::IncorrectFormat);
        }

        let raw = ArbitraryFuzzProgram::arbitrary(u)?;
        Ok(Self {
            instructions: raw.instructions,
            num_locals: raw.num_locals,
            num_args: raw.num_args,
        })
    }
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

pub fn execute_cil_program_with_loader(program: FuzzProgram, loader: Arc<AssemblyLoader>) {
    let _ = execute_cil_program_with_loader_result(program, loader);
}

#[cfg_attr(
    not(feature = "multithreading"),
    allow(
        clippy::arc_with_non_send_sync,
        reason = "single-threaded fuzz execution uses VM APIs that require Arc"
    )
)]
fn execute_cil_program_with_loader_result(
    program: FuzzProgram,
    loader: Arc<AssemblyLoader>,
) -> Result<ExecutorResult, crate::error::VmError> {
    let shared = Arc::new(SharedGlobalState::new(loader.clone()));

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
        instructions: vec![
            Instruction::LoadArgument(0),
            Instruction::Pop,
            Instruction::LoadArgument(0),
            Instruction::Return,
        ],
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

    let mut instructions: Vec<_> = program
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

    if instructions.is_empty() {
        instructions.push(Instruction::Return);
    }

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
            instructions: vec![Instruction::LoadNull, Instruction::Pop, Instruction::Return],
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

    let res_s = loader.register_owned_assembly(res);

    // Construct MethodDescription and TypeDescription for executor
    let _typedef = &res_s.definition()[type_idx];
    let method_def = &res_s.definition()[method_idx];

    let td = TypeDescription::new(res_s.clone(), type_idx);
    let method_index = td
        .definition()
        .methods
        .iter()
        .position(|m| std::ptr::eq(m, method_def))
        .expect("method_def was retrieved from this same definition's methods list");

    let entrypoint = MethodDescription::new(
        td,
        GenericLookup::default(),
        res_s,
        MethodMemberIndex::Method(method_index),
    );

    let mut executor = Executor::new(shared);
    executor.instruction_budget = Some(10000); // Prevent infinite loops
    #[cfg(test)]
    let _trace_target_guard = FuzzEntrypointTraceGuard::install(entrypoint.clone());
    let setup = executor.entrypoint(entrypoint);
    run_after_entrypoint_setup(&mut executor, setup)
}

#[cfg(test)]
struct FuzzEntrypointTraceGuard;

#[cfg(test)]
impl FuzzEntrypointTraceGuard {
    fn install(target: MethodDescription) -> Self {
        crate::dispatch::FUZZ_ENTRYPOINT_TRACE_TARGET.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(
                slot.is_none(),
                "fuzz entrypoint trace target is already set"
            );
            *slot = Some(target);
        });
        Self
    }
}

#[cfg(test)]
impl Drop for FuzzEntrypointTraceGuard {
    fn drop(&mut self) {
        crate::dispatch::FUZZ_ENTRYPOINT_TRACE_TARGET.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
struct FuzzAllMethodTraceGuard;

#[cfg(test)]
impl FuzzAllMethodTraceGuard {
    fn install() -> Self {
        crate::dispatch::FUZZ_CAPTURE_ALL_METHOD_IPS.with(|capture| {
            assert!(
                !capture.replace(true),
                "all-method fuzz trace is already enabled"
            );
        });
        Self
    }
}

#[cfg(test)]
impl Drop for FuzzAllMethodTraceGuard {
    fn drop(&mut self) {
        crate::dispatch::FUZZ_CAPTURE_ALL_METHOD_IPS.with(|capture| capture.set(false));
    }
}

fn run_after_entrypoint_setup(
    executor: &mut Executor,
    setup: Result<(), crate::error::VmError>,
) -> Result<ExecutorResult, crate::error::VmError> {
    setup?;
    Ok(executor.run())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FuzzRun {
        result: ExecutorResult,
        instruction_ips: Vec<usize>,
        dispatched_method_ips: Vec<(String, String, usize)>,
    }

    /// Builds, installs and executes a program, retaining the real executor outcome and every
    /// instruction pointer dispatched by the execution engine.
    fn run_program(instructions: Vec<FuzzInstruction>, num_locals: u8, num_args: u8) -> FuzzRun {
        let loader = if cfg!(miri) {
            get_mock_loader()
        } else {
            get_loader()
        };
        run_program_with_loader(instructions, num_locals, num_args, loader)
    }

    fn run_program_with_loader(
        instructions: Vec<FuzzInstruction>,
        num_locals: u8,
        num_args: u8,
        loader: Arc<AssemblyLoader>,
    ) -> FuzzRun {
        let program = FuzzProgram {
            instructions,
            num_locals,
            num_args,
        };
        crate::dispatch::FUZZ_EXECUTED_INSTRUCTION_IPS.with(|ips| ips.borrow_mut().clear());
        crate::dispatch::FUZZ_DISPATCHED_METHOD_IPS.with(|events| events.borrow_mut().clear());
        let result = execute_cil_program_with_loader_result(program, loader)
            .expect("fuzz regression entrypoint must install successfully");
        let instruction_ips = crate::dispatch::FUZZ_EXECUTED_INSTRUCTION_IPS
            .with(|ips| std::mem::take(&mut *ips.borrow_mut()));
        let dispatched_method_ips = crate::dispatch::FUZZ_DISPATCHED_METHOD_IPS
            .with(|events| std::mem::take(&mut *events.borrow_mut()));
        FuzzRun {
            result,
            instruction_ips,
            dispatched_method_ips,
        }
    }

    fn assert_reached(run: &FuzzRun, expected_ips: &[usize]) {
        for ip in expected_ips {
            assert!(
                run.instruction_ips.contains(ip),
                "instruction at IL index {ip} did not execute; trace: {:?}; outcome: {:?}",
                run.instruction_ips,
                run.result
            );
        }
    }

    fn assert_completed(run: &FuzzRun) {
        assert!(
            matches!(run.result, ExecutorResult::Exited(0)),
            "program did not return successfully: {:?}; trace: {:?}",
            run.result,
            run.instruction_ips
        );
    }

    fn assert_failed_in_vm(run: &FuzzRun) {
        assert!(
            matches!(
                run.result,
                ExecutorResult::Threw(_) | ExecutorResult::Error(_)
            ),
            "expected a managed exception or host VM error, got {:?}",
            run.result
        );
    }

    #[test]
    fn named_arithmetic_memory_and_branch_seeds_decode_and_execute_as_intended() {
        use FuzzInstruction as I;

        let decode = |seed: SemanticFuzzSeed| {
            let bytes = seed.encode();
            FuzzProgram::arbitrary(&mut arbitrary::Unstructured::new(&bytes))
                .expect("named semantic seed must decode")
        };

        let arithmetic = decode(SemanticFuzzSeed::ArithmeticAdd);
        assert_eq!(
            arithmetic.instructions,
            vec![I::LdcI4(42), I::LdcI4(1), I::Add, I::Pop, I::Ret],
            "arithmetic seed must decode to the documented operands and operation"
        );
        let arithmetic_run = run_program(arithmetic.instructions, 0, 1);
        assert_reached(&arithmetic_run, &[0, 1, 2, 3, 4]);
        assert_completed(&arithmetic_run);

        let overflow = decode(SemanticFuzzSeed::ArithmeticOverflow);
        assert_eq!(
            overflow.instructions,
            vec![I::LdcI4(i32::MAX), I::LdcI4(1), I::Add, I::Pop, I::Ret,],
            "overflow seed is unchecked boundary addition followed by stack cleanup"
        );
        let overflow_run = run_program(overflow.instructions, 0, 1);
        assert_reached(&overflow_run, &[0, 1, 2, 3, 4]);
        assert_completed(&overflow_run);

        let memory = decode(SemanticFuzzSeed::MemoryLocalloc);
        assert_eq!(
            memory.instructions,
            vec![
                I::LdcI4(4),
                I::ConvI,
                I::Localloc,
                I::Dup,
                I::LdcI4(1234),
                I::StindI4,
                I::LdindI4,
                I::Pop,
                I::Ret,
            ],
            "memory seed must allocate four bytes and perform the stated indirect write/read"
        );
        let memory_run = run_program(memory.instructions, 0, 1);
        assert_reached(&memory_run, &(0..9).collect::<Vec<_>>());
        assert_completed(&memory_run);

        let branch = decode(SemanticFuzzSeed::ControlBranch);
        assert_eq!(
            branch.instructions,
            vec![I::LdcI4(1), I::Brtrue(1), I::Pop, I::Ret],
            "branch seed must take the true branch over the Pop"
        );
        let branch_run = run_program(branch.instructions, 0, 1);
        assert_eq!(
            branch_run.instruction_ips,
            vec![0, 1, 3],
            "true branch must skip instruction 2 and reach the return"
        );
        assert_completed(&branch_run);

        let stack_deep = decode(SemanticFuzzSeed::StackDeep);
        let mut expected_deep = vec![I::LdcI4(0); 50];
        expected_deep.extend(std::iter::repeat_n(I::Pop, 50));
        expected_deep.push(I::Ret);
        assert_eq!(
            stack_deep.instructions, expected_deep,
            "deep-stack seed must push fifty values before popping them"
        );
        let stack_deep_run = run_program(stack_deep.instructions, 0, 1);
        assert_reached(&stack_deep_run, &(0..101).collect::<Vec<_>>());
        assert_completed(&stack_deep_run);

        let stack_dup = decode(SemanticFuzzSeed::StackDup);
        assert_eq!(
            stack_dup.instructions,
            vec![I::LdcI4(42), I::Dup, I::Add, I::Pop, I::Ret],
            "dup seed duplicates and adds one value, then empties the void method stack"
        );
        let stack_dup_run = run_program(stack_dup.instructions, 0, 1);
        assert_reached(&stack_dup_run, &[0, 1, 2, 3, 4]);
        assert_completed(&stack_dup_run);
    }

    #[test]
    #[cfg_attr(
        not(feature = "multithreading"),
        allow(
            clippy::arc_with_non_send_sync,
            reason = "the setup control stays on one thread and VM APIs require Arc"
        )
    )]
    fn entrypoint_setup_failure_is_reported_instead_of_running() {
        let shared = Arc::new(SharedGlobalState::new(get_mock_loader()));
        let mut executor = Executor::new(shared);
        executor.instruction_budget = Some(0);
        let setup_error =
            crate::error::VmError::Execution(crate::error::ExecutionError::InvalidIP(usize::MAX));
        let result = run_after_entrypoint_setup(&mut executor, Err(setup_error));
        assert!(
            matches!(
                result,
                Err(crate::error::VmError::Execution(
                    crate::error::ExecutionError::InvalidIP(ip)
                )) if ip == usize::MAX
            ),
            "entrypoint setup errors must propagate without running the executor"
        );
    }

    #[test]
    fn instruction_trace_excludes_constructor_helper_ips() {
        // The constructor executes IP 3, while FuzzMain branches over its own IP 3.
        let _method_trace_guard = FuzzAllMethodTraceGuard::install();
        let run = run_program_with_loader(
            vec![
                FuzzInstruction::Newobj(0),
                FuzzInstruction::Pop,
                FuzzInstruction::Br(1),
                FuzzInstruction::LdcI4(0),
                FuzzInstruction::Ret,
            ],
            0,
            1,
            get_mock_loader(),
        );
        assert!(
            run.dispatched_method_ips
                .iter()
                .any(|(type_name, method, ip)| {
                    type_name == "FuzzType" && method == ".ctor" && *ip == 3
                }),
            "constructor control must really execute its IP 3; result: {:?}; dispatches: {:?}",
            run.result,
            run.dispatched_method_ips,
        );
        assert_reached(&run, &[0, 1, 2, 4]);
        assert!(
            !run.instruction_ips.contains(&3),
            "the constructor's IP 3 must not satisfy reachability for skipped FuzzMain IP 3"
        );
        assert_completed(&run);
    }

    /// Simple arithmetic: push two i32s, add, return.
    /// Exercises stack push/pop and arithmetic under miri.
    #[test]
    fn miri_arithmetic_add() {
        let run = run_program(
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
        assert_reached(&run, &[0, 1, 2, 3, 4]);
        assert_completed(&run);
    }

    /// Division by zero exercises VM error/managed-throw handling without panic or UB.
    #[test]
    fn miri_div_by_zero() {
        let run = run_program(
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
        assert_reached(&run, &[0, 1, 2]);
        assert_failed_in_vm(&run);
    }

    /// Stack underflow: pop from empty stack should not cause UB.
    #[test]
    fn miri_stack_underflow() {
        let run = run_program(vec![FuzzInstruction::Pop, FuzzInstruction::Ret], 0, 1);
        assert_reached(&run, &[0]);
        assert_failed_in_vm(&run);
    }

    /// Load and store locals with various types.
    #[test]
    fn miri_locals() {
        let run = run_program(
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
        assert_reached(&run, &(0..9).collect::<Vec<_>>());
        assert_completed(&run);
    }

    /// Load null and throw — exception path should not trigger UB.
    #[test]
    fn miri_null_throw() {
        let run = run_program(vec![FuzzInstruction::Ldnull, FuzzInstruction::Throw], 0, 1);
        assert_reached(&run, &[0, 1]);
        assert_failed_in_vm(&run);
    }

    /// Branch forward past end — should be clamped safely.
    #[test]
    fn miri_branch_out_of_bounds() {
        let run = run_program(vec![FuzzInstruction::Br(127), FuzzInstruction::Ret], 0, 1);
        assert_reached(&run, &[0, 1]);
        assert_completed(&run);
    }

    /// Dup and arithmetic on floats.
    #[test]
    fn miri_float_ops() {
        let run = run_program(
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
        assert_reached(&run, &[0, 1, 2, 3, 4]);
        assert_completed(&run);
    }

    /// Load and store via IntPtr local to exercise native-int paths.
    #[test]
    fn miri_native_int_local() {
        // Local index 6 maps to IntPtr in the local_types array
        let run = run_program(
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
        assert_reached(&run, &[0, 1, 2, 3, 4]);
        assert_completed(&run);
    }

    /// Empty program — just return.
    #[test]
    fn miri_empty_program() {
        let run = run_program(vec![], 0, 1);
        assert_reached(&run, &[0]);
        assert_completed(&run);
    }

    /// Program without return — should handle falling off the end.
    #[test]
    fn miri_no_return_error() {
        let run = run_program(vec![FuzzInstruction::LdcI4(42)], 0, 1);
        assert_reached(&run, &[0]);
        assert_failed_in_vm(&run);
    }

    /// Bitwise operations should not trigger UB.
    #[test]
    fn miri_bitwise_ops() {
        let run = run_program(
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
        assert_reached(&run, &(0..8).collect::<Vec<_>>());
        assert_completed(&run);
    }

    /// Test localloc and indirect memory access.
    #[test]
    fn miri_localloc_indirect() {
        let run = run_program(
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
        assert_reached(&run, &(0..9).collect::<Vec<_>>());
        assert_completed(&run);
    }

    /// Test array creation and element access.
    #[test]
    fn miri_array_ops() {
        let run = run_program(
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
        assert_reached(&run, &(0..10).collect::<Vec<_>>());
        assert_completed(&run);
    }
}
