use crate::{
    context::ResolutionContext, layout::type_layout, pop_args, resolution::ValueResolution,
    vm_expect_stack, vm_pop, vm_push, CallStack, StepResult,
};
use dotnet_types::{
    generics::{ConcreteType, GenericLookup},
    members::{FieldDescription, MethodDescription},
};
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    layout::{FieldLayoutManager, HasLayout, LayoutManager, Scalar},
    object::{HeapStorage, ManagedPtrMetadata, MetadataOwner, Object as ObjectInstance, ObjectRef, ValueType},
    pointer::{ManagedPtr, ManagedPtrOwner},
    StackValue,
};
use dotnetdll::prelude::{BaseType, MethodType, ParameterType};
use std::ptr;

use super::ReflectionExtensions;

pub fn intrinsic_marshal_get_last_pinvoke_error<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let value = unsafe { crate::pinvoke::LAST_ERROR };
    vm_push!(stack, gc, Int32(value));
    StepResult::InstructionStepped
}

pub fn intrinsic_marshal_set_last_pinvoke_error<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [Int32(value)]);
    unsafe {
        crate::pinvoke::LAST_ERROR = value;
    }
    StepResult::InstructionStepped
}

pub fn intrinsic_buffer_memmove<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [NativeInt(len)]);
    let src = vm_pop!(stack, gc).as_ptr();
    let dst = vm_pop!(stack, gc).as_ptr();

    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);
    let total_count = len as usize * layout.size();

    // Check GC safe point before large bulk memory operations
    const LARGE_MEMMOVE_THRESHOLD: usize = 4096;
    if total_count > LARGE_MEMMOVE_THRESHOLD {
        stack.check_gc_safe_point();
    }

    unsafe {
        ptr::copy(src, dst, total_count);
    }
    StepResult::InstructionStepped
}

pub fn intrinsic_memory_marshal_get_array_data_reference<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj)]);
    let Some(array_handle) = obj.0 else {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    };

    let data_ptr = match &array_handle.borrow().storage {
        HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
        _ => panic!("GetArrayDataReference called on non-array"),
    };

    let element_type = stack
        .loader()
        .find_concrete_type(generics.method_generics[0].clone());
    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr(
            data_ptr,
            element_type,
            Some(ManagedPtrOwner::Heap(array_handle)),
            false
        )
    );
    StepResult::InstructionStepped
}

pub fn intrinsic_marshal_size_of<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let concrete_type = if method.method.signature.parameters.is_empty() {
        generics.method_generics[0].clone()
    } else {
        let type_obj = match vm_pop!(stack, gc) {
            StackValue::ObjectRef(o) => o,
            rest => panic!("Marshal.SizeOf(Type) called on non-object: {:?}", rest),
        };
        stack
            .resolve_runtime_type(type_obj)
            .to_concrete(stack.loader())
    };
    let layout = type_layout(concrete_type, &ctx);
    vm_push!(stack, gc, Int32(layout.size() as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_marshal_offset_of<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    use dotnet_value::with_string;
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let field_name_val = vm_pop!(stack, gc);
    let field_name = with_string!(stack, gc, field_name_val, |s| s.as_string());
    let concrete_type = if method.method.signature.parameters.len() == 1 {
        generics.method_generics[0].clone()
    } else {
        let type_obj = match vm_pop!(stack, gc) {
            StackValue::ObjectRef(o) => o,
            rest => panic!(
                "Marshal.OffsetOf(Type, string) called on non-object: {:?}",
                rest
            ),
        };
        stack
            .resolve_runtime_type(type_obj)
            .to_concrete(stack.loader())
    };
    let layout = type_layout(concrete_type.clone(), &ctx);

    if let LayoutManager::FieldLayoutManager(flm) = &*layout {
        let td = stack.loader().find_concrete_type(concrete_type.clone());
        if let Some(field) = flm.get_field(td, &field_name) {
            vm_push!(stack, gc, NativeInt(field.position as isize));
        } else {
            panic!("Field {} not found in type {:?}", field_name, concrete_type);
        }
    } else {
        panic!("Type {:?} does not have field layout", concrete_type);
    }
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::Add<T>(ref T source, nint elementOffset)
pub fn intrinsic_unsafe_as_pointer<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let val = vm_pop!(stack, gc);
    match val {
        StackValue::ManagedPtr(mut ptr) => {
            ptr.owner = None;
            vm_push!(stack, gc, ManagedPtr(ptr));
        }
        _ => panic!("Unsafe.AsPointer called on non-pointer: {:?}", val),
    }
    StepResult::InstructionStepped
}

pub fn intrinsic_unsafe_add<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let target = &generics.method_generics[0];
    let target_type = stack.loader().find_concrete_type(target.clone());
    let layout = type_layout(target.clone(), &ctx);

    let offset_val = vm_pop!(stack, gc);
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => panic!(
            "Unsafe.Add expected Int32 or NativeInt offset, got {:?}",
            offset_val
        ),
    };

    let m_val = vm_pop!(stack, gc);
    let (owner, pinned) = if let StackValue::ManagedPtr(m) = &m_val {
        (m.owner, m.pinned)
    } else {
        (None, false)
    };
    let m = m_val.as_ptr();
    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr(
            unsafe { m.offset(offset * layout.size() as isize) },
            target_type,
            owner,
            pinned
        )
    );
    StepResult::InstructionStepped
}

