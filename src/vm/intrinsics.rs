use super::{CallStack, GCHandle, MethodInfo};
use crate::value::{
    layout::{type_layout, FieldLayoutManager, HasLayout},
    string::CLRString,
    ConcreteType, Context, FieldDescription, GenericLookup, HeapStorage, MethodDescription, Object,
    ObjectRef, StackValue,
};
use dotnetdll::prelude::*;
use std::sync::atomic::{AtomicI32, Ordering};

macro_rules! expect_stack {
    (let $variant:ident ( $inner:ident ) = $v:expr) => {
        let $inner = match $v {
            StackValue::$variant($inner) => $inner,
            err => panic!(
                "invalid type on stack ({:?}), expected {}",
                err,
                stringify!($variant)
            ),
        };
    };
}
pub(crate) use expect_stack;

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
    macro_rules! pop {
        () => {
            stack.pop_stack()
        };
    }
    macro_rules! push {
        ($value:expr) => {
            stack.push_stack(gc, $value)
        };
    }
    macro_rules! with_string {
        ($value:expr, |$s:ident| $code:expr) => {{
            expect_stack!(let ObjectRef(obj) = $value);
            let ObjectRef(Some(obj)) = obj else { todo!("NullPointerException") };
            let heap = obj.borrow();
            let HeapStorage::Str($s) = &*heap else {
                todo!("invalid type on stack, expected string")
            };
            $code
        }};
    }
    macro_rules! ctx {
        () => {
            Context::with_generics(stack.current_context(), &generics)
        };
    }

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

            let new_lookup = GenericLookup::new(type_generics.to_vec());
            let new_ctx = Context::with_generics(ctx!(), &new_lookup);

            let instance = Object::new(td, new_ctx.clone());

            for m in &td.definition.methods {
                if m.runtime_special_name
                    && m.name == ".ctor"
                    && m.signature.instance
                    && m.signature.parameters.is_empty()
                {
                    super::msg!(
                        stack,
                        "-- invoking parameterless constructor for {} --",
                        td.type_name()
                    );

                    stack.constructor_frame(
                        gc,
                        instance,
                        MethodInfo::new(td.resolution, m, new_ctx),
                        new_lookup,
                    );
                    return;
                }
            }

            panic!("could not find a parameterless constructor in {:?}", td)
        }
        "static void System.ArgumentNullException::ThrowIfNull(object, string)" => {
            let target = pop!();
            let argname = pop!();
            if let StackValue::ObjectRef(ObjectRef(None)) = target {
                todo!("ArgumentNullException({:?})", argname)
            }
        }
        "static string System.Environment::GetEnvironmentVariableCore(string)" => {
            let value = with_string!(pop!(), |s| {
                std::env::var(s.as_string()).unwrap_or_default()
            });
            push!(StackValue::string(gc, CLRString::from(value)));
        }
        "static void System.GC::_SuppressFinalize(object)" => {
            // TODO(gc): this object's finalizer should not be called
            let _obj = pop!();
        }
        "[Generic(1)] static bool System.Runtime.CompilerServices.RuntimeHelpers::IsReferenceOrContainsReferences()" => {
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());
            push!(StackValue::Int32(layout.is_or_contains_refs() as i32));
        }
        "static int System.Runtime.InteropServices.Marshal::GetLastPInvokeError()" => {
            let value = unsafe { super::pinvoke::LAST_ERROR };

            push!(StackValue::Int32(value));
        }
        "static void System.Runtime.InteropServices.Marshal::SetLastPInvokeError(int)" => {
            expect_stack!(let Int32(value) = pop!());

            unsafe {
                super::pinvoke::LAST_ERROR = value;
            }
        }
        "static bool System.Runtime.Intrinsics.Vector128::get_IsHardwareAccelerated()" |
        "static bool System.Runtime.Intrinsics.Vector256::get_IsHardwareAccelerated()" |
        "static bool System.Runtime.Intrinsics.Vector512::get_IsHardwareAccelerated()" => {
            // not in a million years, lol
            push!(StackValue::Int32(0));
        }
        "<req System.Runtime.InteropServices.InAttribute> ref T0 System.ReadOnlySpan`1::get_Item(int)" => {
            expect_stack!(let Int32(index) = pop!());
            expect_stack!(let ManagedPtr(m) = pop!());
            if !m.inner_type.type_name().contains("Span") {
                todo!("invalid type on stack");
            }
            let span_layout = FieldLayoutManager::instance_fields(
                m.inner_type,
                ctx!()
            );

            let value_type = &generics.type_generics[0];
            let value_layout = type_layout(value_type.clone(), ctx!());

            let ptr = unsafe {
                m.value
                    // find the span's base pointer
                    .add(span_layout.fields["_reference"].position)
                    // navigate to the specified index
                    .add(value_layout.size() * index as usize)
            };
            push!(StackValue::managed_ptr(ptr, stack.assemblies.find_concrete_type(value_type.clone())));
        }
        "int System.Span`1::get_Length()" |
        "int System.ReadOnlySpan`1::get_Length()" => {
            expect_stack!(let ManagedPtr(m) = pop!());
            if !m.inner_type.type_name().contains("Span") {
                todo!("invalid type on stack");
            }
            let layout = FieldLayoutManager::instance_fields(
                m.inner_type,
                ctx!()
            );
            let value = unsafe {
                let target = m.value.add(layout.fields["_length"].position) as *const i32;
                *target
            };
            push!(StackValue::Int32(value));
        }
        "static int System.Threading.Interlocked::CompareExchange(ref int, int, int)" => {
            expect_stack!(let Int32(comparand) = pop!());
            expect_stack!(let Int32(value) = pop!());
            let target = ref_as_ptr(pop!()) as *mut i32;

            let atomic_view = unsafe { AtomicI32::from_ptr(target) };
            let Ok(prev) = atomic_view.compare_exchange(
                comparand,
                value,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) else {
                panic!("atomic exchange failed??")
            };

            push!(StackValue::Int32(prev));
        }
        "static void System.Threading.Monitor::Exit(object)" => {
            // TODO(threading): release mutex
            let _tag_object = pop!();
        }
        "static void System.Threading.Monitor::ReliableEnter(object, ref bool)" => {
            let success_flag = ref_as_ptr(pop!());

            // TODO(threading): actually acquire mutex
            let _tag_object = pop!();
            // eventually we'll set this properly to indicate success or failure
            // just make it always succeed for now
            unsafe {
                *success_flag = 1u8;
            }
        }
        "[Generic(1)] static M0 System.Threading.Volatile::Read(ref M0)" => {
            let ptr = ref_as_ptr(pop!()) as *const ObjectRef<'gc>;

            let value = unsafe { std::ptr::read_volatile(ptr) };

            push!(StackValue::ObjectRef(value));
        }
        "static void System.Threading.Volatile::Write(ref bool, bool)" => {
            expect_stack!(let Int32(value) = pop!());
            let as_bool = value as u8;

            let src = ref_as_ptr(pop!());

            unsafe { std::ptr::write_volatile(src, as_bool) };
        }
        "char System.String::get_Chars(int)" => {
            expect_stack!(let Int32(index) = pop!());
            let value = with_string!(pop!(), |s| s[index as usize]);
            push!(StackValue::Int32(value as i32));
        }
        "<req System.Runtime.InteropServices.InAttribute> ref char System.String::GetPinnableReference()" => {
            let ptr = with_string!(pop!(), |s| s.as_ptr() as *mut u8);
            let value = StackValue::managed_ptr(ptr, stack.assemblies.corlib_type("System.Char"));
            push!(value);
        }
        "int System.String::get_Length()" => {
            let len = with_string!(pop!(), |s| s.len());
            push!(StackValue::Int32(len as i32));
        }
        "int System.String::IndexOf(char)" => {
            expect_stack!(let Int32(c) = pop!());
            let c = c as u16;
            let index = with_string!(pop!(), |s| s.iter().position(|x| *x == c));

            push!(StackValue::Int32(match index {
                None => -1,
                Some(i) => i as i32,
            }));
        }
        "int System.String::IndexOf(char, int)" => {
            expect_stack!(let Int32(start_at) = pop!());
            expect_stack!(let Int32(c) = pop!());
            let c = c as u16;
            let index = with_string!(pop!(), |s| s.iter().skip(start_at as usize).position(|x| *x == c));

            push!(StackValue::Int32(match index {
                None => -1,
                Some(i) => i as i32,
            }));
        }
        "string System.String::Substring(int)" => {
            expect_stack!(let Int32(start_at) = pop!());
            let value = with_string!(pop!(), |s| s.split_at(start_at as usize).0.to_vec());
            push!(StackValue::string(gc, CLRString::new(value)));
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
