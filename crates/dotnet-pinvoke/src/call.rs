use crate::{
    call_types::{
        AlignedReturnBuffer, TempBuffer, WriteBackSource, apply_write_backs,
        copy_value_type_return_data, ffi_cif_return_layout, validate_typed_return_abi,
    },
    loader::NativeLibraries,
    marshal::param_to_type,
};
use dotnet_types::{
    comparer::decompose_type_source,
    error::{ExecutionError, PInvokeError, TypeResolutionError},
    generics::ConcreteType,
    members::MethodDescription,
};
use dotnet_utils::{ByteOffset, gc::ThreadSafeReadGuard};
use dotnet_value::{
    StackValue,
    object::ObjectRef,
    pointer::{ManagedPtr, PointerOrigin},
};
use dotnet_vm_ops::{
    StepResult,
    ops::{PInvokeContext, ResolutionOps},
};
use dotnetdll::prelude::*;
use gc_arena::Gc;
use libffi::middle::*;
use std::{ffi::c_void, marker::PhantomPinned, ptr::NonNull, sync::Arc};

pub static mut LAST_ERROR: i32 = 0;

type ObjectReadGuard<'a, 'gc> = ThreadSafeReadGuard<'a, dotnet_value::object::ObjectInner<'gc>>;
struct PinnedGuard<'gc> {
    guard: ObjectReadGuard<'gc, 'gc>,
    _pin: PhantomPinned,
}

impl<'gc> PinnedGuard<'gc> {
    pub unsafe fn new(handle: dotnet_value::object::ObjectHandle<'gc>) -> Self {
        let guard = unsafe {
            let lock_ref: &'gc dotnet_utils::gc::ThreadSafeLock<
                dotnet_value::object::ObjectInner<'gc>,
            > = &*Gc::as_ptr(handle);
            lock_ref.borrow()
        };
        Self {
            guard,
            _pin: PhantomPinned,
        }
    }
}

pub fn external_call<'gc>(
    ctx: &mut dyn PInvokeContext<'gc>,
    method: MethodDescription,
    libraries: &NativeLibraries,
) -> StepResult {
    ctx.set_current_intrinsic(Some(method.clone()));
    let res = external_call_impl(ctx, method, libraries);
    ctx.set_current_intrinsic(None);
    res
}

fn resolve_parameter_base_type<'gc, T>(
    p_type: &ParameterType<MethodType>,
    res_ctx: &T,
    default: BaseType<ConcreteType>,
) -> Result<BaseType<ConcreteType>, TypeResolutionError>
where
    T: ResolutionOps<'gc> + ?Sized,
{
    if let ParameterType::Value(t) = p_type {
        Ok(res_ctx.make_concrete(t)?.get().clone())
    } else {
        Ok(default)
    }
}

#[derive(Clone, Copy)]
enum IntegerArgValue {
    Int32(i32),
    Int64(i64),
    NativeInt(isize),
}

impl IntegerArgValue {
    fn default_parameter_base_type(self) -> BaseType<ConcreteType> {
        match self {
            IntegerArgValue::Int32(_) => BaseType::IntPtr,
            IntegerArgValue::Int64(_) => BaseType::Int64,
            IntegerArgValue::NativeInt(_) => BaseType::IntPtr,
        }
    }

    fn raw_value(self) -> i128 {
        match self {
            IntegerArgValue::Int32(val) => i128::from(val),
            IntegerArgValue::Int64(val) => i128::from(val),
            IntegerArgValue::NativeInt(val) => val as i128,
        }
    }

    fn supports_i32_u32_narrowing(self) -> bool {
        !matches!(self, IntegerArgValue::Int32(_))
    }
}

#[derive(Clone, Copy)]
enum IntegerNarrowTarget {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
}

impl IntegerNarrowTarget {
    fn name(self) -> &'static str {
        match self {
            IntegerNarrowTarget::I8 => "i8",
            IntegerNarrowTarget::U8 => "u8",
            IntegerNarrowTarget::I16 => "i16",
            IntegerNarrowTarget::U16 => "u16",
            IntegerNarrowTarget::I32 => "i32",
            IntegerNarrowTarget::U32 => "u32",
        }
    }
}

