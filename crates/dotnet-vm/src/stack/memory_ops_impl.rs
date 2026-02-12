use super::context::VesContext;
use crate::memory::ops::MemoryOps;
use dotnet_types::{TypeDescription, generics::ConcreteType};
use dotnet_utils::gc::{GCHandle, ThreadSafeLock};
use super::ops::{LoaderOps, ResolutionOps};
use dotnet_value::{
    StackValue,
    object::{
        CTSValue, HeapStorage, Object as ObjectInstance, ObjectInner, ObjectRef, ValueType, Vector,
    },
};

impl<'a, 'gc, 'm: 'gc> MemoryOps<'gc> for VesContext<'a, 'gc, 'm> {
    #[inline]
    fn gc(&self) -> GCHandle<'gc> {
        self.gc
    }

    #[inline]
    fn new_vector(&self, element: ConcreteType, size: usize) -> Vector<'gc> {
        self.resolver()
            .new_vector(element, size, &self.current_context())
    }

    #[inline]
    fn new_object(&self, td: TypeDescription) -> ObjectInstance<'gc> {
        self.resolver().new_object(td, &self.current_context())
    }

    #[inline]
    fn new_value_type(&self, t: &ConcreteType, data: StackValue<'gc>) -> ValueType<'gc> {
        self.resolver()
            .new_value_type(t, data, &self.current_context())
    }

    #[inline]
    fn new_cts_value(&self, t: &ConcreteType, data: StackValue<'gc>) -> CTSValue<'gc> {
        self.resolver()
            .new_cts_value(t, data, &self.current_context())
    }

    #[inline]
    fn read_cts_value(&self, t: &ConcreteType, data: &[u8]) -> CTSValue<'gc> {
        let gc = self.gc;
        self.resolver()
            .read_cts_value(t, data, gc, &self.current_context())
    }

    fn clone_object(&self, obj: ObjectRef<'gc>) -> ObjectRef<'gc> {
        let gc = self.gc;
        let h = obj.0.expect("cannot clone null object");
        let inner = h.borrow();
        let cloned_storage = inner.storage.clone();

        let new_inner = ObjectInner {
            #[cfg(any(feature = "memory-validation", debug_assertions))]
            magic: inner.magic,
            owner_id: self.thread_id.get(),
            storage: cloned_storage,
        };

        let new_h = gc_arena::Gc::new(gc, ThreadSafeLock::new(new_inner));
        let new_ref = ObjectRef(Some(new_h));
        self.register_new_object(&new_ref);
        new_ref
    }

    #[inline]
    fn register_new_object(&self, instance: &ObjectRef<'gc>) {
        if let Some(ptr) = instance.0 {
            let addr = gc_arena::Gc::as_ptr(ptr) as usize;
            self.local
                .heap
                ._all_objs
                .borrow_mut()
                .insert(addr, *instance);

            // Add to finalization queue if it has a finalizer
            if let HeapStorage::Obj(o) = &ptr.borrow().storage
                && (o.description.static_initializer().is_some()
                    || o.description
                        .definition()
                        .methods
                        .iter()
                        .any(|m| m.name == "Finalize"))
            {
                self.local
                    .heap
                    .finalization_queue
                    .borrow_mut()
                    .push(*instance);
            }
        }
    }

    #[inline]
    fn heap(&self) -> &crate::memory::heap::HeapManager<'gc> {
        &self.local.heap
    }
}
