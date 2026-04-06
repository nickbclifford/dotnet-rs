use dotnetdll::prelude::Instruction;

include!(concat!(env!("OUT_DIR"), "/instruction_dispatch.rs"));

#[cfg(test)]
mod tests {
    const DISPATCH_TABLE: &str = include_str!(concat!(env!("OUT_DIR"), "/instruction_dispatch.rs"));

    fn normalize_ws(input: &str) -> String {
        input.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn dispatch_table_includes_representative_opcodes_per_category() {
        let dispatch = normalize_ws(DISPATCH_TABLE);

        // Golden checks: one representative opcode mapping per instruction category.
        let expected_mappings = [
            // arithmetic
            "Instruction::Add=>crate::instructions::arithmetic::add(ctx)",
            // calls
            "Instruction::Jump(param0)=>crate::instructions::calls::jmp(ctx,param0)",
            // comparisons
            "Instruction::CompareEqual=>crate::instructions::comparisons::ceq(ctx)",
            // conversions
            "Instruction::ConvertFloat64=>crate::instructions::conversions::conv_r8(ctx)",
            // exceptions
            "Instruction::Throw=>crate::instructions::exceptions::throw(ctx)",
            // flow
            "Instruction::Return=>crate::instructions::flow::ret(ctx)",
            // memory
            "Instruction::LocalMemoryAllocate=>crate::instructions::memory::localloc(ctx)",
            // objects
            "Instruction::NewArray(param0)=>crate::instructions::objects::arrays::newarr(ctx,param0)",
            // reflection
            "Instruction::LoadMethodPointer(param0)=>crate::instructions::reflection::ldftn(ctx,param0)",
            // stack ops
            "Instruction::Pop=>crate::instructions::stack_ops::pop(ctx)",
        ];

        for mapping in expected_mappings {
            assert!(
                dispatch.contains(mapping),
                "expected dispatch table to contain mapping: {mapping}"
            );
        }
    }
}