fn checked_narrow_integer(
    value: IntegerArgValue,
    target: IntegerNarrowTarget,
) -> Result<Vec<u8>, ExecutionError> {
    let raw_value = value.raw_value();

    let bytes = match target {
        IntegerNarrowTarget::I8 => i8::try_from(raw_value)
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| {
                ExecutionError::InternalError(format!(
                    "P/Invoke marshalling: value {} out of range for {}",
                    raw_value,
                    target.name()
                ))
            })?,
        IntegerNarrowTarget::U8 => u8::try_from(raw_value)
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| {
                ExecutionError::InternalError(format!(
                    "P/Invoke marshalling: value {} out of range for {}",
                    raw_value,
                    target.name()
                ))
            })?,
        IntegerNarrowTarget::I16 => i16::try_from(raw_value)
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| {
                ExecutionError::InternalError(format!(
                    "P/Invoke marshalling: value {} out of range for {}",
                    raw_value,
                    target.name()
                ))
            })?,
        IntegerNarrowTarget::U16 => u16::try_from(raw_value)
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| {
                ExecutionError::InternalError(format!(
                    "P/Invoke marshalling: value {} out of range for {}",
                    raw_value,
                    target.name()
                ))
            })?,
        IntegerNarrowTarget::I32 => i32::try_from(raw_value)
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| {
                ExecutionError::InternalError(format!(
                    "P/Invoke marshalling: value {} out of range for {}",
                    raw_value,
                    target.name()
                ))
            })?,
        IntegerNarrowTarget::U32 => u32::try_from(raw_value)
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| {
                ExecutionError::InternalError(format!(
                    "P/Invoke marshalling: value {} out of range for {}",
                    raw_value,
                    target.name()
                ))
            })?,
    };

    Ok(bytes)
}

fn marshal_integer_arg<'gc>(
    value: IntegerArgValue,
    p_type: &ParameterType<MethodType>,
    ctx: &mut dyn PInvokeContext<'gc>,
    i: usize,
    temp_buffers: &mut Vec<TempBuffer>,
    arg_buffer_map: &mut [Option<usize>],
    arg_ptrs: &mut [*mut c_void],
) -> Result<(), StepResult> {
    let p_base_type = resolve_parameter_base_type(p_type, ctx, value.default_parameter_base_type())
        .map_err(|e| StepResult::Error(e.into()))?;

    let narrow_target = match p_base_type {
        BaseType::Int8 => Some(IntegerNarrowTarget::I8),
        BaseType::UInt8 | BaseType::Boolean => Some(IntegerNarrowTarget::U8),
        BaseType::Int16 => Some(IntegerNarrowTarget::I16),
        BaseType::UInt16 | BaseType::Char => Some(IntegerNarrowTarget::U16),
        BaseType::Int32 if value.supports_i32_u32_narrowing() => Some(IntegerNarrowTarget::I32),
        BaseType::UInt32 if value.supports_i32_u32_narrowing() => Some(IntegerNarrowTarget::U32),
        _ => None,
    };

    if let Some(target) = narrow_target {
        let bytes =
            checked_narrow_integer(value, target).map_err(|e| StepResult::Error(e.into()))?;
        temp_buffers.push(TempBuffer::Bytes(bytes));
        let buf_idx = temp_buffers.len() - 1;
        arg_buffer_map[i] = Some(buf_idx);
        arg_ptrs[i] = temp_buffers[buf_idx]
            .as_bytes()
            .map_err(|e| StepResult::Error(e.into()))?
            .as_ptr() as *mut c_void;
        return Ok(());
    }

    match value {
        IntegerArgValue::Int32(val) => {
            temp_buffers.push(TempBuffer::I32(Box::new(val)));
            let idx = temp_buffers.len() - 1;
            arg_buffer_map[i] = Some(idx);
            arg_ptrs[i] = temp_buffers[idx]
                .as_i32()
                .map_err(|e| StepResult::Error(e.into()))? as *const i32
                as *mut i32 as *mut c_void;
        }
        IntegerArgValue::Int64(val) => {
            temp_buffers.push(TempBuffer::I64(Box::new(val)));
            let idx = temp_buffers.len() - 1;
            arg_buffer_map[i] = Some(idx);
            arg_ptrs[i] = temp_buffers[idx]
                .as_i64()
                .map_err(|e| StepResult::Error(e.into()))? as *const i64
                as *mut i64 as *mut c_void;
        }
        IntegerArgValue::NativeInt(val) => {
            temp_buffers.push(TempBuffer::Isize(Box::new(val)));
            let idx = temp_buffers.len() - 1;
            arg_buffer_map[i] = Some(idx);
            arg_ptrs[i] = temp_buffers[idx]
                .as_isize()
                .map_err(|e| StepResult::Error(e.into()))? as *const isize
                as *mut isize as *mut c_void;
        }
    }

    Ok(())
}

