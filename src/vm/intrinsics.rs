use super::{CallStack, GCHandle, MethodInfo};
use crate::{
    utils::decompose_type_source,
    value::{
        layout::*,
        string::CLRString,
        ConcreteType, Context, FieldDescription, GenericLookup, HeapStorage, MethodDescription, Object,
        ObjectRef, StackValue,
    }
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
        StackValue::NativeInt(i) => i as *mut u8,
        err => todo!(
            "invalid type on stack ({:?}), expected managed pointer for ref parameter",
            err
        ),
    }
}

fn span_to_slice<'gc, 'a>(span: Object<'gc>) -> &'a [u8] {
    let mut ptr_data = [0u8; size_of::<usize>()];
    let mut len_data = [0u8; size_of::<i32>()];
    
    ptr_data.copy_from_slice(span.instance_storage.get_field("_reference"));
    len_data.copy_from_slice(span.instance_storage.get_field("_length"));
    
    let ptr = usize::from_ne_bytes(ptr_data);
    let len = i32::from_ne_bytes(len_data) as usize;
    
    unsafe { std::slice::from_raw_parts(ptr as *const u8, len) }
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

            let mut type_generics = vec![];

            let td = match target.get() {
                BaseType::Object => stack.assemblies.corlib_type("System.Object"),
                BaseType::Type { source, .. } => {
                    let (ut, generics) = decompose_type_source(source);
                    type_generics = generics;
                    stack.assemblies.locate_type(target.resolution(), ut)
                }
                err => panic!(
                    "cannot call parameterless constructor on primitive type {:?}",
                    err
                ),
            };

            let new_lookup = GenericLookup::new(type_generics);
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
        "[Generic(1)] static void System.Buffer::Memmove(ref M0, ref M0, nuint)" => {
            expect_stack!(let NativeInt(len) = pop!());
            let src = ref_as_ptr(pop!());
            let dst = ref_as_ptr(pop!());

            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());
            let total_count = len as usize * layout.size();

            unsafe {
                std::ptr::copy(
                    src,
                    dst,
                    total_count
                );
            }
        }
        "static string System.Environment::GetEnvironmentVariableCore(string)" => {
            let value = with_string!(pop!(), |s| {
                std::env::var(s.as_string())
            });
            match value.ok() {
                Some(s) => push!(StackValue::string(gc, CLRString::from(s))),
                None => push!(StackValue::null()),
            }
        }
        "static void System.GC::_SuppressFinalize(object)" => {
            // TODO(gc): this object's finalizer should not be called
            let _obj = pop!();
        }
        "static bool System.MemoryExtensions::Equals(valuetype System.ReadOnlySpan`1<char>, valuetype System.ReadOnlySpan`1<char>, valuetype System.StringComparison)" => {
            expect_stack!(let Int32(_culture_comparison) = pop!()); // TODO(i18n): respect StringComparison rules
            expect_stack!(let ValueType(b) = pop!());
            expect_stack!(let ValueType(a) = pop!());
            
            let a = span_to_slice(*a);
            let b = span_to_slice(*b);
            
            push!(StackValue::Int32((a == b) as i32));
        }
        "[Generic(1)] static bool System.Runtime.CompilerServices.RuntimeHelpers::IsReferenceOrContainsReferences()" => {
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());
            push!(StackValue::Int32(layout.is_or_contains_refs() as i32));
        }
        "[Generic(1)] static ref M0 System.Runtime.CompilerServices.Unsafe::Add(ref M0, nint)" => {
            let target_type = stack.assemblies.find_concrete_type(generics.method_generics[0].clone());
            expect_stack!(let NativeInt(offset) = pop!());
            let m = ref_as_ptr(pop!());
            push!(StackValue::managed_ptr(unsafe { m.offset(offset) }, target_type));
        }
        "[Generic(1)] static M0 System.Runtime.CompilerServices.Unsafe::ReadUnaligned(void*)" => {
            expect_stack!(let NativeInt(ptr) = pop!());
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());

            macro_rules! vol_read_into {
                ($t:ty) => {
                    unsafe {
                        std::ptr::read_volatile(ptr as *const $t)
                    }
                };
            }

            let v = match layout {
                LayoutManager::Scalar(s) => match s {
                    Scalar::ObjectRef => StackValue::ObjectRef(vol_read_into!(ObjectRef)),
                    Scalar::Int8 => StackValue::Int32(vol_read_into!(i8) as i32),
                    Scalar::Int16 => StackValue::Int32(vol_read_into!(i16) as i32),
                    Scalar::Int32 => StackValue::Int32(vol_read_into!(i32)),
                    Scalar::Int64 => StackValue::Int64(vol_read_into!(i64)),
                    Scalar::NativeInt => StackValue::NativeInt(vol_read_into!(isize)),
                    Scalar::Float32 => StackValue::NativeFloat(vol_read_into!(f32) as f64),
                    Scalar::Float64 => StackValue::NativeFloat(vol_read_into!(f64)),
                },
                _ => todo!("unsupported layout for read unaligned"),
            };
            push!(v);
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
        "static bool System.String::Equals(string, string)" => {
            let b = with_string!(pop!(), |b| b.to_vec());
            let a = with_string!(pop!(), |a| a.to_vec());

            push!(StackValue::Int32(if a == b { 1 } else { 0 }));
        }
        "static string System.String::FastAllocateString(int)" => {
            expect_stack!(let Int32(len) = pop!());
            let value = CLRString::new(vec![0u16; len as usize]);
            push!(StackValue::string(gc, value));
        }
        "char System.String::get_Chars(int)" => {
            expect_stack!(let Int32(index) = pop!());
            let value = with_string!(pop!(), |s| s[index as usize]);
            push!(StackValue::Int32(value as i32));
        }
        "<req System.Runtime.InteropServices.InAttribute> ref char System.String::GetPinnableReference()" |
        "ref char System.String::GetRawStringData()" => {
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
        "static valuetype System.ReadOnlySpan`1<char> System.String::op_Implicit(string)" => {
            let (ptr, len) = with_string!(pop!(), |s| (s.as_ptr(), s.len()));

            let span_type = stack.assemblies.corlib_type("System.ReadOnlySpan`1");
            let new_lookup = GenericLookup::new(vec![ctx!().make_concrete(&BaseType::Char)]);
            let ctx = Context::with_generics(ctx!(), &new_lookup);

            let mut span = Object::new(span_type, ctx);
            
            span.instance_storage.get_field_mut("_reference").copy_from_slice(&(ptr as usize).to_ne_bytes());
            span.instance_storage.get_field_mut("_length").copy_from_slice(&(len as i32).to_ne_bytes());

            push!(StackValue::ValueType(Box::new(span)));
        }
        "string System.String::Substring(int)" => {
            expect_stack!(let Int32(start_at) = pop!());
            let value = with_string!(pop!(), |s| s.split_at(start_at as usize).0.to_vec());
            push!(StackValue::string(gc, CLRString::new(value)));
        }
        "static System.Type System.Type::GetTypeFromHandle(valuetype System.RuntimeTypeHandle)" => {
            expect_stack!(let ValueType(handle) = pop!());
            let target = ObjectRef::read(handle.instance_storage.get_field("_value"));
            push!(StackValue::ObjectRef(target));
        }
        "static bool System.Type::op_Equality(System.Type, System.Type)" => {
            expect_stack!(let ObjectRef(o2) = pop!());
            expect_stack!(let ObjectRef(o1) = pop!());
            push!(StackValue::Int32((o1 == o2) as i32));
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
        "static string System.String::Empty" => stack.push_stack(gc, StackValue::string(gc, CLRString::new(vec![]))),
        x => panic!("unsupported load from intrinsic field {}", x),
    }

    stack.increment_ip();
}
