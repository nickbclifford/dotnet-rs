//! Turn a recorded `DynamicMethod` body into a real method (plan 24; completed by plan 25).
//!
//! The managed shim (`DotnetRs.DynamicMethod` in the support assembly) records resolved
//! operations instead of IL bytes. This module converts that record into a one-method in-memory
//! assembly, registers it with the loader and returns the ordinary reflection `MethodInfo`, so
//! `Invoke`, `CreateDelegate`, frames, exception regions and stack traces are the regular paths.
//!
//! Managed vararg calls follow the runtime's unsupported-call behavior;
//! uncommon prefix combinations and raw SignatureHelper encodings are also limited.

use crate::ReflectionIntrinsicHost;
use dotnet_assemblies::AssemblyLoader;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
    resolution::ResolutionS,
    runtime::{RuntimeType, runtime_type_from_concrete},
};
use dotnet_value::{StackValue, object::ObjectRef};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::effects::{NativeResult, VmEffects};
use dotnetdll::{
    binary::signature::kinds::CallingConvention,
    prelude::*,
    resolved::{
        body::{DataSection, Exception, ExceptionKind},
        members::{
            ExternalFieldReference, ExternalMethodReference, FieldReferenceParent, FieldSource,
            GenericMethodInstantiation, Method, MethodReferenceParent, MethodSource, UserMethod,
        },
        signature::{Parameter, ParameterType, ReturnType, StandAloneCallingConvention},
        types::{
            ExternalTypeReference, LocalVariable, ResolutionScope, TypeDefinition, TypeSource,
            UserType, ValueKind,
        },
    },
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

// IDs never recycle, even when a synthetic assembly is eventually evicted. Refuse exhaustion
// rather than wrapping to a name that may still identify a different assembly.
static DYNAMIC_ASSEMBLY_ID: AtomicU64 = AtomicU64::new(0);

type R<T> = Result<T, String>;

fn next_dynamic_assembly_id(counter: &AtomicU64) -> R<u64> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| "dynamic assembly identity space exhausted".into())
}

fn prim(b: BaseType<MethodType>) -> MethodType {
    MethodType::Base(Box::new(b))
}

fn member_type(t: MethodType) -> MemberType {
    match t {
        MethodType::Base(b) => MemberType::Base(Box::new(b.map(member_type))),
        MethodType::TypeGeneric(i) => MemberType::TypeGeneric(i),
        MethodType::MethodGeneric(_) => {
            unreachable!("translated field types have no method generics")
        }
    }
}

struct Builder<'a> {
    loader: &'a AssemblyLoader,
    res: Resolution<'static>,
    assembly_refs: HashMap<String, AssemblyRefIndex>,
    type_refs: HashMap<TypeDescription, TypeRefIndex>,
}