pub fn intrinsic_unsafe_add_byte_offset<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target = &generics.method_generics[0];
    let target_type = stack.loader().find_concrete_type(target.clone());

    let offset_val = vm_pop!(stack, gc);
    let offset = match offset_val {
        StackValue::Int32(i) => i as isize,
        StackValue::NativeInt(i) => i,
        _ => panic!(
            "Unsafe.AddByteOffset expected Int32 or NativeInt offset, got {:?}",
            offset_val
        ),
    };

    let m_val = vm_pop!(stack, gc);
    let (owner, pinned) = if let StackValue::ManagedPtr(m) = &m_val {
        (m.owner, m.pinned)
    } else {
        (None, false)
    };
    let m = m_val.as_ptr();
    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr(
            unsafe { m.offset(offset) },
            target_type,
            owner,
            pinned
        )
    );
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::AreSame<T>(ref T left, ref T right)
pub fn intrinsic_unsafe_are_same<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let m1 = vm_pop!(stack, gc).as_ptr();
    let m2 = vm_pop!(stack, gc).as_ptr();
    vm_push!(stack, gc, Int32((m1 == m2) as i32));
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::As<T>(object value)
/// System.Runtime.CompilerServices.Unsafe::AsRef<T>(ref T source)
pub fn intrinsic_unsafe_as<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    _stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    // Just leave the value on the stack, it's a no-op at runtime
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::As<TFrom, TTo>(ref TFrom source)
pub fn intrinsic_unsafe_as_generic<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    
    let src_type_gen = generics.method_generics[0].clone();
    let dest_type_gen = generics.method_generics[1].clone();
    
    let src_layout = type_layout(src_type_gen, &ctx);
    let dest_layout = type_layout(dest_type_gen, &ctx);
    
    // Safety Check: Casting ref TFrom to ref TTo
    // TTo must be compatible with TFrom's memory layout regarding References
    check_read_safety(&dest_layout, Some(&src_layout), 0);

    let target_type = stack
        .loader()
        .find_concrete_type(generics.method_generics[1].clone());
    let m_val = vm_pop!(stack, gc);
    let (owner, pinned) = match &m_val {
        StackValue::ManagedPtr(m) => (m.owner, m.pinned),
        StackValue::ObjectRef(ObjectRef(Some(h))) => (Some(ManagedPtrOwner::Heap(*h)), false),
        _ => (None, false),
    };
    let m = m_val.as_ptr();
    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr(m, target_type, owner, pinned)
    );
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::AsRef<T>(in T source)
/// System.Runtime.CompilerServices.Unsafe::AsRef<T>(void* ptr)
pub fn intrinsic_unsafe_as_ref_any<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let param_type = &method.method.signature.parameters[0].1;
    if let ParameterType::Value(MethodType::Base(b)) = param_type {
        if matches!(**b, BaseType::ValuePointer(_, _)) {
            return intrinsic_unsafe_as_ref_ptr(gc, stack, method, generics);
        }
    }
    // Otherwise it's the "in T source" (ByRef) overload, which is a no-op
    intrinsic_unsafe_as(gc, stack, method, generics)
}

