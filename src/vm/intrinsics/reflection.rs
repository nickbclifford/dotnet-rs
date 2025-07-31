use crate::{
    value::{ConcreteType, GenericLookup, HeapStorage, MethodDescription, Object, ObjectRef},
    vm::{CallStack, GCHandle},
};

use dotnetdll::prelude::MethodType;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuntimeType {
    pub target: MethodType,
    pub source: MethodDescription,
    pub generics: GenericLookup,
}

impl<'gc> TryFrom<ObjectRef<'gc>> for RuntimeType {
    type Error = String;

    fn try_from(obj: ObjectRef<'gc>) -> Result<Self, Self::Error> {
        obj.as_object(|instance| {
            let mut ct = [0u8; size_of::<usize>()];
            let tn = instance.description.type_name();
            if tn != "DotnetRs.RuntimeType" {
                return Err(tn);
            }
            ct.copy_from_slice(instance.instance_storage.get_field("pointerToKey"));
            Ok(unsafe { &*(usize::from_ne_bytes(ct) as *const RuntimeType) })
        })
        .cloned()
    }
}

impl From<RuntimeType> for ConcreteType {
    fn from(value: RuntimeType) -> Self {
        value
            .generics
            .make_concrete(value.source.resolution(), value.target)
    }
}

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn get_runtime_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> ObjectRef<'gc> {
        if let Some(obj) = self.runtime_types.get(&target) {
            return *obj;
        }
        let rt = self.assemblies.corlib_type("System.RuntimeType");
        let rt_obj = Object::new(rt, self.current_context());
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
        self.register_new_object(&obj_ref);
        let key = target.clone();
        let entry = self.runtime_types.entry(key).insert_entry(obj_ref);
        let type_ptr = entry.key() as *const RuntimeType as usize;
        entry.get().as_object_mut(gc, |instance| {
            instance
                .instance_storage
                .get_field_mut("pointerToKey")
                .copy_from_slice(&type_ptr.to_ne_bytes());
        });
        obj_ref
    }

    pub fn get_handle_for_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> Object<'gc> {
        let rth = self.assemblies.corlib_type("System.RuntimeTypeHandle");
        let mut instance = Object::new(rth, self.current_context());
        let handle_location = instance.instance_storage.get_field_mut("_value");
        self.get_runtime_type(gc, target).write(handle_location);
        instance
    }
}
