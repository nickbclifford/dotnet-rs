use dotnet_rs::{utils::ResolutionS, value::{Context, GenericLookup}, vm::{self, GCArena, MethodInfo}};
use dotnetdll::{prelude::{body, Accessibility::Public, Instruction, Module, Resolution, ReturnType}, resolved::{self, members::Method, signature::MethodSignature}};

fn prepare() -> (GCArena, ResolutionS) {
    let resolution = Box::new(Resolution::new(Module::new("")));
    let resolution = Box::leak(resolution);
    let assemblies = Box::new(dotnet_rs::resolve::Assemblies::new(resolution, String::new()));
    let assemblies = Box::leak(assemblies);
    (GCArena::new(|gc| vm::CallStack::new(gc, assemblies)), resolution)
}

macro_rules! simple_test {
    ($($x:expr),+ $(,)?) => {
        {
            let (mut arena, resolution) = prepare();
            arena.mutate_root(|gc, c| 
            {
                let instructions = vec![$($x),+];
                let length = instructions.len();
                let method = Box::new(Method::new(Public, MethodSignature::static_member(ReturnType::VOID, vec![]), "test", Some(body::Method::new(instructions))));
                let method = Box::leak(method);
                let generics = GenericLookup::new(vec![]);
                c.entrypoint_frame(gc, MethodInfo::new(resolution, method, Context { generics: &generics, assemblies: c.assemblies, resolution }), generics, vec![]);
                for _ in 0..length {
                    c.step(gc);
                }
            });
            arena
        }
    };
}

macro_rules! test_math {
    ($instr:block, $op:tt, $left:literal, $right:literal, $expected:literal) => {
        let mut arena = simple_test! {
            Instruction::LoadConstantInt32($left), 
            Instruction::LoadConstantInt32($right), 
            $instr
        };
        arena.mutate_root(|_, c| match c.pop_stack() {
            dotnet_rs::value::StackValue::Int32(x) => assert_eq!(x, $expected),
            rest => panic!("returned {:?}, expected Int32({})", rest, $expected)
        })
    };
}

#[test]
fn test_add() {
    test_math!({Instruction::Add}, +, 1, 2, 3);
}

#[test]
fn test_multiply() {
    test_math!({Instruction::Multiply}, *, 2, 2, 4);
}

#[test]
fn test_div() {
    test_math!({Instruction::Divide(resolved::il::NumberSign::Signed)}, /, 4, -2, -2);
}

#[test]
fn test_unsigned_div() {
    test_math!({Instruction::Divide(resolved::il::NumberSign::Unsigned)}, /, 4, 2, 2);
}

#[test]
fn test_remainder() {
    test_math!({Instruction::Remainder(resolved::il::NumberSign::Signed)}, %, 5, -2, 1);
}

#[test]
fn test_unsigned_remainder() {
    test_math!({Instruction::Remainder(resolved::il::NumberSign::Unsigned)}, %, 5, 2, 1);
}

#[test]
fn test_and() {
    test_math!({Instruction::And}, &, 3, 1, 1);
}

#[test]
fn test_or() {
    test_math!({Instruction::Or}, |, 3, 1, 3);
}

#[test]
fn test_xor() {
    test_math!({Instruction::Xor}, ^, 3, 1, 2);
}

#[test]
fn test_negate() {
    test_math!({Instruction::Negate}, -, 0 /* not important */, 6, -6);
}

#[test]
fn test_not() {
    test_math!({Instruction::Not}, !, 0 /* not important */, 1, -2);
}

#[test]
fn test_shift_left() {
    test_math!({Instruction::ShiftLeft}, <<, 2, 1, 4);
}

#[test]
fn test_shift_right() {
    test_math!({Instruction::ShiftRight(resolved::il::NumberSign::Signed)}, >>, -2, 1, -1);
}

#[test]
fn test_unsigned_shift_right() {
    test_math!({Instruction::ShiftRight(resolved::il::NumberSign::Unsigned)}, >>, 2, 1, 1);
}