struct PInvokeCallData<'a, 'gc> {
    cif: &'a Cif,
    target_fn: unsafe extern "C" fn(),
    arg_ptrs: &'a mut [*mut c_void],
    write_backs: &'a [(WriteBackSource<'gc>, usize, usize)],
    temp_buffers: &'a [TempBuffer],
}

fn invoke_ffi_call_with_write_backs<'gc>(
    ctx: &mut dyn PInvokeContext<'gc>,
    call_data: &mut PInvokeCallData<'_, 'gc>,
    ret_ptr: *mut c_void,
) -> Result<(), StepResult> {
    // SAFETY: `call_data.cif` is a prepared libffi CIF, `call_data.arg_ptrs` points to
    // marshalling-owned argument storage valid for the duration of the call, and `ret_ptr`
    // points to writable storage matching the return ABI (or is null for void returns).
    unsafe {
        libffi::raw::ffi_call(
            call_data.cif.as_raw_ptr(),
            Some(call_data.target_fn),
            ret_ptr,
            call_data.arg_ptrs.as_mut_ptr(),
        );
    }

    apply_write_backs(ctx, call_data.write_backs, call_data.temp_buffers)
        .map_err(|e| StepResult::Error(e.into()))
}

fn read_pinvoke_return<'gc, T>(
    ctx: &mut dyn PInvokeContext<'gc>,
    call_data: &mut PInvokeCallData<'_, 'gc>,
) -> Result<T, StepResult> {
    validate_typed_return_abi::<T>(call_data.cif).map_err(|e| StepResult::Error(e.into()))?;

    let mut ret = std::mem::MaybeUninit::<T>::uninit();
    invoke_ffi_call_with_write_backs(ctx, call_data, ret.as_mut_ptr() as *mut c_void)?;
    // SAFETY: `ffi_call` initialized the entire return slot after ABI validation.
    Ok(unsafe { ret.assume_init() })
}

