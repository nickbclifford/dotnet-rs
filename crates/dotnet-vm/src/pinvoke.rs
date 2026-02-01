use crate::{
    context::ResolutionContext, exceptions::ExceptionState, layout::LayoutFactory,
    resolution::ValueResolution, tracer::Tracer, CallStack,
};
use dashmap::DashMap;
use dotnet_assemblies::decompose_type_source;
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::MethodDescription,
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    layout::{FieldLayoutManager, LayoutManager, Scalar},
    object::{HeapStorage, ObjectRef},
    string::CLRString,
    StackValue,
};
use dotnetdll::prelude::*;
use gc_arena::{unsafe_empty_collect, Collect};
use libffi::middle::*;
use libloading::{Library, Symbol};
use std::{ffi::c_void, path::PathBuf};

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

pub struct NativeLibraries {
    root: PathBuf,
    libraries: DashMap<String, Library>,
}
unsafe_empty_collect!(NativeLibraries);
impl NativeLibraries {
    pub fn new(root: impl AsRef<str>) -> Self {
        Self {
            root: PathBuf::from(root.as_ref()),
            libraries: DashMap::new(),
        }
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
        if let Some(lib) = self.libraries.get(name) {
            return Ok(lib);
        }

        let path = self
            .find_library_path(name)
            .ok_or_else(|| PInvokeError::LibraryNotFound(name.to_string()))?;

        if let Some(t) = &mut tracer {
            t.trace_interop(0, "RESOLVE", &format!("Resolving library '{}'...", name));
            t.trace_interop(
                0,
                "RESOLVE",
                &format!("Loading library from path: {:?}", path),
            );
        }

        let lib = unsafe { Library::new(&path) }
            .map_err(|e| PInvokeError::LoadError(name.to_string(), e.to_string()))?;

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
        let l = self.get_library(library, tracer)?;
        let sym: Symbol<unsafe extern "C" fn()> = unsafe { l.get(name.as_bytes()) }
            .map_err(|_| PInvokeError::SymbolNotFound(library.to_string(), name.to_string()))?;
        Ok(CodePtr::from_fun(*sym))
    }
}

fn type_to_layout(t: &TypeSource<ConcreteType>, ctx: &ResolutionContext) -> FieldLayoutManager {
    let (ut, type_generics) = decompose_type_source(t);
    let new_lookup = GenericLookup::new(type_generics);
    let new_ctx = ctx.with_generics(&new_lookup);
    let td = new_ctx.locate_type(ut);

    if td.is_null() {
        panic!(
            "P/Invoke marshalling error: Could not resolve type {:?} when calculating layout.",
            ut
        );
    }

    LayoutFactory::instance_fields(td, &new_ctx)
}

fn layout_to_ffi(l: &LayoutManager) -> Type {
    match l {
        LayoutManager::FieldLayoutManager(f) => {
            let mut fields: Vec<_> = f.fields.values().collect();
            fields.sort_by_key(|f| f.position);

            Type::structure(fields.into_iter().map(|f| layout_to_ffi(&f.layout)))
        }
        LayoutManager::ArrayLayoutManager(_) => todo!("marshalling not yet supported for arrays"),
        LayoutManager::Scalar(s) => match s {
            Scalar::Int8 => Type::i8(),
            Scalar::Int16 => Type::i16(),
            Scalar::Int32 => Type::i32(),
            Scalar::Int64 => Type::i64(),
            Scalar::ObjectRef => todo!("marshalling not yet supported for native object refs"),
            Scalar::NativeInt => Type::isize(),
            Scalar::Float32 => Type::f32(),
            Scalar::Float64 => Type::f64(),
            Scalar::ManagedPtr => Type::pointer(),
        },
    }
}

fn type_to_ffi(t: &ConcreteType, ctx: &ResolutionContext) -> Type {
    match t.get() {
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
            value_kind: None | Some(ValueKind::ValueType),
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
        rest => todo!("marshalling not yet supported for {:?}", rest),
    }
}

