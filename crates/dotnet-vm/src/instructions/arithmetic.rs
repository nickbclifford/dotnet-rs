use crate::{instructions::macros::*, CallStack, StepResult};
use dotnet_macros::dotnet_instruction;
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::*;

binary_op!(#[dotnet_instruction(Add)] add, +);
binary_op_result!(
    #[dotnet_instruction(AddOverflow)]
    add_ovf,
    checked_add
);
binary_op!(#[dotnet_instruction(And)] and, &);
binary_op_result!(
    #[dotnet_instruction(Divide)]
    divide,
    div
);

binary_op!(#[dotnet_instruction(Multiply)] multiply, *);
binary_op_result!(
    #[dotnet_instruction(MultiplyOverflow)]
    multiply_ovf,
    checked_mul
);
unary_op!(#[dotnet_instruction(Negate)] negate, -);
unary_op!(
    #[dotnet_instruction(Not)]
    not,
    !
);
binary_op!(#[dotnet_instruction(Or)] or, |);
binary_op_result!(
    #[dotnet_instruction(Remainder)]
    remainder,
    rem
);
binary_op!(#[dotnet_instruction(ShiftLeft)] shl, <<);
binary_op_sgn!(
    #[dotnet_instruction(ShiftRight)]
    shr,
    shr
);
binary_op!(#[dotnet_instruction(Subtract)] subtract, -);
binary_op_result!(
    #[dotnet_instruction(SubtractOverflow)]
    subtract_ovf,
    checked_sub
);
binary_op!(#[dotnet_instruction(Xor)] xor, ^);