fn handle_pinvoke_return<'gc>(
    ctx: &mut dyn PInvokeContext<'gc>,
    method: &MethodDescription,
    arg_count: usize,
    call_data: &mut PInvokeCallData<'_, 'gc>,
) -> StepResult {
    match &method.method().signature.return_type.1 {
        None => {
            if let Err(e) = invoke_ffi_call_with_write_backs(ctx, call_data, std::ptr::null_mut()) {
                return e;
            }
            let _ = ctx.pop_multiple(arg_count);
        }
        Some(ParameterType::Value(t)) => {
            let t = match ctx.make_concrete(t) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            };

            let v = match t.get() {
                BaseType::Boolean => match read_pinvoke_return::<u8>(ctx, call_data) {
                    Ok(v) => StackValue::Int32(v as i32),
                    Err(e) => return e,
                },
                BaseType::Char => match read_pinvoke_return::<u16>(ctx, call_data) {
                    Ok(v) => StackValue::Int32(v as i32),
                    Err(e) => return e,
                },
                BaseType::Int8 => match read_pinvoke_return::<i8>(ctx, call_data) {
                    Ok(v) => StackValue::Int32(v as i32),
                    Err(e) => return e,
                },
                BaseType::UInt8 => match read_pinvoke_return::<u8>(ctx, call_data) {
                    Ok(v) => StackValue::Int32(v as i32),
                    Err(e) => return e,
                },
                BaseType::Int16 => match read_pinvoke_return::<i16>(ctx, call_data) {
                    Ok(v) => StackValue::Int32(v as i32),
                    Err(e) => return e,
                },
                BaseType::UInt16 => match read_pinvoke_return::<u16>(ctx, call_data) {
                    Ok(v) => StackValue::Int32(v as i32),
                    Err(e) => return e,
                },
                BaseType::Int32 => match read_pinvoke_return::<i32>(ctx, call_data) {
                    Ok(v) => StackValue::Int32(v),
                    Err(e) => return e,
                },
                BaseType::UInt32 => match read_pinvoke_return::<u32>(ctx, call_data) {
                    Ok(v) => StackValue::Int32(v as i32),
                    Err(e) => return e,
                },
                BaseType::Int64 => match read_pinvoke_return::<i64>(ctx, call_data) {
                    Ok(v) => StackValue::Int64(v),
                    Err(e) => return e,
                },
                BaseType::UInt64 => match read_pinvoke_return::<u64>(ctx, call_data) {
                    Ok(v) => StackValue::Int64(v as i64),
                    Err(e) => return e,
                },
                BaseType::Float32 => match read_pinvoke_return::<f32>(ctx, call_data) {
                    Ok(v) => StackValue::NativeFloat(v as f64),
                    Err(e) => return e,
                },
                BaseType::Float64 => match read_pinvoke_return::<f64>(ctx, call_data) {
                    Ok(v) => StackValue::NativeFloat(v),
                    Err(e) => return e,
                },
                BaseType::IntPtr => match read_pinvoke_return::<isize>(ctx, call_data) {
                    Ok(v) => StackValue::NativeInt(v),
                    Err(e) => return e,
                },
                BaseType::UIntPtr => match read_pinvoke_return::<usize>(ctx, call_data) {
                    Ok(v) => StackValue::NativeInt(v as isize),
                    Err(e) => return e,
                },
                BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => {
                    match read_pinvoke_return::<*mut u8>(ctx, call_data) {
                        Ok(v) => StackValue::unmanaged_ptr(v),
                        Err(e) => return e,
                    }
                }
                BaseType::Type {
                    value_kind: Some(ValueKind::ValueType),
                    source,
                } => {
                    let (_, _type_generics) = decompose_type_source::<ConcreteType>(source);

                    let concrete = ConcreteType::new(
                        t.resolution(),
                        BaseType::Type {
                            source: source.clone(),
                            value_kind: Some(ValueKind::ValueType),
                        },
                    );
                    let td = ctx
                        .loader()
                        .find_concrete_type(concrete)
                        .expect("Failed to resolve type in pinvoke interop");

                    let instance = match ctx.new_object(td) {
                        Ok(inst) => inst,
                        Err(e) => return StepResult::Error(e.into()),
                    };

                    let (ffi_size, ffi_align) = match ffi_cif_return_layout(call_data.cif) {
                        Ok(v) => v,
                        Err(e) => return StepResult::Error(e.into()),
                    };
                    let mut temp_buffer = match AlignedReturnBuffer::new_zeroed(ffi_size, ffi_align)
                    {
                        Ok(v) => v,
                        Err(e) => return StepResult::Error(e.into()),
                    };

                    if let Err(e) = invoke_ffi_call_with_write_backs(
                        ctx,
                        call_data,
                        temp_buffer.as_mut_ptr() as *mut c_void,
                    ) {
                        return e;
                    }

                    instance.instance_storage.with_data_mut(|guard| {
                        copy_value_type_return_data(guard, temp_buffer.as_bytes());
                    });

                    StackValue::ValueType(instance)
                }
                BaseType::Type { .. }
                | BaseType::Array { .. }
                | BaseType::Vector { .. }
                | BaseType::Object
                | BaseType::String => match read_pinvoke_return::<*mut u8>(ctx, call_data) {
                    Ok(v) => StackValue::unmanaged_ptr(v),
                    Err(e) => return e,
                },
            };
            let _ = ctx.pop_multiple(arg_count);
            ctx.push(v);
        }
        Some(ParameterType::Ref(t)) => {
            let ptr = match read_pinvoke_return::<*mut u8>(ctx, call_data) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let concrete = match ctx.make_concrete(t) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            };
            let td = ctx
                .loader()
                .find_concrete_type(concrete)
                .expect("failed to resolve return type");
            let _ = ctx.pop_multiple(arg_count);
            ctx.push_managed_ptr(ManagedPtr::new(NonNull::new(ptr), td, None, false, None));
        }
        Some(ParameterType::TypedReference) => {
            let ret = match read_pinvoke_return::<[usize; 2]>(ctx, call_data) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let addr = ret[0];
            let type_ptr = ret[1] as *const dotnet_types::TypeDescription;
            if type_ptr.is_null() {
                return StepResult::Error(
                    ExecutionError::InternalError(
                        "null type handle in returned TypedReference".to_string(),
                    )
                    .into(),
                );
            }
            let type_desc = unsafe {
                let arc = Arc::from_raw(type_ptr);
                let clone = arc.clone();
                let _ = Arc::into_raw(arc);
                clone
            };
            let m = ManagedPtr::new(
                NonNull::new(addr as *mut u8),
                (*type_desc).clone(),
                None,
                false,
                Some(ByteOffset(0)),
            );
            let _ = ctx.pop_multiple(arg_count);
            ctx.push(StackValue::TypedRef(m.into(), type_desc));
        }
    }

    StepResult::Continue
}

