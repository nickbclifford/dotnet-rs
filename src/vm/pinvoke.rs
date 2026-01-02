use crate::{
    utils::decompose_type_source,
    value::{
        layout::{FieldLayoutManager, LayoutManager, Scalar},
        ConcreteType, GenericLookup, MethodDescription, Object, ResolutionContext, StackValue,
    },
    vm::{CallStack, GCHandle},
};

use dotnetdll::prelude::*;
use gc_arena::{Collect, unsafe_empty_collect};
use libffi::middle::*;
use libloading::{Library, Symbol};
use std::{collections::HashMap, ffi::c_void, path::PathBuf};

pub static mut LAST_ERROR: i32 = 0;

pub struct NativeLibraries {
    root: PathBuf,
    libraries: HashMap<String, Library>,
}
unsafe_empty_collect!(NativeLibraries);
impl NativeLibraries {
    pub fn new(root: impl AsRef<str>) -> Self {
        Self {
            root: PathBuf::from(root.as_ref()),
            libraries: HashMap::new(),
        }
    }

    pub fn get_library(&mut self, name: &str) -> &Library {
        self.libraries.entry(name.to_string()).or_insert_with(|| {
            let mut path = PathBuf::from(name);
            for d in self.root.read_dir().unwrap() {
                let d = d.unwrap();
                if d.file_name().to_str().unwrap().starts_with(name) {
                    path = d.path();
                    break;
                }
            }
            unsafe { Library::new(path).unwrap() }
        })
    }

    pub fn get_function(&mut self, library: &str, name: &str) -> CodePtr {
        let l = self.get_library(library);
        let sym: Symbol<unsafe extern "C" fn()> = unsafe { l.get(name.as_bytes()) }.unwrap();
        CodePtr::from_fun(*sym)
    }
}

fn type_to_layout(t: &TypeSource<ConcreteType>, ctx: &ResolutionContext) -> FieldLayoutManager {
    let (ut, type_generics) = decompose_type_source(t);
    let new_lookup = GenericLookup::new(type_generics);
    let new_ctx = ctx.with_generics(&new_lookup);
    let td = new_ctx.locate_type(ut);

    FieldLayoutManager::instance_fields(td, &new_ctx)
}

fn layout_to_ffi(l: LayoutManager) -> Type {
    match l {
        LayoutManager::FieldLayoutManager(f) => {
            let mut fields: Vec<_> = f.fields.into_values().collect();
            fields.sort_by_key(|f| f.position);

            Type::structure(fields.into_iter().map(|f| layout_to_ffi(f.layout)))
        }
        LayoutManager::ArrayLayoutManager(_) => todo!("marshalling not yet supported for arrays"),
        LayoutManager::Scalar(s) => {
            use Scalar::*;
            match s {
                Int8 => Type::i8(),
                Int16 => Type::i16(),
                Int32 => Type::i32(),
                Int64 => Type::i64(),
                ObjectRef => todo!("marshalling not yet supported for native object refs"),
                NativeInt => Type::isize(),
                Float32 => Type::f32(),
                Float64 => Type::f64(),
                ManagedPtr => Type::pointer(),
            }
        }
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
            layout_to_ffi(layout.into())
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

        let mut stack_values = vec![];
        for _ in 0..method.method.signature.parameters.len() {
            stack_values.push(self.pop_stack());
        }
        stack_values.reverse();

        let res = method.resolution().0;
        let module = res[p.import_scope].name.as_ref();
        let function = p.import_name.as_ref();

        vm_msg!(
            self,
            "-- calling P/Invoke {} with arguments {stack_values:?} --",
            method
                .method
                .signature
                .show_with_name(res, format!("{module}::{function}"))
        );

        let target = self.runtime.pinvoke.get_function(module, function);

        let ctx = self.current_context();
        let mut args: Vec<Type> = vec![];
        for Parameter(_, p) in &method.method.signature.parameters {
            args.push(param_to_type(p, &ctx));
        }
        let return_type = match &method.method.signature.return_type.1 {
            None => Type::void(),
            Some(s) => param_to_type(s, &ctx),
        };
        let cif = Cif::new(args, return_type);

        let arg_values: Vec<_> = stack_values
            .iter()
            .map(|v| match v {
                StackValue::Int32(i) => Arg::new(i),
                StackValue::Int64(i) => Arg::new(i),
                StackValue::NativeInt(i) => Arg::new(i),
                StackValue::NativeFloat(f) => Arg::new(f),
                StackValue::UnmanagedPtr(p) => Arg::new(p),
                StackValue::ManagedPtr(p) => Arg::new(p),
                StackValue::ValueType(o) => unsafe {
                    std::mem::transmute::<*mut c_void, Arg>(o.instance_storage.get().as_ptr() as _)
                },
                rest => todo!("marshalling not yet supported for {:?}", rest),
            })
            .collect();

        match &method.method.signature.return_type.1 {
            None => {
                let _: c_void = unsafe { cif.call(target, &arg_values) };
            }
            Some(p) => {
                let ParameterType::Value(t) = p else {
                    todo!("marshalling ref/typedref parameters")
                };

                macro_rules! read_return {
                    ($t:ty) => {
                        unsafe { cif.call::<$t>(target, &arg_values) }
                    };
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

                        let mut instance = Object::new(td, &new_ctx);

                        // Arg is just a wrapper around a *mut c_void
                        let mut args: Vec<*mut c_void> = unsafe {
                            let mut v_clone = std::mem::ManuallyDrop::new(arg_values);
                            Vec::from_raw_parts(
                                v_clone.as_mut_ptr() as *mut _,
                                v_clone.len(),
                                v_clone.capacity(),
                            )
                        };

                        unsafe {
                            libffi::raw::ffi_call(
                                cif.as_raw_ptr(),
                                Some(*target.as_fun()),
                                instance.instance_storage.get_mut().as_mut_ptr() as *mut c_void,
                                args.as_mut_ptr(),
                            );
                        }

                        StackValue::ValueType(Box::new(instance))
                    }
                    rest => todo!("marshalling not yet supported for {:?}", rest),
                };
                vm_msg!(self, "-- returning {v:?} --");
                self.push_stack(gc, v);
            }
        }
    }
}
