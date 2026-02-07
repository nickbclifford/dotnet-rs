use crate::{
    StepResult,
    intrinsics::intrinsic_call,
    layout::{LayoutFactory, type_layout},
    resolution::ValueResolution,
    stack::VesContext,
};
use dotnet_assemblies::decompose_type_source;
use dotnet_macros::dotnet_instruction;
use dotnet_types::members::MethodDescription;
use dotnet_utils::gc::GCHandle;
use dotnet_value::{
    StackValue,
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    pointer::UnmanagedPtr,
};
use dotnetdll::prelude::*;
use std::ptr;

pub mod arrays;
pub mod boxing;
pub mod casting;
pub mod fields;

pub(crate) fn get_ptr<'gc>(
    val: &StackValue<'gc>,
) -> (
    *mut u8,
    Option<ObjectRef<'gc>>,
    Option<(usize, usize)>,
    usize,
) {
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
            (ptr, Some(*o), None, 0)
        }
        StackValue::ValueType(o) => {
            let ptr = o.instance_storage.get().as_ptr() as *mut u8;
            (ptr, None, None, 0)
        }
        StackValue::ManagedPtr(m) => (
            m.pointer().map(|p| p.as_ptr()).unwrap_or(ptr::null_mut()),
            m.owner,
            m.stack_slot_origin,
            m.offset,
        ),
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => (p.as_ptr(), None, None, p.as_ptr() as usize),
        StackValue::NativeInt(p) => (*p as *mut u8, None, None, *p as usize),
        _ => panic!("Invalid parent for field/element access: {:?}", val),
    }
}

pub(crate) fn get_ptr_context<'gc, 'm: 'gc>(
    ctx: &VesContext<'_, 'gc, 'm>,
    val: &StackValue<'gc>,
) -> (
    *mut u8,
    Option<ObjectRef<'gc>>,
    Option<(usize, usize)>,
    usize,
) {
    match val {
        StackValue::ManagedPtr(m) => {
            if let Some((idx, offset)) = m.stack_slot_origin {
                let slot_val = ctx.evaluation_stack.get_slot_ref(idx);
                if let StackValue::ValueType(v) = slot_val {
                    let ptr = v.instance_storage.get().as_ptr() as *mut u8;
                    return (
                        unsafe { ptr.add(offset) },
                        None,
                        Some((idx, offset)),
                        offset,
                    );
                }
            }
            (
                m.pointer().map(|p| p.as_ptr()).unwrap_or(ptr::null_mut()),
                m.owner,
                m.stack_slot_origin,
                m.offset,
            )
        }
        _ => get_ptr(val),
    }
}

