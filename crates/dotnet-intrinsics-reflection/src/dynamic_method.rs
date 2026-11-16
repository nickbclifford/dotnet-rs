//! Turn a recorded `DynamicMethod` body into a real method (plan 24; completed by plan 25).
//!
//! The managed shim (`DotnetRs.DynamicMethod` in the support assembly) records resolved
//! operations instead of IL bytes. This module converts that record into a one-method in-memory
//! assembly, registers it with the loader and returns the ordinary reflection `MethodInfo`, so
//! `Invoke`, `CreateDelegate`, frames, exception regions and stack traces are the regular paths.
//!
//! Current limits (each rejected with `NotSupportedException`, never silently wrong): non-generic
//! types and callees only, no nested types, no branches/labels, no fields, a fixed `maxstack`.

use crate::ReflectionIntrinsicHost;
use dotnet_assemblies::AssemblyLoader;
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{
    TypeDescription,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
    resolution::ResolutionS,
    runtime::RuntimeType,
};
use dotnet_value::object::ObjectRef;
use dotnet_vm_data::StepResult;
use dotnetdll::{
    binary::signature::kinds::CallingConvention,
    prelude::*,
    resolved::{
        body::{DataSection, Exception, ExceptionKind},
        members::{
            ExternalMethodReference, Method, MethodReferenceParent, MethodSource, UserMethod,
        },
        signature::{Parameter, ParameterType, ReturnType},
        types::{
            ExternalTypeReference, LocalVariable, ResolutionScope, TypeDefinition, TypeSource,
            UserType, ValueKind,
        },
    },
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_DYNAMIC_ASSEMBLY: AtomicUsize = AtomicUsize::new(0);

type R<T> = Result<T, String>;

fn prim(b: BaseType<MethodType>) -> MethodType {
    MethodType::Base(Box::new(b))
}

struct Builder<'a> {
    loader: &'a AssemblyLoader,
    res: Resolution<'static>,
    assembly_refs: HashMap<String, AssemblyRefIndex>,
    type_refs: HashMap<(String, Option<String>, String), TypeRefIndex>,
}

impl Builder<'_> {
    fn type_ref(&mut self, td: &TypeDescription) -> R<TypeRefIndex> {
        let def = td.definition();
        if def.encloser.is_some() {
            return Err(format!("nested type {}", td.type_name()));
        }
        if !def.generic_parameters.is_empty() {
            return Err(format!("generic type {}", td.type_name()));
        }
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
        let key = (asm_name.clone(), namespace.clone(), def.name.to_string());
        if let Some(idx) = self.type_refs.get(&key) {
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
        let idx = self.res.push_type_reference(ExternalTypeReference::new(
            namespace.map(Into::into),
            def.name.to_string(),
            ResolutionScope::Assembly(asm_idx),
        ));
        self.type_refs.insert(key, idx);
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
            return Err("generic parameter in a callee signature".into());
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
            RuntimeType::Vector(inner) => {
                let inner = self.translate_runtime(inner)?;
                prim(BaseType::Vector(vec![], inner))
            }
            other => return Err(format!("unsupported type {other:?}")),
        })
    }

    fn method_ref(&mut self, md: &MethodDescription) -> R<UserMethod> {
        let method = md.method();
        if !method.generic_parameters.is_empty() || !md.parent_generics.type_generics.is_empty() {
            return Err(format!("generic callee {}", method.name));
        }
        let parent = self.type_node(&md.parent)?;
        let owner = md.resolution();
        let sig = md.signature();
        let mut parameters = Vec::with_capacity(sig.parameters.len());
        for Parameter(_, p) in &sig.parameters {
            let ParameterType::Value(t) = p else {
                return Err(format!("by-ref parameter on callee {}", method.name));
            };
            parameters.push(Parameter(
                vec![],
                ParameterType::Value(self.translate_signature(&owner, t)?),
            ));
        }
        let return_type = match &sig.return_type {
            ReturnType(_, None) => ReturnType(vec![], None),
            ReturnType(_, Some(ParameterType::Value(t))) => ReturnType(
                vec![],
                Some(ParameterType::Value(self.translate_signature(&owner, t)?)),
            ),
            _ => return Err(format!("by-ref return on callee {}", method.name)),
        };
        let signature = MethodSignature {
            instance: sig.instance,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters,
            return_type,
            varargs: None,
        };
        let idx = self.res.push_method_reference(ExternalMethodReference::new(
            MethodReferenceParent::Type(parent),
            method.name.to_string(),
            signature,
        ));
        Ok(UserMethod::Reference(idx))
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
    Ok(match op {
        0x00 => I::NoOperation,
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
        0x2A => I::Return,
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
        0x65 => I::Negate,
        0x66 => I::Not,
        0x28 => I::Call {
            tail_call: false,
            param0: MethodSource::User(b.method_ref(&need_method()?)?),
        },
        0x6F => I::CallVirtual {
            skip_null_check: false,
            param0: MethodSource::User(b.method_ref(&need_method()?)?),
        },
        0x73 => I::NewObject(b.method_ref(&need_method()?)?),
        0x72 => match reference {
            Some(Operand::Str(s)) => I::load_string(s),
            _ => return Err("ldstr needs a string operand".into()),
        },
        0x74 => I::CastClass {
            skip_type_check: false,
            param0: b.translate_runtime(&need_type()?)?,
        },
        0x75 => I::IsInstance(b.translate_runtime(&need_type()?)?),
        0x7A => I::Throw,
        0x8C => I::BoxValue(b.translate_runtime(&need_type()?)?),
        0xA5 => I::UnboxIntoValue(b.translate_runtime(&need_type()?)?),
        0xDC => I::EndFinally,
        0xDD | 0xDE => I::Leave(imm as usize),
        0xFE01 => I::CompareEqual,
        0xFE02 => I::CompareGreater(NumberSign::Signed),
        0xFE03 => I::CompareGreater(NumberSign::Unsigned),
        0xFE04 => I::CompareLess(NumberSign::Signed),
        0xFE05 => I::CompareLess(NumberSign::Unsigned),
        other => {
            return Err(format!(
                "opcode 0x{other:04X} is not supported yet (plan 25)"
            ));
        }
    })
}

