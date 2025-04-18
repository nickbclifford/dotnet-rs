use dotnetdll::prelude::*;
use std::cmp::Ordering;

use super::{intrinsics::*, CallStack, GCHandle, MethodInfo, StepResult};
use crate::value::{
    layout::{type_layout, FieldLayoutManager, HasLayout},
    string::CLRString,
    CTSValue, GenericLookup, HeapStorage, ManagedPtr, MethodDescription, Object, ObjectRef,
    StackValue, TypeDescription, UnmanagedPtr, ValueType,
};

const INTRINSIC_ATTR: &'static str = "System.Runtime.CompilerServices.IntrinsicAttribute";

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    fn find_generic_method(&self, source: &MethodSource) -> (MethodDescription, GenericLookup) {
        let ctx = self.current_context();

        let mut type_generics: Option<Vec<_>> = None;
        let mut method_generics: Option<Vec<_>> = None;

        let method = match source {
            MethodSource::User(u) => *u,
            MethodSource::Generic(g) => {
                method_generics = Some(
                    g.parameters
                        .iter()
                        .map(|t| ctx.generics.make_concrete(ctx.resolution, t.clone()))
                        .collect(),
                );
                g.base
            }
        };

        if let UserMethod::Reference(r) = method {
            if let MethodReferenceParent::Type(t) = &ctx.resolution[r].parent {
                let parent = ctx.generics.make_concrete(ctx.resolution, t.clone());
                if let BaseType::Type {
                    source: TypeSource::Generic { parameters, .. },
                    ..
                } = parent.get()
                {
                    type_generics = Some(parameters.clone());
                }
            }
        }

        let mut new_lookup = GenericLookup::default();
        if let Some(ts) = type_generics {
            new_lookup.type_generics = ts;
        }
        if let Some(ms) = method_generics {
            new_lookup.method_generics = ms;
        }

        (
            self.current_context().locate_method(method, &new_lookup),
            new_lookup,
        )
    }

    fn initialize_static_storage(
        &mut self,
        gc: GCHandle<'gc>,
        description: TypeDescription,
    ) -> bool {
        let value = {
            let mut statics = self.statics.borrow_mut();
            statics.init(description, self.current_context())
        };
        if let Some(m) = value {
            super::msg!(
                self,
                "-- static member accessed, calling static constructor (will return to ip {}) --",
                self.current_frame().state.ip
            );

            self.call_frame(
                gc,
                MethodInfo::new(m.resolution(), m.method),
                self.current_context().generics.clone(), // TODO: which type generics do static constructors get?
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
            () => {
                match (pop!(), pop!()) {
                    (StackValue::ManagedPtr(m), StackValue::NativeInt(r)) => m.value as isize == r,
                    (StackValue::NativeInt(l), StackValue::ManagedPtr(m)) => l == m.value as isize,
                    (StackValue::ObjectRef(l), StackValue::ObjectRef(r)) => l == r,
                    (l, r) => l.partial_cmp(&r) == Some(Ordering::Equal),
                }
            };
        }
        macro_rules! compare {
            ($sgn:expr, $op:tt ( $order:pat )) => {
                match (pop!(), pop!(), $sgn) {
                    (StackValue::Int32(l), StackValue::Int32(r), Unsigned) => {
                        (l as u32) $op (r as u32)
                    }
                    (StackValue::Int32(l), StackValue::NativeInt(r), Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (StackValue::Int64(l), StackValue::Int64(r), Unsigned) => {
                        (l as u64) $op (r as u64)
                    }
                    (StackValue::NativeInt(l), StackValue::Int32(r), Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (StackValue::NativeInt(l), StackValue::NativeInt(r), Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (l, r, _) => {
                        matches!(l.partial_cmp(&r), Some($order))
                    }
                }
            }
        }

        macro_rules! binary_arith_op {
            ($method:ident (f64 $op:tt) $(, { $($pat:pat => $arm:expr, )* } )?) => {
                match (pop!(), pop!()) {
                    (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                        push!(Int32(i1.$method(i2)))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(NativeInt((i1 as isize).$method(i2)))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                        push!(Int64(i1.$method(i2)))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(NativeInt(i1.$method(i2 as isize)))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(NativeInt(i1.$method(i2)))
                    }
                    (StackValue::NativeFloat(f1), StackValue::NativeFloat(f2)) => {
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
            };
            ($op:tt $(, { $($pat:pat => $arm:expr, )* })?) => {
                match (pop!(), pop!()) {
                    (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(NativeInt((i1 as isize) $op i2))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(NativeInt(i1 $op (i2 as isize)))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (StackValue::NativeFloat(f1), StackValue::NativeFloat(f2)) => {
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
            };
        }
        macro_rules! binary_checked_op {
            ($sign:expr, $method:ident (f64 $op:tt) $(, { $($pat:pat => $arm:expr, )* })?) => {
                match (pop!(), pop!(), $sign) {
                    (StackValue::Int32(i1), StackValue::Int32(i2), Signed) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(Int32(val))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2), Signed) => {
                        let Some(val) = (i1 as isize).$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2), Signed) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(Int64(val));
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2), Signed) => {
                        let Some(val) = i1.$method(i2 as isize) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val));
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2), Signed) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val));
                    }
                    (StackValue::NativeFloat(f1), StackValue::NativeFloat(f2), Signed) => {
                        push!(NativeFloat(f1 $op f2));
                    }
                    (StackValue::Int32(i1), StackValue::Int32(i2), Unsigned) => {
                        let Some(val) = (i1 as u32).$method(i2 as u32) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(Int32(val as i32))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2), Unsigned) => {
                        let Some(val) = (i1 as usize).$method(i2 as usize) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val as isize))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2), Unsigned) => {
                        let Some(val) = (i1 as u64).$method(i2 as u64) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(Int64(val as i64));
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2), Unsigned) => {
                        let Some(val) = i1.$method(i2 as isize) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(NativeInt(val as isize));
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2), Unsigned) => {
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
            };
        }
        macro_rules! binary_int_op {
            ($op:tt) => {
                match (pop!(), pop!()) {
                    (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(NativeInt((i1 as isize) $op i2))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(NativeInt(i1 $op (i2 as isize)))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            };
            ($op:tt as unsigned) => {
                match (pop!(), pop!()) {
                    (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                        push!(Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                        push!(Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} unsigned operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            };
        }
        macro_rules! shift_op {
            ($op:tt) => {
                match (pop!(), pop!()) {
                    (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(Int32(i1 $op i2))
                    }
                    (StackValue::Int64(i1), StackValue::Int32(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (StackValue::Int64(i1), StackValue::NativeInt(i2)) => {
                        push!(Int64(i1 $op i2))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(NativeInt(i1 $op i2))
                    }
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            };
            ($op:tt as unsigned) => {
               match (pop!(), pop!()) {
                    (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                        push!(Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (StackValue::Int64(i1), StackValue::Int32(i2)) => {
                        push!(Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (StackValue::Int64(i1), StackValue::NativeInt(i2)) => {
                        push!(Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($op)
                    ),
                }
            };
        }

        macro_rules! check_special_fields {
            ($field:ident) => {
                if $field.field.literal {
                    todo!(
                        "field {}::{} has literal",
                        $field.parent.1.type_name(),
                        $field.field.name
                    );
                }
                if let Some(c) = &$field.field.default {
                    todo!(
                        "field {}::{} has constant {:?}",
                        $field.parent.1.type_name(),
                        $field.field.name,
                        c
                    );
                }
                for a in &$field.field.attributes {
                    let ctor = self.assemblies.locate_attribute($field.parent.0, a);
                    if ctor.parent.1.type_name() == INTRINSIC_ATTR {
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

        super::msg!(
            self,
            "[#{} | ip @ {}] about to execute {}",
            self.frames.len() - 1,
            ip,
            i.show(i_res)
        );

        match i {
            Add => binary_arith_op!(wrapping_add (f64 +), {
                (StackValue::Int32(i), StackValue::ManagedPtr(m)) => {
                    // TODO: proper mechanisms for safety and pointer arithmetic
                    unsafe {
                        push!(ManagedPtr(m.map_value(|p| p.offset(i as isize))))
                    }
                },
                (StackValue::NativeInt(i), StackValue::ManagedPtr(m)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(i))))
                },
                (StackValue::ManagedPtr(m), StackValue::Int32(i)) => unsafe {
                        push!(ManagedPtr(m.map_value(|p| p.offset(i as isize))))
                    },
                (StackValue::ManagedPtr(m), StackValue::NativeInt(i)) => unsafe {
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
            Call {
                tail_call, // TODO
                param0: source,
            } => {
                self.debug_dump();
                let (method, lookup) = self.find_generic_method(source);
                macro_rules! intrinsic {
                    () => {
                        intrinsic_call(gc, self, method, lookup);
                        return StepResult::InstructionStepped;
                    };
                }

                if method.method.internal_call {
                    intrinsic!();
                }

                for a in &method.method.attributes {
                    let ctor = self.assemblies.locate_attribute(method.resolution(), a);
                    if ctor.parent.1.type_name() == INTRINSIC_ATTR {
                        intrinsic!();
                    }
                }

                self.call_frame(
                    gc,
                    MethodInfo::new(method.resolution(), method.method),
                    lookup,
                );
                moved_ip = true;
            }
            CallConstrained(_, _) => todo!("constrained. call"),
            CallIndirect { .. } => todo!("calli"),
            CompareEqual => {
                let val = equal!() as i32;
                push!(Int32(val))
            }
            CompareGreater(sgn) => {
                let val = compare!(sgn, > (Ordering::Greater)) as i32;
                push!(Int32(val))
            }
            CheckFinite => match pop!() {
                StackValue::NativeFloat(f) => {
                    if f.is_infinite() || f.is_nan() {
                        todo!("ArithmeticException in ckfinite");
                    }
                    push!(NativeFloat(f))
                }
                v => todo!("invalid type on stack ({:?}) for ckfinite operation", v),
            },
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
                            StackValue::NativeFloat(f) => {
                                todo!(
                                    "truncate {} towards zero for conversion to {}",
                                    f,
                                    stringify!($t)
                                )
                            }
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
                let value = pop!();
                todo!("conv.r4({:?})", value)
            }
            ConvertFloat64 => {
                let value = pop!();
                todo!("conv.r8({:?})", value)
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
            EndFinally => todo!("endfinally"),
            InitializeMemoryBlock { .. } => todo!("initblk"),
            Jump(_) => todo!("jmp"),
            LoadArgument(i) => {
                let arg = self.get_argument(*i as usize);
                push!(arg);
            }
            LoadArgumentAddress(_) => todo!("ldarga"),
            LoadConstantInt32(i) => push!(Int32(*i)),
            LoadConstantInt64(i) => push!(Int64(*i)),
            LoadConstantFloat32(f) => push!(NativeFloat(*f as f64)),
            LoadConstantFloat64(f) => push!(NativeFloat(*f)),
            LoadMethodPointer(_) => todo!("ldftn"),
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
                    LoadType::Object => todo!("ldind.ref"), // look at intrinsic System.Threading.Volatile::Read impl
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
                    local.data_location() as *mut _,
                    live_type
                ));
            }
            LoadNull => push!(null()),
            Leave(_) => todo!("leave"),
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
                v => todo!("invalid type on stack ({:?}) for logical NOT, expected integer", v),
            },
            NoOperation => {} // :3
            Not => match pop!() {
                StackValue::Int32(i) => push!(Int32(!i)),
                StackValue::Int64(i) => push!(Int64(!i)),
                StackValue::NativeInt(i) => push!(NativeInt(!i)),
                v => todo!("invalid type on stack ({:?}) for bitwise NOT, expected integer", v),
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
                        let StackValue::Int32(v) = val else {
                            todo!("invalid type on stack")
                        };
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
                    StoreType::Object => todo!("stind.ref"),
                }
            }
            StoreLocal(i) => {
                let val = pop!();
                self.set_local(gc, *i as usize, val);
            }
            Subtract => binary_arith_op!(wrapping_sub (f64 -), {
                (StackValue::ManagedPtr(m), StackValue::Int32(i)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(-i as isize))))
                },
                (StackValue::ManagedPtr(m), StackValue::NativeInt(i)) => unsafe {
                    push!(ManagedPtr(m.map_value(|p| p.offset(-i))))
                },
                (StackValue::ManagedPtr(m1), StackValue::ManagedPtr(m2)) => {
                    push!(NativeInt((m1.value as isize) - (m2.value as isize)))
                },
            }),
            SubtractOverflow(sgn) => binary_checked_op!(sgn, checked_sub (f64 -), {
                // TODO: pointer stuff
            }),
            Switch(_) => todo!("switch"),
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
                    if let Some(method) = self.current_context().find_method_in_type(
                        parent,
                        &base_method.method.name,
                        &base_method.method.signature,
                    ) {
                        found = Some((parent, method));
                        break;
                    }
                }

                if let Some((parent, method)) = found {
                    for a in args {
                        push!(a);
                    }
                    self.call_frame(gc, MethodInfo::new(parent.0, method.method), lookup);
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
            IsInstance(_) => todo!("isinst"),
            LoadElement { .. }
            | LoadElementPrimitive { .. }
            | LoadElementAddress { .. }
            | LoadElementAddressReadonly(_) => todo!("ldelem"),
            LoadField {
                param0: source,
                volatile, // TODO
                ..
            } => {
                let field = self.current_context().locate_field(*source);
                let name = &field.field.name;
                let parent = pop!();

                let read_data = |d| {
                    let t = self
                        .current_context()
                        .make_concrete(&field.field.return_type);
                    CTSValue::read(&t, &self.current_context(), d)
                };
                let read_from_pointer = |ptr: *mut u8| {
                    let layout =
                        FieldLayoutManager::instance_fields(field.parent, self.current_context());
                    let field_layout = layout.fields.get(name.as_ref()).unwrap();
                    let slice = unsafe {
                        std::slice::from_raw_parts(
                            ptr.offset(field_layout.position as isize),
                            field_layout.layout.size(),
                        )
                    };
                    read_data(slice)
                };

                check_special_fields!(field);

                let value = match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => todo!("null pointer exception"),
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        let object_type = self.current_context().get_heap_description(h);
                        if !self.current_context().is_a(object_type, field.parent) {
                            panic!(
                                "tried to load field {}::{} from object of type {}",
                                field.parent.1.type_name(),
                                name,
                                object_type.1.type_name()
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
                                field.parent.1.type_name(),
                                name,
                                o.description.1.type_name()
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
                let field = self.current_context().locate_field(*source);
                let name = &field.field.name;
                let parent = pop!();

                let source_ptr = match parent {
                    StackValue::ObjectRef(ObjectRef(None)) => todo!("null pointer exception"),
                    StackValue::ObjectRef(ObjectRef(Some(h))) => {
                        let object_type = self.current_context().get_heap_description(h);
                        if !self.current_context().is_a(object_type, field.parent) {
                            panic!(
                                "tried to load field {}::{} from object of type {}",
                                field.parent.1.type_name(),
                                name,
                                object_type.1.type_name()
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

                let target_type = self.current_context().get_field_type(field);

                if let StackValue::UnmanagedPtr(_) | StackValue::NativeInt(_) = parent {
                    push!(unmanaged_ptr(ptr))
                } else {
                    push!(managed_ptr(ptr, target_type))
                }
            }
            LoadFieldSkipNullCheck(_) => todo!("no.nullcheck ldfld"),
            LoadLength => todo!("ldlen"),
            LoadObject { .. } => todo!("ldobj"),
            LoadStaticField { param0: source, .. } => {
                let field = self.current_context().locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent) {
                    return StepResult::InstructionStepped;
                }

                check_special_fields!(field);

                let value = statics!(|s| {
                    let field_data = s.get(field.parent).get_field(name);
                    let t = self
                        .current_context()
                        .make_concrete(&field.field.return_type);
                    CTSValue::read(&t, &self.current_context(), field_data).into_stack()
                });
                push!(value)
            }
            LoadStaticFieldAddress(source) => {
                let field = self.current_context().locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent) {
                    return StepResult::InstructionStepped;
                }

                let field_type = self.current_context().get_field_type(field);
                let value = statics!(|s| {
                    let field_data = s.get(field.parent).get_field(name);
                    StackValue::managed_ptr(field_data.as_ptr() as *mut _, field_type)
                });

                push!(value)
            }
            LoadString(cs) => {
                let val = StackValue::ObjectRef(ObjectRef::new(
                    gc,
                    HeapStorage::Str(CLRString::new(cs.clone())),
                ));
                push!(val)
            }
            LoadTokenField(_) => todo!("RuntimeFieldHandle"),
            LoadTokenMethod(_) => todo!("RuntimeMethodHandle"),
            LoadTokenType(_) => todo!("RuntimeTypeHandle"),
            LoadVirtualMethodPointer { .. } => todo!("ldvirtftn"),
            MakeTypedReference(_) => todo!("mkrefany"),
            NewArray(_) => todo!("newarr"),
            NewObject(ctor) => {
                let (method, lookup) = self.find_generic_method(&MethodSource::User(*ctor));
                let parent = method.parent;

                // TODO: proper signature checking
                match format!("{:?}", method).as_str() {
                    "void System.IntPtr::.ctor(int)" => match pop!() {
                        StackValue::Int32(i) => push!(NativeInt(i as isize)),
                        rest => todo!("invalid type on stack (expected int32, received {:?}", rest),
                    },
                    _ => {
                        let instance = Object::new(parent, self.current_context());

                        self.constructor_frame(
                            gc,
                            instance,
                            MethodInfo::new(parent.0, method.method),
                            lookup,
                        );
                        moved_ip = true;
                    }
                }
            }
            ReadTypedReferenceType => todo!("refanytype"),
            ReadTypedReferenceValue(_) => todo!("refanyval"),
            Rethrow => todo!("rethrow"),
            Sizeof(_) => todo!("sizeof"),
            StoreElement { .. } | StoreElementPrimitive { .. } => todo!("stelem"),
            StoreField {
                param0: source,
                volatile, // TODO
                ..
            } => {
                let field = self.current_context().locate_field(*source);
                let name = &field.field.name;

                let value = pop!();
                let parent = pop!();

                let ctx = self.current_context();
                let write_data = |dest: &mut [u8]| {
                    let t = ctx.make_concrete(&field.field.return_type);
                    CTSValue::new(&t, &ctx, value).write(dest)
                };
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
                        let object_type = self.current_context().get_heap_description(h);
                        if !self.current_context().is_a(object_type, field.parent) {
                            panic!(
                                "tried to store field {}::{} to object of type {}",
                                field.parent.1.type_name(),
                                name,
                                object_type.1.type_name()
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
                    StackValue::ValueType(_) => todo!("stfld for value type"),
                    StackValue::NativeInt(i) => write_data(slice_from_pointer(i as *mut u8)),
                    StackValue::UnmanagedPtr(UnmanagedPtr(ptr))
                    | StackValue::ManagedPtr(ManagedPtr { value: ptr, .. }) => {
                        write_data(slice_from_pointer(ptr))
                    }
                    rest => panic!("stack value {:?} has no fields", rest),
                }
            }
            StoreFieldSkipNullCheck(_) => todo!("no.nullcheck stfld"),
            StoreObject { .. } => todo!("stobj"),
            StoreStaticField { param0: source, .. } => {
                let field = self.current_context().locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent) {
                    return StepResult::InstructionStepped;
                }

                let value = pop!();
                statics!(|s| {
                    let field_data = s.get_mut(field.parent).get_field_mut(name);
                    let t = self
                        .current_context()
                        .make_concrete(&field.field.return_type);
                    CTSValue::new(&t, &self.current_context(), value).write(field_data);
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
            state!(|s| s.ip += 1);
        }
        StepResult::InstructionStepped
    }
}

// TODO: inheritance of parent fields!
