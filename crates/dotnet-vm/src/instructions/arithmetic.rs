use crate::{StepResult, instructions::macros::*};
use dotnet_macros::dotnet_instruction;
use dotnetdll::prelude::*;

binary_op!(#[dotnet_instruction(Add)] add, +);
binary_op_result!(
    #[dotnet_instruction(AddOverflow(sgn))]
    add_ovf,
    checked_add
);
binary_op!(#[dotnet_instruction(And)] and, &);
binary_op_result!(
    #[dotnet_instruction(Divide(sgn))]
    divide,
    div
);

binary_op!(#[dotnet_instruction(Multiply)] multiply, *);
binary_op_result!(
    #[dotnet_instruction(MultiplyOverflow(sgn))]
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
    #[dotnet_instruction(Remainder(sgn))]
    remainder,
    rem
);
binary_op!(#[dotnet_instruction(ShiftLeft)] shl, <<);
binary_op_sgn!(
    #[dotnet_instruction(ShiftRight(sgn))]
    shr,
    shr
);
binary_op!(#[dotnet_instruction(Subtract)] subtract, -);
binary_op_result!(
    #[dotnet_instruction(SubtractOverflow(sgn))]
    subtract_ovf,
    checked_sub
);
binary_op!(#[dotnet_instruction(Xor)] xor, ^);
