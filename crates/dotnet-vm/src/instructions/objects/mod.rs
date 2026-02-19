use crate::{
    ExceptionOps, StepResult,
    intrinsics::intrinsic_call,
    layout::{LayoutFactory, type_layout},
    resolution::ValueResolution,
    stack::ops::{StackOps, VesOps},
};
use dotnet_macros::dotnet_instruction;
use dotnet_types::{comparer::decompose_type_source, members::MethodDescription};
use dotnet_value::{
    CLRString, StackValue,
    layout::HasLayout,
    object::{HeapStorage, ObjectRef},
    pointer::{ManagedPtr, PointerOrigin, UnmanagedPtr},
};
use dotnetdll::prelude::*;
use tracing::error;

pub mod arrays;
pub mod boxing;
pub mod casting;
pub mod fields;
pub mod typed_references;

pub(crate) fn get_ptr_info<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    val: &StackValue<'gc>,
) -> Result<(PointerOrigin<'gc>, dotnet_utils::ByteOffset), StepResult> {
    match val {
        StackValue::ObjectRef(o) => Ok((
            o.0.map_or(PointerOrigin::Unmanaged, |h| {
                PointerOrigin::Heap(ObjectRef(Some(h)))
            }),
            dotnet_utils::ByteOffset(0),
        )),
        StackValue::ManagedPtr(m) => Ok((m.origin.clone(), m.offset)),
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => Ok((
            PointerOrigin::Unmanaged,
            dotnet_utils::ByteOffset(p.as_ptr() as usize),
        )),
        StackValue::NativeInt(p) => Ok((
            PointerOrigin::Unmanaged,
            dotnet_utils::ByteOffset(*p as usize),
        )),
        StackValue::ValueType(obj) => Ok((
            PointerOrigin::Transient(obj.clone()),
            dotnet_utils::ByteOffset(0),
        )),
        _ => Err(ctx.throw_by_name("System.InvalidProgramException")),
    }
}

pub(crate) fn get_ptr_context<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ExceptionOps<'gc> + ?Sized>(
    ctx: &mut T,
    val: &StackValue<'gc>,
) -> Result<(PointerOrigin<'gc>, dotnet_utils::ByteOffset), StepResult> {
    get_ptr_info(ctx, val)
}

#[dotnet_instruction(NewObject(ctor))]
pub fn new_object<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, ctor: &UserMethod) -> StepResult {
    let (mut method, lookup) = vm_try!(
        ctx.resolver()
            .find_generic_method(&MethodSource::User(*ctor), &ctx.current_context())
    );

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

            let concrete = vm_try!(concrete);
            if let BaseType::Array(element, shape) = concrete.get() {
                let rank = shape.rank;
                let mut dims: Vec<usize> = Vec::with_capacity(rank);
                for _ in 0..rank {
                    let v = ctx.pop();
                    match v {
                        StackValue::Int32(i) => dims.push(i as usize),
                        StackValue::NativeInt(i) => dims.push(i as usize),
                        _ => return ctx.throw_by_name("System.InvalidProgramException"),
                    }
                }
                dims.reverse();

                let total_len: usize = dims.iter().product();

                let res_ctx = ctx.current_context();
                let elem_type = vm_try!(res_ctx.normalize_type(element.clone()));

                let layout = vm_try!(LayoutFactory::create_array_layout(
                    elem_type.clone(),
                    total_len,
                    &res_ctx
                ));
                let total_size_bytes = layout.element_layout.size() * total_len;

                let vec_obj = dotnet_value::object::Vector::new(
                    elem_type,
                    layout,
                    vec![0; total_size_bytes.as_usize()],
                    dims,
                );
                let o = ObjectRef::new(ctx.gc(), HeapStorage::Vec(vec_obj));
                ctx.register_new_object(&o);
                ctx.push(StackValue::ObjectRef(o));
                return StepResult::Continue;
            }
        }
    }

    let parent = method.parent;
    if let (None, Some(ts)) = (&method.method.body, &parent.definition().extends) {
        let (ut, _) = decompose_type_source::<MemberType>(ts);
        let type_name = ut.type_name(parent.resolution.definition());
        // delegate types are only allowed to have these base types
        if matches!(
            type_name.as_ref(),
            "System.Delegate" | "System.MulticastDelegate"
        ) {
            let base = ctx
                .loader()
                .corlib_type(&type_name)
                .expect("Failed to locate corlib base for delegate");
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
        let val = ctx.pop();
        let native_val = match val {
            StackValue::Int32(i) => i as isize,
            StackValue::Int64(i) => i as isize,
            StackValue::NativeInt(i) => i,
            _ => return ctx.throw_by_name("System.InvalidProgramException"),
        };
        ctx.push(StackValue::NativeInt(native_val));
        StepResult::Continue
    } else {
        if ctx.is_intrinsic_cached(method) {
            let is_value_type = vm_try!(ctx.resolver().is_value_type(parent));
            if is_value_type && method_name == ".ctor" && parent_name != "System.String" {
                let arg_count = method.method.signature.parameters.len();
                let args = ctx.pop_multiple(arg_count);

                let res_ctx = ctx
                    .current_context()
                    .for_type_with_generics(parent, &lookup);
                let instance = vm_try!(res_ctx.new_object(parent));

                ctx.push_value_type(instance);
                let this_slot = ctx.top_of_stack() - 1;
                let this_ptr_slot = this_slot + 1;

                // Push placeholder ManagedPtr
                ctx.push_managed_ptr(ManagedPtr::new(None, parent, None, false, None));

                for arg in args {
                    ctx.push(arg);
                }

                // Update ManagedPtr with the now-stable address
                let real_addr = ctx.get_slot_address(this_slot);
                let this_ptr_val = match ctx.get_slot(this_ptr_slot) {
                    StackValue::ManagedPtr(p) => p,
                    _ => unreachable!(),
                };
                // IMPORTANT: Set the stack origin BEFORE updating the cached ptr,
                // otherwise update_cached_ptr will set offset to the absolute address
                // thinking it's an Unmanaged pointer.
                let mut this_ptr_val =
                    this_ptr_val.with_stack_origin(this_slot, dotnet_utils::ByteOffset(0));
                this_ptr_val.update_cached_ptr(real_addr);
                ctx.set_slot(this_ptr_slot, StackValue::ManagedPtr(this_ptr_val));

                let res = intrinsic_call(ctx, method, &lookup);
                if res != StepResult::Continue {
                    return res;
                }

                return StepResult::Continue;
            }

            return intrinsic_call(ctx, method, &lookup);
        }

        let res = ctx.initialize_static_storage(parent, lookup.clone());
        if res != StepResult::Continue {
            return res;
        }

        let res_ctx = ctx
            .current_context()
            .for_type_with_generics(parent, &lookup);
        let instance = vm_try!(res_ctx.new_object(parent));

        vm_try!(ctx.constructor_frame(
            instance,
            vm_try!(crate::MethodInfo::new(
                method,
                &lookup,
                ctx.shared().clone()
            )),
            lookup,
        ));
        StepResult::FramePushed
    }
}