impl Builder<'_> {
    fn type_ref(&mut self, td: &TypeDescription) -> R<TypeRefIndex> {
        let def = td.definition();
        let assembly = td
            .resolution
            .assembly
            .as_ref()
            .ok_or_else(|| format!("type {} has no assembly", td.type_name()))?;
        // Name the exact loaded assembly; a plain name lookup may miss it (the entry assembly
        // is not registered by name) or load a second copy from disk.
        let asm_name = self
            .loader
            .reference_name_for(&td.resolution)
            .ok_or_else(|| format!("type {} has no assembly", td.type_name()))?;
        let namespace = def.namespace.as_ref().map(|n| n.to_string());
        if let Some(idx) = self.type_refs.get(td) {
            return Ok(*idx);
        }
        let asm_idx = match self.assembly_refs.get(&asm_name) {
            Some(i) => *i,
            None => {
                let mut r = ExternalAssemblyReference::new(asm_name.clone());
                r.version = assembly.version;
                let i = self.res.push_assembly_reference(r);
                self.assembly_refs.insert(asm_name, i);
                i
            }
        };
        let scope = if let Some(encloser) = def.encloser {
            ResolutionScope::Nested(
                self.type_ref(&TypeDescription::new(td.resolution.clone(), encloser))?,
            )
        } else {
            ResolutionScope::Assembly(asm_idx)
        };
        let idx = self.res.push_type_reference(ExternalTypeReference::new(
            namespace.map(Into::into),
            def.name.to_string(),
            scope,
        ));
        self.type_refs.insert(td.clone(), idx);
        Ok(idx)
    }

    fn type_node(&mut self, td: &TypeDescription) -> R<MethodType> {
        let idx = self.type_ref(td)?;
        let kind = if ConcreteType::from(td.clone()).is_value_type(self.loader) {
            ValueKind::ValueType
        } else {
            ValueKind::Class
        };
        Ok(prim(BaseType::Type {
            source: TypeSource::User(UserType::Reference(idx)),
            value_kind: Some(kind),
        }))
    }

    /// Translate a callee signature type, which indexes the callee's own resolution.
    fn translate_signature(&mut self, owner: &ResolutionS, t: &MethodType) -> R<MethodType> {
        let MethodType::Base(b) = t else {
            return Ok(t.clone());
        };
        Ok(match &**b {
            BaseType::Boolean => prim(BaseType::Boolean),
            BaseType::Char => prim(BaseType::Char),
            BaseType::Int8 => prim(BaseType::Int8),
            BaseType::UInt8 => prim(BaseType::UInt8),
            BaseType::Int16 => prim(BaseType::Int16),
            BaseType::UInt16 => prim(BaseType::UInt16),
            BaseType::Int32 => prim(BaseType::Int32),
            BaseType::UInt32 => prim(BaseType::UInt32),
            BaseType::Int64 => prim(BaseType::Int64),
            BaseType::UInt64 => prim(BaseType::UInt64),
            BaseType::Float32 => prim(BaseType::Float32),
            BaseType::Float64 => prim(BaseType::Float64),
            BaseType::IntPtr => prim(BaseType::IntPtr),
            BaseType::UIntPtr => prim(BaseType::UIntPtr),
            BaseType::Object => prim(BaseType::Object),
            BaseType::String => prim(BaseType::String),
            BaseType::Type {
                source: TypeSource::User(ut),
                ..
            } => {
                let td = self
                    .loader
                    .locate_type(owner.clone(), *ut)
                    .map_err(|e| e.to_string())?;
                self.type_node(&td)?
            }
            BaseType::Type {
                source: TypeSource::Generic { base, parameters },
                value_kind,
            } => {
                let td = self
                    .loader
                    .locate_type(owner.clone(), *base)
                    .map_err(|e| e.to_string())?;
                let base = UserType::Reference(self.type_ref(&td)?);
                let parameters = parameters
                    .iter()
                    .map(|p| self.translate_signature(owner, p))
                    .collect::<R<Vec<_>>>()?;
                prim(BaseType::Type {
                    source: TypeSource::Generic { base, parameters },
                    value_kind: *value_kind,
                })
            }
            BaseType::Array(inner, shape) => prim(BaseType::Array(
                self.translate_signature(owner, inner)?,
                shape.clone(),
            )),
            BaseType::ValuePointer(m, inner) if m.is_empty() => prim(BaseType::ValuePointer(
                vec![],
                inner
                    .as_ref()
                    .map(|t| self.translate_signature(owner, t))
                    .transpose()?,
            )),
            BaseType::Vector(m, inner) if m.is_empty() => {
                let inner = self.translate_signature(owner, inner)?;
                prim(BaseType::Vector(vec![], inner))
            }
            other => return Err(format!("unsupported callee signature type {other:?}")),
        })
    }

    /// Translate a runtime `Type` argument (return, parameter, local and catch types).
    fn translate_runtime(&mut self, rt: &RuntimeType) -> R<MethodType> {
        Ok(match rt {
            RuntimeType::Boolean => prim(BaseType::Boolean),
            RuntimeType::Char => prim(BaseType::Char),
            RuntimeType::Int8 => prim(BaseType::Int8),
            RuntimeType::UInt8 => prim(BaseType::UInt8),
            RuntimeType::Int16 => prim(BaseType::Int16),
            RuntimeType::UInt16 => prim(BaseType::UInt16),
            RuntimeType::Int32 => prim(BaseType::Int32),
            RuntimeType::UInt32 => prim(BaseType::UInt32),
            RuntimeType::Int64 => prim(BaseType::Int64),
            RuntimeType::UInt64 => prim(BaseType::UInt64),
            RuntimeType::Float32 => prim(BaseType::Float32),
            RuntimeType::Float64 => prim(BaseType::Float64),
            RuntimeType::IntPtr => prim(BaseType::IntPtr),
            RuntimeType::UIntPtr => prim(BaseType::UIntPtr),
            RuntimeType::Object => prim(BaseType::Object),
            RuntimeType::String => prim(BaseType::String),
            RuntimeType::Type(td) => self.type_node(td)?,
            RuntimeType::Generic(td, args) => self.generic_type(td, args)?,
            RuntimeType::Array(inner, rank) => prim(BaseType::Array(
                self.translate_runtime(inner)?,
                dotnetdll::binary::signature::encoded::ArrayShape {
                    rank: *rank as usize,
                    sizes: vec![],
                    lower_bounds: vec![],
                },
            )),
            RuntimeType::Pointer(inner) | RuntimeType::ValuePointer(inner, _) => {
                prim(BaseType::ValuePointer(
                    vec![],
                    if matches!(**inner, RuntimeType::Void) {
                        None
                    } else {
                        Some(self.translate_runtime(inner)?)
                    },
                ))
            }
            RuntimeType::Vector(inner) => {
                let inner = self.translate_runtime(inner)?;
                prim(BaseType::Vector(vec![], inner))
            }
            other => return Err(format!("unsupported type {other:?}")),
        })
    }

    fn generic_type(&mut self, td: &TypeDescription, args: &[RuntimeType]) -> R<MethodType> {
        let base = UserType::Reference(self.type_ref(td)?);
        let parameters = args
            .iter()
            .map(|arg| self.translate_runtime(arg))
            .collect::<R<Vec<_>>>()?;
        let value_kind = if ConcreteType::from(td.clone()).is_value_type(self.loader) {
            ValueKind::ValueType
        } else {
            ValueKind::Class
        };
        Ok(prim(BaseType::Type {
            source: TypeSource::Generic { base, parameters },
            value_kind: Some(value_kind),
        }))
    }

    fn parent_type(&mut self, td: &TypeDescription, lookup: &GenericLookup) -> R<MethodType> {
        let arity = td.definition().generic_parameters.len();
        if arity == 0 {
            return self.type_node(td);
        }
        let args = lookup
            .type_generics
            .get(..arity)
            .ok_or("missing declaring type arguments")?
            .iter()
            .map(|c| {
                runtime_type_from_concrete(self.loader, c)
                    .ok_or_else(|| format!("unsupported generic argument {c:?}"))
            })
            .collect::<R<Vec<_>>>()?;
        self.generic_type(td, &args)
    }

    fn runtime_parameter(&mut self, rt: &RuntimeType) -> R<ParameterType<MethodType>> {
        Ok(match rt {
            RuntimeType::ByRef(inner) => ParameterType::Ref(self.translate_runtime(inner)?),
            RuntimeType::TypedReference => ParameterType::TypedReference,
            _ => ParameterType::Value(self.translate_runtime(rt)?),
        })
    }

    fn signature_parameter(
        &mut self,
        owner: &ResolutionS,
        p: &ParameterType<MethodType>,
    ) -> R<ParameterType<MethodType>> {
        Ok(match p {
            ParameterType::Value(t) => ParameterType::Value(self.translate_signature(owner, t)?),
            ParameterType::Ref(t) => ParameterType::Ref(self.translate_signature(owner, t)?),
            ParameterType::TypedReference => ParameterType::TypedReference,
        })
    }

    fn method_ref(&mut self, method: &(MethodDescription, GenericLookup)) -> R<MethodSource> {
        self.method_ref_with_optional(method, None)
    }

    fn method_ref_with_optional(
        &mut self,
        (md, lookup): &(MethodDescription, GenericLookup),
        optional: Option<&[RuntimeType]>,
    ) -> R<MethodSource> {
        let method = md.method();
        let parent = self.parent_type(&md.parent, lookup)?;
        let owner = md.resolution();
        let sig = md.signature();
        let parameters = sig
            .parameters
            .iter()
            .map(|Parameter(_, p)| Ok(Parameter(vec![], self.signature_parameter(&owner, p)?)))
            .collect::<R<Vec<_>>>()?;
        let return_type = ReturnType(
            vec![],
            sig.return_type
                .1
                .as_ref()
                .map(|p| self.signature_parameter(&owner, p))
                .transpose()?,
        );
        let varargs = optional
            .map(|types| {
                types
                    .iter()
                    .map(|t| Ok(Parameter(vec![], self.runtime_parameter(t)?)))
                    .collect::<R<Vec<_>>>()
            })
            .transpose()?;
        let signature = MethodSignature {
            instance: sig.instance,
            explicit_this: sig.explicit_this,
            calling_convention: sig.calling_convention,
            parameters,
            return_type,
            varargs,
        };
        let idx = self.res.push_method_reference(ExternalMethodReference::new(
            MethodReferenceParent::Type(parent),
            method.name.to_string(),
            signature,
        ));
        let base = UserMethod::Reference(idx);
        let arity = method.generic_parameters.len();
        if arity == 0 {
            return Ok(MethodSource::User(base));
        }
        let args = lookup
            .method_generics
            .get(..arity)
            .ok_or("missing method generic arguments")?
            .iter()
            .map(|c| {
                runtime_type_from_concrete(self.loader, c)
                    .ok_or_else(|| format!("unsupported generic argument {c:?}"))
            })
            .collect::<R<Vec<_>>>()?;
        let args = args
            .iter()
            .map(|a| self.translate_runtime(a))
            .collect::<R<Vec<_>>>()?;
        Ok(MethodSource::Generic(GenericMethodInstantiation::new(
            base, args,
        )))
    }

    fn field_ref(&mut self, (fd, lookup): &(FieldDescription, GenericLookup)) -> R<FieldSource> {
        let field = fd.field();
        let parent = self.parent_type(&fd.parent, lookup)?;
        let field_type =
            self.translate_signature(&fd.resolution(), &field.return_type.clone().into())?;
        let field_type = member_type(field_type);
        Ok(FieldSource::Reference(self.res.push_field_reference(
            ExternalFieldReference::new(
                FieldReferenceParent::Type(parent),
                field_type,
                field.name.to_string().into(),
            ),
        )))
    }
}

