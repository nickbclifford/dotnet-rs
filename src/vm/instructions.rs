use std::{cmp::Ordering, ops::Deref};

use dotnetdll::prelude::{Instruction, MethodSource, NumberSign, ResolvedDebug};

use crate::{
    resolve::WithSource,
    value::{
        string::CLRString, GenericLookup, HeapStorage, ManagedPtr, MethodDescription, ObjectRef,
        StackValue, UnmanagedPtr,
    },
};

use super::{CallResult, CallStack, GCHandle, MethodInfo};

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    fn find_generic_method(
        &self,
        source: &MethodSource,
    ) -> (WithSource<MethodDescription>, GenericLookup) {
        let mut generics: Option<Vec<_>> = None;
        let current_lookup = self.current_frame().generic_inst.clone();
        let method = match source {
            MethodSource::User(u) => *u,
            MethodSource::Generic(g) => {
                generics = Some(
                    g.parameters
                        .iter()
                        .map(|t| current_lookup.make_concrete(t.clone()))
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

    pub fn step(&mut self, gc: GCHandle<'gc>) -> Option<CallResult> {
        use Instruction::*;

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

        let i = state!(|s| {
            let i = &s.info_handle.instructions[s.ip];
            println!(
                "about to execute {}",
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
                        StackValue::NativeInt(i) => i != 0,
                        StackValue::ObjectRef(ObjectRef(o)) => o.is_some(),
                        v => todo!("invalid type on stack ({:?}) for brtrue operation", v),
                    },
                    i
                )
            }
            Call {
                tail_call, // TODO
                param0: source,
            } => {
                let ((res, method), lookup) = self.find_generic_method(source);
                self.call_frame(gc, MethodInfo::new(res, method.method), lookup);
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
                        return Some(CallResult::Threw);
                    }
                    push!(StackValue::NativeFloat(f))
                }
                v => todo!("invalid type on stack ({:?}) for ckfinite operation", v),
            },
            CompareLess(sgn) => {
                let val = compare!(sgn, < (Ordering::Less)) as i32;
                push!(StackValue::Int32(val))
            }
            Convert(_) => {}
            ConvertOverflow(_, _) => {}
            ConvertFloat32 => {}
            ConvertFloat64 => {}
            ConvertUnsignedToFloat => {}
            CopyMemoryBlock { .. } => {}
            Divide(sgn) => match sgn {
                NumberSign::Signed => binary_arith_op!(/, { }),
                NumberSign::Unsigned => binary_int_op!(/ as unsigned),
            },
            Duplicate => {
                let val = self.pop_stack();
                push!(val);
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
            LoadArgumentAddress(_) => {}
            LoadConstantInt32(i) => push!(StackValue::Int32(*i)),
            LoadConstantInt64(i) => push!(StackValue::Int64(*i)),
            LoadConstantFloat32(f) => push!(StackValue::NativeFloat(*f as f64)),
            LoadConstantFloat64(f) => push!(StackValue::NativeFloat(*f)),
            LoadMethodPointer(_) => {}
            LoadIndirect { .. } => {}
            LoadLocal(i) => {
                let local = self.get_local(*i as usize);
                push!(local);
            }
            LoadLocalAddress(_) => {}
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
                return Some(CallResult::Returned);
            }
            ShiftLeft => shift_op!(<<),
            ShiftRight(sgn) => match sgn {
                NumberSign::Signed => shift_op!(>>),
                NumberSign::Unsigned => shift_op!(>> as unsigned),
            },
            StoreArgument(i) => {
                let val = self.pop_stack();
                self.set_argument(gc, *i as usize, val);
            }
            StoreIndirect { .. } => {}
            StoreLocal(i) => {
                let val = self.pop_stack();
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
            BoxValue(_) => {}
            CallVirtual {
                skip_null_check, // TODO
                param0: source,
            } => {
                self.dump_stack();
                let this_value = pop!();
                let this_heap = match this_value {
                    StackValue::ObjectRef(ObjectRef(None)) => todo!("null pointer exception"),
                    StackValue::ObjectRef(ObjectRef(Some(o))) => o,
                    rest => panic!("invalid this argument for virtual call (expected object ref, received {:?})", rest)
                };
                let this_type = match this_heap.deref() {
                    HeapStorage::Obj(o) => o.description,
                    HeapStorage::Vec(v) => todo!("System.Array"),
                    HeapStorage::Str(s) => todo!("System.String"),
                };

                let ((_, base_method), lookup) = self.find_generic_method(source);

                // TODO: check explicit overrides

                for (res, parent) in self.current_context().get_ancestors(this_type) {
                    if let Some(method) = self.current_context().find_method_in_type(
                        parent,
                        &base_method.method.name,
                        &base_method.method.signature,
                    ) {
                        push!(this_value);
                        self.call_frame(gc, MethodInfo::new(res, method.method), lookup);
                        break;
                    }
                }

                todo!("implement virtual calls!!")
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
            LoadField { .. } => {}
            LoadFieldAddress(_) => {}
            LoadFieldSkipNullCheck(_) => {}
            LoadLength => {}
            LoadObject { .. } => {}
            LoadStaticField { .. } => {}
            LoadStaticFieldAddress(_) => {}
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
            NewObject(_) => {}
            ReadTypedReferenceType => {}
            ReadTypedReferenceValue(_) => {}
            Rethrow => {}
            Sizeof(_) => {}
            StoreElement { .. } => {}
            StoreElementPrimitive { .. } => {}
            StoreField { .. } => {}
            StoreFieldSkipNullCheck(_) => {}
            StoreObject { .. } => {}
            StoreStaticField { .. } => {}
            Throw => {
                // expects single value on stack
                // TODO: how will we propagate exceptions up the call stack?
                return Some(CallResult::Threw);
            }
            UnboxIntoAddress { .. } => {}
            UnboxIntoValue(_) => {}
        }
        if !moved_ip {
            state!(|s| s.ip += 1);
        }
        None
    }
}