/// System.Runtime.CompilerServices.Unsafe::AsRef<T>(void* ptr)
pub fn intrinsic_unsafe_as_ref_ptr<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );

    let target_type_gen = generics.method_generics[0].clone();
    let dest_layout = type_layout(target_type_gen.clone(), &ctx);
    
    let target_type = stack
        .loader()
        .find_concrete_type(target_type_gen);
    
    let val = vm_pop!(stack, gc);
    let (ptr, owner, pinned) = match val {
        StackValue::NativeInt(p) => (p as *mut u8, None, false),
        StackValue::ManagedPtr(m) => (
            m.value.expect("Unsafe.AsRef null managed ptr").as_ptr(),
            m.owner,
            m.pinned
        ),
        _ => panic!("Unsafe.AsRef expected pointer, got {:?}", val),
    };

    // Safety Check: Casting ptr to ref T
    let src_layout_obj = if let Some(owner) = owner {
         match owner {
               ManagedPtrOwner::Heap(h) => {
                    let obj = h.borrow();
                    match &obj.storage {
                        HeapStorage::Obj(o) => Some(LayoutManager::FieldLayoutManager(o.instance_storage.layout().as_ref().clone())),
                        HeapStorage::Vec(v) => Some(LayoutManager::ArrayLayoutManager(v.layout.clone())),
                        HeapStorage::Boxed(v) => match v {
                            ValueType::Struct(o) => Some(LayoutManager::FieldLayoutManager(o.instance_storage.layout().as_ref().clone())),
                            ValueType::Pointer(_) => Some(LayoutManager::Scalar(Scalar::ManagedPtr)),
                            _ => None
                        },
                        _ => None
                    }
               },
               ManagedPtrOwner::Stack(s) => {
                    let o = unsafe { s.as_ref() };
                    Some(LayoutManager::FieldLayoutManager(o.instance_storage.layout().as_ref().clone()))
               }
         }
    } else {
        None
    };
    
    let base_addr = if let Some(owner) = owner {
         match owner {
             ManagedPtrOwner::Heap(h) => {
                 let obj = h.borrow();
                 match &obj.storage {
                      HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as usize,
                      HeapStorage::Vec(v) => v.get().as_ptr() as usize,
                      HeapStorage::Boxed(v) => match v {
                          ValueType::Struct(o) => o.instance_storage.get().as_ptr() as usize,
                          _ => 0 
                      },
                      _ => 0
                 }
             },
             ManagedPtrOwner::Stack(s) => {
                  unsafe { s.as_ref().instance_storage.get().as_ptr() as usize }
             }
         }
    } else {
        0
    };
    
    if base_addr != 0 {
         let ptr_addr = ptr as usize;
         let offset = ptr_addr.wrapping_sub(base_addr);
         check_read_safety(&dest_layout, src_layout_obj.as_ref(), offset);
    } else if dest_layout.is_or_contains_refs() {
         panic!("Heap Corruption: Casting unmanaged pointer to Ref type is unsafe");
    }

    vm_push!(
        stack,
        gc,
        StackValue::managed_ptr(ptr, target_type, owner, pinned)
    );
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::SizeOf<T>()
pub fn intrinsic_unsafe_size_of<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);
    vm_push!(stack, gc, Int32(layout.size() as i32));
    StepResult::InstructionStepped
}

/// System.Runtime.CompilerServices.Unsafe::ByteOffset<T>(ref T origin, ref T target)
pub fn intrinsic_unsafe_byte_offset<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let r = vm_pop!(stack, gc).as_ptr();
    let l = vm_pop!(stack, gc).as_ptr();
    let offset = (l as isize) - (r as isize);
    vm_push!(stack, gc, NativeInt(offset));
    StepResult::InstructionStepped
}

fn check_side_table<'gc>(
    obj: &ObjectInstance<'gc>,
    ptr: *const u8,
) -> Option<ManagedPtrMetadata<'gc>> {
    let side_table = obj.managed_ptr_metadata.borrow();
    let base_ptr = obj.instance_storage.get().as_ptr() as usize;
    let offset = (ptr as usize).wrapping_sub(base_ptr);
    side_table.get(offset).cloned()
}

