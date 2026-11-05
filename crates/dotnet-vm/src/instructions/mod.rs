//! CIL instruction handlers for the .NET virtual machine.
//!
//! This module contains implementations of all Common Intermediate Language (CIL)
//! instructions defined in the ECMA-335 specification (Partition III). Each handler
//! manipulates the evaluation stack and execution state through the capability traits in
//! [`crate::stack::ops`].
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
//! # Handlers
//!
//! Each handler is a free function annotated with `#[dotnet_instruction(Opcode)]` from
//! `dotnet-macros`, which registers it with the
//! [`InstructionRegistry`](crate::dispatch::InstructionRegistry). Operands named in the
//! attribute pattern (for example `Branch(target)`) become extra parameters after `ctx`.
//! Handlers are generic over the narrowest capability traits they need rather than the
//! full [`VesOps`](crate::stack::ops::VesOps); [`stack_ops::duplicate`] (the `dup` opcode)
//! only needs [`EvalStackOps`](crate::stack::ops::EvalStackOps).
//!
//! Every handler returns a [`StepResult`](crate::StepResult), which tells the dispatch loop
//! whether to advance, jump, enter or leave a frame, unwind an exception, yield, or stop
//! with a host-side error.
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
