use crate::{
    StepResult, context::ResolutionContext, error::ExecutionError, layout::LayoutFactory,
    resolution::ValueResolution, stack::ops::VesOps, tracer::Tracer,
};
use dashmap::DashMap;
use dotnet_types::{
    comparer::decompose_type_source,
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_utils::{ByteOffset, gc::ThreadSafeReadGuard};
use dotnet_value::{
    StackValue,
    layout::{FieldLayoutManager, LayoutManager, Scalar},
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin},
    string::CLRString,
};
use dotnetdll::prelude::*;
use gc_arena::{Collect, Gc, unsafe_empty_collect};
use libffi::middle::*;
use libloading::{Library, Symbol};
use sptr::Strict;
use std::{ffi::c_void, marker::PhantomPinned, path::PathBuf, ptr::NonNull, sync::Arc};

pub static mut LAST_ERROR: i32 = 0;

#[derive(Debug)]
pub enum PInvokeError {
    LibraryNotFound(String),
    SymbolNotFound(String, String),
    LoadError(String, String),
}

impl std::fmt::Display for PInvokeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PInvokeError::LibraryNotFound(name) => write!(f, "Unable to find library '{}'", name),
            PInvokeError::SymbolNotFound(lib, sym) => write!(
                f,
                "Unable to find entry point '{}' in library '{}'",
                sym, lib
            ),
            PInvokeError::LoadError(name, err) => {
                write!(f, "Failed to load library '{}': {}", name, err)
            }
        }
    }
}

pub trait PInvokeSandbox: Send + Sync {
    fn allow_library(&self, name: &str) -> bool;
    fn allow_function(&self, lib: &str, func: &str) -> bool;
}

pub struct DefaultSandbox;
impl PInvokeSandbox for DefaultSandbox {
    fn allow_library(&self, _name: &str) -> bool {
        true
    }
    fn allow_function(&self, _lib: &str, _func: &str) -> bool {
        true
    }
}

#[cfg(feature = "fuzzing")]
pub struct DenySandbox;
#[cfg(feature = "fuzzing")]
impl PInvokeSandbox for DenySandbox {
    fn allow_library(&self, _name: &str) -> bool {
        false
    }
    fn allow_function(&self, _lib: &str, _func: &str) -> bool {
        false
    }
}

pub struct NativeLibraries {
    root: PathBuf,
    libraries: DashMap<String, Library>,
    sandbox: Arc<dyn PInvokeSandbox>,
}
unsafe_empty_collect!(NativeLibraries);
impl NativeLibraries {
    pub fn new(root: impl AsRef<str>) -> Self {
        Self {
            root: PathBuf::from(root.as_ref()),
            libraries: DashMap::new(),
            sandbox: Arc::new(DefaultSandbox),
        }
    }

    pub fn with_sandbox(mut self, sandbox: Arc<dyn PInvokeSandbox>) -> Self {
        self.sandbox = sandbox;
        self
    }

    fn find_library_path(&self, name: &str) -> Option<PathBuf> {
        let exact = self.root.join(name);
        if exact.exists() {
            return Some(exact);
        }

        // Try with platform extension
        #[cfg(target_os = "linux")]
        let extensions = &[".so", ".dylib", ".dll"];
        #[cfg(target_os = "macos")]
        let extensions = &[".dylib", ".so", ".dll"];
        #[cfg(target_os = "windows")]
        let extensions = &[".dll", ".so", ".dylib"];
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        let extensions = &[".so", ".dll", ".dylib"];

        for ext in extensions {
            let path = self.root.join(format!("{}{}", name, ext));
            if path.exists() {
                return Some(path);
            }
        }

        // Versioned search
        if let Ok(entries) = self.root.read_dir() {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                let file_name = entry.file_name();
                let s = file_name.to_string_lossy();

                if s.starts_with(name) && (s.contains(".so.") || s.contains(".dylib.")) {
                    return Some(path);
                }
            }
        }