fn copy_owner_side_table<'gc>(
    obj: &ObjectInstance<'gc>,
    owner: &ObjectInstance<'gc>,
    ptr: *const u8,
    layout: &FieldLayoutManager,
    gc: GCHandle<'gc>,
) {
    let side_table = owner.managed_ptr_metadata.borrow();
    let src_base = owner.instance_storage.get().as_ptr() as usize;
    let src_offset = (ptr as usize).wrapping_sub(src_base);

    for (&offset, metadata) in &side_table.metadata {
        // If the metadata offset falls within the range we just read_unchecked
        if offset >= src_offset && offset < src_offset + layout.size() {
            let target_offset = offset - src_offset;
            obj.register_metadata(target_offset, metadata.clone(), gc);
        }
    }
}

/// System.Runtime.CompilerServices.Unsafe::ReadUnaligned<T>(void* ptr)
pub fn intrinsic_unsafe_read_unaligned<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );

    let source = vm_pop!(stack, gc);
    let (ptr, owner) = match source {
        StackValue::NativeInt(p) => (p as *mut u8, None),
        StackValue::ManagedPtr(m) => (
            m.value.expect("Unsafe.ReadUnaligned null").as_ptr(),
            m.owner,
        ),
        rest => panic!("invalid source for read_unchecked unaligned: {:?}", rest),
    };

    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);

    // Phase 3: Integrity Check - Prevent reading garbage as GC pointers
    if owner.is_some() || layout.is_or_contains_refs() {
         let src_layout_obj = if let Some(owner) = owner {
              match owner {
                    ManagedPtrOwner::Heap(h) => {
                         let obj = h.borrow();
                         match &obj.storage {
                             HeapStorage::Obj(o) => Some(LayoutManager::FieldLayoutManager(o.instance_storage.layout().as_ref().clone())),
                             HeapStorage::Vec(v) => Some(LayoutManager::ArrayLayoutManager(v.layout.clone())),
                             HeapStorage::Boxed(v) => match v {
                                 ValueType::Struct(o) => Some(LayoutManager::FieldLayoutManager(o.instance_storage.layout().as_ref().clone())),
                                 ValueType::Pointer(_) => Some(LayoutManager::Scalar(Scalar::ManagedPtr)),
                                 _ => None
                             },
                             _ => None
                         }
                    },
                    ManagedPtrOwner::Stack(s) => {
                         let o = unsafe { s.as_ref() };
                         Some(LayoutManager::FieldLayoutManager(o.instance_storage.layout().as_ref().clone()))
                    }
              }
         } else {
             None
         };
         
         let base_addr = if let Some(owner) = owner {
              match owner {
                  ManagedPtrOwner::Heap(h) => {
                      let obj = h.borrow();
                      match &obj.storage {
                           HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as usize,
                           HeapStorage::Vec(v) => v.get().as_ptr() as usize,
                           HeapStorage::Boxed(v) => match v {
                               ValueType::Struct(o) => o.instance_storage.get().as_ptr() as usize,
                               _ => 0 
                           },
                           _ => 0
                      }
                  },
                  ManagedPtrOwner::Stack(s) => {
                       unsafe { s.as_ref().instance_storage.get().as_ptr() as usize }
                  }
              }
         } else {
             0
         };
         
         if base_addr != 0 {
              let ptr_addr = ptr as usize;
              let offset = ptr_addr.wrapping_sub(base_addr);
              check_read_safety(&layout, src_layout_obj.as_ref(), offset);
         } else if layout.is_or_contains_refs() {
              panic!("Heap Corruption: Reading ObjectRef from unmanaged memory is unsafe");
         }
    }

    macro_rules! read_ua {
        ($t:ty) => {
            unsafe { ptr::read_unaligned(ptr as *const $t) }
        };
    }

    let v = match &*layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef => StackValue::ObjectRef(read_ua!(ObjectRef)),
            Scalar::Int8 => StackValue::Int32(read_ua!(i8) as i32),
            Scalar::Int16 => StackValue::Int32(read_ua!(i16) as i32),
            Scalar::Int32 => StackValue::Int32(read_ua!(i32)),
            Scalar::Int64 => StackValue::Int64(read_ua!(i64)),
            Scalar::NativeInt => StackValue::NativeInt(read_ua!(isize)),
            Scalar::Float32 => StackValue::NativeFloat(read_ua!(f32) as f64),
            Scalar::Float64 => StackValue::NativeFloat(read_ua!(f64)),
            Scalar::ManagedPtr => {
                // Read only the pointer value (8 bytes) from memory.
                // ManagedPtr is now stored pointer-sized per ECMA-335.
                let ptr_value = read_ua!(usize);
                let ptr_val =
                    ptr::NonNull::new(ptr_value as *mut u8).expect("Read null managed pointer");
                let target_type = stack.loader().find_concrete_type(target.clone());

                // Try to find metadata from owner's side-table if it's a managed pointer
                let mut metadata = (None, false);
                if let Some(owner) = owner {
                    let m_metadata = match owner {
                        ManagedPtrOwner::Heap(h) => {
                            let obj = h.borrow();
                            if let HeapStorage::Obj(o) = &obj.storage {
                                check_side_table(o, ptr)
                            } else {
                                None
                            }
                        }
                        ManagedPtrOwner::Stack(s) => {
                            let o = unsafe { s.as_ref() };
                            check_side_table(o, ptr)
                        }
                    };

                    if let Some(m) = m_metadata {
                        metadata = (m.recover_owner(), m.pinned);
                    }
                }

                StackValue::ManagedPtr(ManagedPtr::new(
                    Some(ptr_val),
                    target_type,
                    metadata.0,
                    metadata.1,
                ))
            }
        },
        LayoutManager::FieldLayoutManager(f) => {
            // T is a struct, allocate an Object to represent it on the stack as ValueType
            let o = ctx.new_object(stack.loader().find_concrete_type(target.clone()));

            // Copy raw data from source pointer
            unsafe {
                ptr::copy_nonoverlapping(ptr, o.instance_storage.get_mut().as_mut_ptr(), f.size());
            }

            // If source has an owner, copy relevant metadata side-table entries
            if let Some(owner) = owner {
                match owner {
                    ManagedPtrOwner::Heap(h) => {
                        let src_obj = h.borrow();
                        if let HeapStorage::Obj(src_o) = &src_obj.storage {
                            copy_owner_side_table(&o, src_o, ptr, f, gc);
                        }
                    }
                    ManagedPtrOwner::Stack(s) => {
                        let src_o = unsafe { s.as_ref() };
                        copy_owner_side_table(&o, src_o, ptr, f, gc);
                    }
                }
            }

            StackValue::ValueType(Box::new(o))
        }
        _ => panic!("unsupported layout for read_unchecked unaligned"),
    };
    vm_push!(stack, gc, v);
    StepResult::InstructionStepped
}

