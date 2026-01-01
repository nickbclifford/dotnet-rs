pub mod matcher;
pub mod reflection;

use crate::{
    any_match_field, any_match_method, match_field, match_method,
    resolve::SUPPORT_ASSEMBLY,
    utils::decompose_type_source,
    value::{
        layout::*,
        string::{string_intrinsic_call, with_string, CLRString},
        ConcreteType, FieldDescription, GenericLookup, HeapStorage, MethodDescription, Object,
        ObjectRef, ResolutionContext, StackValue,
    },
    vm::{
        intrinsics::reflection::{
            runtime_field_info_intrinsic_call, runtime_method_info_intrinsic_call,
            runtime_type_intrinsic_call,
        },
        CallStack, GCHandle, MethodInfo, StepResult,
    },
};

use dotnetdll::prelude::*;

pub const INTRINSIC_ATTR: &str = "System.Runtime.CompilerServices.IntrinsicAttribute";

pub fn is_intrinsic(method: MethodDescription, assemblies: &crate::resolve::Assemblies) -> bool {
    if method.method.internal_call {
        return true;
    }

    for a in &method.method.attributes {
        let ctor = assemblies.locate_attribute(method.resolution(), a);
        if ctor.parent.type_name() == INTRINSIC_ATTR {
            return true;
        }
    }

    if any_match_method!(method,
        [static System.Environment::GetEnvironmentVariableCore(string)],
        [System.String::GetPinnableReference()],
        [System.String::get_Length()],
        [System.String::get_Chars(int)],
        [System.String::GetRawStringData()],
        [System.String::IndexOf(char)],
        [System.String::IndexOf(char, int)],
        [System.String::Substring(int)],
        [System.String::GetHashCodeOrdinalIgnoreCase()],
        [static System.String::Concat(ReadOnlySpan<char>, ReadOnlySpan<char>, ReadOnlySpan<char>)],
        [static System.Runtime.CompilerServices.RuntimeHelpers::RunClassConstructor(System.RuntimeTypeHandle)],
        [System.Type::get_TypeHandle()]
    ) {
        return true;
    }

    if method.parent.type_name() == "DotnetRs.RuntimeType" {
        return true;
    }
    if method.parent.type_name() == "DotnetRs.MethodInfo"
        || method.parent.type_name() == "DotnetRs.ConstructorInfo"
    {
        return true;
    }
    if method.parent.type_name() == "DotnetRs.FieldInfo" {
        return true;
    }

    false
}

pub fn is_intrinsic_field(
    field: FieldDescription,
    assemblies: &crate::resolve::Assemblies,
) -> bool {
    for a in &field.field.attributes {
        let ctor = assemblies.locate_attribute(field.parent.resolution, a);
        if ctor.parent.type_name() == INTRINSIC_ATTR {
            return true;
        }
    }

    if any_match_field!(field,
        [static System.IntPtr::Zero],
        [static System.String::Empty]
    ) {
        return true;
    }

    false
}