        None
    }

    pub fn get_library(
        &self,
        name: &str,
        mut tracer: Option<&mut Tracer>,
    ) -> Result<dashmap::mapref::one::Ref<'_, String, Library>, PInvokeError> {
        if !self.sandbox.allow_library(name) {
            return Err(PInvokeError::LibraryNotFound(name.to_string()));
        }

        if let Some(lib) = self.libraries.get(name) {
            return Ok(lib);
        }

        let path = self.find_library_path(name);

        if let Some(t) = &mut tracer {
            t.trace_interop(0, "RESOLVE", &format!("Resolving library '{}'...", name));
            t.trace_interop(
                0,
                "RESOLVE",
                &if let Some(p) = &path {
                    format!("Library '{}' found at '{}', now loading", name, p.display())
                } else {
                    format!("Library '{}' not found in root, trying system paths", name)
                },
            );
        }

        let mut names_to_try = vec![];
        if let Some(p) = path {
            names_to_try.push(p.to_string_lossy().to_string());
        } else {
            names_to_try.push(name.to_string());
            #[cfg(target_os = "linux")]
            {
                if name == "libc" {
                    names_to_try.push("libc.so.6".to_string());
                } else if name == "libm" {
                    names_to_try.push("libm.so.6".to_string());
                } else if name == "libdl" {
                    names_to_try.push("libdl.so.2".to_string());
                } else if name == "libpthread" {
                    names_to_try.push("libpthread.so.0".to_string());
                }
            }
        }

        let mut lib = None;
        let mut last_error = None;

        for n in &names_to_try {
            match unsafe { Library::new(n) } {
                Ok(l) => {
                    lib = Some(l);
                    break;
                }
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }

        let lib = lib.ok_or_else(|| {
            PInvokeError::LoadError(
                name.to_string(),
                last_error
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "Unknown error".to_string()),
            )
        })?;

        if let Some(t) = tracer {
            t.trace_interop(0, "RESOLVE", &format!("Successfully loaded '{}'", name));
        }
        self.libraries.entry(name.to_string()).or_insert(lib);
        Ok(self.libraries.get(name).unwrap())
    }

    pub fn get_function(
        &self,
        library: &str,
        name: &str,
        tracer: Option<&mut Tracer>,
    ) -> Result<CodePtr, PInvokeError> {
        if !self.sandbox.allow_function(library, name) {
            return Err(PInvokeError::SymbolNotFound(
                library.to_string(),
                name.to_string(),
            ));
        }
        let l = self.get_library(library, tracer)?;
        let sym: Symbol<unsafe extern "C" fn()> = unsafe { l.get(name.as_bytes()) }
            .map_err(|_| PInvokeError::SymbolNotFound(library.to_string(), name.to_string()))?;
        Ok(CodePtr::from_fun(*sym))
    }
}

fn type_to_layout(t: &TypeSource<ConcreteType>, ctx: &ResolutionContext) -> FieldLayoutManager {
    let (ut, type_generics) = decompose_type_source::<ConcreteType>(t);
    let new_lookup = GenericLookup::new(type_generics);
    let new_ctx = ctx.with_generics(&new_lookup);
    let td = new_ctx
        .locate_type(ut)
        .expect("failed to locate type in pinvoke");

    if td.is_null() {
        panic!(
            "P/Invoke marshalling error: Could not resolve type {:?} when calculating layout.",
            ut
        );
    }

    LayoutFactory::instance_fields(td, &new_ctx).expect("P/Invoke layout resolution failed")
}

fn layout_to_ffi(l: &LayoutManager) -> Type {
    match l {
        LayoutManager::Field(f) => {
            let mut fields: Vec<_> = f.fields.values().collect();
            fields.sort_by_key(|f| f.position);

            Type::structure(fields.into_iter().map(|f| layout_to_ffi(&f.layout)))
        }
        LayoutManager::Array(a) => {
            let elem_type = layout_to_ffi(&a.element_layout);
            Type::structure(std::iter::repeat_n(elem_type, a.length))
        }
        LayoutManager::Scalar(s) => match s {
            Scalar::Int8 => Type::i8(),
            Scalar::UInt8 => Type::u8(),
            Scalar::Int16 => Type::i16(),
            Scalar::UInt16 => Type::u16(),
            Scalar::Int32 => Type::i32(),
            Scalar::Int64 => Type::i64(),
            Scalar::ObjectRef => Type::pointer(),
            Scalar::NativeInt => Type::isize(),
            Scalar::Float32 => Type::f32(),
            Scalar::Float64 => Type::f64(),
            Scalar::ManagedPtr => Type::pointer(),
        },
    }
}