fn register_managed_ptr<'gc>(
    obj: &ObjectInstance<'gc>,
    ptr: ManagedPtr<'gc>,
    value: *const u8,
    gc: GCHandle<'gc>,
) {
    obj.register_managed_ptr(
        (value as usize).wrapping_sub(obj.instance_storage.get().as_ptr() as usize),
        &ptr,
        gc,
    );
}

fn update_owner_side_table<'gc>(
    obj: &ObjectInstance<'gc>,
    dest_o: &ObjectInstance<'gc>,
    ptr: *const u8,
    gc: GCHandle<'gc>,
) {
    let side_table = obj.managed_ptr_metadata.borrow();
    let dest_base = dest_o.instance_storage.get().as_ptr() as usize;
    let dest_offset = (ptr as usize).wrapping_sub(dest_base);

    for (&offset, metadata) in &side_table.metadata {
        // Map source side-table entries to destination offsets
        dest_o.register_metadata(dest_offset + offset, metadata.clone(), gc);
    }
}

fn has_ref_at(layout: &LayoutManager, offset: usize) -> bool {
    match layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef | Scalar::ManagedPtr => offset == 0,
            _ => false,
        },
        LayoutManager::FieldLayoutManager(fm) => {
             for f in fm.fields.values() {
                 if offset >= f.position && offset < f.position + f.layout.size() {
                     return has_ref_at(&f.layout, offset - f.position);
                 }
             }
             false
        }
        LayoutManager::ArrayLayoutManager(am) => {
             let elem_size = am.element_layout.size();
             if elem_size == 0 { return false; }
             let idx = offset / elem_size;
             if idx >= am.length { return false; }
             let rel = offset % elem_size;
             has_ref_at(&am.element_layout, rel)
        }
    }
}

