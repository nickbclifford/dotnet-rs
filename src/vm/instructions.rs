use std::cmp::Ordering;

use dotnetdll::prelude::{
    ConversionType, Instruction, LoadType, MethodSource, NumberSign, ResolvedDebug,
};

use crate::value::{
    layout::{FieldLayoutManager, HasLayout},
    string::CLRString,
    CTSValue, GenericLookup, HeapStorage, ManagedPtr, MethodDescription, Object, ObjectRef,
    StackValue, TypeDescription, UnmanagedPtr, ValueType,
};

use super::{intrinsics::intrinsic_call, CallStack, GCHandle, MethodInfo, StepResult};

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    fn find_generic_method(&self, source: &MethodSource) -> (MethodDescription, GenericLookup) {
        let mut generics: Option<Vec<_>> = None;
        let ctx = self.current_context();
        let current_lookup = ctx.generics.clone();
        let method = match source {
            MethodSource::User(u) => *u,
            MethodSource::Generic(g) => {
                generics = Some(
                    g.parameters
                        .iter()
                        .map(|t| current_lookup.make_concrete(ctx.resolution, t.clone()))
                        .collect(),
                );
                g.base
            }
        };
        let new_lookup = match generics {
            None => current_lookup,
            Some(g) => current_lookup.instantiate_method(g),
        };

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
            // this is super hacky but we want to rerun the static member access instruction once we return from the initializer
            let mut ip = &mut self.current_frame_mut().state.ip;
            // *ip = ip.saturating_sub(1);
            let ip = *ip;
            println!(
                "{} -- static member accessed, calling static constructor (will return to ip {}) --",
                "\t".repeat(self.frames.len() - 1),
                ip
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

        macro_rules! statics {
            (|$s:ident| $body:expr) => {{
                let mut $s = self.statics.borrow_mut();
                $body
            }};
        }

        macro_rules! push {
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
                    (StackValue::ManagedPtr(ManagedPtr(l)), StackValue::NativeInt(r)) => {
                        l as isize == r
                    }
                    (StackValue::NativeInt(l), StackValue::ManagedPtr(ManagedPtr(r))) => {
                        l == r as isize
                    }
                    (StackValue::ObjectRef(l), StackValue::ObjectRef(r)) => l == r,
                    (l, r) => l.partial_cmp(&r) == Some(Ordering::Equal),
                }
            };
        }
        macro_rules! compare {
            ($sgn:expr, $op:tt ( $order:pat )) => {
                match (pop!(), pop!(), $sgn) {
                    (StackValue::Int32(l), StackValue::Int32(r), NumberSign::Unsigned) => {
                        (l as u32) $op (r as u32)
                    }
                    (StackValue::Int32(l), StackValue::NativeInt(r), NumberSign::Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (StackValue::Int64(l), StackValue::Int64(r), NumberSign::Unsigned) => {
                        (l as u64) $op (r as u64)
                    }
                    (StackValue::NativeInt(l), StackValue::Int32(r), NumberSign::Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (StackValue::NativeInt(l), StackValue::NativeInt(r), NumberSign::Unsigned) => {
                        (l as usize) $op (r as usize)
                    }
                    (l, r, _) => {
                        matches!(l.partial_cmp(&r), Some($order))
                    }
                }
            }
        }

        macro_rules! binary_arith_op {
            ($method:ident (f64 $op:tt), { $($pat:pat => $arm:expr, )* }) => {
                match (pop!(), pop!()) {
                    (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                        push!(StackValue::Int32(i1.$method(i2)))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::NativeInt((i1 as isize).$method(i2)))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                        push!(StackValue::Int64(i1.$method(i2)))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(StackValue::NativeInt(i1.$method(i2 as isize)))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::NativeInt(i1.$method(i2)))
                    }
                    (StackValue::NativeFloat(f1), StackValue::NativeFloat(f2)) => {
                        push!(StackValue::NativeFloat(f1 $op f2))
                    }
                    $($pat => $arm,)*
                    (v1, v2) => todo!(
                        "invalid types on stack ({:?}, {:?}) for {} operation",
                        v1,
                        v2,
                        stringify!($method)
                    ),
                }
            };
            ($op:tt, { $($pat:pat => $arm:expr, )* }) => {
                match (pop!(), pop!()) {
                    (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                        push!(StackValue::Int32(i1 $op i2))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::NativeInt((i1 as isize) $op i2))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                        push!(StackValue::Int64(i1 $op i2))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(StackValue::NativeInt(i1 $op (i2 as isize)))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::NativeInt(i1 $op i2))
                    }
                    (StackValue::NativeFloat(f1), StackValue::NativeFloat(f2)) => {
                        push!(StackValue::NativeFloat(f1 $op f2))
                    }
                    $($pat => $arm,)*
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
            ($sign:expr, $method:ident (f64 $op:tt), { $($pat:pat => $arm:expr, )* }) => {
                match (pop!(), pop!(), $sign) {
                    (StackValue::Int32(i1), StackValue::Int32(i2), NumberSign::Signed) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(StackValue::Int32(val))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2), NumberSign::Signed) => {
                        let Some(val) = (i1 as isize).$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(StackValue::NativeInt(val))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2), NumberSign::Signed) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(StackValue::Int64(val));
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2), NumberSign::Signed) => {
                        let Some(val) = i1.$method(i2 as isize) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(StackValue::NativeInt(val));
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2), NumberSign::Signed) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(StackValue::NativeInt(val));
                    }
                    (StackValue::NativeFloat(f1), StackValue::NativeFloat(f2), NumberSign::Signed) => {
                        push!(StackValue::NativeFloat(f1 $op f2));
                    }
                    (StackValue::Int32(i1), StackValue::Int32(i2), NumberSign::Unsigned) => {
                        let Some(val) = (i1 as u32).$method(i2 as u32) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(StackValue::Int32(val as i32))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2), NumberSign::Unsigned) => {
                        let Some(val) = (i1 as usize).$method(i2 as usize) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(StackValue::NativeInt(val as isize))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2), NumberSign::Unsigned) => {
                        let Some(val) = (i1 as u64).$method(i2 as u64) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(StackValue::Int64(val as i64));
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2), NumberSign::Unsigned) => {
                        let Some(val) = i1.$method(i2 as isize) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(StackValue::NativeInt(val as isize));
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2), NumberSign::Unsigned) => {
                        let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                        push!(StackValue::NativeInt(val as isize));
                    }
                    $($pat => $arm,)*
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
                        push!(StackValue::Int32(i1 $op i2))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::NativeInt((i1 as isize) $op i2))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                        push!(StackValue::Int64(i1 $op i2))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(StackValue::NativeInt(i1 $op (i2 as isize)))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::NativeInt(i1 $op i2))
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
                        push!(StackValue::Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                        push!(StackValue::Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(StackValue::NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
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
                        push!(StackValue::Int32(i1 $op i2))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::Int32(i1 $op i2))
                    }
                    (StackValue::Int64(i1), StackValue::Int32(i2)) => {
                        push!(StackValue::Int64(i1 $op i2))
                    }
                    (StackValue::Int64(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::Int64(i1 $op i2))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(StackValue::NativeInt(i1 $op i2))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::NativeInt(i1 $op i2))
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
                        push!(StackValue::Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::Int32(((i1 as u32) $op (i2 as u32)) as i32))
                    }
                    (StackValue::Int64(i1), StackValue::Int32(i2)) => {
                        push!(StackValue::Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (StackValue::Int64(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::Int64(((i1 as u64) $op (i2 as u64)) as i64))
                    }
                    (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                        push!(StackValue::NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
                    }
                    (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                        push!(StackValue::NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
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

        let frame_no = self.frames.len() - 1;
        let i = state!(|s| {
            let i = &s.info_handle.instructions[s.ip];
            println!(
                "{}[#{} | ip @ {}] about to execute {}",
                "\t".repeat(frame_no),
                frame_no,
                s.ip,
                i.show(s.info_handle.source_resolution)
            );
            i
        });

        match i {
            Add => binary_arith_op!(wrapping_add (f64 +), {
                (StackValue::Int32(i), StackValue::ManagedPtr(ManagedPtr(p))) => {
                    // TODO: proper mechanisms for safety and pointer arithmetic
                    unsafe {
                        push!(StackValue::managed_ptr(p.offset(i as isize)))
                    }
                },
                (StackValue::NativeInt(i), StackValue::ManagedPtr(ManagedPtr(p))) => unsafe {
                    push!(StackValue::managed_ptr(p.offset(i)))
                },
                (StackValue::ManagedPtr(ManagedPtr(p)), StackValue::Int32(i)) => unsafe {
                    push!(StackValue::managed_ptr(p.offset(i as isize)))
                },
                (StackValue::ManagedPtr(ManagedPtr(p)), StackValue::NativeInt(i)) => unsafe {
                    push!(StackValue::managed_ptr(p.offset(i)))
                },
            }),
            AddOverflow(sgn) => binary_checked_op!(sgn, checked_add(f64+), {
                // TODO: pointer stuff
            }),
            And => binary_int_op!(&),
            ArgumentList => {}
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
            Breakpoint => {}
            BranchFalsy(i) => {
                conditional_branch!(
                    match pop!() {
                        StackValue::Int32(i) => i == 0,
                        StackValue::Int64(i) => i == 0,
                        StackValue::NativeInt(i) => i == 0,
                        StackValue::ObjectRef(ObjectRef(o)) => o.is_none(),
                        StackValue::UnmanagedPtr(UnmanagedPtr(p))
                        | StackValue::ManagedPtr(ManagedPtr(p)) => p.is_null(),
                        v => todo!("invalid type on stack ({:?}) for brfalse operation", v),
                    },
                    i
                )
            }
            BranchTruthy(i) => {
                conditional_branch!(
                    match pop!() {
                        StackValue::Int32(i) => i != 0,
                        StackValue::Int64(i) => i != 0,
                        StackValue::NativeInt(i) => i != 0,
                        StackValue::ObjectRef(ObjectRef(o)) => o.is_some(),
                        StackValue::UnmanagedPtr(UnmanagedPtr(p))
                        | StackValue::ManagedPtr(ManagedPtr(p)) => !p.is_null(),
                        v => todo!("invalid type on stack ({:?}) for brtrue operation", v),
                    },
                    i
                )
            }
            Call {
                tail_call, // TODO
                param0: source,
            } => {
                let (method, lookup) = self.find_generic_method(source);
                macro_rules! intrinsic {
                    () => {{
                        intrinsic_call(self, method, lookup);
                        return StepResult::InstructionStepped;
                    }};
                }

                if method.method.internal_call {
                    intrinsic!();
                }

                if method
                    .parent
                    .1
                    .type_name()
                    .starts_with("System.Runtime.CompilerServices")
                {
                    for a in &method.method.attributes {
                        // TODO: do I really need a full lookup to get the type name?
                        let ctor = self.assemblies.locate_method(
                            method.resolution(),
                            a.constructor,
                            &lookup,
                        );
                        // it's super cursed but this is basically hardcoded into coreclr
                        if ctor.parent.1.type_name()
                            == "System.Runtime.CompilerServices.IntrinsicAttribute"
                        {
                            intrinsic!();
                        }
                    }
                }
                self.call_frame(
                    gc,
                    MethodInfo::new(method.resolution(), method.method),
                    lookup,
                );
                moved_ip = true;
            }
            CallConstrained(_, _) => {}
            CallIndirect { .. } => {}
            CompareEqual => {
                let val = equal!() as i32;
                push!(StackValue::Int32(val))
            }
            CompareGreater(sgn) => {
                let val = compare!(sgn, > (Ordering::Greater)) as i32;
                push!(StackValue::Int32(val))
            }
            CheckFinite => match pop!() {
                StackValue::NativeFloat(f) => {
                    if f.is_infinite() || f.is_nan() {
                        todo!("ArithmeticException in ckfinite");
                        return StepResult::MethodThrew;
                    }
                    push!(StackValue::NativeFloat(f))
                }
                v => todo!("invalid type on stack ({:?}) for ckfinite operation", v),
            },
            CompareLess(sgn) => {
                let val = compare!(sgn, < (Ordering::Less)) as i32;
                push!(StackValue::Int32(val))
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
                        push!(StackValue::Int32(i as i32));
                    }};
                }

                macro_rules! convert_long_ints {
                    ($variant:ident ( $t:ty )) => {{
                        let i = simple_cast!($t);
                        push!(StackValue::$variant(i));
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
                        push!(StackValue::$variant(i as $vt));
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
                NumberSign::Signed => binary_arith_op!(/, { }),
                NumberSign::Unsigned => binary_int_op!(/ as unsigned),
            },
            Duplicate => {
                let val = pop!();
                push!(val.clone());
                push!(val);
            }
            EndFilter => {}
            EndFinally => {}
            InitializeMemoryBlock { .. } => {}
            Jump(_) => {}
            LoadArgument(i) => {
                let arg = self.get_argument(*i as usize);
                push!(arg);
            }
            LoadArgumentAddress(_) => todo!("ldarga"),
            LoadConstantInt32(i) => push!(StackValue::Int32(*i)),
            LoadConstantInt64(i) => push!(StackValue::Int64(*i)),
            LoadConstantFloat32(f) => push!(StackValue::NativeFloat(*f as f64)),
            LoadConstantFloat64(f) => push!(StackValue::NativeFloat(*f)),
            LoadMethodPointer(_) => {}
            LoadIndirect { param0: t, .. } => {
                let ptr = match pop!() {
                    StackValue::NativeInt(i) => i as *mut u8,
                    StackValue::UnmanagedPtr(UnmanagedPtr(p)) => p,
                    StackValue::ManagedPtr(ManagedPtr(p)) => p,
                    v => todo!(
                        "invalid type on stack ({:?}) for ldind operation, expected pointer",
                        v
                    ),
                };

                macro_rules! load_as_i32 {
                    ($t:ty) => {{
                        let val: $t = unsafe { *(ptr as *mut _) };
                        push!(StackValue::Int32(val as i32));
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
                    LoadType::Object => todo!("ldind.ref"),
                }
            }
            LoadLocal(i) => {
                let local = self.get_local(*i as usize);
                push!(local);
            }
            LoadLocalAddress(i) => {
                let local = self.get_local(*i as usize);
                push!(StackValue::managed_ptr(local.data_location() as *mut _));
            }
            LoadNull => push!(StackValue::null()),
            Leave(_) => {}
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
                push!(StackValue::unmanaged_ptr(ptr));
            }
            Multiply => binary_arith_op!(wrapping_mul (f64 *), {}),
            MultiplyOverflow(sgn) => binary_checked_op!(sgn, checked_mul (f64 *), {}),
            Negate => match pop!() {
                StackValue::Int32(i) => push!(StackValue::Int32(-i)),
                StackValue::Int64(i) => push!(StackValue::Int64(-i)),
                StackValue::NativeInt(i) => push!(StackValue::NativeInt(-i)),
                v => todo!("invalid type on stack ({:?}) for ! operation", v),
            },
            NoOperation => {}
            // TODO: how are booleans + boolean negation represented?
            Not => match pop!() {
                StackValue::Int32(i) => push!(StackValue::Int32(!i)),
                StackValue::Int64(i) => push!(StackValue::Int64(!i)),
                StackValue::NativeInt(i) => push!(StackValue::NativeInt(!i)),
                v => todo!("invalid type on stack ({:?}) for ~ operation", v),
            },
            Or => binary_int_op!(|),
            Pop => {
                pop!();
            }
            Remainder(sgn) => match sgn {
                NumberSign::Signed => binary_arith_op!(%, { }),
                NumberSign::Unsigned => binary_int_op!(% as unsigned),
            },
            Return => {
                // expects single value on stack for non-void methods
                // will be moved around properly by call stack manager
                return StepResult::MethodReturned;
            }
            ShiftLeft => shift_op!(<<),
            ShiftRight(sgn) => match sgn {
                NumberSign::Signed => shift_op!(>>),
                NumberSign::Unsigned => shift_op!(>> as unsigned),
            },
            StoreArgument(i) => {
                let val = pop!();
                self.set_argument(gc, *i as usize, val);
            }
            StoreIndirect { .. } => {}
            StoreLocal(i) => {
                let val = pop!();
                self.set_local(gc, *i as usize, val);
            }
            Subtract => binary_arith_op!(wrapping_sub (f64 -), {
                (StackValue::ManagedPtr(ManagedPtr(p)), StackValue::Int32(i)) => unsafe {
                    push!(StackValue::managed_ptr(p.offset(-i as isize)))
                },
                (StackValue::ManagedPtr(ManagedPtr(p)), StackValue::NativeInt(i)) => unsafe {
                    push!(StackValue::managed_ptr(p.offset(-i)))
                },
                (StackValue::ManagedPtr(ManagedPtr(p1)), StackValue::ManagedPtr(ManagedPtr(p2))) => {
                    push!(StackValue::NativeInt((p1 as isize) - (p2 as isize)))
                },
            }),
            SubtractOverflow(sgn) => binary_checked_op!(sgn, checked_sub (f64 -), {
                // TODO: pointer stuff
            }),
            Switch(_) => {}
            Xor => binary_int_op!(^),
            BoxValue(t) => {
                let t = self.current_context().make_concrete(t);

                let value = pop!();
                push!(StackValue::ObjectRef(ObjectRef::new(
                    gc,
                    HeapStorage::Boxed(ValueType::new(&t, &self.current_context(), value))
                )));
            }
            CallVirtual {
                skip_null_check, // TODO
                param0: source,
            } => {
                let this_value = pop!();
                let this_heap = match this_value {
                    StackValue::ObjectRef(ObjectRef(None)) => todo!("null pointer exception"),
                    StackValue::ObjectRef(ObjectRef(Some(o))) => o,
                    rest => panic!("invalid this argument for virtual call (expected object ref, received {:?})", rest)
                };
                let this_type = self.current_context().get_heap_description(this_heap);

                let (base_method, lookup) = self.find_generic_method(source);

                // TODO: check explicit overrides

                let mut found = None;
                for parent in self.current_context().get_ancestors(this_type) {
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
                    push!(this_value);
                    self.call_frame(gc, MethodInfo::new(parent.0, method.method), lookup);
                } else {
                    panic!("could not resolve virtual call");
                }
            }
            CallVirtualConstrained(_, _) => {}
            CallVirtualTail(_) => {}
            CastClass { .. } => {}
            CopyObject(_) => {}
            InitializeForObject(_) => {}
            IsInstance(_) => {}
            LoadElement { .. } => {}
            LoadElementPrimitive { .. } => {}
            LoadElementAddress { .. } => {}
            LoadElementAddressReadonly(_) => {}
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
                    // forgive me
                    let slice = unsafe {
                        std::slice::from_raw_parts(
                            ptr.byte_offset(field_layout.position as isize),
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
                    | StackValue::ManagedPtr(ManagedPtr(ptr)) => read_from_pointer(ptr),
                    rest => panic!("stack value {:?} has no fields", rest),
                };

                push!(value.into_stack())
            }
            LoadFieldAddress(_) => todo!("ldflda"),
            LoadFieldSkipNullCheck(_) => {}
            LoadLength => {}
            LoadObject { .. } => {}
            LoadStaticField { param0: source, .. } => {
                let field = self.current_context().locate_field(*source);
                let name = &field.field.name;

                if self.initialize_static_storage(gc, field.parent) {
                    return StepResult::InstructionStepped;
                }

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

                let value = statics!(|s| {
                    let field_data = s.get(field.parent).get_field(name);
                    StackValue::managed_ptr(unsafe { field_data.as_ptr() as *mut _ })
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
            LoadTokenField(_) => {}
            LoadTokenMethod(_) => {}
            LoadTokenType(_) => {}
            LoadVirtualMethodPointer { .. } => {}
            MakeTypedReference(_) => {}
            NewArray(_) => {}
            NewObject(ctor) => {
                let (method, lookup) = self.find_generic_method(&MethodSource::User(*ctor));

                let parent = method.parent;
                let instance = Object::new(parent, self.current_context());

                self.constructor_frame(
                    gc,
                    instance,
                    MethodInfo::new(parent.0, method.method),
                    lookup,
                );
                moved_ip = true;
            }
            ReadTypedReferenceType => {}
            ReadTypedReferenceValue(_) => {}
            Rethrow => {}
            Sizeof(_) => {}
            StoreElement { .. } => {}
            StoreElementPrimitive { .. } => {}
            StoreField {
                param0: source,
                volatile, // TODO
                ..
            } => {
                let field = self.current_context().locate_field(*source);
                let name = &field.field.name;

                let value = pop!();
                let parent = pop!();

                let write_data = |dest: &mut [u8]| {
                    let t = self
                        .current_context()
                        .make_concrete(&field.field.return_type);
                    CTSValue::new(&t, &self.current_context(), value).write(dest)
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
                    StackValue::NativeInt(_)
                    | StackValue::UnmanagedPtr(_)
                    | StackValue::ManagedPtr(_) => {
                        todo!("stfld for pointer/ref")
                    }
                    rest => panic!("stack value {:?} has no fields", rest),
                }
            }
            StoreFieldSkipNullCheck(_) => {}
            StoreObject { .. } => {}
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
            UnboxIntoAddress { .. } => {}
            UnboxIntoValue(_) => {}
        }
        if !moved_ip {
            state!(|s| s.ip += 1);
        }
        StepResult::InstructionStepped
    }
}

// TODO: inheritance of parent fields!