fn read_i32s(obj: ObjectRef<'_>) -> R<Vec<i32>> {
    obj.try_as_vector(|v| {
        v.get()
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    })
    .map_err(|e| format!("{e:?}"))
}

fn read_i64s(obj: ObjectRef<'_>) -> R<Vec<i64>> {
    obj.try_as_vector(|v| {
        v.get()
            .chunks_exact(8)
            .map(|c| i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect()
    })
    .map_err(|e| format!("{e:?}"))
}

/// Map one recorded `(opcode, immediate, reference)` triple to a resolved instruction.
fn instruction(
    b: &mut Builder<'_>,
    op: u16,
    imm: i64,
    reference: Option<&Operand>,
) -> R<Instruction> {
    use Instruction as I;
    let need_type = || match reference {
        Some(Operand::Type(t)) => Ok(t.clone()),
        _ => Err(format!("opcode 0x{op:04X} needs a type operand")),
    };
    let need_method = || match reference {
        Some(Operand::Method(m)) => Ok(m.clone()),
        _ => Err(format!("opcode 0x{op:04X} needs a method operand")),
    };
    let need_field = || match reference {
        Some(Operand::Field(f)) => Ok(f.clone()),
        _ => Err(format!("opcode 0x{op:04X} needs a field operand")),
    };
    let call_ref = |b: &mut Builder<'_>| match reference {
        Some(Operand::Method(m)) => b.method_ref(m),
        Some(Operand::MethodWithOptional(m, optional)) => {
            b.method_ref_with_optional(m, Some(optional))
        }
        _ => Err(format!("opcode 0x{op:04X} needs a method operand")),
    };
    Ok(match op {
        0x00 => I::NoOperation,
        0x01 => I::Breakpoint,
        0x02..=0x05 => I::LoadArgument(op - 0x02),
        0x06..=0x09 => I::LoadLocal(op - 0x06),
        0x0A..=0x0D => I::StoreLocal(op - 0x0A),
        0x0E | 0xFE09 => I::LoadArgument(imm as u16),
        0x0F | 0xFE0A => I::LoadArgumentAddress(imm as u16),
        0x10 | 0xFE0B => I::StoreArgument(imm as u16),
        0x11 | 0xFE0C => I::LoadLocal(imm as u16),
        0x12 | 0xFE0D => I::LoadLocalAddress(imm as u16),
        0x13 | 0xFE0E => I::StoreLocal(imm as u16),
        0x14 => I::LoadNull,
        0x15 => I::LoadConstantInt32(-1),
        0x16..=0x1E => I::LoadConstantInt32(i32::from(op - 0x16)),
        // `ldc.i4.s` takes an sbyte; the shim records it through the `byte` overload.
        0x1F => I::LoadConstantInt32(imm as u8 as i8 as i32),
        0x20 => I::LoadConstantInt32(imm as i32),
        0x21 => I::LoadConstantInt64(imm),
        0x22 => I::LoadConstantFloat32(f32::from_bits(imm as u32)),
        0x23 => I::LoadConstantFloat64(f64::from_bits(imm as u64)),
        0x25 => I::Duplicate,
        0x26 => I::Pop,
        0x27 => I::Jump(b.method_ref(&need_method()?)?),
        0x2A => I::Return,
        0x2B | 0x38 => I::Branch(imm as usize),
        0x2C | 0x39 => I::BranchFalsy(imm as usize),
        0x2D | 0x3A => I::BranchTruthy(imm as usize),
        0x2E | 0x3B => I::BranchEqual(imm as usize),
        0x2F | 0x3C => I::BranchGreaterOrEqual(NumberSign::Signed, imm as usize),
        0x30 | 0x3D => I::BranchGreater(NumberSign::Signed, imm as usize),
        0x31 | 0x3E => I::BranchLessOrEqual(NumberSign::Signed, imm as usize),
        0x32 | 0x3F => I::BranchLess(NumberSign::Signed, imm as usize),
        0x33 | 0x40 => I::BranchNotEqual(imm as usize),
        0x34 | 0x41 => I::BranchGreaterOrEqual(NumberSign::Unsigned, imm as usize),
        0x35 | 0x42 => I::BranchGreater(NumberSign::Unsigned, imm as usize),
        0x36 | 0x43 => I::BranchLessOrEqual(NumberSign::Unsigned, imm as usize),
        0x37 | 0x44 => I::BranchLess(NumberSign::Unsigned, imm as usize),
        0x45 => match reference {
            Some(Operand::Targets(targets)) => I::Switch(targets.clone()),
            _ => return Err("switch needs label operands".into()),
        },
        0x46 => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::Int8,
        },
        0x47 => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::UInt8,
        },
        0x48 => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::Int16,
        },
        0x49 => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::UInt16,
        },
        0x4A => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::Int32,
        },
        0x4B => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::UInt32,
        },
        0x4C => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::Int64,
        },
        0x4D => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::IntPtr,
        },
        0x4E => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::Float32,
        },
        0x4F => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::Float64,
        },
        0x50 => I::LoadIndirect {
            unaligned: None,
            volatile: false,
            param0: LoadType::Object,
        },
        0x51 => I::StoreIndirect {
            unaligned: None,
            volatile: false,
            param0: StoreType::Object,
        },
        0x52 => I::StoreIndirect {
            unaligned: None,
            volatile: false,
            param0: StoreType::Int8,
        },
        0x53 => I::StoreIndirect {
            unaligned: None,
            volatile: false,
            param0: StoreType::Int16,
        },
        0x54 => I::StoreIndirect {
            unaligned: None,
            volatile: false,
            param0: StoreType::Int32,
        },
        0x55 => I::StoreIndirect {
            unaligned: None,
            volatile: false,
            param0: StoreType::Int64,
        },
        0x56 => I::StoreIndirect {
            unaligned: None,
            volatile: false,
            param0: StoreType::Float32,
        },
        0x57 => I::StoreIndirect {
            unaligned: None,
            volatile: false,
            param0: StoreType::Float64,
        },
        0xDF => I::StoreIndirect {
            unaligned: None,
            volatile: false,
            param0: StoreType::IntPtr,
        },
        0x58 => I::Add,
        0x59 => I::Subtract,
        0x5A => I::Multiply,
        0x5B => I::Divide(NumberSign::Signed),
        0x5C => I::Divide(NumberSign::Unsigned),
        0x5D => I::Remainder(NumberSign::Signed),
        0x5E => I::Remainder(NumberSign::Unsigned),
        0x5F => I::And,
        0x60 => I::Or,
        0x61 => I::Xor,
        0x62 => I::ShiftLeft,
        0x63 => I::ShiftRight(NumberSign::Signed),
        0x64 => I::ShiftRight(NumberSign::Unsigned),
        0x65 => I::Negate,
        0x66 => I::Not,
        0x67 => I::Convert(ConversionType::Int8),
        0x68 => I::Convert(ConversionType::Int16),
        0x69 => I::Convert(ConversionType::Int32),
        0x6A => I::Convert(ConversionType::Int64),
        0x6B => I::ConvertFloat32,
        0x6C => I::ConvertFloat64,
        0x6D => I::Convert(ConversionType::UInt32),
        0x6E => I::Convert(ConversionType::UInt64),
        0x76 => I::ConvertUnsignedToFloat,
        0xD1 => I::Convert(ConversionType::UInt16),
        0xD2 => I::Convert(ConversionType::UInt8),
        0xD3 => I::Convert(ConversionType::IntPtr),
        0xE0 => I::Convert(ConversionType::UIntPtr),
        0xD6 => I::AddOverflow(NumberSign::Signed),
        0xD7 => I::AddOverflow(NumberSign::Unsigned),
        0xD8 => I::MultiplyOverflow(NumberSign::Signed),
        0xD9 => I::MultiplyOverflow(NumberSign::Unsigned),
        0xDA => I::SubtractOverflow(NumberSign::Signed),
        0xDB => I::SubtractOverflow(NumberSign::Unsigned),
        0xB3 => I::ConvertOverflow(ConversionType::Int8, NumberSign::Signed),
        0xB4 => I::ConvertOverflow(ConversionType::UInt8, NumberSign::Signed),
        0xB5 => I::ConvertOverflow(ConversionType::Int16, NumberSign::Signed),
        0xB6 => I::ConvertOverflow(ConversionType::UInt16, NumberSign::Signed),
        0xB7 => I::ConvertOverflow(ConversionType::Int32, NumberSign::Signed),
        0xB8 => I::ConvertOverflow(ConversionType::UInt32, NumberSign::Signed),
        0xB9 => I::ConvertOverflow(ConversionType::Int64, NumberSign::Signed),
        0xBA => I::ConvertOverflow(ConversionType::UInt64, NumberSign::Signed),
        0xD4 => I::ConvertOverflow(ConversionType::IntPtr, NumberSign::Signed),
        0xD5 => I::ConvertOverflow(ConversionType::UIntPtr, NumberSign::Signed),
        0x82 => I::ConvertOverflow(ConversionType::Int8, NumberSign::Unsigned),
        0x83 => I::ConvertOverflow(ConversionType::Int16, NumberSign::Unsigned),
        0x84 => I::ConvertOverflow(ConversionType::Int32, NumberSign::Unsigned),
        0x85 => I::ConvertOverflow(ConversionType::Int64, NumberSign::Unsigned),
        0x86 => I::ConvertOverflow(ConversionType::UInt8, NumberSign::Unsigned),
        0x87 => I::ConvertOverflow(ConversionType::UInt16, NumberSign::Unsigned),
        0x88 => I::ConvertOverflow(ConversionType::UInt32, NumberSign::Unsigned),
        0x89 => I::ConvertOverflow(ConversionType::UInt64, NumberSign::Unsigned),
        0x8A => I::ConvertOverflow(ConversionType::IntPtr, NumberSign::Unsigned),
        0x8B => I::ConvertOverflow(ConversionType::UIntPtr, NumberSign::Unsigned),
        0xC3 => I::CheckFinite,
        0xC2 => I::ReadTypedReferenceValue(b.translate_runtime(&need_type()?)?),
        0xC6 => I::MakeTypedReference(b.translate_runtime(&need_type()?)?),
        0xD0 => match reference {
            Some(Operand::Type(t)) => I::LoadTokenType(b.translate_runtime(t)?),
            Some(Operand::Method(m)) => I::LoadTokenMethod(b.method_ref(m)?),
            Some(Operand::Field(f)) => I::LoadTokenField(b.field_ref(f)?),
            _ => return Err("ldtoken needs a type, method or field".into()),
        },
        0x28 => I::Call {
            tail_call: false,
            param0: call_ref(b)?,
        },
        0x29 => {
            let Some(Operand::Signature(types)) = reference else {
                return Err("calli needs a signature".into());
            };
            let Some((ret, args)) = types.split_first() else {
                return Err("empty calli signature".into());
            };
            let fixed_count = (imm as u64 >> 32) as usize;
            if fixed_count > args.len() {
                return Err("invalid calli parameter count".into());
            }
            let bits = imm as u32;
            let convention = if bits & 0x100 != 0 {
                match bits & 0xff {
                    1 => StandAloneCallingConvention::DefaultUnmanaged,
                    2 => StandAloneCallingConvention::Cdecl,
                    3 => StandAloneCallingConvention::Stdcall,
                    4 => StandAloneCallingConvention::Thiscall,
                    5 => StandAloneCallingConvention::Fastcall,
                    _ => return Err("invalid unmanaged calling convention".into()),
                }
            } else if bits & 2 != 0 {
                StandAloneCallingConvention::Vararg
            } else {
                StandAloneCallingConvention::DefaultManaged
            };
            let return_type = if matches!(ret, RuntimeType::Void) {
                ReturnType(vec![], None)
            } else {
                ReturnType(vec![], Some(b.runtime_parameter(ret)?))
            };
            let mut parameters = Vec::with_capacity(fixed_count);
            let mut varargs = Vec::with_capacity(args.len() - fixed_count);
            for (index, t) in args.iter().enumerate() {
                let parameter = Parameter(vec![], b.runtime_parameter(t)?);
                if index < fixed_count {
                    parameters.push(parameter);
                } else {
                    varargs.push(parameter);
                }
            }
            I::CallIndirect {
                tail_call: false,
                param0: MethodSignature {
                    instance: bits & 0x100 == 0 && bits & 0x20 != 0,
                    explicit_this: bits & 0x100 == 0 && bits & 0x40 != 0,
                    calling_convention: convention,
                    parameters,
                    return_type,
                    varargs: (!varargs.is_empty()).then_some(varargs),
                },
            }
        }
        0x6F => I::CallVirtual {
            skip_null_check: false,
            param0: call_ref(b)?,
        },
        0x70 => I::CopyObject(b.translate_runtime(&need_type()?)?),
        0x71 => I::LoadObject {
            unaligned: None,
            volatile: false,
            param0: b.translate_runtime(&need_type()?)?,
        },
        0x73 => match call_ref(b)? {
            MethodSource::User(method) => I::NewObject(method),
            MethodSource::Generic(_) => return Err("generic constructor".into()),
        },
        0x72 => match reference {
            Some(Operand::Str(s)) => I::load_string(s),
            _ => return Err("ldstr needs a string operand".into()),
        },
        0x74 => I::CastClass {
            skip_type_check: false,
            param0: b.translate_runtime(&need_type()?)?,
        },
        0x75 => I::IsInstance(b.translate_runtime(&need_type()?)?),
        0x79 => I::UnboxIntoAddress {
            skip_type_check: false,
            param0: b.translate_runtime(&need_type()?)?,
        },
        0x7A => I::Throw,
        0x7B => I::LoadField {
            unaligned: None,
            volatile: false,
            param0: b.field_ref(&need_field()?)?,
        },
        0x7C => I::LoadFieldAddress(b.field_ref(&need_field()?)?),
        0x7D => I::StoreField {
            unaligned: None,
            volatile: false,
            param0: b.field_ref(&need_field()?)?,
        },
        0x7E => I::LoadStaticField {
            volatile: false,
            param0: b.field_ref(&need_field()?)?,
        },
        0x7F => I::LoadStaticFieldAddress(b.field_ref(&need_field()?)?),
        0x80 => I::StoreStaticField {
            volatile: false,
            param0: b.field_ref(&need_field()?)?,
        },
        0x81 => I::StoreObject {
            unaligned: None,
            volatile: false,
            param0: b.translate_runtime(&need_type()?)?,
        },
        0x8C => I::BoxValue(b.translate_runtime(&need_type()?)?),
        0x8D => I::NewArray(b.translate_runtime(&need_type()?)?),
        0x8E => I::LoadLength,
        0x8F => I::LoadElementAddress {
            skip_type_check: false,
            skip_range_check: false,
            skip_null_check: false,
            param0: b.translate_runtime(&need_type()?)?,
        },
        0x90 => I::LoadElementPrimitive {
            skip_range_check: false,
            skip_null_check: false,
            param0: LoadType::Int8,
        },
        0x91 => I::LoadElementPrimitive {
            skip_range_check: false,
            skip_null_check: false,
            param0: LoadType::UInt8,
        },
        0x92 => I::LoadElementPrimitive {
            skip_range_check: false,
            skip_null_check: false,
            param0: LoadType::Int16,
        },
        0x93 => I::LoadElementPrimitive {
            skip_range_check: false,
            skip_null_check: false,
            param0: LoadType::UInt16,
        },
        0x94 => I::LoadElementPrimitive {
            skip_range_check: false,
            skip_null_check: false,
            param0: LoadType::Int32,
        },
        0x95 => I::LoadElementPrimitive {
            skip_range_check: false,
            skip_null_check: false,
            param0: LoadType::UInt32,
        },
        0x96 => I::LoadElementPrimitive {
            skip_range_check: false,
            skip_null_check: false,
            param0: LoadType::Int64,
        },
        0x97 => I::LoadElementPrimitive {
            skip_range_check: false,
            skip_null_check: false,
            param0: LoadType::IntPtr,
        },
        0x98 => I::LoadElementPrimitive {
            skip_range_check: false,
            skip_null_check: false,
            param0: LoadType::Float32,
        },
        0x99 => I::LoadElementPrimitive {
            skip_range_check: false,
            skip_null_check: false,
            param0: LoadType::Float64,
        },
        0x9A => I::LoadElement {
            skip_range_check: false,
            skip_null_check: false,
            param0: prim(BaseType::Object),
        },
        0x9B => I::StoreElementPrimitive {
            skip_type_check: false,
            skip_range_check: false,
            skip_null_check: false,
            param0: StoreType::IntPtr,
        },
        0x9C => I::StoreElementPrimitive {
            skip_type_check: false,
            skip_range_check: false,
            skip_null_check: false,
            param0: StoreType::Int8,
        },
        0x9D => I::StoreElementPrimitive {
            skip_type_check: false,
            skip_range_check: false,
            skip_null_check: false,
            param0: StoreType::Int16,
        },
        0x9E => I::StoreElementPrimitive {
            skip_type_check: false,
            skip_range_check: false,
            skip_null_check: false,
            param0: StoreType::Int32,
        },
        0x9F => I::StoreElementPrimitive {
            skip_type_check: false,
            skip_range_check: false,
            skip_null_check: false,
            param0: StoreType::Int64,
        },
        0xA0 => I::StoreElementPrimitive {
            skip_type_check: false,
            skip_range_check: false,
            skip_null_check: false,
            param0: StoreType::Float32,
        },
        0xA1 => I::StoreElementPrimitive {
            skip_type_check: false,
            skip_range_check: false,
            skip_null_check: false,
            param0: StoreType::Float64,
        },
        0xA2 => I::StoreElement {
            skip_type_check: false,
            skip_range_check: false,
            skip_null_check: false,
            param0: prim(BaseType::Object),
        },
        0xA3 => I::LoadElement {
            skip_range_check: false,
            skip_null_check: false,
            param0: b.translate_runtime(&need_type()?)?,
        },
        0xA4 => I::StoreElement {
            skip_type_check: false,
            skip_range_check: false,
            skip_null_check: false,
            param0: b.translate_runtime(&need_type()?)?,
        },
        0xA5 => I::UnboxIntoValue(b.translate_runtime(&need_type()?)?),
        0xDC => I::EndFinally,
        0xDD | 0xDE => I::Leave(imm as usize),
        0xFE00 => I::ArgumentList,
        0xFE01 => I::CompareEqual,
        0xFE02 => I::CompareGreater(NumberSign::Signed),
        0xFE03 => I::CompareGreater(NumberSign::Unsigned),
        0xFE04 => I::CompareLess(NumberSign::Signed),
        0xFE05 => I::CompareLess(NumberSign::Unsigned),
        0xFE06 => I::LoadMethodPointer(b.method_ref(&need_method()?)?),
        0xFE07 => I::LoadVirtualMethodPointer {
            skip_null_check: false,
            param0: b.method_ref(&need_method()?)?,
        },
        0xFE0F => I::LocalMemoryAllocate,
        0xFE11 => I::EndFilter,
        0xFE15 => I::InitializeForObject(b.translate_runtime(&need_type()?)?),
        0xFE17 => I::CopyMemoryBlock {
            unaligned: None,
            volatile: false,
        },
        0xFE18 => I::InitializeMemoryBlock {
            unaligned: None,
            volatile: false,
        },
        0xFE1A => I::Rethrow,
        0xFE1C => I::Sizeof(b.translate_runtime(&need_type()?)?),
        0xFE1D => I::ReadTypedReferenceType,
        other => {
            return Err(format!(
                "opcode 0x{other:04X} is not supported yet (plan 25)"
            ));
        }
    })
}