fn validate_ref_integrity(
    dest_layout: &LayoutManager,
    base_offset: usize, 
    range_start: usize, 
    range_end: usize,   
    src_layout: &LayoutManager,
) {
     match dest_layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef | Scalar::ManagedPtr => {
                let ref_start = base_offset;
                let ref_end = base_offset + 8;
                
                if ref_start < range_end && ref_end > range_start {
                    if ref_start < range_start {
                         panic!("Heap Corruption: Write starts in the middle of an ObjectRef at {}", ref_start);
                    }
                    
                    if ref_end > range_end {
                        panic!("Heap Corruption: Write ends in the middle of an ObjectRef at {}", ref_start);
                    }
                    
                    let src_offset = ref_start - range_start;
                    if !has_ref_at(src_layout, src_offset) {
                        panic!("Heap Corruption: Writing non-ref data over ObjectRef at offset {}", ref_start);
                    }
                    
                    if ref_start % 8 != 0 {
                        panic!("Heap Corruption: Misaligned ObjectRef in destination at {}", ref_start);
                    }
                }
            }
            _ => {}
        },
        LayoutManager::FieldLayoutManager(fm) => {
            for f in fm.fields.values() {
                 let f_start = base_offset + f.position;
                 let f_end = f_start + f.layout.size();
                 if f_start < range_end && f_end > range_start {
                     validate_ref_integrity(&f.layout, f_start, range_start, range_end, src_layout);
                 }
            }
        },
        LayoutManager::ArrayLayoutManager(am) => {
             if am.element_layout.is_or_contains_refs() {
                  let elem_size = am.element_layout.size();
                  if elem_size == 0 { return; }
                  
                  let rel_start = if range_start > base_offset { range_start - base_offset } else { 0 };
                  let rel_end = if range_end > base_offset { range_end - base_offset } else { 0 };
                  
                  let start_idx = rel_start / elem_size;
                  let end_idx = (rel_end + elem_size - 1) / elem_size;
                  
                  for i in start_idx..end_idx {
                      if i >= am.length { break; }
                      let elem_abs_start = base_offset + i * elem_size;
                      validate_ref_integrity(&am.element_layout, elem_abs_start, range_start, range_end, src_layout);
                  }
             }
        }
    }
}

fn check_refs_in_layout<F>(layout: &LayoutManager, base: usize, callback: &mut F)
where
    F: FnMut(usize) + ?Sized,
{
     match layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef | Scalar::ManagedPtr => callback(base),
            _ => {}
        },
        LayoutManager::FieldLayoutManager(fm) => {
             for f in fm.fields.values() {
                  check_refs_in_layout(&f.layout, base + f.position, callback);
             }
        }
        LayoutManager::ArrayLayoutManager(am) => {
             if am.element_layout.is_or_contains_refs() {
                  let sz = am.element_layout.size();
                  for i in 0..am.length {
                       check_refs_in_layout(&am.element_layout, base + i * sz, callback);
                  }
             }
        }
     }
}

fn check_read_safety(
    result_layout: &LayoutManager,
    src_layout: Option<&LayoutManager>,
    src_ptr_offset: usize,
) {
    check_refs_in_layout(result_layout, 0, &mut |ref_offset| {
         let target_src = src_ptr_offset + ref_offset;
         
         if let Some(sl) = src_layout {
             if !has_ref_at(sl, target_src) {
                  panic!("Heap Corruption: Reading ObjectRef from non-ref memory at offset {}", target_src);
             }
             if target_src % 8 != 0 {
                  panic!("Heap Corruption: Reading misaligned ObjectRef at {}", target_src);
             }
         } else {
             // Reading Ref from unmanaged memory (src_layout is None)
             panic!("Heap Corruption: Reading ObjectRef from unmanaged memory is unsafe");
         }
    });
}

