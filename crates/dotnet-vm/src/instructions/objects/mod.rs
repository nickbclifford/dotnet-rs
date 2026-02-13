use crate::{
    StepResult,
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
    pointer::{PointerOrigin, UnmanagedPtr},
};
use dotnetdll::prelude::*;

pub mod arrays;
pub mod boxing;
pub mod casting;
pub mod fields;
pub mod typed_references;

pub(crate) fn get_ptr_info<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    _ctx: &T,
    val: &StackValue<'gc>,
) -> (PointerOrigin<'gc>, dotnet_utils::ByteOffset) {
    match val {
        StackValue::ObjectRef(o) => (
            o.0.map_or(PointerOrigin::Unmanaged, |h| {
                PointerOrigin::Heap(ObjectRef(Some(h)))
            }),
            dotnet_utils::ByteOffset(0),
        ),
        StackValue::ManagedPtr(m) => (m.origin.clone(), m.offset),
        StackValue::UnmanagedPtr(UnmanagedPtr(p)) => (
            PointerOrigin::Unmanaged,
            dotnet_utils::ByteOffset(p.as_ptr() as usize),
        ),
        StackValue::NativeInt(p) => (
            PointerOrigin::Unmanaged,
            dotnet_utils::ByteOffset(*p as usize),
        ),
        StackValue::ValueType(v) => (PointerOrigin::ValueType(v.clone()), dotnet_utils::ByteOffset(0)),
        _ => panic!("Invalid parent for field/element access: {:?}", val),
    }
}

pub(crate) fn get_ptr_context<'gc, 'm: 'gc, T: StackOps<'gc, 'm> + ?Sized>(
    ctx: &T,
    val: &StackValue<'gc>,
) -> (PointerOrigin<'gc>, dotnet_utils::ByteOffset) {
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
                let mut dims: Vec<usize> = (0..rank)
                    .map(|_| {
                        let v = ctx.pop();
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
            _ => panic!("Invalid argument for IntPtr constructor: {:?}", val),
        };
        ctx.push(StackValue::NativeInt(native_val));
        StepResult::Continue
    } else {
        if ctx.is_intrinsic_cached(method) {
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
    let (origin, offset) = get_ptr_context(ctx, &addr);

    if matches!(origin, PointerOrigin::Unmanaged) && offset.as_usize() == 0 {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let load_type = vm_try!(ctx.make_concrete(param0));
    let res_ctx = ctx.current_context();
    let layout = vm_try!(type_layout(load_type.clone(), &res_ctx));

    let mut source_vec = vec![0u8; layout.size().as_usize()];
    if let Err(_e) = unsafe { ctx.read_bytes(origin, offset, &mut source_vec) } {
        return ctx.throw_by_name("System.AccessViolationException");
    }

    let value =
        vm_try!(res_ctx.read_cts_value(&load_type, &source_vec, ctx.gc())).into_stack(ctx.gc());

    ctx.push(value);
    StepResult::Continue
}

#[dotnet_instruction(StoreObject { param0 })]
pub fn stobj<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, param0: &MethodType) -> StepResult {
    let value = ctx.pop();
    let addr = ctx.pop();

    let concrete_t = vm_try!(ctx.make_concrete(param0));

    let (origin, offset) = get_ptr_context(ctx, &addr);
    if matches!(origin, PointerOrigin::Unmanaged) && offset.as_usize() == 0 {
        return ctx.throw_by_name("System.NullReferenceException");
    }

    let res_ctx = ctx.current_context();
    let layout = vm_try!(type_layout(concrete_t.clone(), &res_ctx));
    let mut bytes = vec![0u8; layout.size().as_usize()];
    vm_try!(res_ctx.new_cts_value(&concrete_t, value)).write(&mut bytes);

    if let Err(_e) = unsafe { ctx.write_bytes(origin, offset, &bytes) } {
        return ctx.throw_by_name("System.AccessViolationException");
    }
    StepResult::Continue
}

#[dotnet_instruction(InitializeForObject(param0))]
pub fn initobj<'gc, 'm: 'gc>(ctx: &mut dyn VesOps<'gc, 'm>, param0: &MethodType) -> StepResult {
    let addr = ctx.pop();
    let (origin, offset) = get_ptr_context(ctx, &addr);
    if matches!(origin, PointerOrigin::Unmanaged) && offset.as_usize() == 0 {
        return ctx.throw_by_name("System.NullReferenceException");
    }

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
