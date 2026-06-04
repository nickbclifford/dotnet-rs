//! CIL instruction handlers for the .NET virtual machine.
//!
//! This module contains implementations of all Common Intermediate Language (CIL)
//! instructions defined in the ECMA-335 specification. Each instruction handler
//! manipulates the evaluation stack and execution state through the [`VesOps`]
//! trait abstraction.
//!
//! # Architecture
//!
//! Instruction handlers are organized by functional category:
//!
//! - **[`arithmetic`]**: Arithmetic operations (`add`, `sub`, `mul`, `div`, `rem`, etc.)
//! - **[`calls`]**: Method invocation (`call`, `callvirt`, `calli`, `ret`, `newobj`)
//! - **[`comparisons`]**: Comparison and equality tests (`ceq`, `cgt`, `clt`)
//! - **[`conversions`]**: Type conversions and casts (`conv.*`, `castclass`, `isinst`)
//! - **[`exceptions`]**: Exception handling (`throw`, `rethrow`, `endfinally`)
//! - **[`flow`]**: Control flow (`br`, `brtrue`, `brfalse`, `switch`, `leave`)
//! - **[`memory`]**: Memory operations (`ldind.*`, `stind.*`, `ldobj`, `stobj`, `cpobj`, `initobj`)
//! - **[`objects`]**: Object operations (arrays, fields, boxing)
//! - **[`reflection`]**: Reflection operations (`ldtoken`, type/method/field metadata)
//! - **[`stack_ops`]**: Stack manipulation (`dup`, `pop`, `ldarg`, `ldloc`, `starg`, `stloc`)
//!
//! # Instruction Registration
//!
//! Handlers are registered with the [`InstructionRegistry`](crate::dispatch::InstructionRegistry)
//! using the `#[dotnet_instruction]` procedural macro from the `dotnet-macros` crate.
//! The registry maps opcode bytes to handler functions, enabling dynamic dispatch
//! during bytecode execution.
//!
//! # Handler Signature
//!
//! All instruction handlers follow this pattern:
//!
//! ```ignore
//! #[dotnet_instruction]
//! pub fn instruction_name<'gc, T: VesOps<'gc>>(
//!     ctx: &mut T,
//!     instr: &Instruction,
//! ) -> StepResult {
//!     // 1. Decode operands from the instruction when needed
//!     let operand = decode_operand(instr);
//!
//!     // 2. Perform operation
//!     let result = compute(operand);
//!
//!     // 3. Push result to evaluation stack
//!     ctx.push_i32(result);
//!
//!     // 4. Return control flow result
//!     StepResult::Continue
//! }
//! ```
//!
//! # Step Results
//!
//! Handlers return [`StepResult`](crate::StepResult) to indicate control flow:
//! - `Continue`: Proceed to next instruction
//! - `Return`: Return from current method
//! - `Exception`: Unwind for exception handling
//! - `Yield`: Pause execution (e.g., for GC safe point)
//!
//! # Trait-Based Design
//!
//! Handlers depend on [`VesOps`](crate::stack::ops::VesOps) rather than concrete
//! [`VesContext`](crate::stack::VesContext), enabling:
//! - **Testing**: Mock implementations for unit tests
//! - **Decoupling**: Handlers don't need full VM context
//! - **Composition**: Different execution modes can provide specialized ops
//!
//! # Example
//!
//! ```ignore
//! use dotnet_macros::dotnet_instruction;
//! use dotnetdll::prelude::Instruction;
//! use crate::{StepResult, stack::ops::VesOps};
//!
//! #[dotnet_instruction]
//! pub fn add<'gc, T: VesOps<'gc>>(ctx: &mut T, _instr: &Instruction) -> StepResult {
//!     // Pop operands, perform addition, and push the result.
//!     // (exact stack-value helpers depend on the handler implementation)
//!     let _ = ctx;
//!     StepResult::Continue
//! }
//! ```
#[macro_use]
pub mod macros;

pub mod arithmetic;
pub mod calls;
pub mod comparisons;
pub mod conversions;
pub mod exceptions;
pub mod flow;
pub mod memory;
pub mod objects;
pub mod reflection;
pub mod stack_ops;

use dotnet_vm_ops::NULL_REF_MSG;