enum Operand {
    Type(RuntimeType),
    Method((MethodDescription, GenericLookup)),
    MethodWithOptional((MethodDescription, GenericLookup), Vec<RuntimeType>),
    Field((FieldDescription, GenericLookup)),
    Str(String),
    Targets(Vec<usize>),
    Signature(Vec<RuntimeType>),
}

#[derive(Default)]
struct Prefixes {
    constraint: Option<MethodType>,
    tail: bool,
    volatile: bool,
    unaligned: Option<Alignment>,
    readonly: bool,
    no_checks: u8,
}

impl Prefixes {
    fn apply(self, instruction: Instruction) -> R<Instruction> {
        use Instruction as I;
        if (self.constraint.is_some() || self.tail)
            && (self.volatile || self.unaligned.is_some() || self.readonly)
            || self.readonly && (self.volatile || self.unaligned.is_some())
            || self.no_checks != 0
                && (self.constraint.is_some()
                    || self.tail
                    || self.volatile
                    || self.unaligned.is_some()
                    || self.readonly)
        {
            return Err("unsupported prefix combination".into());
        }
        if self.constraint.is_some() || self.tail {
            return match (self.constraint, self.tail, instruction) {
                (Some(t), true, I::CallVirtual { param0, .. }) => {
                    Ok(I::CallVirtualConstrainedTail(t, param0))
                }
                (Some(t), false, I::CallVirtual { param0, .. }) => {
                    Ok(I::CallVirtualConstrained(t, param0))
                }
                (Some(t), false, I::Call { param0, .. }) => Ok(I::CallConstrained(t, param0)),
                (Some(t), true, I::Call { param0, .. }) => Ok(I::CallConstrainedTail(t, param0)),
                (None, true, I::Call { param0, .. }) => Ok(I::Call {
                    param0,
                    tail_call: true,
                }),
                (None, true, I::CallIndirect { param0, .. }) => Ok(I::CallIndirect {
                    param0,
                    tail_call: true,
                }),
                (None, true, I::CallVirtual { param0, .. }) => Ok(I::CallVirtualTail(param0)),
                _ => Err("unsupported constrained or tail prefix".into()),
            };
        }
        if self.readonly {
            return match instruction {
                I::LoadElementAddress { param0, .. } => Ok(I::LoadElementAddressReadonly(param0)),
                _ => Err("readonly prefix requires ldelema".into()),
            };
        }
        if self.no_checks != 0 {
            let typ = self.no_checks & 1 != 0;
            let range = self.no_checks & 2 != 0;
            let null = self.no_checks & 4 != 0;
            return Ok(match instruction {
                I::CallVirtual { param0, .. } if !typ && !range => I::CallVirtual {
                    param0,
                    skip_null_check: null,
                },
                I::LoadVirtualMethodPointer { param0, .. } if !typ && !range => {
                    I::LoadVirtualMethodPointer {
                        param0,
                        skip_null_check: null,
                    }
                }
                I::CastClass { param0, .. } if !range && !null => I::CastClass {
                    param0,
                    skip_type_check: typ,
                },
                I::UnboxIntoAddress { param0, .. } if !range && !null => I::UnboxIntoAddress {
                    param0,
                    skip_type_check: typ,
                },
                I::LoadElement { param0, .. } if !typ => I::LoadElement {
                    param0,
                    skip_range_check: range,
                    skip_null_check: null,
                },
                I::LoadElementPrimitive { param0, .. } if !typ => I::LoadElementPrimitive {
                    param0,
                    skip_range_check: range,
                    skip_null_check: null,
                },
                I::LoadElementAddress { param0, .. } => I::LoadElementAddress {
                    param0,
                    skip_type_check: typ,
                    skip_range_check: range,
                    skip_null_check: null,
                },
                I::StoreElement { param0, .. } => I::StoreElement {
                    param0,
                    skip_type_check: typ,
                    skip_range_check: range,
                    skip_null_check: null,
                },
                I::StoreElementPrimitive { param0, .. } => I::StoreElementPrimitive {
                    param0,
                    skip_type_check: typ,
                    skip_range_check: range,
                    skip_null_check: null,
                },
                I::LoadField { param0, .. } if !typ && !range && null => {
                    I::LoadFieldSkipNullCheck(param0)
                }
                I::StoreField { param0, .. } if !typ && !range && null => {
                    I::StoreFieldSkipNullCheck(param0)
                }
                _ => return Err("unsupported no. prefix".into()),
            });
        }
        if self.volatile || self.unaligned.is_some() {
            let unaligned = self.unaligned;
            let volatile = self.volatile;
            return Ok(match instruction {
                I::LoadIndirect { param0, .. } => I::LoadIndirect {
                    param0,
                    unaligned,
                    volatile,
                },
                I::StoreIndirect { param0, .. } => I::StoreIndirect {
                    param0,
                    unaligned,
                    volatile,
                },
                I::LoadField { param0, .. } => I::LoadField {
                    param0,
                    unaligned,
                    volatile,
                },
                I::StoreField { param0, .. } => I::StoreField {
                    param0,
                    unaligned,
                    volatile,
                },
                I::LoadObject { param0, .. } => I::LoadObject {
                    param0,
                    unaligned,
                    volatile,
                },
                I::StoreObject { param0, .. } => I::StoreObject {
                    param0,
                    unaligned,
                    volatile,
                },
                I::CopyMemoryBlock { .. } => I::CopyMemoryBlock {
                    unaligned,
                    volatile,
                },
                I::InitializeMemoryBlock { .. } => I::InitializeMemoryBlock {
                    unaligned,
                    volatile,
                },
                I::LoadStaticField { param0, .. } if unaligned.is_none() => {
                    I::LoadStaticField { param0, volatile }
                }
                I::StoreStaticField { param0, .. } if unaligned.is_none() => {
                    I::StoreStaticField { param0, volatile }
                }
                _ => return Err("unsupported volatile or unaligned prefix".into()),
            });
        }
        Ok(instruction)
    }
}