enum Operand {
    Type(RuntimeType),
    Method(MethodDescription),
    Str(String),
}

#[dotnet_intrinsic(
    "static System.Reflection.MethodInfo DotnetRs.RecordingILGenerator::CompileRecorded(string, System.Type, System.Type[], int[], long[], object[], System.Type[], int[], System.Type[])"
)]
pub fn compile_recorded<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let eh_types = ctx.pop_obj();
    let eh = ctx.pop_obj();
    let locals = ctx.pop_obj();
    let refs = ctx.pop_obj();
    let imm = ctx.pop_obj();
    let ops = ctx.pop_obj();
    let params = ctx.pop_obj();
    let ret = ctx.pop_obj();
    let name = ctx.pop_obj();

    match build(
        ctx,
        [name, ret, params, ops, imm, refs, locals, eh, eh_types],
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

fn build<'gc, T: ReflectionIntrinsicHost<'gc>>(
    ctx: &mut T,
    [name, ret, params, ops, imm, refs, locals, eh, eh_types]: [ObjectRef<'gc>; 9],
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
    let eh_rts = types_of(eh_types)?;
    let ops = read_i32s(ops)?;
    let imm = read_i64s(imm)?;
    let eh = read_i32s(eh)?;

    let mut operands = Vec::new();
    for r in object_refs(refs)? {
        operands.push(if r.0.is_none() {
            None
        } else if let Ok(s) = crate::types::string_from_heap_obj(r) {
            Some(Operand::Str(s))
        } else if let Ok((m, _)) = crate::common::resolve_runtime_method(ctx, r) {
            Some(Operand::Method(m))
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
    let id = NEXT_DYNAMIC_ASSEMBLY.fetch_add(1, Ordering::Relaxed);
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
        parameters.push(Parameter(
            vec![],
            ParameterType::Value(b.translate_runtime(p)?),
        ));
    }
    let return_type = if matches!(ret_rt, RuntimeType::Void) {
        ReturnType(vec![], None)
    } else {
        ReturnType(
            vec![],
            Some(ParameterType::Value(b.translate_runtime(&ret_rt)?)),
        )
    };

    let mut local_variables = Vec::new();
    for l in &local_rts {
        let l = l.as_ref().ok_or("null local type")?;
        local_variables.push(LocalVariable::Variable {
            custom_modifiers: vec![],
            pinned: false,
            by_ref: false,
            var_type: b.translate_runtime(l)?,
        });
    }

    let mut instructions = Vec::with_capacity(ops.len());
    for i in 0..ops.len() {
        instructions.push(instruction(
            &mut b,
            (ops[i] & 0xFFFF) as u16,
            imm[i],
            operands[i].as_ref(),
        )?);
    }

    let mut clauses = Vec::new();
    for (k, c) in eh.chunks_exact(5).enumerate() {
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
    // Shortcut until plan 25: the CLR computes maxstack from the IL; the recorder does not.
    body.header.maximum_stack_size = 32;
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

    let res = loader.register_owned_assembly(b.res);
    Ok(MethodDescription::new(
        TypeDescription::new(res.clone(), holder_idx),
        GenericLookup::default(),
        res,
        MethodMemberIndex::Method(0),
    ))
}
