use crate::value::{GenericLookup, ManagedPtr, MethodDescription, StackValue};

use super::CallStack;

pub fn intrinsic_call<'gc, 'm: 'gc>(
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: GenericLookup,
) {
    // TODO: real signature checking
    match format!("{:?}", method).as_str() {
        "[Generic(1)] static ref M0 System.Runtime.CompilerServices.Unsafe::AsRef(ref M0)" => {
            // as far as I can tell, this signature only converts from readonly to writable ref
            // since we're just doing simple pointers for refs, I think we can safely treat it as a noop
        }
        "[Generic(2)] static ref M1 System.Runtime.CompilerServices.Unsafe::As(ref M0)" => {
            // likewise, this is just converting the destination types of ref pointers
        }
        "[Generic(1)] static M0 System.Runtime.CompilerServices.Unsafe::As(object)" => {
            // now the parameter is a GC object pointer that we're reinterpreting
            // the description says no dynamic type checking - so also a noop?
        }
        "static void System.Threading.Monitor::ReliableEnter(object, ref bool)" => {
            let success_flag = match stack.pop_stack() {
                StackValue::ManagedPtr(ManagedPtr(p)) => p,
                v => todo!(
                    "invalid type on stack ({:?}), expected managed pointer for ref parameter",
                    v
                ),
            };

            // TODO(threading): actually acquire mutex
            let _tag_object = stack.pop_stack();
            // eventually we'll set this properly to indicate success or failure
            // just make it always succeed for now
            unsafe {
                *success_flag = 1u8;
            }
        }
        "static void System.Threading.Monitor::Exit(object)" => {
            // TODO(threading): release mutex
            let _tag_object = stack.pop_stack();
        }
        x => panic!("unsupported intrinsic call to {:?}", x),
    }

    stack.current_frame_mut().state.ip += 1;
}