fn external_call_impl<'gc>(
    ctx: &mut dyn PInvokeContext<'gc>,
    method: MethodDescription,
    libraries: &NativeLibraries,
) -> StepResult {
    let Some(p) = &method.method().pinvoke else {
        unreachable!()
    };

    let mut pinned_objects: Vec<ObjectRef<'gc>> = Vec::new();
    let mut local_guards: Vec<PinnedGuard<'gc>> = Vec::new();
    #[cfg(feature = "multithreading")]
    let mut cross_arena_guards = Vec::new();

    let arg_count = method.method().signature.parameters.len();
    let stack_values = ctx.peek_multiple(arg_count);

    let res = method.resolution();
    let module = if !res.is_null() {
        res.definition()[p.import_scope].name.as_ref()
    } else {
        "UNKNOWN_MODULE"
    };
    let function = p.import_name.as_ref();
    let type_name = method.parent.type_name();

    if ctx.tracer_enabled() {
        ctx.tracer().trace_interop(
            ctx.indent(),
            "CALL",
            &format!(
                "Invoking P/Invoke: [{}] function [{}] in type [{}]",
                module, function, type_name
            ),
        );
    }

    let target_res = if ctx.tracer_enabled() {
        let guard = ctx.tracer();
        libraries.get_function(module, function, Some(guard))
    } else {
        libraries.get_function(module, function, None)
    };

    let target = match target_res {
        Ok(t) => t,
        Err(e) => {
            let (exc_name, msg) = match e {
                PInvokeError::LibraryNotFound(lib) => (
                    "System.DllNotFoundException",
                    format!("Unable to load DLL '{}' or one of its dependencies.", lib),
                ),
                PInvokeError::SymbolNotFound(lib, sym) => (
                    "System.EntryPointNotFoundException",
                    format!(
                        "Unable to find an entry point named '{}' in DLL '{}'.",
                        sym, lib
                    ),
                ),
                PInvokeError::LoadError(lib, err) => (
                    "System.DllNotFoundException",
                    format!("Unable to load DLL '{}': {}", lib, err),
                ),
            };

            let res = ctx.throw_by_name_with_message(exc_name, msg.as_str());
            if res == StepResult::Exception {
                let _ = ctx.pop_multiple(arg_count);
            }
            return res;
        }
    };

    let mut args: Vec<Type> = vec![];
    {
        for Parameter(_, p) in &method.method().signature.parameters {
            args.push(match param_to_type(p, ctx) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            });
        }
    }

    let return_type = match &method.method().signature.return_type.1 {
        None => Type::void(),
        Some(s) => match param_to_type(s, ctx) {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e.into()),
        },
    };
    let mut temp_buffers: Vec<TempBuffer> = vec![];
    let mut write_backs: Vec<(WriteBackSource<'gc>, usize, usize)> = vec![];
    let mut arg_buffer_map: Vec<Option<usize>> = vec![None; stack_values.len()];
    let mut arg_ptrs: Vec<*mut c_void> = vec![std::ptr::null_mut(); stack_values.len()];

    // Pass 1: Prepare buffers
    for (i, (v, Parameter(_, p_type))) in stack_values
        .iter()
        .zip(&method.method().signature.parameters)
        .enumerate()
    {
        let ffi_size = unsafe { (*args[i].as_raw_ptr()).size };
        match v {
            StackValue::Int32(val) => {
                if let Err(e) = marshal_integer_arg(
                    IntegerArgValue::Int32(*val),
                    p_type,
                    ctx,
                    i,
                    &mut temp_buffers,
                    &mut arg_buffer_map,
                    &mut arg_ptrs,
                ) {
                    return e;
                }
            }
            StackValue::Int64(val) => {
                if let Err(e) = marshal_integer_arg(
                    IntegerArgValue::Int64(*val),
                    p_type,
                    ctx,
                    i,
                    &mut temp_buffers,
                    &mut arg_buffer_map,
                    &mut arg_ptrs,
                ) {
                    return e;
                }
            }
            StackValue::NativeInt(val) => {
                if let Err(e) = marshal_integer_arg(
                    IntegerArgValue::NativeInt(*val),
                    p_type,
                    ctx,
                    i,
                    &mut temp_buffers,
                    &mut arg_buffer_map,
                    &mut arg_ptrs,
                ) {
                    return e;
                }
            }
            StackValue::NativeFloat(val) => {
                temp_buffers.push(TempBuffer::F64(Box::new(*val)));
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] = match temp_buffers[idx].as_f64() {
                    Ok(v) => v as *const f64 as *mut f64 as *mut c_void,
                    Err(e) => return StepResult::Error(e.into()),
                };
            }
            StackValue::UnmanagedPtr(val) => {
                temp_buffers.push(TempBuffer::Ptr(Box::new(val.0.as_ptr())));
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] = match temp_buffers[idx].as_ptr() {
                    Ok(v) => v as *const *mut u8 as *mut *mut u8 as *mut c_void,
                    Err(e) => return StepResult::Error(e.into()),
                };
            }
            StackValue::ValueType(o) => {
                let mut data = o.with_data(|d: &[u8]| d.to_vec());
                if data.len() < ffi_size {
                    data.resize(ffi_size, 0);
                }

                if data.is_empty() {
                    temp_buffers.push(TempBuffer::Bytes(vec![0]));
                } else {
                    temp_buffers.push(TempBuffer::Bytes(data));
                }
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] = match temp_buffers[idx].as_bytes() {
                    Ok(buf) => buf.as_ptr() as *mut c_void,
                    Err(e) => return StepResult::Error(e.into()),
                };
            }
            StackValue::ObjectRef(obj) => {
                let ptr = if let Some(h) = obj.0 {
                    ctx.pin_object(*obj);
                    pinned_objects.push(*obj);
                    // SAFETY: `h` is pinned above for the full call duration, so the underlying
                    // object storage address remains stable while libffi uses this pointer.
                    let guard = unsafe { PinnedGuard::new(h) };
                    // SAFETY: The pinned guard guarantees stable backing storage and a valid data
                    // pointer for the lifetime of `guard`.
                    let ptr = unsafe { guard.guard.storage.raw_data_ptr() };
                    local_guards.push(guard);
                    ptr
                } else {
                    std::ptr::null_mut()
                };
                temp_buffers.push(TempBuffer::Ptr(Box::new(ptr)));
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] = match temp_buffers[idx].as_ptr() {
                    Ok(v) => v as *const *mut u8 as *mut *mut u8 as *mut c_void,
                    Err(e) => return StepResult::Error(e.into()),
                };
            }
            StackValue::TypedRef(p, t) => {
                if let Some(owner) = p.owner() {
                    ctx.pin_object(owner);
                    pinned_objects.push(owner);
                }
                let mut bytes = ManagedPtr::serialization_buffer();
                // SAFETY: For heap-backed pointers we pin and keep the guard alive in
                // `local_guards` before taking/offsetting raw addresses; for non-heap pointers we
                // only expose addresses already represented by `ManagedPtr`.
                let addr = unsafe {
                    if let PointerOrigin::Heap(obj) = p.origin() {
                        if let Some(h) = obj.0 {
                            let guard = PinnedGuard::new(h);
                            let ptr = guard
                                .guard
                                .storage
                                .raw_data_ptr()
                                .add(p.byte_offset().as_usize());
                            local_guards.push(guard);
                            sptr::Strict::expose_addr(ptr)
                        } else {
                            p.byte_offset().as_usize()
                        }
                    } else {
                        p.with_data(0, |data| sptr::Strict::expose_addr(data.as_ptr()))
                    }
                };
                let type_ptr = sptr::Strict::expose_addr(Arc::as_ptr(t));
                bytes[0..ObjectRef::SIZE].copy_from_slice(&addr.to_ne_bytes());
                bytes[ObjectRef::SIZE..ManagedPtr::SIZE].copy_from_slice(&type_ptr.to_ne_bytes());

                temp_buffers.push(TempBuffer::Bytes(bytes.to_vec()));
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] = match temp_buffers[idx].as_bytes() {
                    Ok(buf) => buf.as_ptr() as *mut c_void,
                    Err(e) => return StepResult::Error(e.into()),
                };
            }
            StackValue::ManagedPtr(p) => {
                let buf_len = ffi_size;
                let mut buf = vec![0u8; buf_len];
                if buf.is_empty() {
                    buf.push(0);
                }

                let is_ref = matches!(p_type, ParameterType::Ref(_));

                if is_ref {
                    // SAFETY: `p.with_data` validates access against the managed origin and we copy
                    // at most `buf_len` bytes into an owned buffer of the same length.
                    unsafe {
                        p.with_data(buf_len, |data| {
                            let to_copy = std::cmp::min(buf_len, data.len());
                            std::ptr::copy_nonoverlapping(data.as_ptr(), buf.as_mut_ptr(), to_copy);
                        });
                    }
                    temp_buffers.push(TempBuffer::Bytes(buf));
                    let buf_idx = temp_buffers.len() - 1;
                    arg_buffer_map[i] = Some(buf_idx);

                    if let Some(owner) = p.owner() {
                        ctx.pin_object(owner);
                        pinned_objects.push(owner);
                    }

                    write_backs.push((
                        WriteBackSource::Managed(p.origin().clone(), p.byte_offset()),
                        buf_idx,
                        buf_len,
                    ));

                    arg_ptrs[i] = match temp_buffers[buf_idx].as_bytes() {
                        Ok(buf) => buf.as_ptr() as *mut c_void,
                        Err(e) => return StepResult::Error(e.into()),
                    };
                } else {
                    if let Some(owner) = p.owner() {
                        ctx.pin_object(owner);
                        pinned_objects.push(owner);
                    }

                    // SAFETY: Heap pointers are pinned and their guards are kept alive in
                    // `local_guards`; non-heap pointers come from `ManagedPtr` origin data and are
                    // forwarded as-is for the duration of this call.
                    let ptr = unsafe {
                        if let PointerOrigin::Heap(obj) = p.origin() {
                            if let Some(h) = obj.0 {
                                let guard = PinnedGuard::new(h);
                                let ptr = guard
                                    .guard
                                    .storage
                                    .raw_data_ptr()
                                    .add(p.byte_offset().as_usize());
                                local_guards.push(guard);
                                ptr
                            } else {
                                sptr::from_exposed_addr_mut::<u8>(p.byte_offset().as_usize())
                            }
                        } else {
                            p.with_data(0, |data| data.as_ptr().cast_mut())
                        }
                    };

                    temp_buffers.push(TempBuffer::Ptr(Box::new(ptr)));
                    let idx = temp_buffers.len() - 1;
                    arg_buffer_map[i] = Some(idx);
                    arg_ptrs[i] = match temp_buffers[idx].as_ptr() {
                        Ok(v) => v as *const *mut u8 as *mut *mut u8 as *mut c_void,
                        Err(e) => return StepResult::Error(e.into()),
                    };
                }
            }
            #[cfg(feature = "multithreading")]
            StackValue::CrossArenaObjectRef(ptr, _) => {
                // SAFETY: `ptr` is an owned non-null handle to the cross-arena lock cell created
                // by the VM; taking a shared reference here does not outlive the handle.
                let lock = unsafe { &*ptr.as_ptr() };
                let guard = lock.borrow();
                // SAFETY: The borrow guard keeps the object storage alive and immobile while this
                // pointer is used by libffi.
                let p = unsafe { guard.storage.raw_data_ptr() };
                cross_arena_guards.push(guard);
                temp_buffers.push(TempBuffer::Ptr(Box::new(p)));
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] = match temp_buffers[idx].as_ptr() {
                    Ok(v) => v as *const *mut u8 as *mut *mut u8 as *mut c_void,
                    Err(e) => return StepResult::Error(e.into()),
                };
            }
        }
    }

    let cif = Cif::new(args, return_type.clone());

    let target_fn = *target.as_fun();

    if ctx.tracer_enabled() {
        ctx.tracer().trace_interop(
            ctx.indent(),
            "CALLING",
            &format!("{}::{} with {} args", module, function, arg_ptrs.len()),
        );
    }

    let mut call_data = PInvokeCallData {
        cif: &cif,
        target_fn,
        arg_ptrs: arg_ptrs.as_mut_slice(),
        write_backs: &write_backs,
        temp_buffers: &temp_buffers,
    };

    let call_result = handle_pinvoke_return(ctx, &method, arg_count, &mut call_data);

    for obj in pinned_objects {
        ctx.unpin_object(obj);
    }

    call_result
}

