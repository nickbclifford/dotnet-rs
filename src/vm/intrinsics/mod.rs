pub mod reflection;
use reflection::RuntimeType;

use crate::{
    resolve::SUPPORT_ASSEMBLY,
    utils::decompose_type_source,
    value::{
        layout::*,
        string::{string_intrinsic_call, with_string, CLRString},
        ConcreteType, Context, FieldDescription, GenericLookup, HeapStorage, MethodDescription,
        Object, ObjectRef, StackValue,
    },
    vm::{intrinsics::reflection::runtime_type_intrinsic_call, CallStack, GCHandle, MethodInfo},
};

use dotnetdll::prelude::*;

macro_rules! expect_stack {
    (let $variant:ident ( $inner:ident $(as $t:ty)? ) = $v:expr) => {
        let $inner = match $v {
            StackValue::$variant($inner) => $inner,
            err => panic!(
                "invalid type on stack ({:?}), expected {}",
                err,
                stringify!($variant)
            ),
        };
        $(
            let $inner = $inner as $t;
        )?
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

pub fn span_to_slice<'gc, 'a>(span: Object<'gc>) -> &'a [u8] {
    let mut ptr_data = [0u8; size_of::<usize>()];
    let mut len_data = [0u8; size_of::<i32>()];

    ptr_data.copy_from_slice(span.instance_storage.get_field("_reference"));
    len_data.copy_from_slice(span.instance_storage.get_field("_length"));

    let ptr = usize::from_ne_bytes(ptr_data);
    let len = i32::from_ne_bytes(len_data) as usize;

    unsafe { std::slice::from_raw_parts(ptr as *const u8, len) }
}

const STATIC_ARRAY_TYPE_PREFIX: &str = "__StaticArrayInitTypeSize=";

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
    macro_rules! ctx {
        () => {
            Context::with_generics(stack.current_context(), &generics)
        };
    }

    super::msg!(stack, "-- method marked as runtime intrinsic --");

    if method.parent.definition.type_name() == "DotnetRs.RuntimeType" {
        return runtime_type_intrinsic_call(gc, stack, method, generics);
    }

    if method.parent.definition.type_name() == "System.String"
        && method.method.name != "op_Implicit"
    {
        return string_intrinsic_call(gc, stack, method, generics);
    }

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

                    let desc = MethodDescription {
                        parent: td,
                        method: m
                    };

                    stack.constructor_frame(
                        gc,
                        instance,
                        MethodInfo::new(desc, &new_lookup, stack.assemblies),
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
        "static System.Collections.Generic.EqualityComparer`1<T0> System.Collections.Generic.EqualityComparer`1::get_Default()" => {
            let target = MethodType::TypeGeneric(0);
            let rt = stack.get_runtime_type(gc, RuntimeType {
                target,
                source: method,
                generics,
            });
            push!(StackValue::ObjectRef(rt));

            let parent = stack.assemblies.find_in_assembly(
                &ExternalAssemblyReference::new(SUPPORT_ASSEMBLY),
                "DotnetRs.Comparers.Equality"
            );
            let method = parent.definition.methods.iter().find(|m| m.name == "GetDefault").unwrap();
            stack.call_frame(
                gc,
                MethodInfo::new(MethodDescription { parent, method }, &GenericLookup::default(), stack.assemblies),
                GenericLookup::default()
            );
            return;
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
        "static void System.Runtime.CompilerServices.RuntimeHelpers::RunClassConstructor(valuetype System.RuntimeTypeHandle)" => {
            // https://github.com/dotnet/runtime/blob/main/src/libraries/System.Private.CoreLib/src/System/SR.cs#L78
            expect_stack!(let ValueType(handle) = pop!());
            let rt = ObjectRef::read(handle.instance_storage.get_field("_value"));
            let target: RuntimeType = rt.try_into().unwrap();
            let target: ConcreteType = target.into();
            let target = stack.assemblies.find_concrete_type(target);
            if stack.initialize_static_storage(gc, target, generics) {
                let second_to_last = stack.frames.len() - 2;
                let ip = &mut stack.frames[second_to_last].state.ip;
                *ip += 1;
                let i = *ip;
                super::msg!(stack, "-- explicit initialization! setting return ip to {} --", i);
                return;
            }
        }
        "[Generic(1)] static valuetype System.ReadOnlySpan`1<M0> System.Runtime.CompilerServices.RuntimeHelpers::CreateSpan(valuetype System.RuntimeFieldHandle)" => {
            let target = &generics.method_generics[0];
            let target_size = type_layout(target.clone(), ctx!()).size();
            expect_stack!(let ValueType(field_handle) = pop!());

            let mut idx_buf = [0u8; size_of::<usize>()];
            idx_buf.copy_from_slice(field_handle.instance_storage.get_field("_value"));
            let idx = usize::from_ne_bytes(idx_buf);
            let (FieldDescription { field, .. }, lookup) = &stack.runtime_fields[idx];
            let field_type = Context::with_generics(ctx!(), lookup).make_concrete(&field.return_type);
            let field_desc = stack.assemblies.find_concrete_type(field_type.clone());

            let Some(data) = &field.initial_value else { todo!("ArgumentException: field has no initial value") };

            if field_desc.definition.name.starts_with(STATIC_ARRAY_TYPE_PREFIX) {
                let size_str = field_desc.definition.name.split_at(STATIC_ARRAY_TYPE_PREFIX.len()).1;
                let end_idx = match size_str.find('_') {
                    None => size_str.len(),
                    Some(i) => i
                };
                let size_txt = &size_str[..end_idx];
                let size = size_txt.parse::<usize>().unwrap();
                let data = &data[0..size];

                let span = stack.assemblies.corlib_type("System.ReadOnlySpan`1");
                let span_lookup = GenericLookup::new(vec![field_type]);
                let mut instance = Object::new(span, Context::with_generics(ctx!(), &span_lookup));
                instance.instance_storage.get_field_mut("_reference").copy_from_slice(&(data.as_ptr() as usize).to_ne_bytes());
                instance.instance_storage.get_field_mut("_length").copy_from_slice(&((size / target_size) as i32).to_ne_bytes());

                push!(StackValue::ValueType(Box::new(instance)));
            } else {
                todo!("initial field data for {:?}", field_desc);
            }
        }
        "[Generic(1)] static bool System.Runtime.CompilerServices.RuntimeHelpers::IsBitwiseEquatable()" => {
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());
            let value = match layout {
                LayoutManager::Scalar(Scalar::ObjectRef) => false,
                LayoutManager::Scalar(_) => true,
                _ => false
            };
            push!(StackValue::Int32(value as i32));
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
        "[Generic(1)] static bool System.Runtime.CompilerServices.Unsafe::AreSame(ref M0, ref M0)" => {
            let m1 = ref_as_ptr(pop!());
            let m2 = ref_as_ptr(pop!());
            push!(StackValue::Int32((m1 == m2) as i32));
        }
        "[Generic(2)] static ref M1 System.Runtime.CompilerServices.Unsafe::As(ref M0)" => {
            let target_type = stack.assemblies.find_concrete_type(generics.method_generics[1].clone());
            let m = ref_as_ptr(pop!());
            push!(StackValue::managed_ptr(m, target_type));
        }
        "[Generic(1)] static ref M0 System.Runtime.CompilerServices.Unsafe::AsRef(ref M0)" => {
            // I think this is a noop since none of the types or locations are actually changing
        }
        "[Generic(1)] static nint System.Runtime.CompilerServices.Unsafe::ByteOffset(ref M0, ref M0)" => {
            let r = ref_as_ptr(pop!());
            let l = ref_as_ptr(pop!());
            let offset = (l as isize) - (r as isize);
            push!(StackValue::NativeInt(offset));
        }
        "[Generic(1)] static M0 System.Runtime.CompilerServices.Unsafe::ReadUnaligned(void*)" => {
            expect_stack!(let NativeInt(ptr) = pop!());
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());

            macro_rules! read_ua {
                ($t:ty) => {
                    unsafe {
                        std::ptr::read_unaligned(ptr as *const $t)
                    }
                };
            }

            let v = match layout {
                LayoutManager::Scalar(s) => match s {
                    Scalar::ObjectRef => StackValue::ObjectRef(read_ua!(ObjectRef)),
                    Scalar::Int8 => StackValue::Int32(read_ua!(i8) as i32),
                    Scalar::Int16 => StackValue::Int32(read_ua!(i16) as i32),
                    Scalar::Int32 => StackValue::Int32(read_ua!(i32)),
                    Scalar::Int64 => StackValue::Int64(read_ua!(i64)),
                    Scalar::NativeInt => StackValue::NativeInt(read_ua!(isize)),
                    Scalar::Float32 => StackValue::NativeFloat(read_ua!(f32) as f64),
                    Scalar::Float64 => StackValue::NativeFloat(read_ua!(f64)),
                },
                _ => todo!("unsupported layout for read unaligned"),
            };
            push!(v);
        }
        "[Generic(1)] static void System.Runtime.CompilerServices.Unsafe::WriteUnaligned(void*, M0)" => {
            // equivalent to unaligned.stobj
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());
            let value = pop!();
            expect_stack!(let NativeInt(ptr) = pop!());

            macro_rules! write_ua {
                ($variant:ident, $t:ty) => {{
                    expect_stack!(let $variant(v) = value);
                    unsafe {
                        std::ptr::write_unaligned(ptr as *mut _, v as $t);
                    }
                }};
            }

            match layout {
                LayoutManager::Scalar(s) => match s {
                    Scalar::ObjectRef => write_ua!(ObjectRef, ObjectRef),
                    Scalar::Int8 => write_ua!(Int32, i8),
                    Scalar::Int16 => write_ua!(Int32, i16),
                    Scalar::Int32 => write_ua!(Int32, i32),
                    Scalar::Int64 => write_ua!(Int64, i64),
                    Scalar::NativeInt => write_ua!(NativeInt, isize),
                    Scalar::Float32 => write_ua!(NativeFloat, f32),
                    Scalar::Float64 => write_ua!(NativeFloat, f64),
                }
                _ => todo!("unsupported layout for write unaligned"),
            }
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
            use std::sync::atomic::{AtomicI32, Ordering};

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
            // note that this method's signature restricts the generic to only reference types
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
        "static valuetype System.ReadOnlySpan`1<char> System.String::op_Implicit(string)" |
        "static valuetype System.ReadOnlySpan`1<char> System.MemoryExtensions::AsSpan(string)" => {
            let (ptr, len) = with_string!(pop!(), |s| (s.as_ptr(), s.len()));

            let span_type = stack.assemblies.corlib_type("System.ReadOnlySpan`1");
            let new_lookup = GenericLookup::new(vec![ctx!().make_concrete(&BaseType::Char)]);
            let ctx = Context::with_generics(ctx!(), &new_lookup);

            let mut span = Object::new(span_type, ctx);

            span.instance_storage.get_field_mut("_reference").copy_from_slice(&(ptr as usize).to_ne_bytes());
            span.instance_storage.get_field_mut("_length").copy_from_slice(&(len as i32).to_ne_bytes());

            push!(StackValue::ValueType(Box::new(span)));
        }
        "static System.Type System.Type::GetTypeFromHandle(valuetype System.RuntimeTypeHandle)" => {
            expect_stack!(let ValueType(handle) = pop!());
            let target = ObjectRef::read(handle.instance_storage.get_field("_value"));
            push!(StackValue::ObjectRef(target));
        }
        "bool System.Type::get_IsValueType()" => {
            expect_stack!(let ObjectRef(o) = pop!());
            let target: RuntimeType = o.try_into().unwrap();
            let target: ConcreteType = target.into();
            let target = stack.assemblies.find_concrete_type(target);

            let value = target.is_value_type(&ctx!());
            push!(StackValue::Int32(value as i32));
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
        "static string System.String::Empty" => {
            stack.push_stack(gc, StackValue::string(gc, CLRString::new(vec![])))
        }
        x => panic!("unsupported load from intrinsic field {}", x),
    }

    stack.increment_ip();
}
