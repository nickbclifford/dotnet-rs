use crate::StepResult;
use dotnet_utils::gc::GCHandle;
use dotnetdll::prelude::Instruction;

pub type InstructionHandler = for<'gc, 'm> fn(
    &mut crate::stack::VesContext<'_, 'gc, 'm>,
    GCHandle<'gc>,
    &Instruction,
) -> StepResult;

pub type InstructionTable = [Option<InstructionHandler>; Instruction::VARIANT_COUNT];

include!(concat!(env!("OUT_DIR"), "/instruction_table.rs"));

pub fn get_handler(opcode: usize) -> Option<InstructionHandler> {
    if opcode < Instruction::VARIANT_COUNT {
        INSTRUCTION_TABLE[opcode]
    } else {
        None
    }
}