/// System.Runtime.CompilerServices.Unsafe::WriteUnaligned<T>(void* ptr, T value)
pub fn intrinsic_unsafe_write_unaligned<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);
    let value = vm_pop!(stack, gc);

    let dest = vm_pop!(stack, gc);
    let (ptr, owner) = match dest {
        StackValue::NativeInt(p) => (p as *mut u8, None),
        StackValue::ManagedPtr(m) => (
            m.value.expect("Unsafe.WriteUnaligned null").as_ptr(),
            m.owner,
        ),
        rest => panic!("invalid destination for write unaligned: {:?}", rest),
    };

    if let Some(owner) = owner {
        let (base, len) = match owner {
            ManagedPtrOwner::Heap(h) => {
                let obj = h.borrow();
                match &obj.storage {
                    HeapStorage::Obj(o) | HeapStorage::Boxed(ValueType::Struct(o)) => {
                        let guard = o.instance_storage.get();
                        (guard.as_ptr(), guard.len())
                    }
                    HeapStorage::Vec(v) => {
                        let guard = v.get();
                        (guard.as_ptr(), guard.len())
                    }
                    _ => (std::ptr::null(), 0),
                }
            }
            ManagedPtrOwner::Stack(s) => {
                let o = unsafe { s.as_ref() };
                let guard = o.instance_storage.get();
                (guard.as_ptr(), guard.len())
            }
        };

        if !base.is_null() {
            let base_addr = base as usize;
            let ptr_addr = ptr as usize;
            let size = layout.size();

            if ptr_addr < base_addr
                || (ptr_addr - base_addr)
                    .checked_add(size)
                    .is_none_or(|end| end > len)
            {
                panic!(
                    "Unsafe.WriteUnaligned: Heap corruption prevented! Access out of bounds. ptr={:p}, base={:p}, offset={}, size={}, len={}",
                    ptr, base, ptr_addr.wrapping_sub(base_addr), size, len
                );
            }

            // Phase 3: Integrity Check - Prevent corrupting GC pointers
            let offset = ptr_addr.wrapping_sub(base_addr);
            let dest_layout = match owner {
                ManagedPtrOwner::Heap(h) => {
                    let obj = h.borrow();
                    match &obj.storage {
                        HeapStorage::Obj(o) => Some(LayoutManager::FieldLayoutManager(o.instance_storage.layout().as_ref().clone())),
                        HeapStorage::Vec(v) => Some(LayoutManager::ArrayLayoutManager(v.layout.clone())),
                        HeapStorage::Boxed(v) => match v {
                            ValueType::Struct(o) => Some(LayoutManager::FieldLayoutManager(o.instance_storage.layout().as_ref().clone())),
                            ValueType::Pointer(_) => Some(LayoutManager::Scalar(Scalar::ManagedPtr)),
                            _ => None
                        },
                        _ => None
                    }
                },
                ManagedPtrOwner::Stack(s) => {
                    let o = unsafe { s.as_ref() };
                    Some(LayoutManager::FieldLayoutManager(o.instance_storage.layout().as_ref().clone()))
                }
            };

            if let Some(dl) = dest_layout {
                validate_ref_integrity(&dl, 0, offset, offset + size, &layout);
            }
        }
    }

    macro_rules! write_ua {
        ($variant:ident, $t:ty) => {{
            vm_expect_stack!(let $variant(v) = value);
            unsafe {
                ptr::write_unaligned(ptr as *mut _, v as $t);
            }
        }};
    }

    match &*layout {
        LayoutManager::Scalar(s) => match s {
            Scalar::ObjectRef => {
                vm_expect_stack!(let ObjectRef(o) = value);
                unsafe {
                    ptr::write_unaligned(ptr as *mut _, o);
                }

                // Register dynamic root if we are writing a valid object reference
                if let Some(ref_handle) = o.0 {
                    if let Some(owner) = owner {
                        let inner_type = stack.loader().find_concrete_type(target.clone());
                        let metadata = ManagedPtrMetadata {
                            inner_type,
                            owner: MetadataOwner::Heap(ref_handle),
                            pinned: false,
                        };

                        match owner {
                            ManagedPtrOwner::Heap(h) => {
                                let dest_obj = h.borrow();
                                match &dest_obj.storage {
                                    HeapStorage::Obj(dest_o) => {
                                        let base = dest_o.instance_storage.get().as_ptr() as usize;
                                        let offset = (ptr as usize).wrapping_sub(base);
                                        dest_o.register_metadata(offset, metadata, gc);
                                    }
                                    HeapStorage::Vec(dest_v) => {
                                        let base = dest_v.get().as_ptr() as usize;
                                        let offset = (ptr as usize).wrapping_sub(base);
                                        dest_v.register_metadata(offset, metadata, gc);
                                    }
                                    _ => {}
                                }
                            }
                            ManagedPtrOwner::Stack(s) => {
                                let dest_o = unsafe { s.as_ref() };
                                let base = dest_o.instance_storage.get().as_ptr() as usize;
                                let offset = (ptr as usize).wrapping_sub(base);
                                dest_o.register_metadata(offset, metadata, gc);
                            }
                        }
                    }
                }
            }
            Scalar::Int8 => write_ua!(Int32, i8),
            Scalar::Int16 => write_ua!(Int32, i16),
            Scalar::Int32 => write_ua!(Int32, i32),
            Scalar::Int64 => write_ua!(Int64, i64),
            Scalar::NativeInt => write_ua!(NativeInt, isize),
            Scalar::Float32 => write_ua!(NativeFloat, f32),
            Scalar::Float64 => write_ua!(NativeFloat, f64),
            Scalar::ManagedPtr => {
                // Write only the pointer value (8 bytes) to memory.
                // ManagedPtr is now stored pointer-sized per ECMA-335.
                vm_expect_stack!(let ManagedPtr(m) = value);
                unsafe {
                    ptr::write_unaligned(
                        ptr as *mut usize,
                        m.value.expect("Unsafe.WriteUnaligned val null").as_ptr() as usize,
                    );
                }

                // If destination has an owner, update its side-table
                if let Some(owner) = owner {
                    match owner {
                        ManagedPtrOwner::Heap(h) => {
                            let dest_obj = h.borrow();
                            match &dest_obj.storage {
                                HeapStorage::Obj(dest_o) => {
                                    register_managed_ptr(dest_o, m, ptr, gc)
                                }
                                HeapStorage::Vec(dest_v) => {
                                    let base = dest_v.get().as_ptr() as usize;
                                    let offset = (ptr as usize).wrapping_sub(base);
                                    dest_v.register_metadata(
                                        offset,
                                        ManagedPtrMetadata::from_managed_ptr(&m),
                                        gc,
                                    );
                                }
                                _ => {}
                            }
                        }
                        ManagedPtrOwner::Stack(s) => {
                            let dest_o = unsafe { s.as_ref() };
                            register_managed_ptr(dest_o, m, ptr, gc);
                        }
                    }
                }
            }
        },
        LayoutManager::FieldLayoutManager(f) => {
            // T is a struct, extract its Object from ValueType
            vm_expect_stack!(let ValueType(o) = value);

            // Copy raw data to destination pointer
            unsafe {
                ptr::copy_nonoverlapping(o.instance_storage.get().as_ptr(), ptr, f.size());
            }

            // If destination has an owner, update its side-table
            if let Some(owner) = owner {
                match owner {
                    ManagedPtrOwner::Heap(h) => {
                        let mut dest_obj = h.borrow_mut(gc);
                        if let HeapStorage::Obj(dest_o) = &mut dest_obj.storage {
                            update_owner_side_table(&o, dest_o, ptr, gc);
                        }
                    }
                    ManagedPtrOwner::Stack(s) => {
                        let dest_o = unsafe { s.as_ref() };
                        update_owner_side_table(&o, dest_o, ptr, gc);
                    }
                }
            }
        }
        _ => panic!("unsupported layout for write unaligned"),
    }
    StepResult::InstructionStepped
}

pub fn intrinsic_field_intptr_zero<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _field: FieldDescription,
    _type_generics: Vec<ConcreteType>,
    _is_address: bool,
) -> StepResult {
    stack.push_stack(gc, StackValue::NativeInt(0));
    StepResult::InstructionStepped
}
