use crate::{
    instructions::{is_intrinsic_cached, type_layout, LayoutFactory},
    intrinsics::intrinsic_call,
    resolution::ValueResolution,
    CallStack, StepResult,
};
use dotnet_assemblies::decompose_type_source;
use dotnet_macros::dotnet_instruction;
use dotnet_types::members::MethodDescription;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    pointer::UnmanagedPtr,
    StackValue,
};
use dotnetdll::prelude::*;
use std::ptr;

pub mod arrays;
pub mod boxing;
pub mod casting;
pub mod fields;

pub(crate) fn get_ptr<'gc, 'm: 'gc>(
    stack: &mut CallStack<'gc, 'm>,
    gc: GCHandle<'gc>,
    val: StackValue<'gc>,
) -> Result<(*mut u8, Option<ObjectRef<'gc>>), StepResult> {
    match val {
        StackValue::ObjectRef(o @ ObjectRef(Some(h))) => {
            let inner = h.borrow();
            let ptr = match &inner.storage {
                HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as *mut u8,
                HeapStorage::Vec(v) => v.get().as_ptr() as *mut u8,
                HeapStorage::Str(s) => s.as_ptr() as *mut u8,
                HeapStorage::Boxed(v) => match v {
                    dotnet_value::object::ValueType::Struct(s) => {
                        s.instance_storage.get().as_ptr() as *mut u8
                    }
                    _ => ptr::null_mut(),
                },
            };
            Ok((ptr, Some(o)))
        }
        StackValue::ValueType(o) => {
            let ptr = o.instance_storage.get().as_ptr() as *mut u8;
            Ok((ptr, None))
        }
        StackValue::ManagedPtr(m) => Ok((
            m.pointer().map(|p| p.as_ptr()).unwrap_or(ptr::null_mut()),
            m.owner,
        )),
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => Ok((p.as_ptr(), None)),
        StackValue::ObjectRef(ObjectRef(None)) => {
            Err(stack.throw_by_name(gc, "System.NullReferenceException"))
        }
        _ => panic!("Invalid parent for field/element access: {:?}", val),
    }
}

#[dotnet_instruction(NewObject)]
pub fn new_object<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    ctor: &UserMethod,
) -> StepResult {
    let (mut method, lookup) = stack.find_generic_method(&MethodSource::User(*ctor));

    if method.method.name == "CtorArraySentinel" {
        if let UserMethod::Reference(r) = *ctor {
            let resolution = stack.current_frame().source_resolution;
            let method_ref = &resolution[r];

            if let MethodReferenceParent::Type(t) = &method_ref.parent {
                let concrete = {
                    let ctx = stack.current_context();
                    // Use generics from context to resolve the MethodType
                    ctx.generics.make_concrete(resolution, t.clone())
                };

                if let BaseType::Array(element, shape) = concrete.get() {
                    let rank = shape.rank;
                    let mut dims: Vec<usize> = (0..rank)
                        .map(|_| {
                            let v = stack.pop(gc);
                            match v {
                                StackValue::Int32(i) => i as usize,
                                StackValue::NativeInt(i) => i as usize,
                                _ => panic!("Invalid dimension {:?}", v),
                            }
                        })
                        .collect();
                    dims.reverse();

                    let total_len: usize = dims.iter().product();

                    let elem_type_concrete = element.clone();

                    let ctx = stack.current_context();
                    let elem_type = ctx.normalize_type(elem_type_concrete.clone());

                    let layout =
                        LayoutFactory::create_array_layout(elem_type.clone(), total_len, &ctx);
                    let total_size_bytes = layout.element_layout.size() * total_len;

                    let vec_obj = dotnet_value::object::Vector::new(
                        elem_type,
                        layout,
                        vec![0; total_size_bytes],
                        dims,
                    );
                    let o = ObjectRef::new(gc, HeapStorage::Vec(vec_obj));
                    stack.register_new_object(&o);
                    stack.push(gc, StackValue::ObjectRef(o));
                    return StepResult::Continue;
                }
            }
        }
    }

    let parent = method.parent;
    if let (None, Some(ts)) = (&method.method.body, &parent.definition().extends) {
        let (ut, _) = decompose_type_source(ts);
        let type_name = ut.type_name(parent.resolution.definition());
        // delegate types are only allowed to have these base types
        if matches!(
            type_name.as_ref(),
            "System.Delegate" | "System.MulticastDelegate"
        ) {
            let base = stack.loader().corlib_type(&type_name);
            method = MethodDescription {
                parent: base,
                method_resolution: base.resolution,
                method: base
                    .definition()
                    .methods
                    .iter()
                    .find(|m| m.name == ".ctor")
                    .unwrap(),
            };
        }
    }

    let method_name = &*method.method.name;
    let parent_name = method.parent.definition().type_name();
    if parent_name == "System.IntPtr"
        && method_name == ".ctor"
        && method.method.signature.parameters.len() == 1
    {
        let val = stack.pop(gc);
        let native_val = match val {
            StackValue::Int32(i) => i as isize,
            StackValue::Int64(i) => i as isize,
            StackValue::NativeInt(i) => i,
            _ => panic!("Invalid argument for IntPtr constructor: {:?}", val),
        };
        stack.push(gc, StackValue::NativeInt(native_val));
        StepResult::Continue
    } else {
        if is_intrinsic_cached(stack, method) {
            return intrinsic_call(gc, stack, method, &lookup);
        }

        if stack.initialize_static_storage(gc, parent, lookup.clone()) {
            return StepResult::FramePushed;
        }

        let new_ctx = stack
            .current_context()
            .for_type_with_generics(parent, &lookup);
        let instance = new_ctx.new_object(parent);

        stack.constructor_frame(
            gc,
            instance,
            crate::MethodInfo::new(method, &lookup, stack.shared.clone()),
            lookup,
        );
        StepResult::FramePushed
    }
}