#[dotnet_instruction(LoadObject { param0 })]
pub fn ldobj<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, param0: &MethodType) -> StepResult {
    let addr = ctx.pop();

    if addr.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let (origin, offset) = match get_ptr_context(ctx, &addr) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let load_type = vm_try!(ctx.make_concrete(param0));
    let res_ctx = ctx.current_context();
    let layout = vm_try!(type_layout(load_type.clone(), &res_ctx));

    let mut source_vec = vec![0u8; layout.size().as_usize()];
    if let Err(_e) = unsafe { ctx.read_bytes(origin.clone(), offset, &mut source_vec) } {
        return ctx.throw_by_name("System.AccessViolationException");
    }

    let value = vm_try!(res_ctx.read_cts_value(&load_type, &source_vec, ctx.gc())).into_stack();

    ctx.push(value);
    StepResult::Continue
}

#[dotnet_instruction(StoreObject { param0 })]
pub fn stobj<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, param0: &MethodType) -> StepResult {
    let value = ctx.pop();
    let addr = ctx.pop();

    if addr.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let concrete_t = vm_try!(ctx.make_concrete(param0));

    let (origin, offset) = match get_ptr_context(ctx, &addr) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let res_ctx = ctx.current_context();
    let layout = vm_try!(type_layout(concrete_t.clone(), &res_ctx));

    if layout.is_or_contains_refs() {
        if let Err(e) = unsafe { ctx.write_unaligned(origin, offset, value, &layout) } {
            error!("stobj failed: {}", e);
            return ctx.throw_by_name("System.AccessViolationException");
        }
    } else {
        let mut bytes = vec![0u8; layout.size().as_usize()];
        vm_try!(res_ctx.new_cts_value(&concrete_t, value)).write(&mut bytes);

        if let Err(_e) = unsafe { ctx.write_bytes(origin, offset, &bytes) } {
            return ctx.throw_by_name("System.AccessViolationException");
        }
    }
    StepResult::Continue
}

#[dotnet_instruction(InitializeForObject(param0))]
pub fn initobj<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, param0: &MethodType) -> StepResult {
    let addr = ctx.pop();

    if addr.is_null() {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let (origin, offset) = match get_ptr_context(ctx, &addr) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let ct = vm_try!(ctx.make_concrete(param0));
    let res_ctx = ctx.current_context();
    let layout = vm_try!(type_layout(ct.clone(), &res_ctx));

    let zero_bytes = vec![0u8; layout.size().as_usize()];
    if let Err(_e) = unsafe { ctx.write_bytes(origin, offset, &zero_bytes) } {
        return ctx.throw_by_name("System.AccessViolationException");
    }
    StepResult::Continue
}

#[dotnet_instruction(Sizeof(param0))]
pub fn sizeof<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, param0: &MethodType) -> StepResult {
    let target = vm_try!(ctx.make_concrete(param0));
    let res_ctx = ctx.current_context();
    let layout = vm_try!(type_layout(target, &res_ctx));
    ctx.push(StackValue::Int32(layout.size().as_usize() as i32));
    StepResult::Continue
}

#[dotnet_instruction(LoadString(chars))]
pub fn ldstr<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,

    chars: &[u16],
) -> StepResult {
    ctx.push_string(CLRString::new(chars.to_owned()));
    StepResult::Continue
}
