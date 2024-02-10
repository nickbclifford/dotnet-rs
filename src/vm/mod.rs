mod executor;

use super::value::*;
use dotnetdll::prelude::*;
use gc_arena::{Collect, Mutation};

// I.12.3.2

#[derive(Clone, Debug, Collect)]
#[collect(no_drop)]
pub struct MethodState<'gc, 'm> {
    ip: usize,
    stack: Vec<StackValue<'gc>>,
    locals: Vec<StackValue<'gc>>,
    arguments: Vec<StackValue<'gc>>,
    info_handle: MethodInfo<'m>,
    memory_pool: Vec<u8>,
    gc_handle: &'gc Mutation<'gc>
}

#[derive(Copy, Clone, Debug)]
pub struct MethodInfo<'a> {
    signature: &'a ManagedMethod,
    locals: &'a [LocalVariable],
    exceptions: &'a [body::Exception],
}

// TODO: well-typed exceptions
pub type ExecutionResult<'gc> = Result<StackValue<'gc>, StackValue<'gc>>;

macro_rules! binary_arith_op {
    ($self:ident, $method:ident (f64 $op:tt), { $($pat:pat => $arm:expr, )* }) => {
        match ($self.pop(), $self.pop()) {
            (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::Int32(i1.$method(i2)))
            }
            (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::NativeInt((i1 as isize).$method(i2)))
            }
            (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                $self.stack.push(StackValue::Int64(i1.$method(i2)))
            }
            (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::NativeInt(i1.$method(i2 as isize)))
            }
            (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::NativeInt(i1.$method(i2)))
            }
            (StackValue::NativeFloat(f1), StackValue::NativeFloat(f2)) => {
                $self.stack.push(StackValue::NativeFloat(f1 $op f2))
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
        match ($self.pop(), $self.pop()) {
            (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::Int32(i1 $op i2))
            }
            (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::NativeInt((i1 as isize) $op i2))
            }
            (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                $self.stack.push(StackValue::Int64(i1 $op i2))
            }
            (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::NativeInt(i1 $op (i2 as isize)))
            }
            (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::NativeInt(i1 $op i2))
            }
            (StackValue::NativeFloat(f1), StackValue::NativeFloat(f2)) => {
                $self.stack.push(StackValue::NativeFloat(f1 $op f2))
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
        match ($self.pop(), $self.pop(), $sign) {
            (StackValue::Int32(i1), StackValue::Int32(i2), NumberSign::Signed) => {
                let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                $self.stack.push(StackValue::Int32(val))
            }
            (StackValue::Int32(i1), StackValue::NativeInt(i2), NumberSign::Signed) => {
                let Some(val) = (i1 as isize).$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                $self.stack.push(StackValue::NativeInt(val))
            }
            (StackValue::Int64(i1), StackValue::Int64(i2), NumberSign::Signed) => {
                let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                $self.stack.push(StackValue::Int64(val));
            }
            (StackValue::NativeInt(i1), StackValue::Int32(i2), NumberSign::Signed) => {
                let Some(val) = i1.$method(i2 as isize) else { todo!("OverflowException in {}", stringify!($method)) };
                $self.stack.push(StackValue::NativeInt(val));
            }
            (StackValue::NativeInt(i1), StackValue::NativeInt(i2), NumberSign::Signed) => {
                let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                $self.stack.push(StackValue::NativeInt(val));
            }
            (StackValue::NativeFloat(f1), StackValue::NativeFloat(f2), NumberSign::Signed) => {
                $self.stack.push(StackValue::NativeFloat(f1 $op f2));
            }
            (StackValue::Int32(i1), StackValue::Int32(i2), NumberSign::Unsigned) => {
                let Some(val) = (i1 as u32).$method(i2 as u32) else { todo!("OverflowException in {}", stringify!($method)) };
                $self.stack.push(StackValue::Int32(val as i32))
            }
            (StackValue::Int32(i1), StackValue::NativeInt(i2), NumberSign::Unsigned) => {
                let Some(val) = (i1 as usize).$method(i2 as usize) else { todo!("OverflowException in {}", stringify!($method)) };
                $self.stack.push(StackValue::NativeInt(val as isize))
            }
            (StackValue::Int64(i1), StackValue::Int64(i2), NumberSign::Unsigned) => {
                let Some(val) = (i1 as u64).$method(i2 as u64) else { todo!("OverflowException in {}", stringify!($method)) };
                $self.stack.push(StackValue::Int64(val as i64));
            }
            (StackValue::NativeInt(i1), StackValue::Int32(i2), NumberSign::Unsigned) => {
                let Some(val) = i1.$method(i2 as isize) else { todo!("OverflowException in {}", stringify!($method)) };
                $self.stack.push(StackValue::NativeInt(val as isize));
            }
            (StackValue::NativeInt(i1), StackValue::NativeInt(i2), NumberSign::Unsigned) => {
                let Some(val) = i1.$method(i2) else { todo!("OverflowException in {}", stringify!($method)) };
                $self.stack.push(StackValue::NativeInt(val as isize));
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
        match ($self.pop(), $self.pop()) {
            (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::Int32(i1 $op i2))
            }
            (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::NativeInt((i1 as isize) $op i2))
            }
            (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                $self.stack.push(StackValue::Int64(i1 $op i2))
            }
            (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::NativeInt(i1 $op (i2 as isize)))
            }
            (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::NativeInt(i1 $op i2))
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
        match ($self.pop(), $self.pop()) {
            (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::Int32(((i1 as u32) $op (i2 as u32)) as i32))
            }
            (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
            }
            (StackValue::Int64(i1), StackValue::Int64(i2)) => {
                $self.stack.push(StackValue::Int64(((i1 as u64) $op (i2 as u64)) as i64))
            }
            (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
            }
            (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
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
        match ($self.pop(), $self.pop()) {
            (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::Int32(i1 $op i2))
            }
            (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::Int32(i1 $op i2))
            }
            (StackValue::Int64(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::Int64(i1 $op i2))
            }
            (StackValue::Int64(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::Int64(i1 $op i2))
            }
            (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::NativeInt(i1 $op i2))
            }
            (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::NativeInt(i1 $op i2))
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
       match ($self.pop(), $self.pop()) {
            (StackValue::Int32(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::Int32(((i1 as u32) $op (i2 as u32)) as i32))
            }
            (StackValue::Int32(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::Int32(((i1 as u32) $op (i2 as u32)) as i32))
            }
            (StackValue::Int64(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::Int64(((i1 as u64) $op (i2 as u64)) as i64))
            }
            (StackValue::Int64(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::Int64(((i1 as u64) $op (i2 as u64)) as i64))
            }
            (StackValue::NativeInt(i1), StackValue::Int32(i2)) => {
                $self.stack.push(StackValue::NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
            }
            (StackValue::NativeInt(i1), StackValue::NativeInt(i2)) => {
                $self.stack.push(StackValue::NativeInt(((i1 as usize) $op (i2 as usize)) as isize))
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

impl<'gc, 'm> MethodState<'gc, 'm> {
    fn pop(&mut self) -> StackValue<'gc> {
        match self.stack.pop() {
            Some(v) => v,
            None => todo!("stack was empty when attempted to pop"),
        }
    }

    fn execute(&mut self, i: &Instruction) -> Option<ExecutionResult<'gc>> {
        use Instruction::*;

        let mut moved_ip = false;

        match i {
            Add => binary_arith_op!(self, wrapping_add (f64 +), {
                (StackValue::Int32(i), StackValue::ManagedPtr(ManagedPtr(p))) => {
                    // TODO: proper mechanisms for safety and pointer arithmetic
                    unsafe {
                        self.stack
                            .push(StackValue::managed_ptr(p.offset(i as isize)))
                    }
                },
                (StackValue::NativeInt(i), StackValue::ManagedPtr(ManagedPtr(p))) => unsafe {
                    self.stack.push(StackValue::managed_ptr(p.offset(i)))
                },
                (StackValue::ManagedPtr(ManagedPtr(p)), StackValue::Int32(i)) => unsafe {
                    self.stack
                        .push(StackValue::managed_ptr(p.offset(i as isize)))
                },
                (StackValue::ManagedPtr(ManagedPtr(p)), StackValue::NativeInt(i)) => unsafe {
                    self.stack.push(StackValue::managed_ptr(p.offset(i)))
                },
            }),
            AddOverflow(sgn) => binary_checked_op!(self, sgn, checked_add (f64 +), {
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
            CheckFinite => match self.pop() {
                StackValue::NativeFloat(f) => {
                    if f.is_infinite() || f.is_nan() {
                        return Some(Err(todo!("ArithmeticException in ckfinite")));
                    }
                    self.stack.push(StackValue::NativeFloat(f))
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
                let val = self.pop();
                self.stack.push(val.clone());
                self.stack.push(val)
            }
            EndFilter => {}
            EndFinally => {}
            InitializeMemoryBlock { .. } => {}
            Jump(_) => {}
            LoadArgument(i) => self.stack.push(match self.arguments.get(*i as usize) {
                Some(v) => v.clone(),
                None => todo!("invalid argument index to load"),
            }),
            LoadArgumentAddress(_) => {}
            LoadConstantInt32(i) => self.stack.push(StackValue::Int32(*i)),
            LoadConstantInt64(i) => self.stack.push(StackValue::Int64(*i)),
            LoadConstantFloat32(f) => self.stack.push(StackValue::NativeFloat(*f as f64)),
            LoadConstantFloat64(f) => self.stack.push(StackValue::NativeFloat(*f)),
            LoadMethodPointer(_) => {}
            LoadIndirect { .. } => {}
            LoadLocal(i) => self.stack.push(match self.locals.get(*i as usize) {
                Some(v) => v.clone(),
                None => todo!("invalid local variable index to load"),
            }),
            LoadLocalAddress(_) => {}
            LoadNull => self.stack.push(StackValue::ObjectRef(None)),
            Leave(_) => {}
            LocalMemoryAllocate => {
                let size = match self.pop() {
                    StackValue::Int32(i) => i as usize,
                    StackValue::NativeInt(i) => i as usize,
                    v => todo!(
                        "invalid type on stack ({:?}) for local memory allocation size",
                        v
                    ),
                };
                let loc = self.memory_pool.len();
                self.memory_pool.extend(vec![0; size]);
                self.stack.push(StackValue::unmanaged_ptr(
                    self.memory_pool[loc..].as_mut_ptr(),
                ))
            }
            Multiply => binary_arith_op!(self, wrapping_mul (f64 *), {}),
            MultiplyOverflow(sgn) => binary_checked_op!(self, sgn, checked_mul (f64 *), {}),
            Negate => match self.pop() {
                StackValue::Int32(i) => self.stack.push(StackValue::Int32(-i)),
                StackValue::Int64(i) => self.stack.push(StackValue::Int64(-i)),
                StackValue::NativeInt(i) => self.stack.push(StackValue::NativeInt(-i)),
                v => todo!("invalid type on stack ({:?}) for ! operation", v),
            },
            NoOperation => {}
            // TODO: how are booleans + boolean negation represented?
            Not => match self.pop() {
                StackValue::Int32(i) => self.stack.push(StackValue::Int32(!i)),
                StackValue::Int64(i) => self.stack.push(StackValue::Int64(!i)),
                StackValue::NativeInt(i) => self.stack.push(StackValue::NativeInt(!i)),
                v => todo!("invalid type on stack ({:?}) for ~ operation", v),
            },
            Or => binary_int_op!(self, |),
            Pop => {
                self.pop();
            }
            Remainder(sgn) => match sgn {
                NumberSign::Signed => binary_arith_op!(self, %, { }),
                NumberSign::Unsigned => binary_int_op!(self, % as unsigned),
            },
            Return => return Some(Ok(self.pop())),
            ShiftLeft => shift_op!(self, <<),
            ShiftRight(sgn) => match sgn {
                NumberSign::Signed => shift_op!(self, >>),
                NumberSign::Unsigned => shift_op!(self, >> as unsigned),
            },
            StoreArgument(i) => {
                let val = self.pop();
                match self.arguments.get_mut(*i as usize) {
                    Some(v) => *v = val,
                    None => todo!("invalid argument index to store into"),
                }
            }
            StoreIndirect { .. } => {}
            StoreLocal(i) => {
                let val = self.pop();
                match self.locals.get_mut(*i as usize) {
                    Some(v) => *v = val,
                    None => todo!("invalid local variable index to store into"),
                }
            }
            Subtract => binary_arith_op!(self, wrapping_sub (f64 -), {
                (StackValue::ManagedPtr(ManagedPtr(p)), StackValue::Int32(i)) => unsafe {
                    self.stack
                        .push(StackValue::managed_ptr(p.offset(-i as isize)))
                },
                (StackValue::ManagedPtr(ManagedPtr(p)), StackValue::NativeInt(i)) => unsafe {
                    self.stack.push(StackValue::managed_ptr(p.offset(-i)))
                },
                (StackValue::ManagedPtr(ManagedPtr(p1)), StackValue::ManagedPtr(ManagedPtr(p2))) => {
                    self.stack.push(StackValue::NativeInt((p1 as isize) - (p2 as isize)))
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
            Throw => match self.stack.pop() {
                Some(v) => return Some(Err(v)),
                None => todo!("stack was empty when attempted to throw"),
            },
            UnboxIntoAddress { .. } => {}
            UnboxIntoValue(_) => {}
        }
        if !moved_ip {
            self.ip += 1;
        }
        None
    }
}
