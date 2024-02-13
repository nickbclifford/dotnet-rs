mod executor;
mod gc;

use super::value::*;
use crate::vm::gc::CallStack;
use dotnetdll::prelude::*;
use gc_arena::{unsafe_empty_collect, Collect, Mutation};

// I.12.3.2
#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct MethodState<'m> {
    ip: usize,
    info_handle: MethodInfo<'m>,
    memory_pool: Vec<u8>,
}

#[derive(Copy, Clone, Debug)]
pub struct MethodInfo<'a> {
    signature: &'a ManagedMethod,
    locals: &'a [LocalVariable],
    exceptions: &'a [body::Exception],
}
unsafe_empty_collect!(MethodInfo<'_>);

// TODO: well-typed exceptions
pub enum ExecutionResult {
    Returned,
    Threw,
}

impl<'gc, 'm> MethodState<'m> {
    fn execute(
        &mut self,
        gc_handle: &'gc Mutation<'gc>,
        call_stack: &'gc mut CallStack<'gc, 'm>,
        i: &Instruction,
    ) -> Option<ExecutionResult> {
        use Instruction::*;

        macro_rules! push {
            ($val:expr) => {
                call_stack.push_stack(gc_handle, $val)
            }
        }
        macro_rules! pop {
            () => {
                call_stack.pop_stack()
            }
        }

        macro_rules! binary_arith_op {
            ($self:ident, $method:ident (f64 $op:tt), { $($pat:pat => $arm:expr, )* }) => {
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
            ($self:ident, $op:tt, { $($pat:pat => $arm:expr, )* }) => {
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
            ($self:ident, $sign:expr, $method:ident (f64 $op:tt), { $($pat:pat => $arm:expr, )* }) => {
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
            ($self:ident, $op:tt) => {
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
            ($self:ident, $op:tt as unsigned) => {
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
            ($self:ident, $op:tt) => {
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
            ($self:ident, $op:tt as unsigned) => {
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

        let mut moved_ip = false;

        match i {
            Add => binary_arith_op!(self, wrapping_add (f64 +), {
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
            AddOverflow(sgn) => binary_checked_op!(self, sgn, checked_add(f64+), {
                // TODO: pointer stuff
            }),
            And => binary_int_op!(self, &),
            ArgumentList => {}
            BranchEqual(_) => {}
            BranchGreaterOrEqual(_, _) => {}
            BranchGreater(_, _) => {}
            BranchLessOrEqual(_, _) => {}
            BranchLess(_, _) => {}
            BranchNotEqual(_) => {}
            Branch(i) => {
                self.ip = *i;
                moved_ip = true;
            }
            Breakpoint => {}
            BranchFalsy(_) => {}
            BranchTruthy(_) => {}
            Call { .. } => {}
            CallConstrained(_, _) => {}
            CallIndirect { .. } => {}
            CompareEqual => {}
            CompareGreater(_) => {}
            CheckFinite => match pop!() {
                StackValue::NativeFloat(f) => {
                    if f.is_infinite() || f.is_nan() {
                        todo!("ArithmeticException in ckfinite");
                        return Some(ExecutionResult::Threw);
                    }
                    push!(StackValue::NativeFloat(f))
                }
                v => todo!("invalid type on stack ({:?}) for ckfinite operation", v),
            },
            CompareLess(_) => {}
            Convert(_) => {}
            ConvertOverflow(_, _) => {}
            ConvertFloat32 => {}
            ConvertFloat64 => {}
            ConvertUnsignedToFloat => {}
            CopyMemoryBlock { .. } => {}
            Divide(sgn) => match sgn {
                NumberSign::Signed => binary_arith_op!(self, /, { }),
                NumberSign::Unsigned => binary_int_op!(self, / as unsigned),
            },
            Duplicate => {
                let val = call_stack.pop_stack();
                push!(val);
                push!(val);
            }
            EndFilter => {}
            EndFinally => {}
            InitializeMemoryBlock { .. } => {}
            Jump(_) => {}
            LoadArgument(i) => {
                let arg = call_stack.get_argument(*i as usize);
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
                let local = call_stack.get_local(*i as usize);
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
                let loc = self.memory_pool.len();
                self.memory_pool.extend(vec![0; size]);
                push!(StackValue::unmanaged_ptr(
                    self.memory_pool[loc..].as_mut_ptr(),
                ))
            }
            Multiply => binary_arith_op!(self, wrapping_mul (f64 *), {}),
            MultiplyOverflow(sgn) => binary_checked_op!(self, sgn, checked_mul (f64 *), {}),
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
            Or => binary_int_op!(self, |),
            Pop => {
                pop!();
            }
            Remainder(sgn) => match sgn {
                NumberSign::Signed => binary_arith_op!(self, %, { }),
                NumberSign::Unsigned => binary_int_op!(self, % as unsigned),
            },
            Return => {
                // expects single value on stack
                // will be moved around properly by call stack manager
                return Some(ExecutionResult::Returned);
            }
            ShiftLeft => shift_op!(self, <<),
            ShiftRight(sgn) => match sgn {
                NumberSign::Signed => shift_op!(self, >>),
                NumberSign::Unsigned => shift_op!(self, >> as unsigned),
            },
            StoreArgument(i) => {
                let val = call_stack.pop_stack();
                call_stack.set_argument(gc_handle, *i as usize, val);
            }
            StoreIndirect { .. } => {}
            StoreLocal(i) => {
                let val = call_stack.pop_stack();
                call_stack.set_local(gc_handle, *i as usize, val);
            }
            Subtract => binary_arith_op!(self, wrapping_sub (f64 -), {
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
            SubtractOverflow(sgn) => binary_checked_op!(self, sgn, checked_sub (f64 -), {
                // TODO: pointer stuff
            }),
            Switch(_) => {}
            Xor => binary_int_op!(self, ^),
            BoxValue(_) => {}
            CallVirtual { .. } => {}
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
            LoadString(_) => {}
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
                return Some(ExecutionResult::Threw);
            }
            UnboxIntoAddress { .. } => {}
            UnboxIntoValue(_) => {}
        }
        if !moved_ip {
            self.ip += 1;
        }
        None
    }
}
