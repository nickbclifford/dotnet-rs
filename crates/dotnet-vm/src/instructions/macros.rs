macro_rules! binary_op {
    ($(#[$attr:meta])* $func_name:ident, $op:tt) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
            let v2 = stack.pop(gc);
            let v1 = stack.pop(gc);
            stack.push(gc, v1 $op v2);
            StepResult::Continue
        }
    };
}

macro_rules! binary_op_result {
    ($(#[$attr:meta])* $func_name:ident, $method:ident) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc>(
            gc: GCHandle<'gc>,
            stack: &mut CallStack<'gc, 'm>,
            sgn: NumberSign,
        ) -> StepResult {
            let v2 = stack.pop(gc);
            let v1 = stack.pop(gc);
            match v1.$method(v2, sgn) {
                Ok(v) => {
                    stack.push(gc, v);
                    StepResult::Continue
                }
                Err(e) => stack.throw_by_name(gc, e),
            }
        }
    };
}

macro_rules! binary_op_sgn {
    ($(#[$attr:meta])* $func_name:ident, $method:ident) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc>(
            gc: GCHandle<'gc>,
            stack: &mut CallStack<'gc, 'm>,
            sgn: NumberSign,
        ) -> StepResult {
            let v2 = stack.pop(gc);
            let v1 = stack.pop(gc);
            stack.push(gc, v1.$method(v2, sgn));
            StepResult::Continue
        }
    };
}

macro_rules! unary_op {
    ($(#[$attr:meta])* $func_name:ident, $op:tt) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc>(gc: GCHandle<'gc>, stack: &mut CallStack<'gc, 'm>) -> StepResult {
            let v = stack.pop(gc);
            stack.push(gc, $op v);
            StepResult::Continue
        }
    };
}

macro_rules! comparison_op {
    ($(#[$attr:meta])* $func_name:ident, $pat:pat) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc>(
            gc: GCHandle<'gc>,
            stack: &mut CallStack<'gc, 'm>,
            sgn: NumberSign,
        ) -> StepResult {
            let v2 = stack.pop(gc);
            let v1 = stack.pop(gc);
            let val = matches!(v1.compare(&v2, sgn), Some($pat)) as i32;
            stack.push_i32(gc, val);
            StepResult::Continue
        }
    };
}

pub(crate) use binary_op;
pub(crate) use binary_op_result;
pub(crate) use binary_op_sgn;
pub(crate) use comparison_op;
pub(crate) use unary_op;

macro_rules! load_var {
    ($(#[$attr:meta])* $func_name:ident, $get_method:ident) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc>(
            gc: GCHandle<'gc>,
            stack: &mut CallStack<'gc, 'm>,
            index: u16,
        ) -> StepResult {
            let val = stack.$get_method(index as usize);
            stack.push(gc, val);
            StepResult::Continue
        }
    };
}

macro_rules! store_var {
    ($(#[$attr:meta])* $func_name:ident, $set_method:ident) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc>(
            gc: GCHandle<'gc>,
            stack: &mut CallStack<'gc, 'm>,
            index: u16,
        ) -> StepResult {
            let val = stack.pop(gc);
            stack.$set_method(gc, index as usize, val);
            StepResult::Continue
        }
    };
}

macro_rules! load_const {
    ($(#[$attr:meta])* $func_name:ident, $arg_type:ty, $expr:expr) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc>(
            gc: GCHandle<'gc>,
            stack: &mut CallStack<'gc, 'm>,
            val: $arg_type,
        ) -> StepResult {
            stack.push(gc, ($expr)(val));
            StepResult::Continue
        }
    };
}

pub(crate) use load_const;
pub(crate) use load_var;
pub(crate) use store_var;
