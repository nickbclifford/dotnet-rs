use super::value::*;
use dotnetdll::prelude::*;

// I.12.3.2

pub struct MethodState {
    ip: usize,
    stack: Vec<StackValue>,
    locals: Vec<StackValue>,
    arguments: Vec<StackValue>,
    info_handle: MethodInfo,
    memory_pool: Vec<u8>,
}

pub struct MethodInfo {
    signature: ManagedMethod,
    locals: Vec<LocalVariable>,
    exceptions: Vec<body::Exception>,
}

// TODO: well-typed exceptions
pub type ExecutionResult = Result<StackValue, StackValue>;

macro_rules! binary_arith_op {
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
impl MethodState {
    fn pop(&mut self) -> StackValue {
        match self.stack.pop() {
            Some(v) => v,
            None => todo!("stack was empty when attempted to pop"),
        }
    }

    fn execute(&mut self, i: &Instruction) -> Option<ExecutionResult> {
        use Instruction::*;
        match i {
            Add => binary_arith_op!(self, +, {
                (StackValue::Int32(i), StackValue::ManagedPtr(p)) => {
                    // TODO: proper mechanisms for safety and pointer arithmetic
                    unsafe {
                        self.stack
                            .push(StackValue::ManagedPtr(p.offset(i as isize)))
                    }
                },
                (StackValue::NativeInt(i), StackValue::ManagedPtr(p)) => unsafe {
                    self.stack.push(StackValue::ManagedPtr(p.offset(i)))
                },
                (StackValue::ManagedPtr(p), StackValue::Int32(i)) => unsafe {
                    self.stack
                        .push(StackValue::ManagedPtr(p.offset(i as isize)))
                },
                (StackValue::ManagedPtr(p), StackValue::NativeInt(i)) => unsafe {
                    self.stack.push(StackValue::ManagedPtr(p.offset(i)))
                },
            }),
            AddOverflow(_) => todo!("add with overflow"),
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
            }
            Breakpoint => {}
            BranchFalsy(_) => {}
            BranchTruthy(_) => {}
            Call { .. } => {}
            CallConstrained(_, _) => {}
            CallIndirect { .. } => {}
            CompareEqual => {}
            CompareGreater(_) => {}
            CheckFinite => {}
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
            LocalMemoryAllocate => {}
            Multiply => binary_arith_op!(self, *, { }),
            MultiplyOverflow(_) => {}
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
            Return => {}
            ShiftLeft => {}
            ShiftRight(_) => {}
            StoreArgument(_) => {}
            StoreIndirect { .. } => {}
            StoreLocal(_) => {}
            Subtract => binary_arith_op!(self, -, {
                (StackValue::ManagedPtr(p), StackValue::Int32(i)) => unsafe {
                    self.stack
                        .push(StackValue::ManagedPtr(p.offset(-i as isize)))
                },
                (StackValue::ManagedPtr(p), StackValue::NativeInt(i)) => unsafe {
                    self.stack.push(StackValue::ManagedPtr(p.offset(-i)))
                },
                (StackValue::ManagedPtr(p1), StackValue::ManagedPtr(p2)) => {
                    self.stack.push(StackValue::NativeInt((p1 as isize) - (p2 as isize)))
                },
            }),
            SubtractOverflow(_) => todo!("subtract with overflow"),
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
        None
    }
}