#[dotnet_instruction(NewObject(ctor))]
pub fn new_object<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    ctor: &UserMethod,
) -> StepResult {
    let (mut method, lookup) = ctx
        .resolver()
        .find_generic_method(&MethodSource::User(*ctor), &ctx.current_context());

    if method.method.name == "CtorArraySentinel"
        && let UserMethod::Reference(r) = *ctor
    {
        let resolution = ctx.current_frame().source_resolution;
        let method_ref = &resolution[r];

        if let MethodReferenceParent::Type(t) = &method_ref.parent {
            let concrete = {
                let res_ctx = ctx.current_context();
                res_ctx.generics.make_concrete(resolution, t.clone())
            };

            if let BaseType::Array(element, shape) = concrete.get() {
                let rank = shape.rank;
                let mut dims: Vec<usize> = (0..rank)
                    .map(|_| {
                        let v = ctx.pop(gc);
                        match v {
                            StackValue::Int32(i) => i as usize,
                            StackValue::NativeInt(i) => i as usize,
                            _ => panic!("Invalid dimension {:?}", v),
                        }
                    })
                    .collect();
                dims.reverse();

                let total_len: usize = dims.iter().product();

                let res_ctx = ctx.current_context();
                let elem_type = res_ctx.normalize_type(element.clone());

                let layout =
                    LayoutFactory::create_array_layout(elem_type.clone(), total_len, &res_ctx);
                let total_size_bytes = layout.element_layout.size() * total_len;

                let vec_obj = dotnet_value::object::Vector::new(
                    elem_type,
                    layout,
                    vec![0; total_size_bytes],
                    dims,
                );
                let o = ObjectRef::new(gc, HeapStorage::Vec(vec_obj));
                ctx.register_new_object(&o);
                ctx.push(gc, StackValue::ObjectRef(o));
                return StepResult::Continue;
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
            let base = ctx.loader().corlib_type(&type_name);
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
        let val = ctx.pop(gc);
        let native_val = match val {
            StackValue::Int32(i) => i as isize,
            StackValue::Int64(i) => i as isize,
            StackValue::NativeInt(i) => i,
            _ => panic!("Invalid argument for IntPtr constructor: {:?}", val),
        };
        ctx.push(gc, StackValue::NativeInt(native_val));
        StepResult::Continue
    } else {
        if ctx.is_intrinsic_cached(method) {
            return intrinsic_call(gc, ctx, method, &lookup);
        }

        let res = ctx.initialize_static_storage(gc, parent, lookup.clone());
        if res != StepResult::Continue {
            return res;
        }

        let res_ctx = ctx
            .current_context()
            .for_type_with_generics(parent, &lookup);
        let instance = res_ctx.new_object(parent);

        ctx.constructor_frame(
            gc,
            instance,
            crate::MethodInfo::new(method, &lookup, ctx.shared().clone()),
            lookup,
        );
        StepResult::FramePushed
    }
}

#[dotnet_instruction(LoadObject { param0 })]
pub fn ldobj<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: &MethodType,
) -> StepResult {
    let addr = ctx.pop(gc);
    let (source_ptr, _owner, _origin, _offset) = get_ptr_context(ctx, &addr);

    if source_ptr.is_null() {
        return ctx.throw_by_name(gc, "System.NullReferenceException");
    }

    let load_type = ctx.make_concrete(param0);
    let res_ctx = ctx.current_context();
    let layout = type_layout(load_type.clone(), &res_ctx);

    let mut source_vec = vec![0u8; layout.size()];
    unsafe { ptr::copy_nonoverlapping(source_ptr, source_vec.as_mut_ptr(), layout.size()) };
    let value = res_ctx
        .read_cts_value(&load_type, &source_vec, gc)
        .into_stack(gc);

    ctx.push(gc, value);
    StepResult::Continue
}

#[dotnet_instruction(StoreObject { param0 })]
pub fn stobj<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: &MethodType,
) -> StepResult {
    let value = ctx.pop(gc);
    let addr = ctx.pop(gc);

    let concrete_t = ctx.make_concrete(param0);

    let (dest_ptr, _owner, _origin, _offset) = get_ptr_context(ctx, &addr);
    if dest_ptr.is_null() {
        return ctx.throw_by_name(gc, "System.NullReferenceException");
    }

    let res_ctx = ctx.current_context();
    let layout = type_layout(concrete_t.clone(), &res_ctx);
    let mut bytes = vec![0u8; layout.size()];
    res_ctx.new_cts_value(&concrete_t, value).write(&mut bytes);

    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), dest_ptr, bytes.len());
    }
    StepResult::Continue
}

#[dotnet_instruction(InitializeForObject(param0))]
pub fn initobj<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: &MethodType,
) -> StepResult {
    let addr = ctx.pop(gc);
    let (target, _owner, _origin, _offset) = get_ptr_context(ctx, &addr);
    if target.is_null() {
        return ctx.throw_by_name(gc, "System.NullReferenceException");
    }

    let ct = ctx.make_concrete(param0);
    let res_ctx = ctx.current_context();
    let layout = type_layout(ct.clone(), &res_ctx);

    unsafe { ptr::write_bytes(target, 0, layout.size()) };
    StepResult::Continue
}

#[dotnet_instruction(Sizeof(param0))]
pub fn sizeof<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    param0: &MethodType,
) -> StepResult {
    let target = ctx.make_concrete(param0);
    let res_ctx = ctx.current_context();
    let layout = type_layout(target, &res_ctx);
    ctx.push(gc, StackValue::Int32(layout.size() as i32));
    StepResult::Continue
}

#[dotnet_instruction(LoadString(chars))]
pub fn ldstr<'gc, 'm: 'gc>(
    ctx: &mut VesContext<'_, 'gc, 'm>,
    gc: GCHandle<'gc>,
    chars: &[u16],
) -> StepResult {
    use dotnet_value::string::CLRString;

    let s = CLRString::new(chars.to_owned());
    let storage = HeapStorage::Str(s);
    let obj_ref = ObjectRef::new(gc, storage);
    ctx.push(gc, StackValue::ObjectRef(obj_ref));
    StepResult::Continue
}