pub fn span_to_slice<'gc, 'a>(span: Object<'gc>) -> &'a [u8] {
    let mut ptr_data = [0u8; ObjectRef::SIZE];
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
) -> StepResult {
    macro_rules! pop {
        () => {
            vm_pop!(stack)
        };
    }
    macro_rules! push {
        ($($args:tt)*) => {
            vm_push!(stack, gc, $($args)*)
        };
    }
    macro_rules! msg {
        ($($args:tt)*) => {
            vm_msg!($($args)*)
        }
    }
    let ctx = ResolutionContext::for_method(method, stack.assemblies, &generics);
    macro_rules! ctx {
        () => {
            &ctx
        };
    }

    msg!(stack, "-- method marked as runtime intrinsic --");

    if method.parent.definition.type_name() == "DotnetRs.RuntimeType" {
        return runtime_type_intrinsic_call(gc, stack, method, generics);
    }

    if method.parent.definition.type_name() == "DotnetRs.MethodInfo"
        || method.parent.definition.type_name() == "DotnetRs.ConstructorInfo"
    {
        return runtime_method_info_intrinsic_call(gc, stack, method, generics);
    }

    if method.parent.definition.type_name() == "DotnetRs.FieldInfo" {
        return runtime_field_info_intrinsic_call(gc, stack, method, generics);
    }

    if method.parent.definition.type_name() == "System.String"
        && method.method.name != "op_Implicit"
    {
        return string_intrinsic_call(gc, stack, method, generics);
    }

    match_method!(method, {
        [static System.Activator::CreateInstance<1>()] => {
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
            let new_ctx = ctx!().with_generics(&new_lookup);

            let instance = Object::new(td, &new_ctx);

            for m in &td.definition.methods {
                if m.runtime_special_name
                    && m.name == ".ctor"
                    && m.signature.instance
                    && m.signature.parameters.is_empty()
                {
                    msg!(
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
                    return StepResult::InstructionStepped;
                }
            }

            panic!("could not find a parameterless constructor in {:?}", td)
        },
        [static System.ArgumentNullException::ThrowIfNull(object, string)] => {
            let _param_name = pop!();
            let target = pop!();
            if let StackValue::ObjectRef(ObjectRef(None)) = target {
                return stack.throw_by_name(gc, "System.ArgumentNullException");
            }
        },
        [static System.Buffer::Memmove<1>(ref !!0, ref !!0, nint)] => {
            vm_expect_stack!(let NativeInt(len) = pop!());
            let src = pop!().as_ptr();
            let dst = pop!().as_ptr();

            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());
            let total_count = len as usize * layout.size();

            unsafe {
                std::ptr::copy(src, dst, total_count);
            }
        },
        [static System.Runtime.CompilerServices.RuntimeHelpers::GetMethodTable(object)] => {
            let obj = pop!();
            let object_type = match obj {
                StackValue::ObjectRef(ObjectRef(Some(h))) => ctx!().get_heap_description(h),
                StackValue::ObjectRef(ObjectRef(None)) => todo!("GetMethodTable(null)"),
                _ => panic!("GetMethodTable called on non-object: {:?}", obj),
            };

            // Check if we already have a method table for this type
            let mt_ptr = stack
                .method_tables
                .borrow()
                .get(&object_type)
                .map(|p| p.as_ref().as_ptr() as isize);
            if let Some(ptr) = mt_ptr {
                push!(StackValue::NativeInt(ptr));
            } else {
                // Otherwise create it
                let mt_type = stack
                    .assemblies
                    .corlib_type("System.Runtime.CompilerServices.MethodTable");
                let mt_ctx = stack.current_context().for_type(mt_type);
                let layout = FieldLayoutManager::instance_fields(mt_type, &mt_ctx);

                let mut data = vec![0u8; layout.total_size].into_boxed_slice();

                // Find Flags field
                if let Some(field_layout) = layout.fields.get("Flags") {
                    let mut flags: u32 = 0;
                    if object_type.has_finalizer(ctx!()) {
                        flags |= 0x100000;
                    }
                    data[field_layout.position..field_layout.position + 4]
                        .copy_from_slice(&flags.to_ne_bytes());
                }

                let ptr = data.as_ptr();
                stack.method_tables.borrow_mut().insert(object_type, data);
                push!(StackValue::NativeInt(ptr as isize));
            }
        },
        [static System.Environment::GetEnvironmentVariableCore(string)] => {
            let value = with_string!(stack, gc, pop!(), |s| {
                std::env::var(s.as_string())
            });
            match value.ok() {
                Some(s) => push!(StackValue::string(gc, CLRString::from(s))),
                None => push!(StackValue::null()),
            }
        },
        [static "System.Collections.Generic.EqualityComparer`1"::get_Default()] => {
            let target = MethodType::TypeGeneric(0);
            let rt = stack.get_runtime_type(gc, stack.make_runtime_type(&stack.current_context(), &target));
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
            return StepResult::InstructionStepped;
        },
        [static System.GC::_SuppressFinalize(object)]
        | [static System.GC::SuppressFinalizeInternal(object)] => {
            // TODO(gc): this object's finalizer should not be called
            let _obj = pop!();
        },
        [static System.MemoryExtensions::Equals(ReadOnlySpan<char>, ReadOnlySpan<char>, System.StringComparison)] => {
            vm_expect_stack!(let Int32(_culture_comparison) = pop!()); // TODO(i18n): respect StringComparison rules
            vm_expect_stack!(let ValueType(b) = pop!());
            vm_expect_stack!(let ValueType(a) = pop!());

            let a = span_to_slice(*a);
            let b = span_to_slice(*b);

            push!(StackValue::Int32((a == b) as i32));
        },
        [static System.Runtime.CompilerServices.RuntimeHelpers::RunClassConstructor(System.RuntimeTypeHandle)] => {
            // https://github.com/dotnet/runtime/blob/main/src/libraries/System.Private.CoreLib/src/System/SR.cs#L78
            vm_expect_stack!(let ValueType(handle) = pop!());
            let rt = ObjectRef::read(handle.instance_storage.get_field("_value"));
            let target = stack.resolve_runtime_type(rt.expect_object_ref());
            let target: ConcreteType = target.to_concrete(stack.assemblies);
            let target = stack.assemblies.find_concrete_type(target);
            if stack.initialize_static_storage(gc, target, generics) {
                let second_to_last = stack.frames.len() - 2;
                let ip = &mut stack.frames[second_to_last].state.ip;
                *ip += 1;
                let i = *ip;
                msg!(stack, "-- explicit initialization! setting return ip to {} --", i);
                return StepResult::InstructionStepped;
            }
        },
        [static System.Runtime.CompilerServices.RuntimeHelpers::CreateSpan<1>(System.RuntimeFieldHandle)] => {
            let target = &generics.method_generics[0];
            let target_size = type_layout(target.clone(), ctx!()).size();
            vm_expect_stack!(let ValueType(field_handle) = pop!());

            let mut idx_buf = [0u8; ObjectRef::SIZE];
            idx_buf.copy_from_slice(field_handle.instance_storage.get_field("_value"));
            let idx = usize::from_ne_bytes(idx_buf);
            let (FieldDescription { field, .. }, lookup) = &stack.runtime_fields[idx];
            let field_type = ctx!().with_generics(lookup).make_concrete(&field.return_type);
            let field_desc = stack.assemblies.find_concrete_type(field_type.clone());

            let Some(data) = &field.initial_value else { return stack.throw_by_name(gc, "System.ArgumentException") };

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
                let mut instance = Object::new(span, &ctx!().with_generics(&span_lookup));
                instance.instance_storage.get_field_mut("_reference").copy_from_slice(&(data.as_ptr() as usize).to_ne_bytes());
                instance.instance_storage.get_field_mut("_length").copy_from_slice(&((size / target_size) as i32).to_ne_bytes());

                push!(StackValue::ValueType(Box::new(instance)));
            } else {
                todo!("initial field data for {:?}", field_desc);
            }
        },
        [static System.Runtime.CompilerServices.RuntimeHelpers::IsBitwiseEquatable<1>()] => {
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());
            let value = match layout {
                LayoutManager::Scalar(Scalar::ObjectRef) => false,
                LayoutManager::Scalar(_) => true,
                _ => false
            };
            push!(StackValue::Int32(value as i32));
        },
        [static System.Runtime.CompilerServices.RuntimeHelpers::IsReferenceOrContainsReferences<1>()] => {
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());
            push!(StackValue::Int32(layout.is_or_contains_refs() as i32));
        },
        [static System.Runtime.CompilerServices.Unsafe::Add<1>(ref !!0, nint)] => {
            let target_type = stack.assemblies.find_concrete_type(generics.method_generics[0].clone());
            vm_expect_stack!(let NativeInt(offset) = pop!());
            let m = pop!().as_ptr();
            push!(StackValue::managed_ptr(unsafe { m.offset(offset) }, target_type));
        },
        [static System.Runtime.CompilerServices.Unsafe::AreSame<1>(ref !!0, ref !!0)] => {
            let m1 = pop!().as_ptr();
            let m2 = pop!().as_ptr();
            push!(StackValue::Int32((m1 == m2) as i32));
        },
        [static System.Runtime.CompilerServices.Unsafe::As<2>(ref !!0)] => {
            let target_type = stack.assemblies.find_concrete_type(generics.method_generics[1].clone());
            let m = pop!().as_ptr();
            push!(StackValue::managed_ptr(m, target_type));
        },
        [static System.Runtime.CompilerServices.Unsafe::AsRef<1>(ref !!0)] => {
            // I think this is a noop since none of the types or locations are actually changing
        },
        [static System.Runtime.CompilerServices.Unsafe::ByteOffset<1>(ref !!0, ref !!0)] => {
            let r = pop!().as_ptr();
            let l = pop!().as_ptr();
            let offset = (l as isize) - (r as isize);
            push!(StackValue::NativeInt(offset));
        },
        [static System.Runtime.CompilerServices.Unsafe::ReadUnaligned<1>(nint)]
        | [static System.Runtime.CompilerServices.Unsafe::ReadUnaligned<1>(* void)] => {
            vm_expect_stack!(let NativeInt(ptr) = pop!());
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
        },
        [static System.Runtime.CompilerServices.Unsafe::WriteUnaligned<1>(nint, !!0)] => {
            // equivalent to unaligned.stobj
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), ctx!());
            let value = pop!();
            vm_expect_stack!(let NativeInt(ptr) = pop!());

            macro_rules! write_ua {
                ($variant:ident, $t:ty) => {{
                    vm_expect_stack!(let $variant(v) = value);
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
        },
        [static System.Runtime.InteropServices.Marshal::GetLastPInvokeError()] => {
            let value = unsafe { super::pinvoke::LAST_ERROR };

            push!(StackValue::Int32(value));
        },
        [static System.Runtime.InteropServices.Marshal::SetLastPInvokeError(int)] => {
            vm_expect_stack!(let Int32(value) = pop!());

            unsafe {
                super::pinvoke::LAST_ERROR = value;
            }
        },
        [static System.Runtime.Intrinsics.Vector128::get_IsHardwareAccelerated()]
        | [static System.Runtime.Intrinsics.Vector256::get_IsHardwareAccelerated()]
        | [static System.Runtime.Intrinsics.Vector512::get_IsHardwareAccelerated()] => {
            // not in a million years, lol
            push!(StackValue::Int32(0));
        },
        ["System.ReadOnlySpan`1"::get_Item(int)] => {
            vm_expect_stack!(let Int32(index) = pop!());
            vm_expect_stack!(let ManagedPtr(m) = pop!());
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
        },
        ["System.Span`1"::get_Length()] | ["System.ReadOnlySpan`1"::get_Length()] => {
            vm_expect_stack!(let ManagedPtr(m) = pop!());
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
        },
        [static System.Threading.Interlocked::CompareExchange(ref int, int, int)] => {
            use std::sync::atomic::{AtomicI32, Ordering};

            vm_expect_stack!(let Int32(comparand) = pop!());
            vm_expect_stack!(let Int32(value) = pop!());
            let target = pop!().as_ptr() as *mut i32;

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
        },
        [static System.Threading.Monitor::Exit(object)] => {
            // TODO(threading): release mutex
            let _tag_object = pop!();
        },
        [static System.Threading.Monitor::ReliableEnter(object, ref bool)] => {
            let success_flag = pop!().as_ptr();

            // TODO(threading): actually acquire mutex
            let _tag_object = pop!();
            // eventually we'll set this properly to indicate success or failure
            // just make it always succeed for now
            unsafe {
                *success_flag = 1u8;
            }
        },
        [static System.Threading.Monitor::TryEnter_FastPath(object)] => {
            let _obj = pop!();
            // TODO(threading): actually acquire mutex
            push!(StackValue::Int32(1));
        },
        [static System.Threading.Volatile::Read<1>(ref !!0)] => {
            // note that this method's signature restricts the generic to only reference types
            let ptr = pop!().as_ptr() as *const ObjectRef<'gc>;

            let value = unsafe { std::ptr::read_volatile(ptr) };

            push!(StackValue::ObjectRef(value));
        },
        [static System.Threading.Volatile::Write(ref bool, bool)] => {
            vm_expect_stack!(let Int32(value) = pop!());
            let as_bool = value as u8;

            let src = pop!().as_ptr();

            unsafe { std::ptr::write_volatile(src, as_bool) };
        },
        [static System.String::op_Implicit(string)]
        | [static System.MemoryExtensions::AsSpan(string)] => {
            let (ptr, len) = with_string!(stack, gc, pop!(), |s| (s.as_ptr(), s.len()));

            let span_type = stack.assemblies.corlib_type("System.ReadOnlySpan`1");
            let new_lookup = GenericLookup::new(vec![ctx!().make_concrete(&BaseType::Char)]);
            let ctx = ctx!().with_generics(&new_lookup);

            let mut span = Object::new(span_type, &ctx);

            span.instance_storage.get_field_mut("_reference").copy_from_slice(&(ptr as usize).to_ne_bytes());
            span.instance_storage.get_field_mut("_length").copy_from_slice(&(len as i32).to_ne_bytes());

            push!(StackValue::ValueType(Box::new(span)));
        },
        [static System.Type::GetTypeFromHandle(System.RuntimeTypeHandle)] => {
            vm_expect_stack!(let ValueType(handle) = pop!());
            let target = ObjectRef::read(handle.instance_storage.get_field("_value"));
            push!(StackValue::ObjectRef(target));
        },
        [static System.Reflection.MethodBase::GetMethodFromHandle(System.RuntimeMethodHandle)] => {
            vm_expect_stack!(let ValueType(handle) = pop!());
            let target = ObjectRef::read(handle.instance_storage.get_field("_value"));
            push!(StackValue::ObjectRef(target));
        },
        [static System.Reflection.FieldInfo::GetFieldFromHandle(System.RuntimeFieldHandle)] => {
            vm_expect_stack!(let ValueType(handle) = pop!());
            let target = ObjectRef::read(handle.instance_storage.get_field("_value"));
            push!(StackValue::ObjectRef(target));
        },
        [static System.RuntimeTypeHandle::ToIntPtr(System.RuntimeTypeHandle)] => {
            vm_expect_stack!(let ValueType(handle) = pop!());
            let target = handle.instance_storage.get_field("_value");
            let val = usize::from_ne_bytes(target.try_into().unwrap());
            push!(StackValue::NativeInt(val as isize));
        },
        [System.RuntimeMethodHandle::GetFunctionPointer()] => {
            vm_expect_stack!(let ValueType(handle) = pop!());
            let method_obj = ObjectRef::read(handle.instance_storage.get_field("_value"));
            let (method, lookup) = stack.resolve_runtime_method(method_obj);
            let index = stack.get_runtime_method_index(gc, *method, lookup.clone());
            push!(StackValue::NativeInt(index as isize));
        },
        [System.Type::get_IsValueType()] => {
            vm_expect_stack!(let ObjectRef(o) = pop!());
            let target = stack.resolve_runtime_type(o);
            let target_ct = target.to_concrete(stack.assemblies);
            let target_desc = stack.assemblies.find_concrete_type(target_ct);
            let value = target_desc.is_value_type(ctx!());
            push!(StackValue::Int32(value as i32));
        },
        [static System.Type::op_Equality(any, any)] => {
            vm_expect_stack!(let ObjectRef(o2) = pop!());
            vm_expect_stack!(let ObjectRef(o1) = pop!());
            push!(StackValue::Int32((o1 == o2) as i32));
        },
        [System.Type::get_TypeHandle()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());

            let rth = stack.assemblies.corlib_type("System.RuntimeTypeHandle");
            let mut instance = Object::new(rth, ctx!());
            obj.write(instance.instance_storage.get_field_mut("_value"));

            push!(StackValue::ValueType(Box::new(instance)));
        },
    });

    stack.increment_ip();
    StepResult::InstructionStepped
}

pub fn intrinsic_field<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    field: FieldDescription,
    _type_generics: Vec<ConcreteType>,
) {
    match_field!(field, {
        [static System.IntPtr::Zero] => {
            stack.push_stack(gc, StackValue::NativeInt(0));
        },
        [static System.String::Empty] => {
            stack.push_stack(gc, StackValue::string(gc, CLRString::new(vec![])));
        },
    })
    .expect("unsupported load from intrinsic field");

    stack.increment_ip();
}
