use crate::instructions::StepResult;
use crate::CallStack;
use dotnet_utils::gc::GCHandle;
use dotnet_macros::dotnet_instruction;
use dotnetdll::prelude::*;
use crate::intrinsics::reflection::ReflectionExtensions;
use dotnet_value::{StackValue, object::{ObjectRef, HeapStorage}};
use crate::resolution::ValueResolution;

#[dotnet_instruction(LoadTokenType)]
pub fn ldtoken_type<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodType
) -> StepResult {
    let runtime_type = {
        let ctx = stack.current_context();
        stack.make_runtime_type(&ctx, param0)
    };
    
    let rt_obj = stack.get_runtime_type(gc, runtime_type);
    
    let ctx = stack.current_context();
    let rth = stack.loader().corlib_type("System.RuntimeTypeHandle");
    let instance = ctx.new_object(rth);
    rt_obj.write(&mut instance.instance_storage.get_field_mut_local(rth, "_value"));
    
    stack.push(gc, StackValue::ValueType(Box::new(instance)));
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadTokenMethod)]
pub fn ldtoken_method<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &MethodSource
) -> StepResult {
    let (method, lookup) = stack.find_generic_method(param0);
    
    let method_obj = stack.get_runtime_method_obj(gc, method, lookup);
    
    let rmh = stack.loader().corlib_type("System.RuntimeMethodHandle");
    let instance = stack.current_context().new_object(rmh);
    method_obj.write(&mut instance.instance_storage.get_field_mut_local(rmh, "_value"));
    
    stack.push(gc, StackValue::ValueType(Box::new(instance)));
    StepResult::InstructionStepped
}

#[dotnet_instruction(LoadTokenField)]
pub fn ldtoken_field<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    param0: &FieldSource
) -> StepResult {
    let (field, lookup) = stack.locate_field(param0.clone());
    
    let field_obj = stack.get_runtime_field_obj(gc, field, lookup);
    
    let ctx = stack.current_context();
    let rfh = stack.loader().corlib_type("System.RuntimeFieldHandle");
    let instance = ctx.new_object(rfh);
    field_obj.write(&mut instance.instance_storage.get_field_mut_local(rfh, "_value"));
    
    stack.push(gc, StackValue::ValueType(Box::new(instance)));
    StepResult::InstructionStepped
}