#[dotnet_intrinsic(
    "static System.Reflection.MethodInfo DotnetRs.RecordingILGenerator::CompileRecorded(string, System.Type, System.Type[], int[], long[], object[], System.Type[], int[], int[], System.Type[], int)",
    select = compile_recorded_select,
    fallback = compile_recorded_fallback
)]
pub async fn compile_recorded_native(mut fx: VmEffects) -> NativeResult {
    let prepared = {
        let mut vm = fx.vm().await?;
        vm.with(|region| region.reflection_compile_recorded_target())?
    };
    let (method, lookup) = match prepared {
        Ok(target) => target,
        Err(error) => return Err(error),
    };
    #[cfg(feature = "multithreading")]
    {
        fx.await_canonical(dotnet_vm_ops::effects::CanonicalKey::Method(
            method.clone(),
            lookup.clone(),
        ))
        .await?;
        return fx
            .vm()
            .await?
            .with(|region| region.reflection_method_complete(&method, &lookup))?;
    }
    #[cfg(not(feature = "multithreading"))]
    {
        let _ = (method, lookup);
        Err(dotnet_vm_ops::effects::NativeFailure::Host(
            dotnet_types::error::ExecutionError::InternalError(
                "recorded reflection activation requires multithreading".into(),
            )
            .into(),
        ))
    }
}

