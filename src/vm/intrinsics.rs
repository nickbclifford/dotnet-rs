use dotnetdll::prelude::ResolvedDebug;

use crate::value::{GenericLookup, MethodDescription};

use super::CallStack;

pub fn intrinsic_call<'gc, 'm: 'gc>(
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: GenericLookup,
) {
    // TODO: real signature checking
    match (
        method.method.name.as_ref(),
        method.method.signature.show(method.resolution()).as_ref(),
    ) {
        ("AsRef", "[Generic(1)] static ref M0 (ref M0)") => {
            // as far as I can tell, this signature only converts from readonly to writable ref
            // since we're just doing simple pointers for refs, I think we can safely treat it as a noop
        }
        ("As", "[Generic(2)] static ref M1 (ref M0)") => {
            // likewise, this is just converting the destination types of ref pointers
        }
        ("As", "[Generic(1)] static M0 (object)") => {
            // now the parameter is a GC object pointer that we're reinterpreting
            // the description says no dynamic type checking - so also a noop?
        }
        x => panic!("unsupported intrinsic call to {:?}", x),
    }

    stack.current_frame_mut().state.ip += 1;
}
