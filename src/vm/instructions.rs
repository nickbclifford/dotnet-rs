use super::{intrinsics::*, CallStack, GCHandle, MethodInfo, StepResult};
use crate::utils::decompose_type_source;
use crate::value::ConcreteType;
use crate::{
    value::{
        layout::{type_layout, FieldLayoutManager, HasLayout},
        string::CLRString,
        CTSValue, Context, GenericLookup, HeapStorage, ManagedPtr, MethodDescription, Object,
        ObjectRef, StackValue, TypeDescription, UnmanagedPtr, ValueType, Vector,
    },
    vm::exceptions::{HandlerKind, ProtectedSection},
};
use dotnetdll::prelude::*;
use std::{cmp::Ordering, rc::Rc};

const INTRINSIC_ATTR: &'static str = "System.Runtime.CompilerServices.IntrinsicAttribute";

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    fn find_generic_method(&self, source: &MethodSource) -> (MethodDescription, GenericLookup) {
        let ctx = self.current_context();

        let mut new_lookup = ctx.generics.clone();

        let method = match source {
            MethodSource::User(u) => *u,
            MethodSource::Generic(g) => {
                new_lookup.method_generics =
                    g.parameters.iter().map(|t| ctx.make_concrete(t)).collect();
                g.base
            }
        };

        if let UserMethod::Reference(r) = method {
            if let MethodReferenceParent::Type(t) = &ctx.resolution[r].parent {
                let parent = ctx.make_concrete(t);
                if let BaseType::Type {
                    source: TypeSource::Generic { parameters, .. },
                    ..
                } = parent.get()
                {
                    new_lookup.type_generics = parameters.clone();
                }
            }
        }

        (ctx.locate_method(method, &new_lookup), new_lookup)
    }

    pub fn initialize_static_storage(
        &mut self,
        gc: GCHandle<'gc>,
        description: TypeDescription,
        generics: GenericLookup,
    ) -> bool {
        let ctx = Context::with_generics(self.current_context(), &generics);
        let value = {
            let mut statics = self.statics.borrow_mut();
            statics.init(description, ctx.clone())
        };
        if let Some(m) = value {
            super::msg!(
                self,
                "-- calling static constructor (will return to ip {}) --",
                self.current_frame().state.ip
            );
            self.call_frame(
                gc,
                MethodInfo::new(m.resolution(), m.method, &generics, &self.assemblies),
                generics,
            );
            true
        } else {
            false
        }
    }

    pub fn step(&mut self, gc: GCHandle<'gc>) -> StepResult {
        use Instruction::*;
        use NumberSign::*;

        macro_rules! statics {
            (|$s:ident| $body:expr) => {{
                #[allow(unused_mut)]
                let mut $s = self.statics.borrow_mut();
                $body
            }};
        }

        macro_rules! push {
            ($variant:ident ( $($args:expr),* )) => {
                push!(StackValue::$variant($($args),*))
            };
            ($val:expr) => {
                self.push_stack(gc, $val)
            };
        }
        macro_rules! pop {
            () => {
                self.pop_stack()
            };
        }

        macro_rules! state {
            (|$state:ident| $body:expr) => {{
                let frame = self.current_frame_mut();
                let $state = &mut frame.state;
                $body
            }};
        }

        let mut moved_ip = false;

        macro_rules! branch {
            ($ip:expr) => {{
                state!(|s| s.ip = *$ip);
                moved_ip = true;
            }};
        }
        macro_rules! conditional_branch {
            ($condition:expr, $ip:expr) => {{
                if $condition {
                    branch!($ip);
                }
            }};
        }
        macro_rules! equal {
            () => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (ManagedPtr(m), NativeInt(r)) => m.value as isize == r,
                    (NativeInt(l), ManagedPtr(m)) => l == m.value as isize,
                    (ObjectRef(l), ObjectRef(r)) => l == r,
                    (l, r) => l.partial_cmp(&r) == Some(Ordering::Equal),
                }
            }};
        }
        macro_rules! compare {
            ($sgn:expr, $op:tt ( $order:pat )) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2, $sgn) {
                    (Int32(l), Int32(r), Unsigned) => {
                        (l as u32) $op (r as u32)
                    }
                    (Int32(l), NativeInt(r), Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (Int64(l), Int64(r), Unsigned) => {
                        (l as u64) $op (r as u64)
                    }
                    (NativeInt(l), Int32(r), Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (NativeInt(l), NativeInt(r), Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (l, r, _) => {
                        matches!(l.partial_cmp(&r), Some($order))
                    }
                }
            }}
        }

        macro_rules! binary_arith_op {
            ($method:ident (f64 $op:tt) $(, { $($pat:pat => $arm:expr, )* } )?) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(i1.$method(i2)))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(NativeInt((i1 as isize).$method(i2)))
                    }
                    (Int64(i1), Int64(i2)) => {
                        push!(Int64(i1.$method(i2)))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(i1.$method(i2 as isize)))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(i1.$method(i2)))
                    }
                    (NativeFloat(f1), NativeFloat(f2)) => {
                        push!(NativeFloat(f1 $op f2))
                    }
                    $($($pat => $arm,)*)?
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($method)
                    ),
                }
            }};
            ($op:tt $(, { $($pat:pat => $arm:expr, )* })?) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(NativeInt((i1 as isize) $op i2))
                    }
                    (Int64(i1), Int64(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(i1 $op (i2 as isize)))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (NativeFloat(f1), NativeFloat(f2)) => {
                        push!(NativeFloat(f1 $op f2))
                    }
                    $($($pat => $arm,)*)?
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            }}
        }
        macro_rules! binary_checked_op {
            ($sign:expr, $method:ident (f64 $op:tt) $(, { $($pat:pat => $arm:expr, )* })?) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2, $sign) {
                    (Int32(i1), Int32(i2), Signed) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(Int32(val))
                    }
                    (Int32(i1), NativeInt(i2), Signed) => {
                        let Some(val) = (i1 as isize).$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val))
                    }
                    (Int64(i1), Int64(i2), Signed) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(Int64(val));
                    }
                    (NativeInt(i1), Int32(i2), Signed) => {
                        let Some(val) = i1.$method(i2 as isize) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val));
                    }
                    (NativeInt(i1), NativeInt(i2), Signed) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val));
                    }
                    (NativeFloat(f1), NativeFloat(f2), Signed) => {
                        push!(NativeFloat(f1 $op f2));
                    }
                    (Int32(i1), Int32(i2), Unsigned) => {
                        let Some(val) = (i1 as u32).$method(i2 as u32) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(Int32(val as i32))
                    }
                    (Int32(i1), NativeInt(i2), Unsigned) => {
                        let Some(val) = (i1 as usize).$method(i2 as usize) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val as isize))
                    }
                    (Int64(i1), Int64(i2), Unsigned) => {
                        let Some(val) = (i1 as u64).$method(i2 as u64) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(Int64(val as i64));
                    }
                    (NativeInt(i1), Int32(i2), Unsigned) => {
                        let Some(val) = i1.$method(i2 as isize) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val as isize));
                    }
                    (NativeInt(i1), NativeInt(i2), Unsigned) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val as isize));
                    }
                    $($($pat => $arm,)*)?
                    (v1, v2, _) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} {:?} operation",
                        v1,
                        v2,
                        stringify!($method),
                        $sign
                    ),
                }
            }}
        }
        macro_rules! binary_int_op {
            ($op:tt) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(NativeInt((i1 as isize) $op i2))
                    }
                    (Int64(i1), Int64(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(i1 $op (i2 as isize)))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            }};
            ($op:tt as unsigned) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (Int64(i1), Int64(i2)) => {
                        push!(Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} unsigned operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            }}
        }
        macro_rules! shift_op {
            ($op:tt) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (Int64(i1), Int32(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (Int64(i1), NativeInt(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            }};
            ($op:tt as unsigned) => {{
                use StackValue::*;
                let value2 = pop!();
                let value1 = pop!();
                match (value1, value2) {
                    (Int32(i1), Int32(i2)) => {
                        push!(Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (Int32(i1), NativeInt(i2)) => {
                        push!(Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (Int64(i1), Int32(i2)) => {
                        push!(Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (Int64(i1), NativeInt(i2)) => {
                        push!(Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (NativeInt(i1), Int32(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (NativeInt(i1), NativeInt(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            }}
        }

        macro_rules! check_special_fields {
            ($field:ident) => {
                if $field.field.literal {
                    todo!(
                        "field {}::{} has literal",
                        $field.parent.type_name(),
                        $field.field.name
                    );
                }
                if let Some(c) = &$field.field.default {
                    todo!(
                        "field {}::{} has constant {:?}",
                        $field.parent.type_name(),
                        $field.field.name,
                        c
                    );
                }
                for a in &$field.field.attributes {
                    let ctor = self
                        .assemblies
                        .locate_attribute($field.parent.resolution, a);
                    if ctor.parent.type_name() == INTRINSIC_ATTR {
                        intrinsic_field(
                            gc,
                            self,
                            $field,
                            self.current_context().generics.type_generics.clone(),
                        );
                        return StepResult::InstructionStepped;
                    }
                }
            };
        }

        macro_rules! is_nullish {
            ($val:expr) => {
                match $val {
                    StackValue::Int32(i) => i == 0,
                    StackValue::Int64(i) => i == 0,
                    StackValue::NativeInt(i) => i == 0,
                    StackValue::ObjectRef(ObjectRef(o)) => o.is_none(),
                    StackValue::UnmanagedPtr(UnmanagedPtr(p))
                    | StackValue::ManagedPtr(ManagedPtr { value: p, .. }) => p.is_null(),
                    v => todo!("invalid type on stack ({:?}) for truthiness check", v),
                }
            };
        }

        let i = state!(|s| &s.info_handle.instructions[s.ip]);
        let ip = state!(|s| s.ip);
        let i_res = state!(|s| s.info_handle.source_resolution);

        let sections = state!(|s| s.info_handle.exceptions.clone());
        for ps in sections {
            if ps.instructions.contains(&ip)
                && !self
                    .current_frame()
                    .protected_sections
                    .iter()
                    .any(|(s, _)| Rc::ptr_eq(&ps, s))
            {
                let handlers = ps.handlers.clone();
                self.current_frame_mut()
                    .protected_sections
                    .push_front((ps, handlers));
            }
        }

        super::msg!(
            self,
            "[#{} | ip @ {}] {}",
            self.frames.len() - 1,
            ip,
            i.show(i_res.0)
        );

        self.debug_dump();

        match i {
            Add => binary_arith_op!(wrapping_add (f64 +), {
                (Int32(i), ManagedPtr(m)) => {
                    // TODO: proper mechanisms for safety and pointer arithmetic
                    unsafe {
                        push!(ManagedPtr(m.map_value(|p| p.offset(i as isize))))
                    }
                },
                (NativeInt(i), ManagedPtr(m)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(i))))
                },
                (ManagedPtr(m), Int32(i)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(i as isize))))
                },
                (ManagedPtr(m), NativeInt(i)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(i))))
                },
            }),
            AddOverflow(sgn) => binary_checked_op!(sgn, checked_add(f64+), {
                // TODO: pointer stuff
            }),
            And => binary_int_op!(&),
            ArgumentList => todo!("arglist"),
            BranchEqual(i) => {
                conditional_branch!(equal!(), i)
            }
            BranchGreaterOrEqual(sgn, i) => {
                conditional_branch!(compare!(sgn, >= (Ordering::Greater | Ordering::Equal)), i)
            }
            BranchGreater(sgn, i) => {
                conditional_branch!(compare!(sgn, > (Ordering::Greater)), i)
            }
            BranchLessOrEqual(sgn, i) => {
                conditional_branch!(compare!(sgn, <= (Ordering::Less | Ordering::Equal)), i)
            }
            BranchLess(sgn, i) => conditional_branch!(compare!(sgn, < (Ordering::Less)), i),
            BranchNotEqual(i) => {
                conditional_branch!(!equal!(), i)
            }
            Branch(i) => branch!(i),
            Breakpoint => todo!("break"),
            BranchFalsy(i) => {
                conditional_branch!(is_nullish!(pop!()), i)
            }
            BranchTruthy(i) => {
                conditional_branch!(!is_nullish!(pop!()), i)
            }
            Call { param0: source, .. } => {
                let (method, lookup) = self.find_generic_method(source);
                macro_rules! intrinsic {
                    () => {
                        intrinsic_call(gc, self, method, lookup);
                        return StepResult::InstructionStepped;
                    };
                }

                // TODO: more centralized location for which methods are explicitly handled in this manner
                if method.method.internal_call
                    || (method.parent.type_name() == "System.Environment"
                        && method.method.name == "GetEnvironmentVariableCore")
                    || (method.parent.type_name() == "System.String"
                        && matches!(
                            method.method.name.as_ref(),
                            "GetPinnableReference"
                                | "get_Length"
                                | "get_Chars"
                                | "GetRawStringData"
                                | "IndexOf"
                                | "Substring"
                                | "Concat"
                        ))
                    || (method.parent.type_name()
                        == "System.Runtime.CompilerServices.RuntimeHelpers"
                        && method.method.name == "RunClassConstructor")
                {
                    intrinsic!();
                }

                let res = method.resolution();

                for a in &method.method.attributes {
                    let ctor = self.assemblies.locate_attribute(res, a);
                    if ctor.parent.type_name() == INTRINSIC_ATTR {
                        intrinsic!();
                    }
                }

                if method.method.pinvoke.is_some() {
                    self.external_call(method, gc);
                    return StepResult::InstructionStepped;
                }

                self.call_frame(
                    gc,
                    MethodInfo::new(res, method.method, &lookup, &self.assemblies),
                    lookup,
                );
                moved_ip = true;
            }
            CallConstrained(constraint, source) => {
                // according to the standard, this doesn't really make sense
                // because the constrained prefix should only be on callvirt
                // however, this appears to be used for static interface dispatch?

                let constraint_type = self.current_context().make_concrete(constraint);
                let (method, lookup) = self.find_generic_method(source);

                let td = self.assemblies.find_concrete_type(constraint_type.clone());

                for o in td.definition.overrides.iter() {
                    let target = self
                        .current_context()
                        .locate_method(o.implementation, &lookup);
                    let declaration = self.current_context().locate_method(o.declaration, &lookup);
                    if method == declaration {
                        super::msg!(self, "-- dispatching to {:?} --", target);
                        self.call_frame(
                            gc,
                            MethodInfo::new(
                                target.resolution(),
                                target.method,
                                &lookup,
                                &self.assemblies,
                            ),
                            lookup,
                        );
                        return StepResult::InstructionStepped;
                    }
                }

                panic!(
                    "could not find method to dispatch to for constrained call({:?}, {:?})",
                    constraint_type, method
                );
            }
            CallIndirect { .. } => todo!("calli"),
            CompareEqual => {
                let val = equal!() as i32;
                push!(Int32(val))
            }
            CompareGreater(sgn) => {
                let val = compare!(sgn, > (Ordering::Greater)) as i32;
                push!(Int32(val))
            }
            CheckFinite => {
                expect_stack!(let NativeFloat(f) = pop!());
                if f.is_infinite() || f.is_nan() {
                    todo!("ArithmeticException in ckfinite");
                }
                push!(NativeFloat(f));
            }
            CompareLess(sgn) => {
                let val = compare!(sgn, < (Ordering::Less)) as i32;
                push!(Int32(val))
            }
            Convert(t) => {
                let value = pop!();

                macro_rules! simple_cast {
                    ($t:ty) => {
                        match value {
                            StackValue::Int32(i) => i as $t,
                            StackValue::Int64(i) => i as $t,
                            StackValue::NativeInt(i) => i as $t,
                            StackValue::NativeFloat(f) => f as $t,
                            v => todo!(
                                "invalid type on stack ({:?}) for conversion to {}",
                                v,
                                stringify!($t)
                            ),
                        }
                    };
                }

                macro_rules! convert_short_ints {
                    ($t:ty) => {{
                        let i = simple_cast!($t);
                        push!(Int32(i as i32));
                    }};
                }

                macro_rules! convert_long_ints {
                    ($variant:ident ( $t:ty )) => {{
                        let i = simple_cast!($t);
                        push!($variant(i));
                    }};
                    ($variant:ident ( $t:ty as $vt:ty )) => {{
                        let i = match value {
                            // all Rust casts from signed types will sign extend
                            // so first we have to make them unsigned so they'll properly zero extend
                            StackValue::Int32(i) => (i as u32) as $t,
                            StackValue::Int64(i) => (i as u64) as $t,
                            StackValue::NativeInt(i) => (i as usize) as $t,
                            StackValue::UnmanagedPtr(UnmanagedPtr(p)) |
                            StackValue::ManagedPtr(ManagedPtr { value: p, .. }) => (p as usize) as $t,
                            StackValue::NativeFloat(f) => {
                                todo!("truncate {} towards zero for conversion to {}", f, stringify!($t))
                            }
                            v => todo!("invalid type on stack ({:?}) for conversion to {}", v, stringify!($t)),
                        };
                        push!($variant(i as $vt));
                    }}
                }

                match t {
                    ConversionType::Int8 => convert_short_ints!(i8),
                    ConversionType::UInt8 => convert_short_ints!(u8),
                    ConversionType::Int16 => convert_short_ints!(i16),
                    ConversionType::UInt16 => convert_short_ints!(u16),
                    ConversionType::Int32 => convert_short_ints!(i32),
                    ConversionType::UInt32 => convert_short_ints!(u32),
                    ConversionType::Int64 => convert_long_ints!(Int64(i64)),
                    ConversionType::UInt64 => convert_long_ints!(Int64(u64 as i64)),
                    ConversionType::IntPtr => convert_long_ints!(NativeInt(isize)),
                    ConversionType::UIntPtr => convert_long_ints!(NativeInt(usize as isize)),
                }
            }
            ConvertOverflow(t, sgn) => {
                let value = pop!();
                todo!(
                    "{:?} conversion to {:?} with overflow detection ({:?})",
                    t,
                    sgn,
                    value
                )
            }
            ConvertFloat32 => {
                let v = match pop!() {
                    StackValue::Int32(i) => i as f32,
                    StackValue::Int64(i) => i as f32,
                    StackValue::NativeInt(i) => i as f32,
                    StackValue::NativeFloat(i) => i as f32,
                    rest => todo!(
                        "invalid type on stack ({:?}) for conversion to float32",
                        rest
                    ),
                };
                push!(NativeFloat(v as f64));
            }
            ConvertFloat64 => {
                let v = match pop!() {
                    StackValue::Int32(i) => i as f64,
                    StackValue::Int64(i) => i as f64,
                    StackValue::NativeInt(i) => i as f64,
                    StackValue::NativeFloat(i) => i,
                    rest => todo!(
                        "invalid type on stack ({:?}) for conversion to float64",
                        rest
                    ),
                };
                push!(NativeFloat(v));
            }
            ConvertUnsignedToFloat => {
                let value = pop!();
                todo!("conv.r.un({:?})", value)
            }
            CopyMemoryBlock { .. } => {}
            Divide(sgn) => match sgn {
                Signed => binary_arith_op!(/),
                Unsigned => binary_int_op!(/ as unsigned),
            },
            Duplicate => {
                let val = pop!();
                push!(val.clone());
                push!(val);
            }
            EndFilter => todo!("endfilter"),
            EndFinally => {} // is this actually a nop?
            InitializeMemoryBlock { .. } => todo!("initblk"),
            Jump(_) => todo!("jmp"),
            LoadArgument(i) => {
                let arg = self.get_argument(*i as usize);
                push!(arg);
            }
            LoadArgumentAddress(i) => {
                let arg = self.get_argument(*i as usize);
                let live_type = arg.contains_type(self.current_context());
                push!(managed_ptr(
                    self.get_argument_address(*i as usize) as *mut _,
                    live_type
                ));
            }
            LoadConstantInt32(i) => push!(Int32(*i)),
            LoadConstantInt64(i) => push!(Int64(*i)),
            LoadConstantFloat32(f) => push!(NativeFloat(*f as f64)),
            LoadConstantFloat64(f) => push!(NativeFloat(*f)),
            LoadMethodPointer(source) => {
                let (method, lookup) = self.find_generic_method(source);
                let idx = match self
                    .runtime_methods
                    .iter()
                    .position(|(m, g)| *m == method && *g == lookup)
                {
                    Some(i) => i,
                    None => {
                        self.runtime_methods.push((method, lookup));
                        self.runtime_methods.len() - 1
                    }
                };
                push!(NativeInt(idx as isize));
            }
            LoadIndirect { param0: t, .. } => {
                let ptr = match pop!() {
                    StackValue::NativeInt(i) => i as *mut u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p,
                    StackValue::ManagedPtr(m) => m.value,
                    v => todo!(
                        "invalid type on stack ({:?}) for ldind operation, expected pointer",
                        v
                    ),
                };

                macro_rules! load_as_i32 {
                    ($t:ty) => {{
                        let val: $t = unsafe { *(ptr as *const _) };
                        push!(Int32(val as i32));
                    }};
                }

                match t {
                    LoadType::Int8 => load_as_i32!(i8),
                    LoadType::UInt8 => load_as_i32!(u8),
                    LoadType::Int16 => load_as_i32!(i16),
                    LoadType::UInt16 => load_as_i32!(u16),
                    LoadType::Int32 => load_as_i32!(i32),
                    LoadType::UInt32 => load_as_i32!(u32),
                    LoadType::Int64 => todo!("ldind.i8"),
                    LoadType::Float32 => todo!("ldind.r4"),
                    LoadType::Float64 => todo!("ldind.r8"),
                    LoadType::IntPtr => todo!("ldind.i"),
                    LoadType::Object => {
                        let val = unsafe { *(ptr as *const ObjectRef) };
                        push!(ObjectRef(val));
                    }
                }
            }
            LoadLocal(i) => {
                let local = self.get_local(*i as usize);
                push!(local);
            }
            LoadLocalAddress(i) => {
                let local = self.get_local(*i as usize);
                let live_type = local.contains_type(self.current_context());
                push!(managed_ptr(
                    self.get_local_address(*i as usize) as *mut _,
                    live_type
                ));
            }
            LoadNull => push!(null()),
            Leave(jump_target) => {
                // this means we made it to the end of the try block

                // NOTE this is mostly AI generated with a few manual tweaks

                // in almost every case, this will only be one section
                // however, it's possible to have nested finally blocks with no other instructions in between
                // (if there were other instructions, the outer section would have another Leave at the end anyway)
                let sections_to_leave = self
                    .current_frame()
                    .protected_sections
                    .iter()
                    .filter(|(s, _)| !s.instructions.contains(jump_target))
                    .count();

                let mut finally_handlers = Vec::new();
                for (section, handlers) in self
                    .current_frame_mut()
                    .protected_sections
                    .drain(..sections_to_leave)
                {
                    for handler in handlers {
                        if let HandlerKind::Finally = handler.kind {
                            finally_handlers.push((section.clone(), handler.clone()));
                        }
                    }
                }

                if finally_handlers.is_empty() {
                    branch!(jump_target);
                } else {
                    // Execute finally handlers from innermost to outermost
                    finally_handlers.reverse();

                    // Take the first handler to execute
                    let (_, first_handler) = finally_handlers.remove(0);

                    if !finally_handlers.is_empty() {
                        let leave_section = ProtectedSection {
                            instructions: ip..*jump_target,
                            handlers: finally_handlers.into_iter().map(|(_, h)| h).collect(),
                        };

                        self.current_frame_mut()
                            .protected_sections
                            .push_front((Rc::new(leave_section), Vec::new()));
                    }

                    branch!(&first_handler.instructions.start);
                }
            }
            LocalMemoryAllocate => {
                let size = match pop!() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    v => todo!(
                        "invalid type on stack ({:?}) for local memory allocation size",
                        v
                    ),
                };
                let ptr = state!(|s| {
                    let loc = s.memory_pool.len();
                    s.memory_pool.extend(vec![0; size]);
                    s.memory_pool[loc..].as_mut_ptr()
                });
                push!(unmanaged_ptr(ptr));
            }
            Multiply => binary_arith_op!(wrapping_mul (f64 *)),
            MultiplyOverflow(sgn) => binary_checked_op!(sgn, checked_mul (f64 *)),
            Negate => match pop!() {
                StackValue::Int32(i) => push!(Int32(-i)),
                StackValue::Int64(i) => push!(Int64(-i)),
                StackValue::NativeInt(i) => push!(NativeInt(-i)),
                v => todo!(
                    "invalid type on stack ({:?}) for logical NOT, expected integer",
                    v
                ),
            },
            NoOperation => {} // :3
            Not => match pop!() {
                StackValue::Int32(i) => push!(Int32(!i)),
                StackValue::Int64(i) => push!(Int64(!i)),
                StackValue::NativeInt(i) => push!(NativeInt(!i)),
                v => todo!(
                    "invalid type on stack ({:?}) for bitwise NOT, expected integer",
                    v
                ),
            },
            Or => binary_int_op!(|),
            Pop => {
                pop!();
            }
            Remainder(sgn) => match sgn {
                Signed => binary_arith_op!(%),
                Unsigned => binary_int_op!(% as unsigned),
            },
            Return => {
                // expects a single value on stack for non-void methods
                // call stack manager will put this in the right spot for the caller
                return StepResult::MethodReturned;
            }
            ShiftLeft => shift_op!(<<),
            ShiftRight(sgn) => match sgn {
                Signed => shift_op!(>>),
                Unsigned => shift_op!(>> as unsigned),
            },
            StoreArgument(i) => {
                let val = pop!();
                self.set_argument(gc, *i as usize, val);
            }
            StoreIndirect {
                param0: store_type, ..
            } => {
                let val = pop!();
                let ptr = match pop!() {
                    StackValue::NativeInt(i) => i as *mut u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p,
                    StackValue::ManagedPtr(m) => m.value,
                    v => todo!(
                        "invalid type on stack ({:?}) for stind operation, expected pointer",
                        v
                    ),
                };

                macro_rules! store_from_i32 {
                    ($t:ty) => {{
                        expect_stack!(let Int32(v) = val);
                        let p = ptr as *mut $t;
                        unsafe {
                            *p = v as $t;
                        }
                    }};
                }

                match store_type {
                    StoreType::Int8 => store_from_i32!(i8),
                    StoreType::Int16 => store_from_i32!(i16),
                    StoreType::Int32 => store_from_i32!(i32),
                    StoreType::Int64 => todo!("stind.i8"),
                    StoreType::Float32 => todo!("stind.r4"),
                    StoreType::Float64 => todo!("stind.r8"),
                    StoreType::IntPtr => todo!("stind.i"),
                    StoreType::Object => {
                        expect_stack!(let ObjectRef(o) = val);
                        let p = ptr as *mut ObjectRef;
                        unsafe {
                            *p = o;
                        }
                    }
                }
            }
            StoreLocal(i) => {
                let val = pop!();
                self.set_local(gc, *i as usize, val);
            }
            Subtract => binary_arith_op!(wrapping_sub (f64 -), {
                (ManagedPtr(m), Int32(i)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(-i as isize))))
                },
                (ManagedPtr(m), NativeInt(i)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(-i))))
                },
                (ManagedPtr(m1), ManagedPtr(m2)) => {
                    push!(NativeInt((m1.value as isize) - (m2.value as isize)))
                },
            }),
            SubtractOverflow(sgn) => binary_checked_op!(sgn, checked_sub (f64 -), {
                // TODO: pointer stuff
            }),
            Switch(targets) => {
                expect_stack!(let Int32(value as usize) = pop!());
                conditional_branch!(value < targets.len(), &targets[value]);
            }
            Xor => binary_int_op!(^),
            BoxValue(t) => {
                let t = self.current_context().make_concrete(t);

                let value = pop!();

                if let StackValue::ObjectRef(_) = value {
                    // boxing is a noop for all reference types
                    push!(value);
                } else {
                    push!(ObjectRef(ObjectRef::new(
                        gc,
                        HeapStorage::Boxed(ValueType::new(&t, &self.current_context(), value))
                    )));
                }
            }
            CallVirtual { param0: source, .. } => {
                let (base_method, lookup) = self.find_generic_method(source);

                // see note in Call
                if base_method.parent.type_name() == "System.String"
                    && matches!(
                        base_method.method.name.as_ref(),
                        "GetPinnableReference"
                            | "get_Length"
                            | "get_Chars"
                            | "GetRawStringData"
                            | "IndexOf"
                            | "Substring"
                            | "GetHashCodeOrdinalIgnoreCase"
                    )
                {
                    intrinsic_call(gc, self, base_method, lookup);
                    return StepResult::InstructionStepped;
                }

                let num_args = 1 + base_method.method.signature.parameters.len();
                let mut args = Vec::new();
                for _ in 0..num_args {
                    args.push(pop!());
                }
                args.reverse();

                // value types are passed as managed pointers (I.8.9.7)
                let this_value = args[0].clone();
                let this_type = match this_value {
                    StackValue::ObjectRef(ObjectRef(None)) => todo!("null pointer exception"),
                    StackValue::ObjectRef(ObjectRef(Some(o))) => {
                        self.current_context().get_heap_description(o)
                    },
                    StackValue::ManagedPtr(m) => m.inner_type,
                    rest => panic!("invalid this argument for virtual call (expected ObjectRef or ManagedPtr, received {:?})", rest)
                };

                // TODO: check explicit overrides

                let mut found = None;
                for (parent, _) in self.current_context().get_ancestors(this_type) {
                    if let Some(method) = self.assemblies.find_method_in_type(
                        parent,
                        &base_method.method.name,
                        &base_method.method.signature,
                        base_method.resolution(),
                    ) {
                        found = Some((parent, method));
                        break;
                    }
                }

                if let Some((parent, method)) = found {
                    for a in args {
                        push!(a);
                    }
                    if method.method.internal_call {
                        intrinsic_call(gc, self, method, lookup);
                        return StepResult::InstructionStepped;
                    }
                    self.call_frame(
                        gc,
                        MethodInfo::new(
                            parent.resolution,
                            method.method,
                            &lookup,
                            &self.assemblies,
                        ),
                        lookup,
                    );
                    moved_ip = true;
                } else {
                    panic!("could not resolve virtual call");
                }
            }
            CallVirtualConstrained(_, _) => todo!("constrained.callvirt"),
            CallVirtualTail(_) => todo!("tail.callvirt"),
            CastClass { .. } => todo!("castclass"),
            CopyObject(_) => todo!("cpobj"),
            InitializeForObject(t) => {
                let ctx = self.current_context();
                let layout = type_layout(ctx.make_concrete(t), ctx);

                let target = match pop!() {
                    StackValue::ManagedPtr(m) => m.value,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p,
                    err => todo!("invalid type on stack ({:?}) for initobj", err),
                };

                let s = unsafe { std::slice::from_raw_parts_mut(target, layout.size()) };
                s.fill(0);
            }
            IsInstance(target) => {
                expect_stack!(let ObjectRef(target_obj) = pop!());
                if let ObjectRef(Some(o)) = target_obj {
                    let ctx = self.current_context();
                    let obj_type = ctx.get_heap_description(o);
                    let target_type = self
                        .assemblies
                        .find_concrete_type(ctx.make_concrete(target));

                    if ctx.is_a(obj_type, target_type) {
                        push!(ObjectRef(target_obj));
                    } else {
                        push!(null());
                    }
                } else {
                    // isinst returns null for null inputs (III.4.6)
                    push!(null());
                }
            }
            LoadElement {
                param0: load_type, ..
            } => {
                expect_stack!(let Int32(index as usize) = pop!());
                expect_stack!(let ObjectRef(obj) = pop!());

                let ctx = self.current_context();
                let load_type = ctx.make_concrete(load_type);
                let value = obj.as_vector(|array| {
                    let elem_size = array.layout.element_layout.size();
                    let target = &array.get()[(elem_size * index)..(elem_size * (index + 1))];
                    CTSValue::read(&load_type, &ctx, target).into_stack()
                });
                push!(value);
            }
            LoadElementPrimitive {
                param0: load_type, ..
            } => {
                use LoadType::*;

                let index = match pop!() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => todo!(
                        "invalid index for stelem (expected int32 or native int, received {:?})",
                        rest
                    ),
                };
                let array = pop!();

                expect_stack!(let ObjectRef(obj) = array);

                let elem_size: usize = match load_type {
                    Int8 | UInt8 => 1,
                    Int16 | UInt16 => 2,
                    Int32 | UInt32 => 4,
                    Int64 => 8,
                    Float32 => 4,
                    Float64 => 8,
                    IntPtr => size_of::<usize>(),
                    Object => ObjectRef::SIZE,
                };

                let value = obj.as_vector(|array| {
                    let target = &array.get()[(elem_size * index)..(elem_size * (index + 1))];
                    macro_rules! from_bytes {
                        ($t:ty) => {
                            <$t>::from_ne_bytes(
                                target.try_into().expect("source data was too small"),
                            )
                        };
                    }

                    match load_type {
                        Int8 => StackValue::Int32(from_bytes!(i8) as i32),
                        UInt8 => StackValue::Int32(from_bytes!(u8) as i32),
                        Int16 => StackValue::Int32(from_bytes!(i16) as i32),
                        UInt16 => StackValue::Int32(from_bytes!(u16) as i32),
                        Int32 => StackValue::Int32(from_bytes!(i32)),
                        UInt32 => StackValue::Int32(from_bytes!(u32) as i32),
                        Int64 => StackValue::Int64(from_bytes!(i64)),
                        Float32 => StackValue::NativeFloat(from_bytes!(f32) as f64),
                        Float64 => StackValue::NativeFloat(from_bytes!(f64)),
                        IntPtr => StackValue::NativeInt(from_bytes!(usize) as isize),
                        Object => StackValue::ObjectRef(ObjectRef::read(target)),
                    }
                });
                push!(value);
            }
            LoadElementAddress { .. } | LoadElementAddressReadonly(_) => todo!("ldelema"),
            LoadField {
                param0: source,
                volatile, // TODO
                ..
            } => {
                let parent = pop!();
                let (field, lookup) = self.current_context().locate_field(*source);

                check_special_fields!(field);

                let ctx = Context::with_generics(self.current_context(), &lookup);
                let name = &field.field.name;
                let t = ctx.get_field_type(field);

                let read_data = |d| CTSValue::read(&t, &ctx, d);
                let read_from_pointer = |ptr: *mut u8| {
                    let layout = FieldLayoutManager::instance_fields(field.parent, ctx.clone());
                    let field_layout = layout.fields.get(name.as_ref()).unwrap();
                    let slice = unsafe {
                        std::slice::from_raw_parts(
                            ptr.offset(field_layout.position as isize),
                            field_layout.layout.size(),
                        )
                    };
                    read_data(slice)
                };

                let value = match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => todo!("null pointer exception"),
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        let object_type = self.current_context().get_heap_description(h);
                        if !self.current_context().is_a(object_type, field.parent) {
                            panic!(
                                "tried to load field {}::{} from object of type {}",
                                field.parent.type_name(),
                                name,
                                object_type.type_name()
                            )
                        }

                        let data = h.borrow();
                        match &*data {
                            HeapStorage::Vec(_) => todo!("field on array"),
                            HeapStorage::Obj(o) => read_data(o.instance_storage.get_field(name)),
                            HeapStorage::Str(_) => todo!("field on string"),
                            HeapStorage::Boxed(_) => todo!("field on boxed value type"),
                        }
                    }
                    StackValue::ValueType(ref o) => {
                        if !self.current_context().is_a(o.description, field.parent) {
                            panic!(
                                "tried to load field {}::{} from object of type {}",
                                field.parent.type_name(),
                                name,
                                o.description.type_name()
                            )
                        }
                        read_data(o.instance_storage.get_field(name))
                    }
                    StackValue::NativeInt(i) => read_from_pointer(i as *mut u8),
                    StackValue::UnmanagedPtr(UnmanagedPtr(ptr))
                    | StackValue::ManagedPtr(ManagedPtr { value: ptr, .. }) => {
                        read_from_pointer(ptr)
                    }
                    rest => panic!("stack value {:?} has no fields", rest),
                };

                push!(value.into_stack())
            }
            LoadFieldAddress(source) => {
                let (field, lookup) = self.current_context().locate_field(*source);
                let name = &field.field.name;
                let parent = pop!();
                let ctx = Context::with_generics(self.current_context(), &lookup);

                let source_ptr = match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => todo!("null pointer exception"),
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        let object_type = ctx.get_heap_description(h);
                        if !ctx.is_a(object_type, field.parent) {
                            panic!(
                                "tried to load field {}::{} from object of type {}",
                                field.parent.type_name(),
                                name,
                                object_type.type_name()
                            )
                        }

                        let data = h.borrow();
                        match &*data {
                            HeapStorage::Vec(_) => todo!("field on array"),
                            HeapStorage::Obj(o) => o.instance_storage.get().as_ptr() as *mut u8,
                            HeapStorage::Str(_) => todo!("field on string"),
                            HeapStorage::Boxed(_) => todo!("field on boxed value type"),
                        }
                    }
                    StackValue::NativeInt(i) => i as *mut u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(ptr))
                    | StackValue::ManagedPtr(ManagedPtr { value: ptr, .. }) => ptr,
                    rest => panic!("cannot load field address from stack value {:?}", rest),
                };

                let layout =
                    FieldLayoutManager::instance_fields(field.parent, self.current_context());
                let ptr = unsafe {
                    source_ptr.offset(layout.fields.get(name.as_ref()).unwrap().position as isize)
                };

                let target_type = self.current_context().get_field_desc(field);

                if let StackValue::UnmanagedPtr(_) | StackValue::NativeInt(_) = parent {
                    push!(unmanaged_ptr(ptr))
                } else {
                    push!(managed_ptr(ptr, target_type))
                }
            }
            LoadFieldSkipNullCheck(_) => todo!("no.nullcheck ldfld"),
            LoadLength => {
                expect_stack!(let ObjectRef(obj) = pop!());
                let len = obj.as_vector(|a| a.layout.length as isize);
                push!(NativeInt(len));
            }
            LoadObject { .. } => todo!("ldobj"),
            LoadStaticField { param0: source, .. } => {
                let (field, lookup) = self.current_context().locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent, lookup.clone()) {
                    return StepResult::InstructionStepped;
                }

                check_special_fields!(field);

                let ctx = Context::with_generics(self.current_context(), &lookup);
                let value = statics!(|s| {
                    let field_data = s.get(field.parent).get_field(name);
                    let t = ctx.make_concrete(&field.field.return_type);
                    CTSValue::read(&t, &ctx, field_data).into_stack()
                });
                push!(value)
            }
            LoadStaticFieldAddress(source) => {
                let (field, lookup) = self.current_context().locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent, lookup.clone()) {
                    return StepResult::InstructionStepped;
                }

                let ctx = Context::with_generics(self.current_context(), &lookup);
                let field_type = ctx.get_field_desc(field);
                let value = statics!(|s| {
                    let field_data = s.get(field.parent).get_field(name);
                    StackValue::managed_ptr(field_data.as_ptr() as *mut _, field_type)
                });

                push!(value)
            }
            LoadString(cs) => {
                let val = StackValue::string(gc, CLRString::new(cs.clone()));
                push!(val)
            }
            LoadTokenField(source) => {
                let (field, lookup) = self.current_context().locate_field(*source);
                let idx = match self
                    .runtime_fields
                    .iter()
                    .position(|(f, g)| *f == field && *g == lookup)
                {
                    Some(i) => i,
                    None => {
                        self.runtime_fields.push((field, lookup));
                        self.runtime_fields.len() - 1
                    }
                };

                let rfh = self.assemblies.corlib_type("System.RuntimeFieldHandle");
                let mut instance = Object::new(rfh, self.current_context());
                instance
                    .instance_storage
                    .get_field_mut("_value")
                    .copy_from_slice(&idx.to_ne_bytes());

                push!(ValueType(Box::new(instance)));
            }
            LoadTokenMethod(_) => todo!("RuntimeMethodHandle"),
            LoadTokenType(target) => {
                // TODO: System.Type documentation suggests that maybe we need to preserve generic variables
                let target_type = self.current_context().make_concrete(target);

                let instance = self.get_handle_for_type(gc, target_type);

                push!(ValueType(Box::new(instance)))
            }
            LoadVirtualMethodPointer { .. } => todo!("ldvirtftn"),
            MakeTypedReference(_) => todo!("mkrefany"),
            NewArray(elem_type) => {
                let length = match pop!() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => todo!(
                        "invalid length for newarr (expected int32 or native int, received {:?})",
                        rest
                    ),
                };
                let ctx = self.current_context();
                let mut elem_type = ctx.make_concrete(elem_type);
                macro_rules! normalize_primitives {
                    ($($name:literal => $t:ident),+) => {{
                        let repr = format!("{:?}", elem_type);
                        $(
                            if repr.as_str() == $name || repr.as_str() == format!("[System.Runtime]{}", $name) {
                                elem_type = ctx.make_concrete(&BaseType::$t);
                            }
                        )+
                    }}
                }

                normalize_primitives! {
                    "System.Boolean" => Boolean,
                    "System.Char" => Char,
                    "System.Byte" => UInt8,
                    "System.SByte" => Int8,
                    "System.Int16" => Int16,
                    "System.UInt16" => UInt16,
                    "System.Int32" => Int32,
                    "System.UInt32" => UInt32,
                    "System.Int64" => Int64,
                    "System.UInt64" => UInt64,
                    "System.Single" => Float32,
                    "System.Double" => Float64,
                    "System.IntPtr" => IntPtr,
                    "System.UIntPtr" => UIntPtr,
                    "System.Object" => Object,
                    "System.String" => String
                }

                if let BaseType::Type { source, value_kind } = elem_type.get_mut() {
                    // generic context doesn't really matter here since we're just doing pure ancestry lookups
                    let (ut, _) = decompose_type_source(source);
                    let td = ctx.locate_type(ut);
                    if td.is_value_type(&ctx) {
                        *value_kind = Some(ValueKind::ValueType);
                    }
                }

                let v = Vector::new(elem_type, length, ctx);
                let o = ObjectRef::new(gc, HeapStorage::Vec(v));
                self.register_new_object(&o);
                push!(ObjectRef(o));
            }
            NewObject(ctor) => {
                let (mut method, lookup) = self.find_generic_method(&MethodSource::User(*ctor));
                let parent = method.parent;
                let new_ctx = Context::with_generics(self.current_context(), &lookup);

                match (&method.method.body, &parent.definition.extends) {
                    (None, Some(ts)) => {
                        let (ut, _) = decompose_type_source(ts);
                        let type_name = ut.type_name(parent.resolution.0);
                        // delegate types are only allowed to have these base types
                        if matches!(
                            type_name.as_ref(),
                            "System.Delegate" | "System.MulticastDelegate"
                        ) {
                            let base = self.assemblies.corlib_type(&type_name);
                            method = MethodDescription {
                                parent: base,
                                method: base
                                    .definition
                                    .methods
                                    .iter()
                                    .find(|m| m.name == ".ctor")
                                    .unwrap(),
                            };
                        }
                    }
                    _ => {}
                }

                // TODO: proper signature checking
                match format!("{:?}", method).as_str() {
                    "void System.IntPtr::.ctor(int)" => {
                        expect_stack!(let Int32(i) = pop!());
                        push!(NativeInt(i as isize));
                    }
                    _ => {
                        let instance = Object::new(parent, new_ctx.clone());

                        self.constructor_frame(
                            gc,
                            instance,
                            MethodInfo::new(
                                method.resolution(),
                                method.method,
                                &lookup,
                                &self.assemblies,
                            ),
                            lookup,
                        );
                        moved_ip = true;
                    }
                }
            }
            ReadTypedReferenceType => todo!("refanytype"),
            ReadTypedReferenceValue(_) => todo!("refanyval"),
            Rethrow => todo!("rethrow"),
            Sizeof(t) => {
                let ctx = self.current_context();
                let target = ctx.make_concrete(t);
                let layout = type_layout(target, ctx);
                push!(Int32(layout.size() as i32));
            }
            StoreElement { param0: source, .. } => {
                let value = pop!();
                expect_stack!(let Int32(index as usize) = pop!());
                expect_stack!(let ObjectRef(obj) = pop!());
                let ObjectRef(Some(heap)) = obj else {
                    todo!("NullPointerException")
                };
                let mut heap = heap.borrow_mut(gc);
                let HeapStorage::Vec(array) = &mut *heap else {
                    todo!("expected array for stelem, received {:?}", heap)
                };

                let ctx = self.current_context();
                let store_type = ctx.make_concrete(source);
                let elem_size = array.layout.element_layout.size();

                let target = &mut array.get_mut()[(elem_size * index)..(elem_size * (index + 1))];
                let data = CTSValue::new(&store_type, &ctx, value);
                data.write(target);
            }
            StoreElementPrimitive {
                param0: store_type, ..
            } => {
                use StoreType::*;

                let value = pop!();
                let index = match pop!() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    rest => todo!(
                        "invalid index for stelem (expected int32 or native int, received {:?})",
                        rest
                    ),
                };
                let array = pop!();

                let mut data: Vec<u8> = match value {
                    StackValue::Int32(i) => i.to_ne_bytes().to_vec(),
                    StackValue::Int64(i) => i.to_ne_bytes().to_vec(),
                    StackValue::NativeInt(i) => i.to_ne_bytes().to_vec(),
                    StackValue::NativeFloat(f) => f.to_ne_bytes().to_vec(),
                    StackValue::ObjectRef(o) => {
                        let mut vec = vec![0; ObjectRef::SIZE];
                        o.write(&mut vec);
                        vec
                    }
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => {
                        (p as usize).to_ne_bytes().to_vec()
                    }
                    StackValue::ManagedPtr(m) => (m.value as usize).to_ne_bytes().to_vec(),
                    StackValue::ValueType(_) => {
                        panic!("received valuetype for StoreElementPrimitive")
                    }
                };

                expect_stack!(let ObjectRef(obj) = array);
                let ObjectRef(Some(heap)) = obj else {
                    todo!("NullPointerException")
                };
                let mut heap = heap.borrow_mut(gc);
                let HeapStorage::Vec(array) = &mut *heap else {
                    todo!("expected array for stelem, received {:?}", heap)
                };

                let elem_size: usize = match store_type {
                    Int8 => 1,
                    Int16 => 2,
                    Int32 => 4,
                    Int64 => 8,
                    Float32 => 4,
                    Float64 => 8,
                    IntPtr => size_of::<usize>(),
                    Object => ObjectRef::SIZE,
                };
                data.truncate(elem_size);

                let target = &mut array.get_mut()[(elem_size * index)..(elem_size * (index + 1))];
                target.copy_from_slice(&data);
            }
            StoreField {
                param0: source,
                volatile, // TODO
                ..
            } => {
                let value = pop!();
                let parent = pop!();

                let ctx = self.current_context();
                let (field, lookup) = ctx.locate_field(*source);
                let ctx = Context::with_generics(ctx, &lookup);

                let t = ctx.get_field_type(field);
                let name = &field.field.name;

                let write_data = |dest: &mut [u8]| CTSValue::new(&t, &ctx, value).write(dest);
                let slice_from_pointer = |dest: *mut u8| {
                    let layout = FieldLayoutManager::instance_fields(field.parent, ctx.clone());
                    let field_layout = layout.fields.get(name.as_ref()).unwrap();
                    unsafe {
                        std::slice::from_raw_parts_mut(
                            dest.offset(field_layout.position as isize),
                            field_layout.layout.size(),
                        )
                    }
                };

                match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => todo!("null pointer exception"),
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        let object_type = ctx.get_heap_description(h);
                        if !ctx.is_a(object_type, field.parent) {
                            panic!(
                                "tried to store field {}::{} to object of type {}",
                                field.parent.type_name(),
                                name,
                                object_type.type_name()
                            )
                        }

                        let mut data = h.unlock(gc).borrow_mut();
                        match &mut *data {
                            HeapStorage::Vec(_) => todo!("field on array"),
                            HeapStorage::Obj(o) => {
                                write_data(o.instance_storage.get_field_mut(name))
                            }
                            HeapStorage::Str(_) => todo!("field on string"),
                            HeapStorage::Boxed(_) => todo!("field on boxed value type"),
                        }
                    }
                    StackValue::NativeInt(i) => write_data(slice_from_pointer(i as *mut u8)),
                    StackValue::UnmanagedPtr(UnmanagedPtr(ptr))
                    | StackValue::ManagedPtr(ManagedPtr { value: ptr, .. }) => {
                        write_data(slice_from_pointer(ptr))
                    }
                    rest => panic!(
                        "invalid type on stack (expected object or pointer, received {:?})",
                        rest
                    ),
                }
            }
            StoreFieldSkipNullCheck(_) => todo!("no.nullcheck stfld"),
            StoreObject { .. } => todo!("stobj"),
            StoreStaticField { param0: source, .. } => {
                let (field, lookup) = self.current_context().locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent, lookup.clone()) {
                    return StepResult::InstructionStepped;
                }

                let value = pop!();
                let ctx = Context::with_generics(self.current_context(), &lookup);
                statics!(|s| {
                    let field_data = s.get_mut(field.parent).get_field_mut(name);
                    let t = ctx.make_concrete(&field.field.return_type);
                    CTSValue::new(&t, &ctx, value).write(field_data);
                });
            }
            Throw => {
                // expects single value on stack
                // TODO: how will we propagate exceptions up the call stack?
                return StepResult::MethodThrew;
            }
            UnboxIntoAddress { .. } => todo!("unbox"),
            UnboxIntoValue(_) => todo!("unbox.any"),
        }
        if !moved_ip {
            self.increment_ip();
        }
        StepResult::InstructionStepped
    }

    pub fn get_runtime_type(
        &mut self,
        gc: GCHandle<'gc>,
        target_type: ConcreteType,
    ) -> ObjectRef<'gc> {
        if let Some(obj) = self.runtime_types.get(&target_type) {
            return *obj;
        }
        let rt = self.assemblies.corlib_type("System.RuntimeType");
        let rt_obj = Object::new(rt, self.current_context());
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
        self.register_new_object(&obj_ref);
        let key = target_type.clone();
        let entry = self.runtime_types.entry(key).insert_entry(obj_ref);
        let type_ptr = entry.key() as *const ConcreteType as usize;
        entry.get().as_object_mut(gc, |instance| {
            instance
                .instance_storage
                .get_field_mut("concreteType")
                .copy_from_slice(&type_ptr.to_ne_bytes());
        });
        obj_ref
    }

    pub fn get_handle_for_type(
        &mut self,
        gc: GCHandle<'gc>,
        target_type: ConcreteType,
    ) -> Object<'gc> {
        let rth = self.assemblies.corlib_type("System.RuntimeTypeHandle");
        let mut instance = Object::new(rth, self.current_context());
        let handle_location = instance.instance_storage.get_field_mut("_value");
        self.get_runtime_type(gc, target_type)
            .write(handle_location);
        instance
    }
}