fn type_to_ffi(t: &ConcreteType, ctx: &ResolutionContext) -> Type {
    match t.get() {
        BaseType::Boolean => Type::u8(),
        BaseType::Char => Type::u16(),
        BaseType::Int8 => Type::i8(),
        BaseType::UInt8 => Type::u8(),
        BaseType::Int16 => Type::i16(),
        BaseType::UInt16 => Type::u16(),
        BaseType::Int32 => Type::i32(),
        BaseType::UInt32 => Type::u32(),
        BaseType::Int64 => Type::i64(),
        BaseType::UInt64 => Type::u64(),
        BaseType::Float32 => Type::f32(),
        BaseType::Float64 => Type::f64(),
        BaseType::IntPtr => Type::isize(),
        BaseType::UIntPtr => Type::usize(),
        BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => Type::pointer(),
        BaseType::Type {
            value_kind: Some(ValueKind::ValueType),
            source,
        } => {
            let type_generics = match source {
                TypeSource::Generic { parameters, .. } => parameters.clone(),
                _ => vec![],
            };
            let new_lookup = GenericLookup::new(type_generics);
            let new_ctx = ctx.with_generics(&new_lookup);

            let layout = type_to_layout(source, &new_ctx);
            layout_to_ffi(&layout.into())
        }
        BaseType::Type {
            value_kind: None | Some(ValueKind::Class),
            ..
        } => Type::pointer(),
        BaseType::Array { .. } | BaseType::Vector { .. } | BaseType::String | BaseType::Object => {
            Type::pointer()
        }
    }
}

fn param_to_type(
    p: &ParameterType<MethodType>,
    ctx: &ResolutionContext,
) -> Result<Type, dotnet_types::error::TypeResolutionError> {
    match p {
        ParameterType::Value(t) => Ok(type_to_ffi(&ctx.make_concrete(t)?, ctx)),
        ParameterType::Ref(_) => Ok(Type::pointer()),
        ParameterType::TypedReference => {
            Ok(Type::structure(vec![Type::pointer(), Type::pointer()]))
        }
    }
}

enum WriteBackSource<'gc> {
    Managed(PointerOrigin<'gc>, ByteOffset),
    #[allow(dead_code)]
    Raw(NonNull<u8>),
}

enum TempBuffer {
    I32(Box<i32>),
    I64(Box<i64>),
    Isize(Box<isize>),
    F64(Box<f64>),
    Ptr(Box<*mut u8>),
    Bytes(Vec<u8>),
}

impl TempBuffer {
    fn as_i32(&self) -> &i32 {
        match self {
            TempBuffer::I32(val) => val,
            _ => panic!("P/Invoke temp buffer type mismatch (i32)"),
        }
    }

    fn as_i64(&self) -> &i64 {
        match self {
            TempBuffer::I64(val) => val,
            _ => panic!("P/Invoke temp buffer type mismatch (i64)"),
        }
    }

    fn as_isize(&self) -> &isize {
        match self {
            TempBuffer::Isize(val) => val,
            _ => panic!("P/Invoke temp buffer type mismatch (isize)"),
        }
    }

    fn as_f64(&self) -> &f64 {
        match self {
            TempBuffer::F64(val) => val,
            _ => panic!("P/Invoke temp buffer type mismatch (f64)"),
        }
    }

    fn as_ptr(&self) -> &*mut u8 {
        match self {
            TempBuffer::Ptr(val) => val,
            _ => panic!("P/Invoke temp buffer type mismatch (ptr)"),
        }
    }

    fn as_bytes(&self) -> &[u8] {
        match self {
            TempBuffer::Bytes(buf) => buf,
            _ => panic!("P/Invoke temp buffer type mismatch (bytes)"),
        }
    }
}

type ObjectReadGuard<'a, 'gc> = ThreadSafeReadGuard<'a, dotnet_value::object::ObjectInner<'gc>>;
struct PinnedGuard<'gc> {
    guard: ObjectReadGuard<'gc, 'gc>,
    _pin: PhantomPinned,
}

