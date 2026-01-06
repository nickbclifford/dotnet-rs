use crate::{
    any_match_field, any_match_method,
    assemblies::{AssemblyLoader, SUPPORT_ASSEMBLY},
    match_field, match_method,
    types::{
        generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
    },
    utils::decompose_type_source,
    value::{
        layout::*,
        object::{HeapStorage, Object, ObjectRef},
        pointer::ManagedPtr,
        string::{string_intrinsic_call, with_string, CLRString},
        StackValue,
    },
    vm::{
        context::ResolutionContext,
        intrinsics::reflection::{
            runtime_field_info_intrinsic_call, runtime_method_info_intrinsic_call,
            runtime_type_intrinsic_call,
        },
        sync::{SyncBlockOps, SyncManagerOps},
        CallStack, GCHandle, GCHandleType, MethodInfo, StepResult,
    },
    vm_expect_stack, vm_msg, vm_pop, vm_push,
};
use dotnetdll::prelude::*;
use std::{
    env,
    ptr::{self, NonNull},
    slice,
    sync::atomic,
    thread,
};

pub mod matcher;
pub mod reflection;

pub const INTRINSIC_ATTR: &str = "System.Runtime.CompilerServices.IntrinsicAttribute";

pub fn is_intrinsic(method: MethodDescription, loader: &AssemblyLoader) -> bool {
    if method.method.internal_call {
        return true;
    }

    for a in &method.method.attributes {
        let ctor = loader.locate_attribute(method.resolution(), a);
        if ctor.parent.type_name() == INTRINSIC_ATTR {
            return true;
        }
    }

    any_match_method!(method,
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
        [System.Type::get_TypeHandle()],
        [static System.Runtime.InteropServices.Marshal::SizeOf(System.Type)],
        [static System.Runtime.InteropServices.Marshal::SizeOf<1>()],
        [static System.Runtime.InteropServices.Marshal::OffsetOf(System.Type, string)],
        [static System.Runtime.InteropServices.Marshal::OffsetOf<1>(string)],
        [static System.GC::Collect()],
        [static System.GC::Collect(int)],
        [static System.GC::Collect(int, System.GCCollectionMode)],
        [static System.GC::KeepAlive(object)],
        [static System.GC::WaitForPendingFinalizers()],
        [static System.GC::SuppressFinalize(object)],
        [static System.GC::ReRegisterForFinalize(object)],
    )
}