#[dotnet_instruction(LoadObject)]
pub fn ldobj<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let addr = stack.pop(gc);
    let source_ptr = addr.as_ptr();

    if source_ptr.is_null() {
        return stack.throw_by_name(gc, "System.NullReferenceException");
    }

    let ctx = stack.current_context();
    let load_type = ctx.make_concrete(param0);
    let layout = type_layout(load_type.clone(), &ctx);

    let mut source_vec = vec![0u8; layout.size()];
    unsafe { ptr::copy_nonoverlapping(source_ptr, source_vec.as_mut_ptr(), layout.size()) };
    let value = ctx
        .read_cts_value(&load_type, &source_vec, gc)
        .into_stack(gc);

    stack.push(gc, value);
    StepResult::Continue
}

#[dotnet_instruction(StoreObject)]
pub fn stobj<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let value = stack.pop(gc);
    let addr = stack.pop(gc);

    let ctx = stack.current_context();
    let concrete_t = ctx.make_concrete(param0);

    let dest_ptr = match &addr {
        StackValue::NativeInt(i) => {
            if *i == 0 {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
            *i as *mut u8
        }
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::ManagedPtr(m) => match m.pointer() {
            Some(ptr) => ptr.as_ptr(),
            None => return stack.throw_by_name(gc, "System.NullReferenceException"),
        },
        _ => panic!("stobj: expected pointer on stack, got {:?}", addr),
    };

    let layout = type_layout(concrete_t.clone(), &ctx);
    let mut bytes = vec![0u8; layout.size()];
    ctx.new_cts_value(&concrete_t, value).write(&mut bytes);

    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), dest_ptr, bytes.len());
    }
    StepResult::Continue
}

#[dotnet_instruction(InitializeForObject)]
pub fn initobj<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let addr = stack.pop(gc);
    let target = match addr {
        StackValue::NativeInt(i) => {
            if i == 0 {
                return stack.throw_by_name(gc, "System.NullReferenceException");
            }
            i as *mut u8
        }
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p.as_ptr(),
        StackValue::ManagedPtr(m) => match m.pointer() {
            Some(ptr) => ptr.as_ptr(),
            None => return stack.throw_by_name(gc, "System.NullReferenceException"),
        },
        _ => panic!("initobj: expected pointer on stack, got {:?}", addr),
    };

    let ctx = stack.current_context();
    let ct = ctx.make_concrete(param0);
    let layout = type_layout(ct.clone(), &ctx);

    unsafe { ptr::write_bytes(target, 0, layout.size()) };
    StepResult::Continue
}

#[dotnet_instruction(Sizeof)]
pub fn sizeof<'gc, 'm: 'gc>(
    _gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType,
) -> StepResult {
    let ctx = stack.current_context();
    let target = ctx.make_concrete(param0);
    let layout = type_layout(target, &ctx);
    stack.push(_gc, StackValue::Int32(layout.size() as i32));
    StepResult::Continue
}

#[dotnet_instruction(LoadString)]
pub fn ldstr<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    chars: &[u16],
) -> StepResult {
    use dotnet_value::string::CLRString;

    let s = CLRString::new(chars.to_owned());
    let storage = HeapStorage::Str(s);
    let obj_ref = ObjectRef::new(gc, storage);
    stack.push(gc, StackValue::ObjectRef(obj_ref));
    StepResult::Continue
}