fn param_to_type(p: &ParameterType<MethodType>, ctx: &ResolutionContext) -> Type {
    let ParameterType::Value(t) = p else {
        todo!("marshalling ref/typedref parameters")
    };
    type_to_ffi(&ctx.make_concrete(t), ctx)
}

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn external_call(&mut self, method: MethodDescription, gc: GCHandle<'gc>) {
        let Some(p) = &method.method.pinvoke else {
            unreachable!()
        };

        let arg_count = method.method.signature.parameters.len();
        let mut stack_values = vec![];
        for i in 0..arg_count {
            stack_values.push(self.peek_stack_at(arg_count - 1 - i));
        }

        let res = method.resolution();
        let module = if !res.is_null() {
            res.definition()[p.import_scope].name.as_ref()
        } else {
            "UNKNOWN_MODULE"
        };
        let function = p.import_name.as_ref();
        let type_name = method.parent.type_name();

        vm_trace_interop!(
            self,
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
        vm_trace_interop!(self, "ARGS", "Signature: {:?}", arg_types);

        vm_trace_interop!(self, "ARGS", "Values:    {:?}", stack_values);

        let target_res = if self.tracer_enabled() {
            let mut guard = self.tracer();
            self.pinvoke()
                .get_function(module, function, Some(&mut *guard))
        } else {
            self.pinvoke().get_function(module, function, None)
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

                let exception_type = self.loader().corlib_type(exc_name);
                let exception_instance = self.current_context().new_object(exception_type);
                let exception = ObjectRef::new(gc, HeapStorage::Obj(exception_instance));

                let message_ref =
                    StackValue::string(gc, CLRString::from(msg.as_str())).as_object_ref();
                exception.as_object_mut(gc, |obj| {
                    if obj.instance_storage.has_field(exception_type, "_message") {
                        let mut field = obj
                            .instance_storage
                            .get_field_mut_local(exception_type, "_message");
                        message_ref.write(&mut field);
                    }
                });

                self.execution.exception_mode = ExceptionState::Throwing(exception);
                return;
            }
        };

        let ctx = self.current_context();
        let mut args: Vec<Type> = vec![];
        for Parameter(_, p) in &method.method.signature.parameters {
            args.push(param_to_type(p, &ctx));
        }

        vm_trace!(self, "  Preparing return type...");
        let return_type = match &method.method.signature.return_type.1 {
            None => Type::void(),
            Some(s) => {
                vm_trace!(self, "  Resolving return type: {:?}", s);
                let t = param_to_type(s, &ctx);
                vm_trace!(self, "  Resolved return type to FFI type.");
                t
            }
        };
        let mut ptr_args: Vec<*mut c_void> = vec![];
        let mut temp_buffers: Vec<Vec<u8>> = vec![];
        let mut write_backs: Vec<(std::ptr::NonNull<u8>, usize, usize)> = vec![];
        let mut arg_buffer_map: Vec<Option<usize>> = vec![None; stack_values.len()];

        // Pass 1: Prepare buffers
        for (i, v) in stack_values.iter().enumerate() {
            let ffi_size = unsafe { (*args[i].as_raw_ptr()).size };
            match v {
                StackValue::ValueType(o) => {
                    let mut data = o.instance_storage.get().to_vec();
                    if data.len() < ffi_size {
                        data.resize(ffi_size, 0);
                    }

                    if data.is_empty() {
                        temp_buffers.push(vec![0]);
                    } else {
                        temp_buffers.push(data);
                    }
                    arg_buffer_map[i] = Some(temp_buffers.len() - 1);
                }
                StackValue::ManagedPtr(p) => {
                    let mut handled = false;
                    if let Some(val_ptr) = p.value {
                        // Trust the pointer and size (marshaling without owner tracking)
                        let current_ptr = val_ptr.as_ptr();
                        let buf_len = ffi_size;
                        let mut buf = vec![0u8; buf_len];
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                current_ptr,
                                buf.as_mut_ptr(),
                                buf_len,
                            );
                        }
                        temp_buffers.push(buf);
                        let buf_idx = temp_buffers.len() - 1;
                        arg_buffer_map[i] = Some(buf_idx);
                        write_backs.push((val_ptr, buf_idx, buf_len));
                        handled = true;
                    }

                    if !handled {
                        let ptr = p
                            .value
                            .map(|x| x.as_ptr() as *mut c_void)
                            .unwrap_or(std::ptr::null_mut());
                        ptr_args.push(ptr);
                    }
                }
                _ => {}
            }
        }

        let cif = Cif::new(args, return_type.clone());

        let mut ptr_args_iter = ptr_args.iter();
        let mut arg_values: Vec<Arg> = vec![];

        for (i, v) in stack_values.iter().enumerate() {
            let arg = match v {
                StackValue::Int32(ref i) => Arg::new(i),
                StackValue::Int64(ref i) => Arg::new(i),
                StackValue::NativeInt(ref i) => Arg::new(i),
                StackValue::NativeFloat(ref f) => Arg::new(f),
                StackValue::UnmanagedPtr(ref p) => Arg::new(&p.0),
                StackValue::ManagedPtr(_) => {
                    if let Some(idx) = arg_buffer_map[i] {
                        Arg::new(&temp_buffers[idx][0])
                    } else {
                        let ptr_ref = ptr_args_iter.next().unwrap();
                        Arg::new(ptr_ref)
                    }
                }
                StackValue::ValueType(_) => {
                    let idx = arg_buffer_map[i].unwrap();
                    Arg::new(&temp_buffers[idx][0])
                }
                rest => panic!(
                    "marshalling not yet supported for {:?} in P/Invoke calling {}::{}",
                    rest, module, function
                ),
            };
            arg_values.push(arg);
        }

        let do_write_back = || unsafe {
            for (dest_ptr, buf_idx, len) in &write_backs {
                let buf = &temp_buffers[*buf_idx];
                std::ptr::copy_nonoverlapping(buf.as_ptr(), dest_ptr.as_ptr(), *len);
            }
        };

        vm_trace_interop!(
            self,
            "CALLING",
            "{}::{} with {} args",
            module,
            function,
            arg_values.len()
        );
        match &method.method.signature.return_type.1 {
            None => {
                vm_trace_interop!(self, "PRE-CALL", "(void) {}::{}", module, function);
                let _: c_void = unsafe { cif.call(target, &arg_values) };
                do_write_back();
                vm_trace_interop!(self, "POST-CALL", "(void) {}::{}", module, function);
                for _ in 0..arg_count {
                    self.pop_stack(gc);
                }
            }
            Some(p) => {
                let ParameterType::Value(t) = p else {
                    todo!("marshalling ref/typedref parameters")
                };

                macro_rules! read_return {
                    ($t:ty) => {{
                        vm_trace_interop!(self, "PRE-CALL", "(ret) {}::{}", module, function);
                        let res = unsafe { cif.call::<$t>(target, &arg_values) };
                        do_write_back();
                        vm_trace_interop!(self, "POST-CALL", "(ret) {}::{}", module, function);
                        res
                    }};
                }

                macro_rules! read_into_i32 {
                    ($t:ty) => {{
                        StackValue::Int32(read_return!($t) as i32)
                    }};
                }

                let t = ctx.make_concrete(t);
                let v = match t.get() {
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
                    BaseType::Type { source, .. } => {
                        let (ut, type_generics) = decompose_type_source(source);
                        let new_lookup = GenericLookup::new(type_generics);
                        let new_ctx = ctx.with_generics(&new_lookup);
                        let td = new_ctx.locate_type(ut);

                        let instance = new_ctx.new_object(td);

                        // We need an array of pointers to the arguments for ffi_call.
                        // Since arg_values: Vec<Arg> already contains these pointers,
                        // we can just collect them into a new Vec.
                        let mut arg_ptrs: Vec<*mut c_void> = arg_values
                            .iter()
                            .map(|arg| unsafe {
                                // SAFETY: libffi::middle::Arg is a wrapper around a raw pointer.
                                // We cast it to a raw pointer to pass to the raw ffi_call.
                                *(arg as *const _ as *const *mut c_void)
                            })
                            .collect();

                        vm_trace_interop!(
                            self,
                            "CALLING",
                            "(raw struct return) {}::{} with {} args",
                            module,
                            function,
                            arg_ptrs.len()
                        );
                        vm_trace_interop!(self, "PRE-CALL", "(struct) {}::{}", module, function);
                        let allocated_size = instance.instance_storage.get().len();
                        let ffi_size = unsafe { (*return_type.as_raw_ptr()).size };

                        // Check for buffer overflow risk
                        if ffi_size > allocated_size {
                            vm_trace_interop!(self, "WARNING", "Buffer overflow detected! FFI expects {} bytes, but object has {} bytes. Using temp buffer.", ffi_size, allocated_size);
                            let mut temp_buffer = vec![0u8; ffi_size];
                            unsafe {
                                libffi::raw::ffi_call(
                                    cif.as_raw_ptr(),
                                    Some(*target.as_fun()),
                                    temp_buffer.as_mut_ptr() as *mut c_void,
                                    arg_ptrs.as_mut_ptr(),
                                );
                            }
                            // Copy valid data back to the object
                            let mut guard = instance.instance_storage.get_mut();
                            guard.copy_from_slice(&temp_buffer[..allocated_size]);
                        } else {
                            unsafe {
                                libffi::raw::ffi_call(
                                    cif.as_raw_ptr(),
                                    Some(*target.as_fun()),
                                    instance.instance_storage.get_mut().as_mut_ptr() as *mut c_void,
                                    arg_ptrs.as_mut_ptr(),
                                );
                            }
                        }
                        do_write_back();
                        vm_trace_interop!(self, "POST-CALL", "(struct) {}::{}", module, function);

                        StackValue::ValueType(Box::new(instance))
                    }
                    rest => todo!("marshalling not yet supported for {:?}", rest),
                };
                vm_trace!(self, "-- returning {v:?} --");
                for _ in 0..arg_count {
                    self.pop_stack(gc);
                }
                self.push_stack(gc, v);
            }
        }
        vm_trace_interop!(self, "RETURN", "Returned from {}::{}", module, function);
    }
}
