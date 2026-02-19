#![no_main]
use dotnet_vm::fuzzing::{FuzzProgram, execute_cil_program};
use libfuzzer_sys::fuzz_target;
use std::panic;

fuzz_target!(|program: FuzzProgram| {
    // Catch panics to identify bugs in the executor
    let _ = panic::catch_unwind(|| {
        execute_cil_program(program);
    });
});
