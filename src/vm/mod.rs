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
            Add => {}
            AddOverflow(_) => {}
            And => {}
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
            Divide(_) => {}
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
                None => todo!("invalid argument index to load")
            }),
            LoadArgumentAddress(_) => {}
            LoadConstantInt32(i) => self.stack.push(StackValue::Int32(*i)),
            LoadConstantInt64(i) =>  self.stack.push(StackValue::Int64(*i)),
            LoadConstantFloat32(f) => self.stack.push(StackValue::NativeFloat(*f as f64)),
            LoadConstantFloat64(f) => self.stack.push(StackValue::NativeFloat(*f)),
            LoadMethodPointer(_) => {}
            LoadIndirect { .. } => {}
            LoadLocal(i) => self.stack.push(match self.locals.get(*i as usize) {
                Some(v) => v.clone(),
                None => todo!("invalid local variable index to load")
            }),
            LoadLocalAddress(_) => {}
            LoadNull => self.stack.push(StackValue::ObjectRef(None)),
            Leave(_) => {}
            LocalMemoryAllocate => {}
            Multiply => {}
            MultiplyOverflow(_) => {}
            Negate => {}
            NoOperation => {}
            Not => {}
            Or => {}
            Pop => self.pop(),
            Remainder(_) => {}
            Return => {}
            ShiftLeft => {}
            ShiftRight(_) => {}
            StoreArgument(_) => {}
            StoreIndirect { .. } => {}
            StoreLocal(_) => {}
            Subtract => {}
            SubtractOverflow(_) => {}
            Switch(_) => {}
            Xor => {}
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