pub fn compile_recorded_select<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &T,
    _operands: dotnet_vm_ops::effects::NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    _generics: &GenericLookup,
) -> dotnet_vm_ops::effects::NativeSelection {
    if ctx.reflection_method_requires_homing() {
        dotnet_vm_ops::effects::NativeSelection::Async(dotnet_vm_ops::effects::NativeInvocation {
            start: 0,
            length: 0,
            parameter_count: 11,
        })
    } else {
        dotnet_vm_ops::effects::NativeSelection::Fallback
    }
}

pub fn compile_recorded_fallback<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let max_stack = ctx.pop_i32();
    let eh_types = ctx.pop_obj();
    let eh = ctx.pop_obj();
    let local_pins = ctx.pop_obj();
    let locals = ctx.pop_obj();
    let refs = ctx.pop_obj();
    let imm = ctx.pop_obj();
    let ops = ctx.pop_obj();
    let params = ctx.pop_obj();
    let ret = ctx.pop_obj();
    let name = ctx.pop_obj();

    match build(
        ctx,
        [
            name, ret, params, ops, imm, refs, locals, local_pins, eh, eh_types,
        ],
        max_stack,
    ) {
        Ok(md) => {
            let obj = crate::common::get_runtime_method_obj(ctx, md, GenericLookup::default());
            ctx.push_obj(obj);
            StepResult::Continue
        }
        Err(msg) => ctx.throw_by_name_with_message(
            "System.NotSupportedException",
            &format!("DynamicMethod: {msg}"),
        ),
    }
}

