macro_rules! vm_pop {
    ($ctx:expr) => {{
        #[allow(unused_imports)]
        use $crate::stack::ops::EvalStackOps;
        match $ctx.pop_safe() {
            Ok(v) => v,
            Err(e) => return StepResult::Error(e),
        }
    }};
}

macro_rules! binary_op {
    ($(#[$attr:meta])* $func_name:ident, $op:tt) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc, T: $crate::stack::ops::EvalStackOps<'gc>>(
            ctx: &mut T,
        ) -> StepResult {
            #[allow(unused_imports)]
            use $crate::stack::ops::EvalStackOps;
            let v2 = vm_pop!(ctx);
            let v1 = vm_pop!(ctx);
            ctx.push(v1 $op v2);
            StepResult::Continue
        }
    };
}

macro_rules! binary_op_result {
    ($(#[$attr:meta])* $func_name:ident, $method:ident) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc, T: $crate::stack::ops::EvalStackOps<'gc> + $crate::stack::ops::ExceptionOps<'gc>>(
            ctx: &mut T,
            sgn: NumberSign,
        ) -> StepResult {
            #[allow(unused_imports)]
            use $crate::stack::ops::EvalStackOps;
            let v2 = vm_pop!(ctx);
            let v1 = vm_pop!(ctx);
            match v1.$method(v2, sgn) {
                Ok(v) => {
                    ctx.push(v);
                    StepResult::Continue
                }
                Err(e) => ctx.throw_by_name_with_message(e.exception_type, e.message),
            }
        }
    };
}

macro_rules! binary_op_sgn {
    ($(#[$attr:meta])* $func_name:ident, $method:ident) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc, T: $crate::stack::ops::EvalStackOps<'gc>>(
            ctx: &mut T,
            sgn: NumberSign,
        ) -> StepResult {
            #[allow(unused_imports)]
            use $crate::stack::ops::EvalStackOps;
            let v2 = vm_pop!(ctx);
            let v1 = vm_pop!(ctx);
            ctx.push(v1.$method(v2, sgn));
            StepResult::Continue
        }
    };
}

macro_rules! unary_op {
    ($(#[$attr:meta])* $func_name:ident, $op:tt) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc, T: $crate::stack::ops::EvalStackOps<'gc>>(
            ctx: &mut T,
        ) -> StepResult {
            #[allow(unused_imports)]
            use $crate::stack::ops::EvalStackOps;
            let v = vm_pop!(ctx);
            ctx.push($op v);
            StepResult::Continue
        }
    };
}

macro_rules! comparison_op {
    ($(#[$attr:meta])* $func_name:ident, $pat:pat) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc, T: $crate::stack::ops::EvalStackOps<'gc> + $crate::stack::ops::TypedStackOps<'gc>>(
            ctx: &mut T,
            sgn: NumberSign,
        ) -> StepResult {
            #[allow(unused_imports)]
            use $crate::stack::ops::{EvalStackOps, TypedStackOps};
            let v2 = vm_pop!(ctx);
            let v1 = vm_pop!(ctx);
            let val = matches!(v1.compare(&v2, sgn), Some($pat)) as i32;
            ctx.push_i32(val);
            StepResult::Continue
        }
    };
}

macro_rules! load_var {
    ($(#[$attr:meta])* $func_name:ident, $get_method:ident, $index_type:ident) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc, T: $crate::stack::ops::VariableOps<'gc> + $crate::stack::ops::EvalStackOps<'gc>>(
            ctx: &mut T,
            index: u16,
        ) -> StepResult {
            #[allow(unused_imports)]
            use $crate::stack::ops::{ArgumentOps, EvalStackOps, LocalOps, VariableOps};
            let val = ctx.$get_method($crate::$index_type(index as usize));
            ctx.push(val);
            StepResult::Continue
        }
    };
}

macro_rules! store_var {
    ($(#[$attr:meta])* $func_name:ident, $set_method:ident, $index_type:ident) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc, T: $crate::stack::ops::VariableOps<'gc> + $crate::stack::ops::EvalStackOps<'gc>>(
            ctx: &mut T,
            index: u16,
        ) -> StepResult {
            #[allow(unused_imports)]
            use $crate::stack::ops::{ArgumentOps, EvalStackOps, LocalOps, VariableOps};
            let val = vm_pop!(ctx);
            ctx.$set_method($crate::$index_type(index as usize), val);
            StepResult::Continue
        }
    };
}

macro_rules! load_const {
    ($(#[$attr:meta])* $func_name:ident, $arg_type:ty, $expr:expr) => {
        $(#[$attr])*
        pub fn $func_name<'gc, 'm: 'gc, T: $crate::stack::ops::EvalStackOps<'gc>>(
            ctx: &mut T,
            val: $arg_type,
        ) -> StepResult {
            #[allow(unused_imports)]
            use $crate::stack::ops::EvalStackOps;
            ctx.push(($expr)(val));
            StepResult::Continue
        }
    };
}
