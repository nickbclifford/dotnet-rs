use std::{collections::HashMap, path::PathBuf};

use dotnetdll::prelude::*;
use libffi::middle::*;
use libloading::{Library, Symbol};

use crate::{
    value::{MethodDescription, StackValue},
    vm::{CallStack, GCHandle},
};

pub static mut LAST_ERROR: i32 = 0;

pub struct NativeLibraries {
    root: PathBuf,
    libraries: HashMap<String, Library>,
}
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

        let module = method.resolution()[p.import_scope].name.as_ref();
        let function = p.import_name.as_ref();

        super::msg!(
            self,
            "-- calling P/Invoke {} with arguments {stack_values:?} --",
            method
                .method
                .signature
                .show_with_name(method.resolution(), format!("{module}::{function}"))
        );

        let target = self.pinvoke.get_function(module, function);

        let ctx = self.current_context();
        let param_to_type = |p: &ParameterType<MethodType>| {
            let ParameterType::Value(t) = p else {
                todo!("marshalling ref/typedref parameters")
            };
            match ctx.make_concrete(t).get() {
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
                rest => todo!("marshalling not yet supported for {:?}", rest),
            }
        };

        let mut args: Vec<Type> = vec![];
        for Parameter(_, p) in &method.method.signature.parameters {
            args.push(param_to_type(p));
        }
        let return_type = match &method.method.signature.return_type.1 {
            None => Type::void(),
            Some(s) => param_to_type(s),
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
                rest => todo!("marshalling not yet supported for {:?}", rest),
            })
            .collect();

        match &method.method.signature.return_type.1 {
            None => {
                let _: std::ffi::c_void = unsafe { cif.call(target, &arg_values) };
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

                let v = match ctx.make_concrete(t).get() {
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
                    rest => todo!("marshalling not yet supported for {:?}", rest),
                };
                super::msg!(self, "-- returning {v:?} --");
                self.push_stack(gc, v);
            }
        }

        self.increment_ip();
    }
}
