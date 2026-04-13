#[macro_export]
macro_rules! trait_alias {
    () => {};

    (
        $(#[$($attrs:tt)*])*
        $vis:vis trait $name:ident = $($rest:tt)*
    ) => {
        $crate::trait_alias!(
            @scan_attrs_no_generics
            [with_blanket_impl]
            []
            $(#[$($attrs)*])*
            @end
            [$vis]
            [$name]
            []
            $($rest)*
        );
    };

    (
        $(#[$($attrs:tt)*])*
        $vis:vis trait $name:ident < $($lifetimes:lifetime),+ > = $($rest:tt)*
    ) => {
        $crate::trait_alias!(
            @scan_attrs_with_generics
            [with_blanket_impl]
            []
            $(#[$($attrs)*])*
            @end
            [$vis]
            [$name]
            [$($lifetimes),+]
            []
            $($rest)*
        );
    };

    (@scan_attrs_no_generics
        [$mode:ident]
        [$($kept:tt)*]
        #[no_blanket_impl]
        $($rest:tt)*
    ) => {
        $crate::trait_alias!(
            @scan_attrs_no_generics
            [without_blanket_impl]
            [$($kept)*]
            $($rest)*
        );
    };

    (@scan_attrs_no_generics
        [$mode:ident]
        [$($kept:tt)*]
        #[$($attr:tt)*]
        $($rest:tt)*
    ) => {
        $crate::trait_alias!(
            @scan_attrs_no_generics
            [$mode]
            [$($kept)* #[$($attr)*]]
            $($rest)*
        );
    };

    (@scan_attrs_no_generics
        [$mode:ident]
        [$($kept:tt)*]
        @end
        [$vis:vis]
        [$name:ident]
        [$($bounds:tt)*]
        $($rest:tt)*
    ) => {
        $crate::trait_alias!(
            @collect_no_generics
            [$mode]
            [$($kept)*]
            [$vis]
            [$name]
            [$($bounds)*]
            $($rest)*
        );
    };

    (@scan_attrs_with_generics
        [$mode:ident]
        [$($kept:tt)*]
        #[no_blanket_impl]
        $($rest:tt)*
    ) => {
        $crate::trait_alias!(
            @scan_attrs_with_generics
            [without_blanket_impl]
            [$($kept)*]
            $($rest)*
        );
    };

    (@scan_attrs_with_generics
        [$mode:ident]
        [$($kept:tt)*]
        #[$($attr:tt)*]
        $($rest:tt)*
    ) => {
        $crate::trait_alias!(
            @scan_attrs_with_generics
            [$mode]
            [$($kept)* #[$($attr)*]]
            $($rest)*
        );
    };

    (@scan_attrs_with_generics
        [$mode:ident]
        [$($kept:tt)*]
        @end
        [$vis:vis]
        [$name:ident]
        [$($lifetimes:lifetime),+]
        [$($bounds:tt)*]
        $($rest:tt)*
    ) => {
        $crate::trait_alias!(
            @collect_with_generics
            [$mode]
            [$($kept)*]
            [$vis]
            [$name]
            [$($lifetimes),+]
            [$($bounds)*]
            $($rest)*
        );
    };

    (@collect_no_generics
        [$mode:ident]
        [$($attrs:tt)*]
        [$vis:vis]
        [$name:ident]
        [$($bounds:tt)*]
        ;
        $($tail:tt)*
    ) => {
        $crate::trait_alias!(
            @emit_no_generics
            [$mode]
            [$($attrs)*]
            $vis trait $name = $($bounds)+
        );
        $crate::trait_alias! { $($tail)* }
    };

    (@collect_no_generics
        [$mode:ident]
        [$($attrs:tt)*]
        [$vis:vis]
        [$name:ident]
        [$($bounds:tt)*]
        $next:tt
        $($rest:tt)*
    ) => {
        $crate::trait_alias!(
            @collect_no_generics
            [$mode]
            [$($attrs)*]
            [$vis]
            [$name]
            [$($bounds)* $next]
            $($rest)*
        );
    };

    (@collect_with_generics
        [$mode:ident]
        [$($attrs:tt)*]
        [$vis:vis]
        [$name:ident]
        [$($lifetimes:lifetime),+]
        [$($bounds:tt)*]
        ;
        $($tail:tt)*
    ) => {
        $crate::trait_alias!(
            @emit_with_generics
            [$mode]
            [$($attrs)*]
            $vis trait $name < $($lifetimes),+ > = $($bounds)+
        );
        $crate::trait_alias! { $($tail)* }
    };

    (@collect_with_generics
        [$mode:ident]
        [$($attrs:tt)*]
        [$vis:vis]
        [$name:ident]
        [$($lifetimes:lifetime),+]
        [$($bounds:tt)*]
        $next:tt
        $($rest:tt)*
    ) => {
        $crate::trait_alias!(
            @collect_with_generics
            [$mode]
            [$($attrs)*]
            [$vis]
            [$name]
            [$($lifetimes),+]
            [$($bounds)* $next]
            $($rest)*
        );
    };

    (@emit_no_generics
        [with_blanket_impl]
        [$($attrs:tt)*]
        $vis:vis trait $name:ident = $($bounds:tt)+
    ) => {
        $($attrs)*
        $vis trait $name: $($bounds)+ {}

        impl<T: $($bounds)+ + ?Sized> $name for T {}
    };

    (@emit_no_generics
        [without_blanket_impl]
        [$($attrs:tt)*]
        $vis:vis trait $name:ident = $($bounds:tt)+
    ) => {
        $($attrs)*
        $vis trait $name: $($bounds)+ {}
    };

    (@emit_with_generics
        [with_blanket_impl]
        [$($attrs:tt)*]
        $vis:vis trait $name:ident < $($lifetimes:lifetime),+ > = $($bounds:tt)+
    ) => {
        $($attrs)*
        $vis trait $name<$($lifetimes),+>: $($bounds)+ {}

        impl<$($lifetimes,)+ T: $($bounds)+ + ?Sized> $name<$($lifetimes),+> for T {}
    };

    (@emit_with_generics
        [without_blanket_impl]
        [$($attrs:tt)*]
        $vis:vis trait $name:ident < $($lifetimes:lifetime),+ > = $($bounds:tt)+
    ) => {
        $($attrs)*
        $vis trait $name<$($lifetimes),+>: $($bounds)+ {}
    };

    (
        $($unsupported:tt)+
    ) => {
        compile_error!(
            "invalid trait_alias! input; expected `trait_alias! { $(#[])? vis trait Name<'gc>? = Bound + Bound; ... }`"
        );
    };
}

#[macro_export]
macro_rules! vm_try {
    ($expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => return $crate::StepResult::Error(dotnet_types::error::VmError::from(e)),
        }
    };
}