/// Build the recorded method from the native activation's rooted operands without consuming or
/// re-reading guest arguments. The caller clears the activation operands only after this returns.
pub fn compile_recorded_from_operands<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    operands: &[StackValue<'gc>],
) -> Result<MethodDescription, String> {
    let object = |index: usize| match operands.get(index) {
        Some(StackValue::ObjectRef(object)) => Ok(*object),
        _ => Err(format!("argument {index} is not an object reference")),
    };
    let max_stack = match operands.get(10) {
        Some(StackValue::Int32(value)) => *value,
        _ => return Err("maxstack is not an Int32".into()),
    };
    build(
        ctx,
        [
            object(0)?,
            object(1)?,
            object(2)?,
            object(3)?,
            object(4)?,
            object(5)?,
            object(6)?,
            object(7)?,
            object(8)?,
            object(9)?,
        ],
        max_stack,
    )
}

fn build<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    [
        name,
        ret,
        params,
        ops,
        imm,
        refs,
        locals,
        local_pins,
        eh,
        eh_types,
    ]: [ObjectRef<'gc>; 10],
    max_stack: i32,
) -> R<MethodDescription> {
    let gc = ctx.gc_with_token(&ctx.no_active_borrows_token());
    let object_refs = |obj: ObjectRef<'gc>| -> R<Vec<ObjectRef<'gc>>> {
        obj.try_as_vector(|v| v.object_ref_elements(&gc).collect::<Vec<_>>())
            .map_err(|e| format!("{e:?}"))
    };
    let name = crate::types::string_from_heap_obj(name).map_err(|e| format!("{e:?}"))?;
    let ret_rt = crate::common::resolve_runtime_type(ctx, ret).map_err(|e| e.to_string())?;
    let types_of = |obj: ObjectRef<'gc>| -> R<Vec<Option<RuntimeType>>> {
        object_refs(obj)?
            .into_iter()
            .map(|t| {
                if t.0.is_none() {
                    Ok(None)
                } else {
                    crate::common::resolve_runtime_type(ctx, t)
                        .map(Some)
                        .map_err(|e| e.to_string())
                }
            })
            .collect()
    };
    let param_rts = types_of(params)?;
    let local_rts = types_of(locals)?;
    let local_pins = read_i32s(local_pins)?;
    if local_pins.len() != local_rts.len() {
        return Err("inconsistent local recording".into());
    }
    let eh_rts = types_of(eh_types)?;
    let ops = read_i32s(ops)?;
    let imm = read_i64s(imm)?;
    let eh = read_i32s(eh)?;

    let mut operands = Vec::new();
    for (i, r) in object_refs(refs)?.into_iter().enumerate() {
        operands.push(if r.0.is_none() {
            None
        } else if ops.get(i).map(|op| op & 0xFFFF) == Some(0x45) {
            Some(Operand::Targets(
                read_i32s(r)?.into_iter().map(|x| x as usize).collect(),
            ))
        } else if ops.get(i).map(|op| op & 0xFFFF) == Some(0x29) {
            Some(Operand::Signature(
                types_of(r)?
                    .into_iter()
                    .collect::<Option<Vec<_>>>()
                    .ok_or("null calli type")?,
            ))
        } else if matches!(ops.get(i).map(|op| op & 0xFFFF), Some(0x28 | 0x6F | 0x73))
            && imm.get(i) == Some(&1)
        {
            let call = object_refs(r)?;
            if call.len() != 2 {
                return Err("invalid optional argument recording".into());
            }
            let method =
                crate::common::resolve_runtime_method(ctx, call[0]).map_err(|e| e.to_string())?;
            let optional = types_of(call[1])?
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or("null optional argument type")?;
            Some(Operand::MethodWithOptional(method, optional))
        } else if matches!(ops.get(i).map(|op| op & 0xFFFF), Some(0x7B..=0x80))
            || (ops.get(i).map(|op| op & 0xFFFF) == Some(0xD0) && imm.get(i) == Some(&3))
        {
            Some(Operand::Field(
                crate::common::resolve_runtime_field(ctx, r).map_err(|e| e.to_string())?,
            ))
        } else if ops.get(i).map(|op| op & 0xFFFF) == Some(0x72) {
            Some(Operand::Str(
                crate::types::string_from_heap_obj(r).map_err(|e| format!("{e:?}"))?,
            ))
        } else if (ops.get(i).map(|op| op & 0xFFFF) == Some(0xD0) && imm.get(i) == Some(&2))
            || matches!(
                ops.get(i).map(|op| op & 0xFFFF),
                Some(0x27 | 0x28 | 0x6F | 0x73 | 0xFE06 | 0xFE07)
            )
        {
            Some(Operand::Method(
                crate::common::resolve_runtime_method(ctx, r).map_err(|e| e.to_string())?,
            ))
        } else {
            Some(Operand::Type(
                crate::common::resolve_runtime_type(ctx, r).map_err(|e| e.to_string())?,
            ))
        });
    }
    if ops.len() != imm.len() || ops.len() != operands.len() {
        return Err("inconsistent recording".into());
    }

    let loader = ctx.loader().clone();
    let id = next_dynamic_assembly_id(&DYNAMIC_ASSEMBLY_ID)?;
    let asm_name = format!("DynamicMethod${id}");
    let mut res = Resolution::new(Module::new(format!("{asm_name}.dll")));
    res.assembly = Some(Assembly::new(asm_name));
    let mut b = Builder {
        loader: &loader,
        res,
        assembly_refs: HashMap::new(),
        type_refs: HashMap::new(),
    };

    let object_td = loader
        .corlib_wkt(dotnet_types::WellKnown::Object)
        .map_err(|e| e.to_string())?;
    let object_ref = b.type_ref(&object_td)?;
    let mut holder = TypeDefinition::new(None, "DynamicMethods");
    holder.extends = Some(TypeSource::User(UserType::Reference(object_ref)));
    let holder_idx = b.res.push_type_definition(holder);

    let mut parameters = Vec::new();
    for p in &param_rts {
        let p = p.as_ref().ok_or("null parameter type")?;
        parameters.push(Parameter(vec![], b.runtime_parameter(p)?));
    }
    let return_type = if matches!(ret_rt, RuntimeType::Void) {
        ReturnType(vec![], None)
    } else {
        ReturnType(vec![], Some(b.runtime_parameter(&ret_rt)?))
    };

    let mut local_variables = Vec::new();
    for (l, &pinned) in local_rts.iter().zip(&local_pins) {
        let l = l.as_ref().ok_or("null local type")?;
        let (by_ref, local_type) = match l {
            RuntimeType::ByRef(inner) => (true, inner.as_ref()),
            _ => (false, l),
        };
        local_variables.push(LocalVariable::Variable {
            custom_modifiers: vec![],
            pinned: pinned != 0,
            by_ref,
            var_type: b.translate_runtime(local_type)?,
        });
    }

    let mut instructions = Vec::with_capacity(ops.len());
    let mut prefixes = Prefixes::default();
    for i in 0..ops.len() {
        let op = (ops[i] & 0xFFFF) as u16;
        match op {
            0xFE12 => {
                prefixes.unaligned = Some(match imm[i] {
                    1 => Alignment::Byte,
                    2 => Alignment::Double,
                    4 => Alignment::Quad,
                    _ => return Err("invalid unaligned prefix".into()),
                })
            }
            0xFE13 => prefixes.volatile = true,
            0xFE14 => prefixes.tail = true,
            0xFE16 => {
                let Some(Operand::Type(t)) = &operands[i] else {
                    return Err("constrained prefix needs a type".into());
                };
                prefixes.constraint = Some(b.translate_runtime(t)?);
            }
            0xFE19 => {
                if imm[i] < 0 || imm[i] > 7 {
                    return Err("invalid no. prefix".into());
                }
                prefixes.no_checks = imm[i] as u8;
            }
            0xFE1E => prefixes.readonly = true,
            _ => {
                let instr = instruction(&mut b, op, imm[i], operands[i].as_ref())?;
                instructions.push(std::mem::take(&mut prefixes).apply(instr)?);
                continue;
            }
        }
        // Keep instruction indices stable for labels and exception clauses.
        instructions.push(Instruction::NoOperation);
    }
    if prefixes.tail
        || prefixes.constraint.is_some()
        || prefixes.volatile
        || prefixes.unaligned.is_some()
        || prefixes.readonly
        || prefixes.no_checks != 0
    {
        return Err("prefix without an instruction".into());
    }

    let mut clauses = Vec::new();
    for (k, c) in eh.chunks_exact(6).enumerate() {
        let (kind, try_start, try_end, handler_start, handler_end) = (
            c[0],
            c[1] as usize,
            c[2] as usize,
            c[3] as usize,
            c[4] as usize,
        );
        let kind = match kind {
            0 => ExceptionKind::TypedException(
                b.translate_runtime(eh_rts[k].as_ref().ok_or("catch clause without a type")?)?,
            ),
            2 => ExceptionKind::Finally,
            1 => ExceptionKind::Filter {
                offset: c[5] as usize,
            },
            4 => ExceptionKind::Fault,
            other => return Err(format!("exception clause kind {other}")),
        };
        clauses.push(Exception {
            kind,
            try_offset: try_start,
            try_length: try_end - try_start,
            handler_offset: handler_start,
            handler_length: handler_end - handler_start,
        });
    }

    let mut body = body::Method::with_locals(local_variables, instructions);
    body.header.maximum_stack_size = u16::try_from(max_stack)
        .map_err(|_| "invalid maxstack")?
        .into();
    if !clauses.is_empty() {
        body.data_sections
            .push(DataSection::ExceptionHandlers(clauses));
    }
    let signature = MethodSignature {
        instance: false,
        explicit_this: false,
        calling_convention: CallingConvention::Default,
        parameters,
        return_type,
        varargs: None,
    };
    b.res.push_method(
        holder_idx,
        Method::new(Accessibility::Public, signature, name, Some(body)),
    );

    let res = loader.register_dynamic_assembly(b.res);
    Ok(MethodDescription::new(
        TypeDescription::new(res.clone(), holder_idx),
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_assembly_ids_do_not_wrap() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(next_dynamic_assembly_id(&counter).unwrap(), u64::MAX - 1);
        assert!(next_dynamic_assembly_id(&counter).is_err());
        assert!(next_dynamic_assembly_id(&counter).is_err());
    }
}