#[cfg(test)]
use crate::call_types::apply_write_backs_with;

#[cfg(test)]
mod tests {
    use super::*;
    use dotnet_types::error::MemoryAccessError;

    #[test]
    fn by_ref_write_back_propagates_managed_write_failures() {
        let temp_buffers = vec![TempBuffer::Bytes(vec![1, 2, 3, 4])];
        let write_backs = vec![(
            WriteBackSource::Managed(PointerOrigin::Unmanaged, ByteOffset(0)),
            0usize,
            4usize,
        )];

        let err = apply_write_backs_with(&write_backs, &temp_buffers, |_origin, _offset, _data| {
            Err(MemoryAccessError::InvalidOrigin)
        })
        .expect_err("managed write-back failure should be surfaced");

        assert_eq!(err, MemoryAccessError::InvalidOrigin);
    }

    #[test]
    fn by_ref_write_back_raw_destination_copies_bytes() {
        let temp_buffers = vec![TempBuffer::Bytes(vec![9, 8, 7, 6])];
        let mut destination = vec![0u8; 4];
        let dest_ptr = NonNull::new(destination.as_mut_ptr()).expect("destination ptr is non-null");
        let write_backs = vec![(WriteBackSource::Raw(dest_ptr), 0usize, 4usize)];

        apply_write_backs_with(
            &write_backs,
            &temp_buffers,
            |_origin, _offset, _data| Ok(()),
        )
        .expect("raw write-back copy should succeed");

        assert_eq!(destination, vec![9, 8, 7, 6]);
    }

    #[test]
    fn value_type_return_buffer_honors_alignment_and_zero_init() {
        let mut buffer =
            AlignedReturnBuffer::new_zeroed(32, 16).expect("buffer allocation should succeed");
        assert_eq!((buffer.as_mut_ptr() as usize) % 16, 0);
        assert!(buffer.as_bytes().iter().all(|b| *b == 0));
    }

    #[test]
    fn value_type_return_copy_truncates_to_destination_size() {
        let mut destination = [0u8; 4];
        let source = [1u8, 2, 3, 4, 5, 6];
        copy_value_type_return_data(&mut destination, &source);
        assert_eq!(destination, [1, 2, 3, 4]);
    }

    #[test]
    fn typed_return_abi_mismatch_is_rejected() {
        let cif = Cif::new(Vec::new(), Type::u64());
        let err = validate_typed_return_abi::<u32>(&cif)
            .expect_err("mismatched ffi/rust return ABI should fail");
        assert!(matches!(err, ExecutionError::InternalError(_)));
    }
}
