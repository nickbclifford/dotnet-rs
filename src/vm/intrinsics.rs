use crate::value::{ConcreteType, Context, FieldDescription, GenericLookup, MethodDescription, Object, ObjectRef, StackValue};
use dotnetdll::prelude::{BaseType, TypeSource};
use std::sync::atomic::{AtomicI32, Ordering};

use super::{CallStack, GCHandle, MethodInfo};

fn ref_as_ptr(v: StackValue) -> *mut u8 {
    match v {
        StackValue::ManagedPtr(m) => m.value,
        err => todo!(
            "invalid type on stack ({:?}), expected managed pointer for ref parameter",
            err
        ),
    }
}

pub fn intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: GenericLookup,
) {
    super::msg!(stack, "-- method marked as runtime intrinsic --");

    // TODO: real signature checking
    match format!("{:?}", method).as_str() {
        "[Generic(1)] static M0 System.Activator::CreateInstance()" => {
            let target = &generics.method_generics[0];

            let mut type_generics: &[ConcreteType] = &[];

            let td = match target.get() {
                BaseType::Object => stack.assemblies.corlib_type("System.Object"),
                BaseType::Type { source, .. } => {
                    let ut = match source {
                        TypeSource::User(u) => *u,
                        TypeSource::Generic { base, parameters } => {
                            type_generics = parameters.as_slice();
                            *base
                        }
                    };
                    stack.assemblies.locate_type(target.resolution(), ut)
                }
                err => panic!(
                    "cannot call parameterless constructor on primitive type {:?}",
                    err
                ),
            };

            let new_lookup = GenericLookup {
                type_generics: type_generics.to_vec(),
                method_generics: vec![],
            };

            let new_ctx = Context {
                generics: &new_lookup,
                ..stack.current_context()
            };

            let instance = Object::new(td, new_ctx);

            for m in &td.1.methods {
                if m.runtime_special_name
                    && m.name == ".ctor"
                    && m.signature.instance
                    && m.signature.parameters.is_empty()
                {
                    super::msg!(
                        stack,
                        "-- invoking parameterless constructor for {} --",
                        td.1.type_name()
                    );

                    stack.constructor_frame(
                        gc,
                        instance,
                        MethodInfo::new(td.0, m),
                        new_lookup,
                    );
                    return;
                }
            }

            panic!("could not find a parameterless constructor in {:?}", td)
        }
        "static void System.ArgumentNullException::ThrowIfNull(object, string)" => {
            let target = stack.pop_stack();
            let argname = stack.pop_stack();
            if let StackValue::ObjectRef(ObjectRef(None)) = target {
                todo!("ArgumentNullException({:?})", argname)
            }
        }
        "static void System.GC::_SuppressFinalize(object)" => {
            // TODO(gc): this object's finalizer should not be called
            let _obj = stack.pop_stack();
        }
        "static int System.Threading.Interlocked::CompareExchange(ref int, int, int)" => {
            let StackValue::Int32(comparand) = stack.pop_stack() else {
                todo!("invalid type on stack")
            };
            let StackValue::Int32(value) = stack.pop_stack() else {
                todo!("invalid type on stack")
            };
            let target = ref_as_ptr(stack.pop_stack()) as *mut i32;

            let atomic_view = unsafe { AtomicI32::from_ptr(target) };
            let Ok(prev) = atomic_view.compare_exchange(
                comparand,
                value,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) else {
                panic!("atomic exchange failed??")
            };

            stack.push_stack(gc, StackValue::Int32(prev));
        }
        "static void System.Threading.Monitor::Exit(object)" => {
            // TODO(threading): release mutex
            let _tag_object = stack.pop_stack();
        }
        "static void System.Threading.Monitor::ReliableEnter(object, ref bool)" => {
            let success_flag = ref_as_ptr(stack.pop_stack());

            // TODO(threading): actually acquire mutex
            let _tag_object = stack.pop_stack();
            // eventually we'll set this properly to indicate success or failure
            // just make it always succeed for now
            unsafe {
                *success_flag = 1u8;
            }
        }
        "[Generic(1)] static M0 System.Threading.Volatile::Read(ref M0)" => {
            let ptr = ref_as_ptr(stack.pop_stack()) as *const ObjectRef<'gc>;

            let value = unsafe { std::ptr::read_volatile(ptr) };

            stack.push_stack(gc, StackValue::ObjectRef(value));
        }
        "static void System.Threading.Volatile::Write(ref bool, bool)" => {
            let value = match stack.pop_stack() {
                StackValue::Int32(i) => i as u8,
                err => todo!("invalid type on stack ({:?}), expected i32 for bool", err),
            };

            let src = ref_as_ptr(stack.pop_stack());

            unsafe { std::ptr::write_volatile(src, value) };
        }
        x => panic!("unsupported intrinsic call to {}", x),
    }

    stack.increment_ip();
}

pub fn intrinsic_field<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    field: FieldDescription,
    _type_generics: Vec<ConcreteType>,
) {
    // TODO: real signature checking
    match format!("{:?}", field).as_str() {
        "static nint System.IntPtr::Zero" => stack.push_stack(gc, StackValue::NativeInt(0)),
        x => panic!("unsupported load from intrinsic field {}", x),
    }

    stack.increment_ip();
}