pub fn is_intrinsic_field(field: FieldDescription, loader: &AssemblyLoader) -> bool {
    for a in &field.field.attributes {
        let ctor = loader.locate_attribute(field.parent.resolution, a);
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
    let ptr_data = span.instance_storage.get_field("_reference");
    let mut len_data = [0u8; size_of::<i32>()];

    let ptr = ManagedPtr::read(ptr_data).value;
    len_data.copy_from_slice(span.instance_storage.get_field("_length"));

    let len = i32::from_ne_bytes(len_data) as usize;

    unsafe { slice::from_raw_parts(ptr.as_ptr() as *const u8, len) }
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
            vm_msg!(stack, $($args)*)
        }
    }
    let ctx = ResolutionContext::for_method(method, stack.loader(), &generics);

    msg!("-- method marked as runtime intrinsic --");

    if method.parent.definition().type_name() == "DotnetRs.RuntimeType" {
        return runtime_type_intrinsic_call(gc, stack, method, generics);
    }

    if method.parent.definition().type_name() == "DotnetRs.MethodInfo"
        || method.parent.definition().type_name() == "DotnetRs.ConstructorInfo"
    {
        return runtime_method_info_intrinsic_call(gc, stack, method, generics);
    }

    if method.parent.definition().type_name() == "DotnetRs.FieldInfo" {
        return runtime_field_info_intrinsic_call(gc, stack, method, generics);
    }

    if method.parent.definition().type_name() == "System.String"
        && method.method.name != "op_Implicit"
    {
        return string_intrinsic_call(gc, stack, method, generics);
    }

    macro_rules! pop_native {
        () => {{
            let val = pop!();
            match val {
                StackValue::NativeInt(i) => i,
                StackValue::ValueType(v)
                    if v.description.type_name() == "System.IntPtr"
                        || v.description.type_name() == "System.UIntPtr" =>
                {
                    let mut buf = [0u8; size_of::<isize>()];
                    buf.copy_from_slice(v.instance_storage.get_field("_value"));
                    isize::from_ne_bytes(buf)
                }
                _ => panic!("expected native int or IntPtr, got {:?}", val),
            }
        }};
    }

    match_method!(method, {
        [static System.Activator::CreateInstance<1>()] => {
            // Check GC safe point before reflection-based object instantiation
            stack.check_gc_safe_point();

            let target = &generics.method_generics[0];

            let mut type_generics = vec![];

            let td = match target.get() {
                BaseType::Object => stack.loader().corlib_type("System.Object"),
                BaseType::Type { source, .. } => {
                    let (ut, generics) = decompose_type_source(source);
                    type_generics = generics;
                    stack.loader().locate_type(target.resolution(), ut)
                }
                err => panic!(
                    "cannot call parameterless constructor on primitive type {:?}",
                    err
                ),
            };

            let new_lookup = GenericLookup::new(type_generics);
            let new_ctx = ctx.with_generics(&new_lookup);

            let instance = Object::new(td, &new_ctx);

            for m in &td.definition().methods {
                if m.runtime_special_name
                    && m.name == ".ctor"
                    && m.signature.instance
                    && m.signature.parameters.is_empty()
                {
                    msg!(
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
                        MethodInfo::new(desc, &new_lookup, stack.loader()),
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
            let layout = type_layout(target.clone(), &ctx);
            let total_count = len as usize * layout.size();

            // Check GC safe point before large bulk memory operations
            // Threshold: operations moving more than 4KB of data
            const LARGE_MEMMOVE_THRESHOLD: usize = 4096;
            if total_count > LARGE_MEMMOVE_THRESHOLD {
                stack.check_gc_safe_point();
            }

            unsafe {
                ptr::copy(src, dst, total_count);
            }
        },
        [static System.Runtime.InteropServices.MemoryMarshal::GetArrayDataReference<1>(!!0[])] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let Some(array_handle) = obj.0 else {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            };

            let data_ptr = match &array_handle.borrow().storage {
                HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
                _ => panic!("GetArrayDataReference called on non-array"),
            };

            let element_type = stack.loader().find_concrete_type(generics.method_generics[0].clone());
            push!(StackValue::managed_ptr(data_ptr, element_type, Some(array_handle), false));
        },
        [static System.Runtime.CompilerServices.RuntimeHelpers::GetMethodTable(object)] => {
            let obj = pop!();
            let object_type = match obj {
                StackValue::ObjectRef(ObjectRef(Some(h))) => ctx.get_heap_description(h),
                StackValue::ObjectRef(ObjectRef(None)) => return stack.throw_by_name(gc, "System.NullReferenceException"),
                _ => panic!("GetMethodTable called on non-object: {:?}", obj),
            };

            // Check if we already have a method table for this type
            let mt_ptr = stack
                .method_tables_read()
                .get(&object_type)
                .map(|p| p.as_ptr() as isize);
            if let Some(ptr) = mt_ptr {
                push!(NativeInt(ptr));
            } else {
                // Otherwise create it
                let mt_type = stack
                    .loader()
                    .corlib_type("System.Runtime.CompilerServices.MethodTable");
                let mt_ctx = stack.current_context().for_type(mt_type);
                let layout = FieldLayoutManager::instance_fields(mt_type, &mt_ctx);

                let mut data = vec![0u8; layout.total_size].into_boxed_slice();

                // Find Flags field
                if let Some(field_layout) = layout.fields.get("Flags") {
                    let mut flags: u32 = 0;
                    if object_type.has_finalizer(&ctx) {
                        flags |= 0x100000;
                    }
                    data[field_layout.position..field_layout.position + 4]
                        .copy_from_slice(&flags.to_ne_bytes());
                }

                let ptr = data.as_ptr();
                stack.method_tables_write().insert(object_type, data);
                push!(NativeInt(ptr as isize));
            }
        },
        [static System.Environment::GetEnvironmentVariableCore(string)] => {
            let value = with_string!(stack, gc, pop!(), |s| {
                env::var(s.as_string())
            });
            match value.ok() {
                Some(s) => push!(string(s)),
                None => push!(null()),
            }
        },
        [static "System.Collections.Generic.EqualityComparer`1"::get_Default()] => {
            let target = MethodType::TypeGeneric(0);
            let rt = stack.get_runtime_type(gc, stack.make_runtime_type(&stack.current_context(), &target));
            push!(ObjectRef(rt));

            let parent = stack.loader().find_in_assembly(
                &ExternalAssemblyReference::new(SUPPORT_ASSEMBLY),
                "DotnetRs.Comparers.Equality"
            );
            let method = parent.definition().methods.iter().find(|m| m.name == "GetDefault").unwrap();
            stack.call_frame(
                gc,
                MethodInfo::new(MethodDescription { parent, method }, &GenericLookup::default(), stack.loader()),
                GenericLookup::default()
            );
            return StepResult::InstructionStepped;
        },
        [static System.GC::SuppressFinalize(object)] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            if let Some(handle) = obj.0 {
                if let Some(o) = handle.borrow_mut(gc).storage.as_obj_mut() {
                    o.finalizer_suppressed = true;
                }
            }
        },
        [static System.GC::ReRegisterForFinalize(object)] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            if let Some(handle) = obj.0 {
                let is_obj = handle.borrow_mut(gc).storage.as_obj_mut().map(|o| {
                    o.finalizer_suppressed = false;
                    true
                }).unwrap_or(false);
                if is_obj {
                    stack.register_new_object(&obj);
                }
            }
        },
        [static System.GC::Collect()] => {
            stack.heap().needs_full_collect.set(true);
        },
        [static System.GC::Collect(int)] => {
            pop!();
            stack.heap().needs_full_collect.set(true);
        },
        [static System.GC::Collect(int, System.GCCollectionMode)] => {
            pop!();
            pop!();
            stack.heap().needs_full_collect.set(true);
        },
        [static System.GC::WaitForPendingFinalizers()] => {
            if !stack.heap().pending_finalization.borrow().is_empty() || stack.heap().processing_finalizer.get() {
                stack.current_frame_mut().state.ip -= 1;
                return StepResult::InstructionStepped;
            }
        },
        [static System.GC::KeepAlive(object)] => {
            pop!();
        },
        [static System.MemoryExtensions::Equals(ReadOnlySpan<char>, ReadOnlySpan<char>, System.StringComparison)] => {
            vm_expect_stack!(let Int32(_culture_comparison) = pop!()); // TODO(i18n): respect StringComparison rules
            vm_expect_stack!(let ValueType(b) = pop!());
            vm_expect_stack!(let ValueType(a) = pop!());

            let a = span_to_slice(*a);
            let b = span_to_slice(*b);

            push!(Int32((a == b) as i32));
        },
        [static System.Runtime.CompilerServices.RuntimeHelpers::RunClassConstructor(System.RuntimeTypeHandle)] => {
            // https://github.com/dotnet/runtime/blob/main/src/libraries/System.Private.CoreLib/src/System/SR.cs#L78
            vm_expect_stack!(let ValueType(handle) = pop!());
            let rt = ObjectRef::read(handle.instance_storage.get_field("_value"));
            let target = stack.resolve_runtime_type(rt.expect_object_ref());
            let target: ConcreteType = target.to_concrete(stack.loader());
            let target = stack.loader().find_concrete_type(target);
            if stack.initialize_static_storage(gc, target, generics) {
                let second_to_last = stack.execution.frames.len() - 2;
                let ip = &mut stack.execution.frames[second_to_last].state.ip;
                *ip += 1;
                let i = *ip;
                msg!("-- explicit initialization! setting return ip to {} --", i);
                return StepResult::InstructionStepped;
            }
        },
        [static System.Runtime.CompilerServices.RuntimeHelpers::CreateSpan<1>(System.RuntimeFieldHandle)] => {
            let element_type = &generics.method_generics[0];
            let element_size = type_layout(element_type.clone(), &ctx).size();
            vm_expect_stack!(let ValueType(field_handle) = pop!());

            // Extract the field index from RuntimeFieldHandle
            let field_index = {
                let mut ptr_buf = [0u8; ObjectRef::SIZE];
                ptr_buf.copy_from_slice(field_handle.instance_storage.get_field("_value"));
                let obj_ref = ObjectRef::read(&ptr_buf);
                let handle_obj = obj_ref.0.expect("Null pointer in RuntimeFieldHandle");

                match &handle_obj.borrow().storage {
                    HeapStorage::Obj(o) => {
                        let mut idx_buf = [0u8; size_of::<usize>()];
                        idx_buf.copy_from_slice(o.instance_storage.get_field("index"));
                        usize::from_ne_bytes(idx_buf)
                    }
                    _ => panic!("RuntimeFieldHandle._value does not point to an object"),
                }
            };

            let (FieldDescription { field, .. }, lookup) = stack.runtime_fields_read()[field_index].clone();
            let field_type = ctx.with_generics(&lookup).make_concrete(&field.return_type);
            let field_desc = stack.loader().find_concrete_type(field_type.clone());

            let Some(initial_data) = &field.initial_value else {
                return stack.throw_by_name(gc, "System.ArgumentException");
            };

            if field_desc.definition().name.starts_with(STATIC_ARRAY_TYPE_PREFIX) {
                // Parse the size from the type name (e.g., "__StaticArrayInitTypeSize=123")
                let size_str = &field_desc.definition().name[STATIC_ARRAY_TYPE_PREFIX.len()..];
                let size_end = size_str.find('_').unwrap_or(size_str.len());
                let array_size = size_str[..size_end].parse::<usize>().unwrap();
                let data_slice = &initial_data[..array_size];

                // Create a ReadOnlySpan<T> pointing to the static data
                let span_type = stack.loader().corlib_type("System.ReadOnlySpan`1");
                let span_lookup = GenericLookup::new(vec![field_type]);
                let mut span_instance = Object::new(span_type, &ctx.with_generics(&span_lookup));

                // Set the _reference field to point to the data
                let element_desc = stack.loader().find_concrete_type(element_type.clone());
                let data_ptr = StackValue::managed_ptr(data_slice.as_ptr() as *mut u8, element_desc, None, false);
                vm_expect_stack!(let ManagedPtr(m) = data_ptr);
                m.write(span_instance.instance_storage.get_field_mut("_reference"));

                // Set the _length field
                let element_count = (array_size / element_size) as i32;
                span_instance.instance_storage.get_field_mut("_length")
                    .copy_from_slice(&element_count.to_ne_bytes());

                push!(ValueType(Box::new(span_instance)));
            } else {
                todo!("initial field data for {:?}", field_desc);
            }
        },
        [static System.Runtime.CompilerServices.RuntimeHelpers::IsBitwiseEquatable<1>()] => {
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), &ctx);
            let value = match layout {
                LayoutManager::Scalar(Scalar::ObjectRef) => false,
                LayoutManager::Scalar(_) => true,
                _ => false
            };
            push!(Int32(value as i32));
        },
        [static System.Runtime.CompilerServices.RuntimeHelpers::IsReferenceOrContainsReferences<1>()] => {
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), &ctx);
            push!(Int32(layout.is_or_contains_refs() as i32));
        },
        [static System.Runtime.CompilerServices.Unsafe::Add<1>(ref !!0, nint)] => {
            let target = &generics.method_generics[0];
            let target_type = stack.loader().find_concrete_type(target.clone());
            let layout = type_layout(target.clone(), &ctx);
            vm_expect_stack!(let NativeInt(offset) = pop!());
            let m_val = pop!();
            let (owner, pinned) = if let StackValue::ManagedPtr(m) = &m_val {
                (m.owner, m.pinned)
            } else {
                (None, false)
            };
            let m = m_val.as_ptr();
            push!(managed_ptr(
                unsafe { m.offset(offset * layout.size() as isize) },
                target_type,
                owner,
                pinned
            ));
        },
        [static System.Runtime.CompilerServices.Unsafe::AreSame<1>(ref !!0, ref !!0)] => {
            let m1 = pop!().as_ptr();
            let m2 = pop!().as_ptr();
            push!(Int32((m1 == m2) as i32));
        },
        [static System.Runtime.CompilerServices.Unsafe::As<1>(object)]
        | [static System.Runtime.CompilerServices.Unsafe::AsRef<1>(ref !!0)] => {
            let o = pop!();
            push!(o);
        },
        [static System.Runtime.CompilerServices.Unsafe::As<2>(ref !!0)] => {
            let target_type = stack.loader().find_concrete_type(generics.method_generics[1].clone());
            let m_val = pop!();
            let (owner, pinned) = match &m_val {
                StackValue::ManagedPtr(m) => (m.owner, m.pinned),
                StackValue::ObjectRef(ObjectRef(Some(h))) => (Some(*h), false),
                _ => (None, false)
            };
            let m = m_val.as_ptr();
            push!(managed_ptr(m, target_type, owner, pinned));
        },
        [static System.Runtime.CompilerServices.Unsafe::AsRef<1>(* void)] => {
            let target_type = stack.loader().find_concrete_type(generics.method_generics[0].clone());
            vm_expect_stack!(let NativeInt(ptr) = pop!());
            push!(managed_ptr(ptr as *mut u8, target_type, None, false));
        },
        [static System.Runtime.CompilerServices.Unsafe::SizeOf<1>()] => {
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), &ctx);
            push!(Int32(layout.size() as i32));
        },
        [static System.Runtime.CompilerServices.Unsafe::ByteOffset<1>(ref !!0, ref !!0)] => {
            let r = pop!().as_ptr();
            let l = pop!().as_ptr();
            let offset = (l as isize) - (r as isize);
            push!(NativeInt(offset));
        },
        [static System.Runtime.CompilerServices.Unsafe::ReadUnaligned<1>(nint)]
        | [static System.Runtime.CompilerServices.Unsafe::ReadUnaligned<1>(* void)] => {
            vm_expect_stack!(let NativeInt(ptr) = pop!());
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), &ctx);

            macro_rules! read_ua {
                ($t:ty) => {
                    unsafe {
                        ptr::read_unaligned(ptr as *const $t)
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
                    Scalar::ManagedPtr => StackValue::ManagedPtr(read_ua!(ManagedPtr<'gc>)),
                },
                _ => panic!("unsupported layout for read unaligned"),
            };
            push!(v);
        },
        [static System.Runtime.CompilerServices.Unsafe::WriteUnaligned<1>(nint, !!0)] => {
            // equivalent to unaligned.stobj
            let target = &generics.method_generics[0];
            let layout = type_layout(target.clone(), &ctx);
            let value = pop!();
            vm_expect_stack!(let NativeInt(ptr) = pop!());

            macro_rules! write_ua {
                ($variant:ident, $t:ty) => {{
                    vm_expect_stack!(let $variant(v) = value);
                    unsafe {
                        ptr::write_unaligned(ptr as *mut _, v as $t);
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
                    Scalar::ManagedPtr => write_ua!(ManagedPtr, ManagedPtr<'gc>),
                }
                _ => panic!("unsupported layout for write unaligned"),
            }
        },
        [static System.Runtime.InteropServices.Marshal::GetLastPInvokeError()] => {
            let value = unsafe { super::pinvoke::LAST_ERROR };

            push!(Int32(value));
        },
        [static System.Runtime.InteropServices.Marshal::SizeOf(System.Type)]
        | [static System.Runtime.InteropServices.Marshal::SizeOf<1>()] => {
            let concrete_type = if method.method.signature.parameters.is_empty() {
                generics.method_generics[0].clone()
            } else {
                let type_obj = match pop!() {
                    StackValue::ObjectRef(o) => o,
                    rest => panic!("Marshal.SizeOf(Type) called on non-object: {:?}", rest),
                };
                stack.resolve_runtime_type(type_obj).to_concrete(stack.loader())
            };
            let layout = type_layout(concrete_type, &ctx);
            push!(Int32(layout.size() as i32));
        },
        [static System.Runtime.InteropServices.Marshal::OffsetOf(System.Type, string)]
        | [static System.Runtime.InteropServices.Marshal::OffsetOf<1>(string)] => {
            let field_name_val = pop!();
            let field_name = with_string!(stack, gc, field_name_val, |s| s.as_string());
            let concrete_type = if method.method.signature.parameters.len() == 1 {
                generics.method_generics[0].clone()
            } else {
                let type_obj = match pop!() {
                    StackValue::ObjectRef(o) => o,
                    rest => panic!("Marshal.OffsetOf(Type, string) called on non-object: {:?}", rest),
                };
                stack.resolve_runtime_type(type_obj).to_concrete(stack.loader())
            };
            let layout = type_layout(concrete_type.clone(), &ctx);

            if let LayoutManager::FieldLayoutManager(flm) = layout {
                if let Some(field) = flm.fields.get(&field_name) {
                    push!(NativeInt(field.position as isize));
                } else {
                    panic!("Field {} not found in type {:?}", field_name, concrete_type);
                }
            } else {
                panic!("Type {:?} does not have field layout", concrete_type);
            }
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
            push!(Int32(0));
        },
        ["System.ReadOnlySpan`1"::get_Item(int)] => {
            vm_expect_stack!(let Int32(index) = pop!());
            vm_expect_stack!(let ManagedPtr(m) = pop!());
            if !m.inner_type.type_name().contains("Span") {
                panic!("invalid type on stack");
            }
            let span_layout = FieldLayoutManager::instance_fields(
                m.inner_type,
                &ctx
            );

            let value_type = &generics.type_generics[0];
            let value_layout = type_layout(value_type.clone(), &ctx);

            let ptr = unsafe {
                m.value
                    // find the span's base pointer
                    .add(span_layout.fields["_reference"].position)
                    // navigate to the specified index
                    .add(value_layout.size() * index as usize)
            };
            push!(managed_ptr(
                ptr.as_ptr(),
                stack.loader().find_concrete_type(value_type.clone()),
                m.owner,
                m.pinned
            ));
        },
        ["System.Span`1"::get_Length()] | ["System.ReadOnlySpan`1"::get_Length()] => {
            vm_expect_stack!(let ManagedPtr(m) = pop!());
            if !m.inner_type.type_name().contains("Span") {
                panic!("invalid type on stack");
            }
            let layout = FieldLayoutManager::instance_fields(
                m.inner_type,
                &ctx
            );
            let value = unsafe {
                let target = m.value.as_ptr().add(layout.fields["_length"].position) as *const i32;
                *target
            };
            push!(Int32(value));
        },
        [static System.Threading.Interlocked::CompareExchange(ref int, int, int)] => {
            use atomic::AtomicI32;
            use crate::vm::sync::Ordering;

            vm_expect_stack!(let Int32(comparand) = pop!());
            vm_expect_stack!(let Int32(value) = pop!());
            let target = pop!().as_ptr() as *mut i32;

            // Use SeqCst ordering to match .NET's strong memory model guarantees
            let atomic_view = unsafe { AtomicI32::from_ptr(target) };
            let prev = match atomic_view.compare_exchange(
                comparand,
                value,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(prev) | Err(prev) => prev,
            };

            push!(Int32(prev));
        },
        [static System.Threading.Monitor::Exit(object)] => {
            vm_expect_stack!(let ObjectRef(obj_ref) = pop!());

            if obj_ref.0.is_some() {
                // Get the current thread ID from the call stack
                let thread_id = stack.thread_id.get();
                assert_ne!(thread_id, 0,"Monitor.Exit called from unregistered thread");

                // Get or access the sync block
                let sync_block_index = obj_ref.as_object(|o| o.sync_block_index);

                if let Some(index) = sync_block_index {
                    if let Some(sync_block) = stack.shared.sync_blocks.get_sync_block(index) {
                        if !sync_block.exit(thread_id) {
                            panic!("SynchronizationLockException: Object synchronization method was called from an unsynchronized block of code.");
                        }
                    }
                }
            } else {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
        },
        [static System.Threading.Monitor::ReliableEnter(object, ref bool)] => {
            let success_flag = pop!().as_ptr();
            vm_expect_stack!(let ObjectRef(obj_ref) = pop!());

            if obj_ref.0.is_some() {
                // Get the current thread ID from the call stack
                let thread_id = stack.thread_id.get();
                assert_ne!(thread_id, 0,"Monitor.ReliableEnter called from unregistered thread");

                // Get or create sync block
                let (_index, sync_block) = stack.shared.sync_blocks.get_or_create_sync_block(
                    &obj_ref,
                    || obj_ref.as_object(|o| o.sync_block_index),
                    |new_index| {
                        obj_ref.as_object_mut(gc, |o| {
                            o.sync_block_index = Some(new_index);
                        });
                    },
                );

                // Enter the monitor
                while !stack.shared.sync_blocks.try_enter_block(sync_block.clone(), thread_id, &stack.shared.metrics) {
                    stack.check_gc_safe_point();
                    thread::yield_now();
                }

                // Set success flag
                unsafe {
                    *success_flag = 1u8;
                }
            } else {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
        },
        [static System.Threading.Monitor::TryEnter_FastPath(object)] => {
            vm_expect_stack!(let ObjectRef(obj_ref) = pop!());

            if obj_ref.0.is_some() {
                // Get the current thread ID from the call stack
                let thread_id = stack.thread_id.get();
                assert_ne!(thread_id, 0,"Monitor.TryEnter_FastPath called from unregistered thread");

                // Get or create sync block
                let (_index, sync_block) = stack.shared.sync_blocks.get_or_create_sync_block(
                    &obj_ref,
                    || obj_ref.as_object(|o| o.sync_block_index),
                    |new_index| {
                        obj_ref.as_object_mut(gc, |o| {
                            o.sync_block_index = Some(new_index);
                        });
                    },
                );
                let success = sync_block.try_enter(thread_id);

                push!(Int32(if success { 1 } else { 0 }));
            } else {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
        },
        [static System.Threading.Monitor::TryEnter(object, int, ref bool)] => {
            let success_flag = pop!().as_ptr();
            vm_expect_stack!(let Int32(timeout_ms) = pop!());
            vm_expect_stack!(let ObjectRef(obj_ref) = pop!());

            if obj_ref.0.is_some() {
                // Get the current thread ID from the call stack
                let thread_id = stack.thread_id.get();
                assert_ne!(thread_id, 0,"Monitor.TryEnter called from unregistered thread");

                let (_index, sync_block) = stack.shared.sync_blocks.get_or_create_sync_block(
                    &obj_ref,
                    || obj_ref.as_object(|o| o.sync_block_index),
                    |new_index| {
                        obj_ref.as_object_mut(gc, |o| {
                            o.sync_block_index = Some(new_index);
                        });
                    },
                );

                let success = sync_block.enter_with_timeout(thread_id, timeout_ms as u64, &stack.shared.metrics);

                unsafe {
                    *success_flag = if success { 1u8 } else { 0u8 };
                }
            } else {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
        },
        [static System.Threading.Monitor::TryEnter(object, int)] => {
            vm_expect_stack!(let Int32(timeout_ms) = pop!());
            vm_expect_stack!(let ObjectRef(obj_ref) = pop!());

            if obj_ref.0.is_some() {
                // Get the current thread ID from the call stack
                let thread_id = stack.thread_id.get();
                assert_ne!(thread_id, 0,"Monitor.TryEnter called from unregistered thread");

                let (_index, sync_block) = stack.shared.sync_blocks.get_or_create_sync_block(
                    &obj_ref,
                    || obj_ref.as_object(|o| o.sync_block_index),
                    |new_index| {
                        obj_ref.as_object_mut(gc, |o| {
                            o.sync_block_index = Some(new_index);
                        });
                    },
                );

                let success = sync_block.enter_with_timeout(thread_id, timeout_ms as u64, &stack.shared.metrics);
                push!(Int32(if success { 1 } else { 0 }));
            } else {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
        },
        [static System.Threading.Volatile::Read<1>(ref !!0)] => {
            // note that this method's signature restricts the generic to only reference types
            let ptr = pop!().as_ptr() as *const ObjectRef<'gc>;

            let value = unsafe { ptr::read_volatile(ptr) };
            // Ensure acquire semantics to match .NET memory model
            atomic::fence(crate::vm::sync::Ordering::Acquire);

            push!(ObjectRef(value));
        },
        [static System.Threading.Volatile::Write(ref bool, bool)] => {
            vm_expect_stack!(let Int32(value) = pop!());
            let as_bool = value as u8;

            let src = pop!().as_ptr();

            // Ensure release semantics to match .NET memory model
            atomic::fence(crate::vm::sync::Ordering::Release);
            unsafe { ptr::write_volatile(src, as_bool) };
        },
        [static System.String::op_Implicit(string)]
        | [static System.MemoryExtensions::AsSpan(string)] => {
            let (ptr, len) = with_string!(stack, gc, pop!(), |s| (s.as_ptr(), s.len()));

            let span_type = stack.loader().corlib_type("System.ReadOnlySpan`1");
            let new_lookup = GenericLookup::new(vec![ctx.make_concrete(&BaseType::Char)]);
            let ctx = ctx.with_generics(&new_lookup);

            let mut span = Object::new(span_type, &ctx);

            let char_type = stack.loader().find_concrete_type(ctx.make_concrete(&BaseType::Char));
            let managed = ManagedPtr::new(
                NonNull::new(ptr as *mut u8).expect("String pointer should not be null"),
                char_type,
                None,
                false,
            );
            managed.write(span.instance_storage.get_field_mut("_reference"));
            span.instance_storage.get_field_mut("_length").copy_from_slice(&(len as i32).to_ne_bytes());

            push!(ValueType(Box::new(span)));
        },
        [static System.Type::GetTypeFromHandle(System.RuntimeTypeHandle)]
        | [static System.Reflection.MethodBase::GetMethodFromHandle(System.RuntimeMethodHandle)]
        | [static System.Reflection.FieldInfo::GetFieldFromHandle(System.RuntimeFieldHandle)] => {
            vm_expect_stack!(let ValueType(handle) = pop!());
            let target = ObjectRef::read(handle.instance_storage.get_field("_value"));
            push!(ObjectRef(target));
        },
        [static System.RuntimeTypeHandle::ToIntPtr(System.RuntimeTypeHandle)] => {
            vm_expect_stack!(let ValueType(handle) = pop!());
            let target = handle.instance_storage.get_field("_value");
            let val = usize::from_ne_bytes(target.try_into().unwrap());
            push!(NativeInt(val as isize));
        },
        [System.RuntimeMethodHandle::GetFunctionPointer()] => {
            vm_expect_stack!(let ValueType(handle) = pop!());
            let method_obj = ObjectRef::read(handle.instance_storage.get_field("_value"));
            let (method, lookup) = stack.resolve_runtime_method(method_obj);
            let index = stack.get_runtime_method_index(method, lookup.clone());
            push!(NativeInt(index as isize));
        },
        [System.Type::get_IsValueType()] => {
            vm_expect_stack!(let ObjectRef(o) = pop!());
            let target = stack.resolve_runtime_type(o);
            let target_ct = target.to_concrete(stack.loader());
            let target_desc = stack.loader().find_concrete_type(target_ct);
            let value = target_desc.is_value_type(&ctx);
            push!(Int32(value as i32));
        },
        [static System.Type::op_Equality(any, any)] => {
            vm_expect_stack!(let ObjectRef(o2) = pop!());
            vm_expect_stack!(let ObjectRef(o1) = pop!());
            push!(Int32((o1 == o2) as i32));
        },
        [System.Type::get_TypeHandle()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());

            let rth = stack.loader().corlib_type("System.RuntimeTypeHandle");
            let mut instance = Object::new(rth, &ctx);
            obj.write(instance.instance_storage.get_field_mut("_value"));

            push!(ValueType(Box::new(instance)));
        },
        [static System.Runtime.InteropServices.GCHandle::InternalAlloc(object, System.Runtime.InteropServices.GCHandleType)] => {
            vm_expect_stack!(let Int32(handle_type) = pop!());
            vm_expect_stack!(let ObjectRef(obj) = pop!());

            let handle_type = GCHandleType::from(handle_type);
            let index = {
                let mut handles = stack.heap().gchandles.borrow_mut();
                if let Some(i) = handles.iter().position(|h| h.is_none()) {
                    handles[i] = Some((obj, handle_type));
                    i
                } else {
                    handles.push(Some((obj, handle_type)));
                    handles.len() - 1
                }
            };

            if handle_type == GCHandleType::Pinned {
                stack.heap().pinned_objects.borrow_mut().insert(obj);

                // Trace pinning event
                if stack.tracer_enabled() {
                    let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
                    stack.shared.tracer.lock().trace_gc_pin(stack.indent(), "PINNED", addr);
                }
            }

            // Trace GC handle allocation
            if stack.tracer_enabled() {
                let handle_type_str = format!("{:?}", handle_type);
                let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
                stack.shared.tracer.lock().trace_gc_handle(stack.indent(), "ALLOC", &handle_type_str, addr);
            }

            push!(NativeInt((index + 1) as isize));
        },
        [static System.Runtime.InteropServices.GCHandle::InternalFree(nint)] => {
            let handle = pop_native!();
            if handle != 0 {
                let index = (handle - 1) as usize;
                let mut handles = stack.heap().gchandles.borrow_mut();
                if index < handles.len() {
                    if let Some((obj, handle_type)) = handles[index] {
                        if handle_type == GCHandleType::Pinned {
                            stack.heap().pinned_objects.borrow_mut().remove(&obj);

                            // Trace unpinning event
                            if stack.tracer_enabled() {
                                let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
                                stack.shared.tracer.lock().trace_gc_pin(stack.indent(), "UNPINNED", addr);
                            }
                        }

                        // Trace GC handle free
                        if stack.tracer_enabled() {
                            let handle_type_str = format!("{:?}", handle_type);
                            let addr = obj.0.map(|p| gc_arena::Gc::as_ptr(p) as usize).unwrap_or(0);
                            stack.shared.tracer.lock().trace_gc_handle(stack.indent(), "FREE", &handle_type_str, addr);
                        }
                    }
                    handles[index] = None;
                }
            }
        },
        [static System.Runtime.InteropServices.GCHandle::InternalGet(nint)] => {
            let handle = pop_native!();
            let result = if handle == 0 {
                ObjectRef(None)
            } else {
                let index = (handle - 1) as usize;
                let handles = stack.heap().gchandles.borrow();
                match handles.get(index) {
                    Some(Some((obj, _))) => *obj,
                    _ => ObjectRef(None),
                }
            };
            push!(ObjectRef(result));
        },
        [static System.Runtime.InteropServices.GCHandle::InternalSet(nint, object)] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let handle = pop_native!();
            if handle != 0 {
                let index = (handle - 1) as usize;
                let mut handles = stack.heap().gchandles.borrow_mut();
                if index < handles.len() {
                    if let Some(entry) = &mut handles[index] {
                        if entry.1 == GCHandleType::Pinned {
                            let mut pinned = stack.heap().pinned_objects.borrow_mut();
                            pinned.remove(&entry.0);
                            pinned.insert(obj);
                        }
                        entry.0 = obj;
                    }
                }
            }
        },
        [static System.Runtime.InteropServices.GCHandle::InternalAddrOfPinnedObject(nint)] => {
            let handle = pop_native!();
            let addr = if handle == 0 {
                0
            } else {
                let index = (handle - 1) as usize;
                let handles = stack.heap().gchandles.borrow();
                if index < handles.len() {
                    if let Some(entry) = &handles[index] {
                        if entry.1 == GCHandleType::Pinned {
                            if let Some(ptr) = entry.0.0 {
                                match &ptr.borrow().storage {
                                    HeapStorage::Obj(_) => ptr.as_ptr() as isize,
                                    HeapStorage::Vec(v) => v.get().as_ptr() as isize,
                                    HeapStorage::Str(s) => s.as_ptr() as isize,
                                    _ => 0,
                                }
                            } else {
                                0
                            }
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                } else {
                    0
                }
            };
            push!(NativeInt(addr));
        },
    });

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
            vm_push!(stack, gc, string(CLRString::new(vec![])));
        },
    })
    .expect("unsupported load from intrinsic field");
}
