macro_rules! runtime_type_impls {
    (
        simple_types: {
            $( $variant:ident => $name:literal ),* $(,)?
        },
        complex_types: {
            $(
                $complex_variant:ident
                $( ( $($tuple_arg:tt)* ) )?
                $( { $($struct_arg:tt)* } )?
            ),* $(,)?
        },
        resolution: |$res_loader:ident| {
            $( $res_pat:pat => $res_expr:expr ),* $(,)?
        },
        get_name: {
            $( $name_pat:pat => $name_expr:expr ),* $(,)?
        },
        to_concrete: |$conc_loader:ident, $conc_res:ident| {
            $( $conc_pat:pat => $conc_expr:expr ),* $(,)?
        }
    ) => {
        #[derive(Clone, PartialEq, Eq, Hash, Debug)]
        pub enum RuntimeType {
            $( $variant, )*
            $(
                $complex_variant
                $( ( $($tuple_arg)* ) )?
                $( { $($struct_arg)* } )?,
            )*
        }

        impl RuntimeType {
            pub fn resolution(&self, $res_loader: &impl TypeResolver) -> ResolutionS {
                use RuntimeType::*;
                match self {
                    $( $variant => $res_loader.corlib_type("System.Object").resolution, )*
                    $( $res_pat => $res_expr, )*
                }
            }

            pub fn get_name(&self) -> String {
                use RuntimeType::*;
                match self {
                    $( $variant => $name.to_string(), )*
                    $( $name_pat => $name_expr, )*
                }
            }

            pub fn to_concrete(&self, $conc_loader: &impl TypeResolver) -> ConcreteType {
                let $conc_res = $conc_loader.corlib_type("System.Object").resolution;
                use RuntimeType::*;
                match self {
                    $( $variant => ConcreteType::new($conc_res, BaseType::$variant), )*
                    $( $conc_pat => $conc_expr, )*
                }
            }
        }
    };
}
