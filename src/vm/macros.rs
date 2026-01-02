#[macro_export]
macro_rules! vm_pop {
    ($stack:expr) => {
        $stack.pop_stack()
    };
}

#[macro_export]
macro_rules! vm_push {
    ($stack:expr, $gc:expr, string($gc_ignore:expr, $val:expr)) => {
        {
            let obj = $crate::value::ObjectRef::new($gc, $crate::value::HeapStorage::Str($val));
            $stack.register_new_object(&obj);
            $stack.push_stack($gc, $crate::value::StackValue::ObjectRef(obj))
        }
    };
    ($stack:expr, $gc:expr, $variant:ident ( $($args:expr),* )) => {
        $stack.push_stack($gc, $crate::value::StackValue::$variant($($args),*))
    };
    ($stack:expr, $gc:expr, $val:expr) => {
        $stack.push_stack($gc, $val)
    };
}

#[macro_export]
macro_rules! vm_expect_stack {
    (let $variant:ident ( $inner:ident $(as $t:ty)? ) = $v:expr) => {
        let $inner = match $v {
            $crate::value::StackValue::$variant($inner) => $inner,
            err => panic!(
                "invalid type on stack ({:?}), expected {}",
                err,
                stringify!($variant)
            ),
        };
        $(
            let $inner = $inner as $t;
        )?
    };
}

#[macro_export]
macro_rules! vm_msg {
    ($src:expr, $($format:tt)*) => {
        $src.msg(format_args!($($format)*))
    }
}