impl<'gc> PinnedGuard<'gc> {
    pub unsafe fn new(handle: dotnet_value::object::ObjectHandle<'gc>) -> Self {
        // SAFETY: The caller must ensure that the object is pinned and will stay reachable.
        // We use Gc::as_ptr to obtain a stable address and cast it to a reference with 'gc lifetime.
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

pub fn external_call<'ctx, 'gc, 'm: 'gc>(
    ctx: &'ctx mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
) -> StepResult {
    ctx.set_current_intrinsic(Some(method));
    let res = external_call_impl(ctx, method);
    ctx.set_current_intrinsic(None);
    res
}

fn external_call_impl<'ctx, 'gc, 'm: 'gc>(
    ctx: &'ctx mut dyn VesOps<'gc, 'm>,
    method: MethodDescription,
) -> StepResult {
    let Some(p) = &method.method.pinvoke else {
        unreachable!()
    };

    let mut pinned_objects: Vec<ObjectRef<'gc>> = Vec::new();
    let mut local_guards: Vec<PinnedGuard<'gc>> = Vec::new();
    #[cfg(feature = "multithreaded-gc")]
    let mut cross_arena_guards = Vec::new();

    let arg_count = method.method.signature.parameters.len();
    let stack_values = ctx.peek_multiple(arg_count);

    let res = method.resolution();
    let module = if !res.is_null() {
        res.definition()[p.import_scope].name.as_ref()
    } else {
        "UNKNOWN_MODULE"
    };
    let function = p.import_name.as_ref();
    let type_name = method.parent.type_name();

    vm_trace_interop!(
        ctx,
        "CALL",
        "Invoking P/Invoke: [{}] function [{}] in type [{}]",
        module,
        function,
        type_name
    );
    let arg_types: Vec<String> = method
        .method
        .signature
        .parameters
        .iter()
        .map(|p| format!("{:?}", p.1))
        .collect();
    vm_trace_interop!(ctx, "ARGS", "Signature: {:?}", arg_types);
    vm_trace_interop!(ctx, "ARGS", "Values:    {:?}", stack_values);

    let target_res = if ctx.tracer_enabled() {
        let mut guard = ctx.tracer();
        ctx.shared()
            .pinvoke
            .get_function(module, function, Some(&mut *guard))
    } else {
        ctx.shared().pinvoke.get_function(module, function, None)
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

            let exception_type = match ctx.loader().corlib_type(exc_name) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            };
            let exception_instance = match ctx.current_context().new_object(exception_type) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            };
            let exception = ObjectRef::new(ctx.gc(), HeapStorage::Obj(exception_instance));

            let message_ref =
                StackValue::string(ctx.gc(), CLRString::from(msg.as_str())).as_object_ref();
            exception.as_object_mut(ctx.gc(), |obj| {
                if obj.instance_storage.has_field(exception_type, "_message") {
                    let mut field = obj
                        .instance_storage
                        .get_field_mut_local(exception_type, "_message");
                    message_ref.write(&mut field);
                }
            });

            let _ = ctx.throw(exception);
            let _ = ctx.pop_multiple(arg_count);
            return StepResult::Exception;
        }
    };

    let mut args: Vec<Type> = vec![];
    {
        let res_ctx = ctx.current_context();
        for Parameter(_, p) in &method.method.signature.parameters {
            args.push(match param_to_type(p, &res_ctx) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            });
        }
    }

    vm_trace!(ctx, "  Preparing return type...");
    let return_type = match &method.method.signature.return_type.1 {
        None => Type::void(),
        Some(s) => {
            vm_trace!(ctx, "  Resolving return type: {:?}", s);
            let res_ctx = ctx.current_context();
            let t = match param_to_type(s, &res_ctx) {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            };
            vm_trace!(ctx, "  Resolved return type to FFI type.");
            t
        }
    };
    let mut temp_buffers: Vec<TempBuffer> = vec![];
    let mut write_backs: Vec<(WriteBackSource<'gc>, usize, usize)> = vec![];
    let mut arg_buffer_map: Vec<Option<usize>> = vec![None; stack_values.len()];
    let mut arg_ptrs: Vec<*mut c_void> = vec![std::ptr::null_mut(); stack_values.len()];

    macro_rules! checked_narrow {
        ($val:expr, $t:ty, $name:expr, $idx:expr) => {{
            let v = <$t>::try_from(*$val).map_err(|_| {
                ExecutionError::InternalError(format!(
                    "P/Invoke marshalling: value {} out of range for {}",
                    *$val, $name
                ))
            });
            let v = match v {
                Ok(v) => v,
                Err(e) => return StepResult::Error(e.into()),
            };
            temp_buffers.push(TempBuffer::Bytes(v.to_ne_bytes().to_vec()));
            let buf_idx = temp_buffers.len() - 1;
            arg_buffer_map[$idx] = Some(buf_idx);
            arg_ptrs[$idx] = temp_buffers[buf_idx].as_bytes().as_ptr() as *mut c_void;
        }};
    }

    // Pass 1: Prepare buffers
    for (i, (v, Parameter(_, p_type))) in stack_values
        .iter()
        .zip(&method.method.signature.parameters)
        .enumerate()
    {
        let ffi_size = unsafe { (*args[i].as_raw_ptr()).size };
        match v {
            StackValue::Int32(val) => {
                let res_ctx = ctx.current_context();
                let p_base_type = if let ParameterType::Value(t) = p_type {
                    match res_ctx.make_concrete(t) {
                        Ok(c) => c.get().clone(),
                        Err(e) => return StepResult::Error(e.into()),
                    }
                } else {
                    BaseType::IntPtr
                };

                match p_base_type {
                    BaseType::Int8 => checked_narrow!(val, i8, "i8", i),
                    BaseType::UInt8 | BaseType::Boolean => checked_narrow!(val, u8, "u8", i),
                    BaseType::Int16 => checked_narrow!(val, i16, "i16", i),
                    BaseType::UInt16 | BaseType::Char => checked_narrow!(val, u16, "u16", i),
                    _ => {
                        temp_buffers.push(TempBuffer::I32(Box::new(*val)));
                        let idx = temp_buffers.len() - 1;
                        arg_buffer_map[i] = Some(idx);
                        arg_ptrs[i] =
                            temp_buffers[idx].as_i32() as *const i32 as *mut i32 as *mut c_void;
                    }
                }
            }
            StackValue::Int64(val) => {
                let res_ctx = ctx.current_context();
                let p_base_type = if let ParameterType::Value(t) = p_type {
                    match res_ctx.make_concrete(t) {
                        Ok(c) => c.get().clone(),
                        Err(e) => return StepResult::Error(e.into()),
                    }
                } else {
                    BaseType::Int64
                };

                match p_base_type {
                    BaseType::Int8 => checked_narrow!(val, i8, "i8", i),
                    BaseType::UInt8 | BaseType::Boolean => checked_narrow!(val, u8, "u8", i),
                    BaseType::Int16 => checked_narrow!(val, i16, "i16", i),
                    BaseType::UInt16 | BaseType::Char => checked_narrow!(val, u16, "u16", i),
                    BaseType::Int32 => checked_narrow!(val, i32, "i32", i),
                    BaseType::UInt32 => checked_narrow!(val, u32, "u32", i),
                    _ => {
                        temp_buffers.push(TempBuffer::I64(Box::new(*val)));
                        let idx = temp_buffers.len() - 1;
                        arg_buffer_map[i] = Some(idx);
                        arg_ptrs[i] =
                            temp_buffers[idx].as_i64() as *const i64 as *mut i64 as *mut c_void;
                    }
                }
            }
            StackValue::NativeInt(val) => {
                let res_ctx = ctx.current_context();
                let p_base_type = if let ParameterType::Value(t) = p_type {
                    match res_ctx.make_concrete(t) {
                        Ok(c) => c.get().clone(),
                        Err(e) => return StepResult::Error(e.into()),
                    }
                } else {
                    BaseType::IntPtr
                };

                match p_base_type {
                    BaseType::Int8 => checked_narrow!(val, i8, "i8", i),
                    BaseType::UInt8 | BaseType::Boolean => checked_narrow!(val, u8, "u8", i),
                    BaseType::Int16 => checked_narrow!(val, i16, "i16", i),
                    BaseType::UInt16 | BaseType::Char => checked_narrow!(val, u16, "u16", i),
                    BaseType::Int32 => checked_narrow!(val, i32, "i32", i),
                    BaseType::UInt32 => checked_narrow!(val, u32, "u32", i),
                    _ => {
                        temp_buffers.push(TempBuffer::Isize(Box::new(*val)));
                        let idx = temp_buffers.len() - 1;
                        arg_buffer_map[i] = Some(idx);
                        arg_ptrs[i] = temp_buffers[idx].as_isize() as *const isize as *mut isize
                            as *mut c_void;
                    }
                }
            }
            StackValue::NativeFloat(val) => {
                temp_buffers.push(TempBuffer::F64(Box::new(*val)));
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] = temp_buffers[idx].as_f64() as *const f64 as *mut f64 as *mut c_void;
            }
            StackValue::UnmanagedPtr(val) => {
                temp_buffers.push(TempBuffer::Ptr(Box::new(val.0.as_ptr())));
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] =
                    temp_buffers[idx].as_ptr() as *const *mut u8 as *mut *mut u8 as *mut c_void;
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
                arg_ptrs[i] = temp_buffers[idx].as_bytes().as_ptr() as *mut c_void;
            }
            StackValue::ObjectRef(obj) => {
                let ptr = if let Some(h) = obj.0 {
                    ctx.pin_object(*obj);
                    pinned_objects.push(*obj);
                    let guard = unsafe { PinnedGuard::new(h) };
                    let ptr = unsafe { guard.guard.storage.raw_data_ptr() };
                    local_guards.push(guard);
                    ptr
                } else {
                    std::ptr::null_mut()
                };
                temp_buffers.push(TempBuffer::Ptr(Box::new(ptr)));
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] =
                    temp_buffers[idx].as_ptr() as *const *mut u8 as *mut *mut u8 as *mut c_void;
            }
            StackValue::TypedRef(p, t) => {
                if let Some(owner) = p.owner() {
                    ctx.pin_object(owner);
                    pinned_objects.push(owner);
                }
                let mut bytes = ManagedPtr::serialization_buffer();
                let addr = unsafe {
                    if let PointerOrigin::Heap(obj) = p.origin {
                        if let Some(h) = obj.0 {
                            let guard = PinnedGuard::new(h);
                            let ptr = guard.guard.storage.raw_data_ptr().add(p.offset.as_usize());
                            local_guards.push(guard);
                            ptr.expose_addr()
                        } else {
                            p.offset.as_usize() // Should be null + offset
                        }
                    } else {
                        p.with_data(0, |data| data.as_ptr().expose_addr())
                    }
                };
                let type_ptr = Arc::as_ptr(t).expose_addr();
                bytes[0..ObjectRef::SIZE].copy_from_slice(&addr.to_ne_bytes());
                bytes[ObjectRef::SIZE..ManagedPtr::SIZE].copy_from_slice(&type_ptr.to_ne_bytes());

                temp_buffers.push(TempBuffer::Bytes(bytes.to_vec()));
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] = temp_buffers[idx].as_bytes().as_ptr() as *mut c_void;
            }
            StackValue::ManagedPtr(p) => {
                let buf_len = ffi_size;
                let mut buf = vec![0u8; buf_len];
                if buf.is_empty() {
                    buf.push(0);
                }

                let is_ref = matches!(p_type, ParameterType::Ref(_));

                if is_ref {
                    // For ref parameters, we use the write-back mechanism
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
                        WriteBackSource::Managed(p.origin.clone(), p.offset),
                        buf_idx,
                        buf_len,
                    ));

                    arg_ptrs[i] = temp_buffers[buf_idx].as_bytes().as_ptr() as *mut c_void;
                } else {
                    // For direct pointers (void*, int*), we just pass the address
                    if let Some(owner) = p.owner() {
                        ctx.pin_object(owner);
                        pinned_objects.push(owner);
                    }

                    let ptr = unsafe {
                        if let PointerOrigin::Heap(obj) = p.origin {
                            if let Some(h) = obj.0 {
                                let guard = PinnedGuard::new(h);
                                let ptr =
                                    guard.guard.storage.raw_data_ptr().add(p.offset.as_usize());
                                local_guards.push(guard);
                                ptr
                            } else {
                                sptr::from_exposed_addr_mut::<u8>(p.offset.as_usize())
                            }
                        } else {
                            p.with_data(0, |data| data.as_ptr().cast_mut())
                        }
                    };

                    temp_buffers.push(TempBuffer::Ptr(Box::new(ptr)));
                    let idx = temp_buffers.len() - 1;
                    arg_buffer_map[i] = Some(idx);
                    arg_ptrs[i] =
                        temp_buffers[idx].as_ptr() as *const *mut u8 as *mut *mut u8 as *mut c_void;
                }
            }
            #[cfg(feature = "multithreaded-gc")]
            StackValue::CrossArenaObjectRef(ptr, _) => {
                let lock = unsafe { &*ptr.as_ptr() };
                let guard = lock.borrow();
                let p = unsafe { guard.storage.raw_data_ptr() };
                cross_arena_guards.push(guard);
                temp_buffers.push(TempBuffer::Ptr(Box::new(p)));
                let idx = temp_buffers.len() - 1;
                arg_buffer_map[i] = Some(idx);
                arg_ptrs[i] =
                    temp_buffers[idx].as_ptr() as *const *mut u8 as *mut *mut u8 as *mut c_void;
            }
        }
    }

    let cif = Cif::new(args, return_type.clone());

    let do_write_back = |ctx: &mut dyn VesOps<'gc, 'm>| {
        for (source, buf_idx, len) in &write_backs {
            let buf = temp_buffers[*buf_idx].as_bytes();
            match source {
                WriteBackSource::Managed(origin, offset) => unsafe {
                    let _ = ctx.write_bytes(origin.clone(), *offset, &buf[..*len]);
                },
                WriteBackSource::Raw(dest_ptr) => unsafe {
                    std::ptr::copy_nonoverlapping(buf.as_ptr(), dest_ptr.as_ptr(), *len);
                },
            }
        }
    };

    let target_fn = *target.as_fun();

    vm_trace_interop!(
        ctx,
        "CALLING",
        "{}::{} with {} args",
        module,
        function,
        arg_ptrs.len()
    );

    macro_rules! read_return {
        ($t:ty) => {{
            let mut ret = std::mem::MaybeUninit::<$t>::uninit();
            vm_trace_interop!(ctx, "PRE-CALL", "(ret) {}::{}", module, function);
            unsafe {
                libffi::raw::ffi_call(
                    cif.as_raw_ptr(),
                    Some(target_fn),
                    ret.as_mut_ptr() as *mut c_void,
                    arg_ptrs.as_mut_ptr(),
                );
            }
            do_write_back(ctx);
            vm_trace_interop!(ctx, "POST-CALL", "(ret) {}::{}", module, function);
            unsafe { ret.assume_init() }
        }};
    }

    match &method.method.signature.return_type.1 {
        None => {
            vm_trace_interop!(ctx, "PRE-CALL", "(void) {}::{}", module, function);
            unsafe {
                libffi::raw::ffi_call(
                    cif.as_raw_ptr(),
                    Some(target_fn),
                    std::ptr::null_mut(),
                    arg_ptrs.as_mut_ptr(),
                );
            }
            do_write_back(ctx);
            vm_trace_interop!(ctx, "POST-CALL", "(void) {}::{}", module, function);
            let _ = ctx.pop_multiple(arg_count);
        }
        Some(ParameterType::Value(t)) => {
            macro_rules! read_into_i32 {
                ($t:ty) => {{ StackValue::Int32(read_return!($t) as i32) }};
            }

            let res_ctx = ctx.current_context();
            let t = vm_try!(res_ctx.make_concrete(t));
            let v = match t.get() {
                BaseType::Boolean => read_into_i32!(u8),
                BaseType::Char => read_into_i32!(u16),
                BaseType::Int8 => read_into_i32!(i8),
                BaseType::UInt8 => read_into_i32!(u8),
                BaseType::Int16 => read_into_i32!(i16),
                BaseType::UInt16 => read_into_i32!(u16),
                BaseType::Int32 => read_into_i32!(i32),
                BaseType::UInt32 => read_into_i32!(u32),
                BaseType::Int64 => StackValue::Int64(read_return!(i64)),
                BaseType::UInt64 => StackValue::Int64(read_return!(u64) as i64),
                BaseType::Float32 => StackValue::NativeFloat(read_return!(f32) as f64),
                BaseType::Float64 => StackValue::NativeFloat(read_return!(f64)),
                BaseType::IntPtr => StackValue::NativeInt(read_return!(isize)),
                BaseType::UIntPtr => StackValue::NativeInt(read_return!(usize) as isize),
                BaseType::ValuePointer(_, _) | BaseType::FunctionPointer(_) => {
                    StackValue::unmanaged_ptr(read_return!(*mut u8))
                }
                BaseType::Type {
                    value_kind: Some(ValueKind::ValueType),
                    source,
                } => {
                    let (ut, type_generics) = decompose_type_source::<ConcreteType>(source);
                    let new_lookup = GenericLookup::new(type_generics);
                    let new_ctx = res_ctx.with_generics(&new_lookup);
                    let td = new_ctx
                        .locate_type(ut)
                        .expect("Failed to locate type in pinvoke interop");

                    let instance = match new_ctx.new_object(td) {
                        Ok(inst) => inst,
                        Err(e) => return StepResult::Error(e.into()),
                    };

                    vm_trace_interop!(
                        ctx,
                        "CALLING",
                        "(raw struct return) {}::{} with {} args",
                        module,
                        function,
                        arg_ptrs.len()
                    );
                    let allocated_size = instance.instance_storage.get().len();
                    let ffi_size = unsafe { (*return_type.as_raw_ptr()).size };

                    // Check for buffer overflow risk
                    if ffi_size > allocated_size {
                        vm_trace_interop!(
                            ctx,
                            "WARNING",
                            "Buffer overflow detected! FFI expects {} bytes, but object has {} bytes. Using temp buffer.",
                            ffi_size,
                            allocated_size
                        );
                        let mut temp_buffer = Vec::with_capacity(ffi_size);
                        unsafe {
                            libffi::raw::ffi_call(
                                cif.as_raw_ptr(),
                                Some(target_fn),
                                temp_buffer.as_mut_ptr() as *mut c_void,
                                arg_ptrs.as_mut_ptr(),
                            );
                            temp_buffer.set_len(ffi_size);
                        }
                        // Copy valid data back to the object
                        let mut guard = instance.instance_storage.get_mut();
                        guard.copy_from_slice(&temp_buffer[..allocated_size]);
                    } else {
                        unsafe {
                            libffi::raw::ffi_call(
                                cif.as_raw_ptr(),
                                Some(target_fn),
                                instance.instance_storage.get_mut().as_mut_ptr() as *mut c_void,
                                arg_ptrs.as_mut_ptr(),
                            );
                        }
                    }
                    do_write_back(ctx);
                    vm_trace_interop!(ctx, "POST-CALL", "(struct) {}::{}", module, function);

                    StackValue::ValueType(instance)
                }
                BaseType::Type { .. }
                | BaseType::Array { .. }
                | BaseType::Vector { .. }
                | BaseType::Object
                | BaseType::String => StackValue::unmanaged_ptr(read_return!(*mut u8)),
            };
            vm_trace!(ctx, "-- returning {v:?} --");
            let _ = ctx.pop_multiple(arg_count);
            ctx.push(v);
        }
        Some(ParameterType::Ref(t)) => {
            let ptr = read_return!(*mut u8);
            let res_ctx = ctx.current_context();
            let concrete = vm_try!(res_ctx.make_concrete(t));
            let td = ctx
                .loader()
                .find_concrete_type(concrete)
                .expect("failed to resolve return type");
            let _ = ctx.pop_multiple(arg_count);
            ctx.push_managed_ptr(ManagedPtr::new(NonNull::new(ptr), td, None, false, None));
        }
        Some(ParameterType::TypedReference) => {
            let mut ret = std::mem::MaybeUninit::<[usize; 2]>::uninit();
            unsafe {
                libffi::raw::ffi_call(
                    cif.as_raw_ptr(),
                    Some(target_fn),
                    ret.as_mut_ptr() as *mut c_void,
                    arg_ptrs.as_mut_ptr(),
                );
            }
            do_write_back(ctx);
            let ret = unsafe { ret.assume_init() };
            let addr = ret[0];
            let type_ptr = ret[1] as *const dotnet_types::TypeDescription;
            if type_ptr.is_null() {
                panic!("null type handle in returned TypedReference");
            }
            let type_desc = unsafe {
                let arc = Arc::from_raw(type_ptr);
                let clone = arc.clone();
                let _ = Arc::into_raw(arc);
                clone
            };
            let m = ManagedPtr::new(
                NonNull::new(addr as *mut u8),
                *type_desc.clone(),
                None,
                false,
                Some(ByteOffset(0)),
            );
            let _ = ctx.pop_multiple(arg_count);
            ctx.push(StackValue::TypedRef(m, type_desc));
        }
    }
    vm_trace_interop!(ctx, "RETURN", "Returned from {}::{}", module, function);

    for obj in pinned_objects {
        ctx.unpin_object(obj);
    }

    StepResult::Continue
}
