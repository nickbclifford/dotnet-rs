use super::{CallStack, GCHandle, MethodInfo};
use crate::{
    utils::decompose_type_source,
    value::{
        layout::*, string::CLRString, ConcreteType, Context, FieldDescription, GenericLookup,
        HeapStorage, MethodDescription, Object, ObjectRef, StackValue, TypeDescription,
    },
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
use crate::resolve::SUPPORT_ASSEMBLY;
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

fn from_runtime_type(obj: ObjectRef) -> ConcreteType {
    obj.as_object(|instance| {
        let mut ct = [0u8; size_of::<usize>()];
        ct.copy_from_slice(instance.instance_storage.get_field("concreteType"));
        unsafe { &*(usize::from_ne_bytes(ct) as *const ConcreteType) }
    })
    .clone()
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
        "DotnetRs.Assembly DotnetRs.RuntimeType::GetAssembly()" => {
            expect_stack!(let ObjectRef(obj) = pop!());

            let target_type = from_runtime_type(obj);
            let target_type = stack.assemblies.find_concrete_type(target_type);

            let value = match stack.runtime_asms.get(&target_type.resolution) {
                Some(o) => *o,
                None => {
                    let resolution = obj.as_object(|i| i.description.resolution);
                    let definition = resolution.0.type_definitions
                        .iter()
                        .find(|a| a.type_name() == "DotnetRs.Assembly")
                        .unwrap();
                    let mut asm_handle = Object::new(TypeDescription { resolution, definition }, ctx!());
                    let data = (target_type.resolution.as_raw() as usize).to_ne_bytes();
                    asm_handle.instance_storage.get_field_mut("resolution").copy_from_slice(&data);
                    let v = ObjectRef::new(gc, HeapStorage::Obj(asm_handle));
                    stack.runtime_asms.insert(target_type.resolution, v);
                    v
                }
            };
            push!(StackValue::ObjectRef(value));
        }
        "string DotnetRs.RuntimeType::GetNamespace()" => {
            expect_stack!(let ObjectRef(obj) = pop!());
            let target = from_runtime_type(obj);
            let target = stack.assemblies.find_concrete_type(target);
            match &target.definition.namespace {
                Some(ns) => {
                    push!(StackValue::string(gc, CLRString::from(ns.as_ref())));
                },
                None => {
                    push!(StackValue::null());
                }
            }
        }
        "string DotnetRs.RuntimeType::GetName()" => {
            // just the regular name, not the fully qualified name
            // https://learn.microsoft.com/en-us/dotnet/api/system.reflection.memberinfo.name?view=net-9.0#remarks
            expect_stack!(let ObjectRef(obj) = pop!());
            let target = from_runtime_type(obj);
            let target = stack.assemblies.find_concrete_type(target);
            push!(StackValue::string(gc, CLRString::from(target.definition.name.as_ref())));
        }
        "valuetype [System.Runtime]System.RuntimeTypeHandle DotnetRs.RuntimeType::GetTypeHandle()" => {
            expect_stack!(let ObjectRef(obj) = pop!());

            let rth = stack.assemblies.corlib_type("System.RuntimeTypeHandle");
            let mut instance = Object::new(rth, ctx!());
            obj.write(instance.instance_storage.get_field_mut("_value"));

            push!(StackValue::ValueType(Box::new(instance)));
        }
        "[System.Runtime]System.Type DotnetRs.RuntimeType::MakeGenericType([System.Runtime]System.Type[])" => {
            expect_stack!(let ObjectRef(parameters) = pop!());
            expect_stack!(let ObjectRef(target) = pop!());
            let rt = from_runtime_type(target);
            match rt.get() {
                BaseType::Type { source, .. } => {
                    let (ut, _) = decompose_type_source(source);
                    let name = ut.type_name(rt.resolution().0);
                    let fragments: Vec<_> = name.split('`').collect();
                    if fragments.len() <= 1 {
                        todo!("ArgumentException: type is not generic")
                    }
                    let n_params: usize = fragments[1].parse().unwrap();
                    let mut params = vec![];
                    for i in 0..n_params {
                        parameters.as_vector(|v| {
                            let start = i * size_of::<ObjectRef>();
                            let end = start + size_of::<ObjectRef>();
                            let param = ObjectRef::read(&v.get()[start..end]);
                            params.push(from_runtime_type(param));
                        });
                    }
                    let new_type = ConcreteType::new(rt.resolution(), BaseType::Type {
                        source: TypeSource::generic(ut, params),
                        value_kind: None
                    });
                    let new_rt = stack.get_runtime_type(gc, new_type);
                    push!(StackValue::ObjectRef(new_rt));
                }
                _ => todo!("ArgumentException: cannot make generic type from {:?}", rt),
            }
        }
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
                        MethodInfo::new(td.resolution, m, &new_lookup, &stack.assemblies),
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
            let target = generics.type_generics[0].clone();
            let rt = stack.get_runtime_type(gc, target);
            push!(StackValue::ObjectRef(rt));

            let parent = stack.assemblies.find_in_assembly(
                &ExternalAssemblyReference::new(SUPPORT_ASSEMBLY),
                "DotnetRs.Comparers.Equality"
            );
            let method = parent.definition.methods.iter().find(|m| m.name == "GetDefault").unwrap();
            stack.call_frame(
                gc,
                MethodInfo::new(parent.resolution, method, &GenericLookup::default(), &stack.assemblies),
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
            let target = from_runtime_type(rt);
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
            let field_type = Context::with_generics(ctx!(), &lookup).make_concrete(&field.return_type);
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
        "static string System.String::Concat(valuetype System.ReadOnlySpan`1<char>, valuetype System.ReadOnlySpan`1<char>, valuetype System.ReadOnlySpan`1<char>)" => {
            expect_stack!(let ValueType(span2) = pop!());
            expect_stack!(let ValueType(span1) = pop!());
            expect_stack!(let ValueType(span0) = pop!());

            fn char_span_into_str(span: Object) -> Vec<u16> {
                span_to_slice(span).chunks_exact(2).map(|c| u16::from_ne_bytes([c[0], c[1]])).collect::<Vec<_>>()
            }

            let data0 = char_span_into_str(*span0);
            let data1 = char_span_into_str(*span1);
            let data2 = char_span_into_str(*span2);

            let value = CLRString::new(data0.into_iter().chain(data1.into_iter()).chain(data2.into_iter()).collect());
            push!(StackValue::string(gc, value));
        }
        "int System.String::GetHashCodeOrdinalIgnoreCase()" => {
            use std::hash::*;

            let mut h = DefaultHasher::new();
            let value = with_string!(pop!(), |s| String::from_utf16_lossy(&s).to_uppercase().into_bytes());
            value.hash(&mut h);
            let code = h.finish();

            push!(StackValue::Int32(code as i32));
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
        "bool System.Type::get_IsValueType()" => {
            expect_stack!(let ObjectRef(o) = pop!());
            let target = from_runtime_type(o);
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
