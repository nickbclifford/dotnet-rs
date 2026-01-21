use crate::{
    assemblies::SUPPORT_ASSEMBLY,
    pop_args,
    types::{
        generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
        runtime::{RuntimeMethodSignature, RuntimeType},
        TypeDescription,
    },
    utils::decompose_type_source,
    value::{
        layout::{LayoutManager, Scalar},
        object::{HeapStorage, Object, ObjectRef},
        StackValue,
    },
    vm::{
        context::ResolutionContext,
        layout::type_layout,
        resolution::{TypeResolutionExt, ValueResolution},
        CallStack, MethodInfo, StepResult,
    },
    utils::gc::GCHandle,
    vm_pop, vm_push,
};
use dotnetdll::prelude::{BaseType, Kind, MemberType, MethodType, TypeSource};

#[cfg(feature = "multithreaded-gc")]
use crate::utils::sync::Ordering;

fn get_runtime_member_index<T: PartialEq>(
    members: &mut Vec<(T, GenericLookup)>,
    member: T,
    lookup: GenericLookup,
) -> usize {
    members
        .iter()
        .position(|(m, g)| *m == member && *g == lookup)
        .unwrap_or_else(|| {
            members.push((member, lookup));
            members.len() - 1
        })
}

pub trait ReflectionExtensions<'gc, 'm> {
    fn pre_initialize_reflection(&mut self, gc: GCHandle<'gc>);
    fn get_runtime_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> ObjectRef<'gc>;
    fn resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> RuntimeType;
    fn resolve_runtime_method(&self, obj: ObjectRef<'gc>) -> (MethodDescription, GenericLookup);
    fn resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup);
    fn make_runtime_type(&self, ctx: &ResolutionContext, t: &MethodType) -> RuntimeType;
    fn get_handle_for_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> Object<'gc>;
    fn get_runtime_method_index(&mut self, method: MethodDescription, lookup: GenericLookup)
        -> u16;
    #[cfg(not(feature = "multithreaded-gc"))]
    fn get_runtime_field_index(&mut self, field: FieldDescription, lookup: GenericLookup) -> u16;
    fn get_runtime_method_obj(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc>;
    fn get_runtime_field_obj(
        &mut self,
        gc: GCHandle<'gc>,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc>;
}

impl<'gc, 'm: 'gc> ReflectionExtensions<'gc, 'm> for CallStack<'gc, 'm> {
    fn pre_initialize_reflection(&mut self, gc: GCHandle<'gc>) {
        let blessed = [
            RuntimeType::Void,
            RuntimeType::Boolean,
            RuntimeType::Char,
            RuntimeType::Int8,
            RuntimeType::UInt8,
            RuntimeType::Int16,
            RuntimeType::UInt16,
            RuntimeType::Int32,
            RuntimeType::UInt32,
            RuntimeType::Int64,
            RuntimeType::UInt64,
            RuntimeType::Float32,
            RuntimeType::Float64,
            RuntimeType::IntPtr,
            RuntimeType::UIntPtr,
            RuntimeType::Object,
            RuntimeType::String,
        ];

        for t in blessed {
            self.get_runtime_type(gc, t);
        }
    }

    fn get_runtime_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> ObjectRef<'gc> {
        if let Some(obj) = self.runtime_types_read().get(&target) {
            return *obj;
        }

        #[cfg(feature = "multithreaded-gc")]
        let index = *self
            .shared
            .shared_runtime_types
            .entry(target.clone())
            .or_insert_with(|| {
                let idx = self
                    .shared
                    .next_runtime_type_index
                    .fetch_add(1, Ordering::SeqCst);
                self.shared
                    .shared_runtime_types_rev
                    .insert(idx, target.clone());
                idx
            });

        #[cfg(not(feature = "multithreaded-gc"))]
        let index = {
            let mut list = self.runtime_types_list_write();
            let index = list.len();
            list.push(target.clone());
            index
        };

        let rt = self.loader().corlib_type("DotnetRs.RuntimeType");
        let rt_obj = self.current_context().new_object(rt);
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
        self.register_new_object(&obj_ref);

        // Set the index field
        obj_ref.as_object_mut(gc, |instance| {
            instance
                .instance_storage
                .get_field_mut_local(rt, "index")
                .copy_from_slice(&index.to_ne_bytes());
        });

        self.runtime_types_write().insert(target, obj_ref);
        obj_ref
    }

    fn resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> RuntimeType {
        obj.as_object(|instance| {
            let ct = instance
                .instance_storage
                .get_field_local(instance.description, "index");
            let index = usize::from_ne_bytes(ct.try_into().unwrap());
            #[cfg(feature = "multithreaded-gc")]
            return self
                .shared
                .shared_runtime_types_rev
                .get(&index)
                .map(|e| e.clone())
                .expect("invalid runtime type index");

            #[cfg(not(feature = "multithreaded-gc"))]
            self.runtime_types_list_read()[index].clone()
        })
    }

    fn resolve_runtime_method(&self, obj: ObjectRef<'gc>) -> (MethodDescription, GenericLookup) {
        obj.as_object(|instance| {
            let data = instance
                .instance_storage
                .get_field_local(instance.description, "index");
            let index = usize::from_ne_bytes(data.try_into().unwrap());
            #[cfg(feature = "multithreaded-gc")]
            return self
                .shared
                .shared_runtime_methods_rev
                .get(&index)
                .map(|e| e.clone())
                .expect("invalid runtime method index");

            #[cfg(not(feature = "multithreaded-gc"))]
            self.runtime_methods_read()[index].clone()
        })
    }

    fn resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> (FieldDescription, GenericLookup) {
        obj.as_object(|instance| {
            let data = instance
                .instance_storage
                .get_field_local(instance.description, "index");
            let index = usize::from_ne_bytes(data.try_into().unwrap());
            #[cfg(feature = "multithreaded-gc")]
            return self
                .shared
                .shared_runtime_fields_rev
                .get(&index)
                .map(|e| e.clone())
                .expect("invalid runtime field index");

            #[cfg(not(feature = "multithreaded-gc"))]
            self.runtime_fields_read()[index].clone()
        })
    }

    fn make_runtime_type(&self, ctx: &ResolutionContext, t: &MethodType) -> RuntimeType {
        match t {
            MethodType::Base(b) => match &**b {
                BaseType::Boolean => RuntimeType::Boolean,
                BaseType::Char => RuntimeType::Char,
                BaseType::Int8 => RuntimeType::Int8,
                BaseType::UInt8 => RuntimeType::UInt8,
                BaseType::Int16 => RuntimeType::Int16,
                BaseType::UInt16 => RuntimeType::UInt16,
                BaseType::Int32 => RuntimeType::Int32,
                BaseType::UInt32 => RuntimeType::UInt32,
                BaseType::Int64 => RuntimeType::Int64,
                BaseType::UInt64 => RuntimeType::UInt64,
                BaseType::Float32 => RuntimeType::Float32,
                BaseType::Float64 => RuntimeType::Float64,
                BaseType::IntPtr => RuntimeType::IntPtr,
                BaseType::UIntPtr => RuntimeType::UIntPtr,
                BaseType::Object => RuntimeType::Object,
                BaseType::String => RuntimeType::String,
                BaseType::Type { source, .. } => {
                    let (ut, generics) = decompose_type_source(source);
                    let td = ctx.locate_type(ut);
                    if generics.is_empty() {
                        RuntimeType::Type(td)
                    } else {
                        RuntimeType::Generic(
                            td,
                            generics
                                .iter()
                                .map(|g| self.make_runtime_type(ctx, g))
                                .collect(),
                        )
                    }
                }
                BaseType::Vector(_, t) => {
                    RuntimeType::Vector(Box::new(self.make_runtime_type(ctx, &t.clone())))
                }
                BaseType::Array(t, shape) => RuntimeType::Array(
                    Box::new(self.make_runtime_type(ctx, &t.clone())),
                    shape.rank as u32,
                ),
                BaseType::ValuePointer(_, t) => match t {
                    Some(inner) => {
                        RuntimeType::Pointer(Box::new(self.make_runtime_type(ctx, &inner.clone())))
                    }
                    None => RuntimeType::IntPtr,
                },
                BaseType::FunctionPointer(_sig) => {
                    RuntimeType::FunctionPointer(RuntimeMethodSignature)
                }
            },
            MethodType::TypeGeneric(i) => RuntimeType::TypeParameter {
                owner: ctx.type_owner.expect("missing type owner"),
                index: *i as u16,
            },
            MethodType::MethodGeneric(i) => RuntimeType::MethodParameter {
                owner: ctx.method_owner.expect("missing method owner"),
                index: *i as u16,
            },
        }
    }

    fn get_handle_for_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> Object<'gc> {
        let rth = self.loader().corlib_type("System.RuntimeTypeHandle");
        let mut instance = self.current_context().new_object(rth);
        let handle_location = instance.instance_storage.get_field_mut_local(rth, "_value");
        self.get_runtime_type(gc, target).write(handle_location);
        instance
    }

    fn get_runtime_method_index(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> u16 {
        let mut methods = self.runtime_methods_write();
        let idx = get_runtime_member_index(&mut methods, method, lookup);
        idx as u16
    }

    #[cfg(not(feature = "multithreaded-gc"))]
    fn get_runtime_field_index(&mut self, field: FieldDescription, lookup: GenericLookup) -> u16 {
        let mut fields = self.runtime_fields_write();
        let idx = get_runtime_member_index(&mut fields, field, lookup);
        idx as u16
    }

    fn get_runtime_method_obj(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        if let Some(obj) = self
            .runtime_method_objs_read()
            .get(&(method, lookup.clone()))
        {
            return *obj;
        }

        #[cfg(feature = "multithreaded-gc")]
        let index = *self
            .shared
            .shared_runtime_methods
            .entry((method, lookup.clone()))
            .or_insert_with(|| {
                let idx = self
                    .shared
                    .next_runtime_method_index
                    .fetch_add(1, Ordering::SeqCst);
                self.shared
                    .shared_runtime_methods_rev
                    .insert(idx, (method, lookup.clone()));
                idx
            });

        #[cfg(not(feature = "multithreaded-gc"))]
        let index = self.get_runtime_method_index(method, lookup.clone()) as usize;

        let is_ctor = method.method.name == ".ctor" || method.method.name == ".cctor";
        let class_name = if is_ctor {
            "DotnetRs.ConstructorInfo"
        } else {
            "DotnetRs.MethodInfo"
        };

        let rt = self.loader().corlib_type(class_name);
        let rt_obj = self.current_context().new_object(rt);
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
        self.register_new_object(&obj_ref);

        // Set the index field
        obj_ref.as_object_mut(gc, |instance| {
            instance
                .instance_storage
                .get_field_mut_local(rt, "index")
                .copy_from_slice(&index.to_ne_bytes());
        });

        self.runtime_method_objs_write()
            .insert((method, lookup), obj_ref);
        obj_ref
    }

    fn get_runtime_field_obj(
        &mut self,
        gc: GCHandle<'gc>,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        if let Some(obj) = self.runtime_field_objs_read().get(&(field, lookup.clone())) {
            return *obj;
        }

        #[cfg(feature = "multithreaded-gc")]
        let index = *self
            .shared
            .shared_runtime_fields
            .entry((field, lookup.clone()))
            .or_insert_with(|| {
                let idx = self
                    .shared
                    .next_runtime_field_index
                    .fetch_add(1, Ordering::SeqCst);
                self.shared
                    .shared_runtime_fields_rev
                    .insert(idx, (field, lookup.clone()));
                idx
            });

        #[cfg(not(feature = "multithreaded-gc"))]
        let index = self.get_runtime_field_index(field, lookup.clone()) as usize;

        let rt = self.loader().corlib_type("DotnetRs.FieldInfo");
        let rt_obj = self.current_context().new_object(rt);
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
        self.register_new_object(&obj_ref);

        // Set the index field
        obj_ref.as_object_mut(gc, |instance| {
            instance
                .instance_storage
                .get_field_mut_local(rt, "index")
                .copy_from_slice(&index.to_ne_bytes());
        });

        self.runtime_field_objs_write()
            .insert((field, lookup), obj_ref);
        obj_ref
    }
}

pub fn intrinsic_assembly_get_custom_attributes<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let num_args = method.method.signature.parameters.len()
        + if method.method.signature.instance {
            1
        } else {
            0
        };
    for _ in 0..num_args {
        stack.pop_stack(gc);
    }

    // Return an empty array of Attribute
    let attribute_type = stack.loader().corlib_type("System.Attribute");
    let array = stack.current_context().new_vector(attribute_type.into(), 0);
    let obj = ObjectRef::new(gc, HeapStorage::Vec(array));
    stack.register_new_object(&obj);
    vm_push!(stack, gc, ObjectRef(obj));

    StepResult::InstructionStepped
}

pub fn intrinsic_attribute_get_custom_attributes<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let num_args = method.method.signature.parameters.len()
        + if method.method.signature.instance {
            1
        } else {
            0
        };
    for _ in 0..num_args {
        stack.pop_stack(gc);
    }

    // Return an empty array of Attribute
    let attribute_type = stack.loader().corlib_type("System.Attribute");
    let array = stack.current_context().new_vector(attribute_type.into(), 0);
    let obj = ObjectRef::new(gc, HeapStorage::Vec(array));
    stack.register_new_object(&obj);
    vm_push!(stack, gc, ObjectRef(obj));

    StepResult::InstructionStepped
}

pub fn runtime_type_intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    macro_rules! pop {
        () => {
            vm_pop!(stack, gc)
        };
    }
    macro_rules! push {
        ($($args:tt)*) => {
            vm_push!(stack, gc, $($args)*)
        };
    }

    let method_name = &*method.method.name;
    let param_count = method.method.signature.parameters.len();

    let result = match (method_name, param_count) {
        ("CreateInstanceCheckThis", 0) => {
            pop_args!(stack, gc, [ObjectRef(_obj)]);
            // For now, we don't perform any actual checks.
            // In a real VM, this would check if the type is abstract, has a ctor, etc.
            Some(StepResult::InstructionStepped)
        }
        ("GetAssembly" | "get_Assembly", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);

            let target_type = stack.resolve_runtime_type(obj);
            let resolution = target_type.resolution(stack.loader());

            let cached_asm = stack.runtime_asms_read().get(&resolution).copied();
            if let Some(o) = cached_asm {
                push!(ObjectRef(o));
                return StepResult::InstructionStepped;
            }

            let support_res = stack.loader().get_assembly(SUPPORT_ASSEMBLY);
            let definition = support_res
                .definition()
                .type_definitions
                .iter()
                .find(|a| a.type_name() == "DotnetRs.Assembly")
                .expect("could find DotnetRs.Assembly in support library");
            let ctx = ResolutionContext::new(
                generics,
                stack.loader(),
                support_res,
                stack.shared.caches.clone(),
            );
            let mut asm_handle = ctx.new_object(TypeDescription::new(support_res, definition));
            let data = (resolution.as_raw() as usize).to_ne_bytes();
            asm_handle
                .instance_storage
                .get_field_mut_local(asm_handle.description, "resolution")
                .copy_from_slice(&data);
            let v = ObjectRef::new(gc, HeapStorage::Obj(asm_handle));
            stack.register_new_object(&v);

            stack.runtime_asms_write().insert(resolution, v);
            push!(ObjectRef(v));
            Some(StepResult::InstructionStepped)
        }
        ("GetNamespace" | "get_Namespace", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);
            let target_type = stack.resolve_runtime_type(obj);
            match target_type {
                RuntimeType::Type(td) | RuntimeType::Generic(td, _) => {
                    match td.definition().namespace.as_ref() {
                        None => push!(null()),
                        Some(n) => push!(string(n)),
                    }
                }
                _ => push!(string("System")),
            }
            Some(StepResult::InstructionStepped)
        }
        ("GetName" | "get_Name", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);
            let target_type = stack.resolve_runtime_type(obj);
            push!(string(target_type.get_name()));
            Some(StepResult::InstructionStepped)
        }
        ("GetBaseType" | "get_BaseType", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);
            let target_type = stack.resolve_runtime_type(obj);
            match target_type {
                RuntimeType::Type(td) | RuntimeType::Generic(td, _)
                    if td.definition().extends.is_some() =>
                {
                    // Get the first ancestor (the direct parent)
                    let mut ancestors = stack.loader().ancestors(td);
                    ancestors.next(); // skip self
                    if let Some((base_td, base_generics)) = ancestors.next() {
                        let base_rt = if base_td.definition().extends.is_none()
                            && base_td.type_name() == "System.Object"
                        {
                            RuntimeType::Object
                        } else if base_generics.is_empty() {
                            RuntimeType::Type(base_td)
                        } else {
                            // Convert member type generic parameters to runtime types
                            let runtime_generics: Vec<RuntimeType> = base_generics
                                .iter()
                                .map(|mt| match mt {
                                    MemberType::Base(b) => {
                                        match &**b {
                                            BaseType::Type { source, .. } => {
                                                let (ut, sub_generics) =
                                                    decompose_type_source(source);
                                                let sub_td =
                                                    stack.loader().locate_type(td.resolution, ut);
                                                if sub_generics.is_empty() {
                                                    RuntimeType::Type(sub_td)
                                                } else {
                                                    // TODO: properly handle generic types
                                                    RuntimeType::Type(sub_td)
                                                }
                                            }
                                            BaseType::Object => RuntimeType::Object,
                                            BaseType::String => RuntimeType::String,
                                            BaseType::Boolean => RuntimeType::Boolean,
                                            BaseType::Char => RuntimeType::Char,
                                            BaseType::Int8 => RuntimeType::Int8,
                                            BaseType::UInt8 => RuntimeType::UInt8,
                                            BaseType::Int16 => RuntimeType::Int16,
                                            BaseType::UInt16 => RuntimeType::UInt16,
                                            BaseType::Int32 => RuntimeType::Int32,
                                            BaseType::UInt32 => RuntimeType::UInt32,
                                            BaseType::Int64 => RuntimeType::Int64,
                                            BaseType::UInt64 => RuntimeType::UInt64,
                                            BaseType::Float32 => RuntimeType::Float32,
                                            BaseType::Float64 => RuntimeType::Float64,
                                            BaseType::IntPtr => RuntimeType::IntPtr,
                                            BaseType::UIntPtr => RuntimeType::UIntPtr,
                                            _ => RuntimeType::Object, // Fallback
                                        }
                                    }
                                    MemberType::TypeGeneric(i) => RuntimeType::TypeParameter {
                                        owner: td,
                                        index: *i as u16,
                                    },
                                })
                                .collect();
                            RuntimeType::Generic(base_td, runtime_generics)
                        };
                        let rt_obj = stack.get_runtime_type(gc, base_rt);
                        push!(ObjectRef(rt_obj));
                    } else {
                        push!(null());
                    }
                }
                _ => push!(null()),
            }
            Some(StepResult::InstructionStepped)
        }
        ("GetIsGenericType" | "get_IsGenericType", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);
            let target_type = stack.resolve_runtime_type(obj);
            let is_generic = matches!(target_type, RuntimeType::Generic(_, _));
            push!(Int32(if is_generic { 1 } else { 0 }));
            Some(StepResult::InstructionStepped)
        }
        ("GetGenericTypeDefinition" | "get_GenericTypeDefinition", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);
            let target_type = stack.resolve_runtime_type(obj);
            match target_type {
                RuntimeType::Generic(td, _) => {
                    let n_params = td.definition().generic_parameters.len();
                    let mut params = Vec::with_capacity(n_params);
                    for i in 0..n_params {
                        params.push(RuntimeType::TypeParameter {
                            owner: td,
                            index: i as u16,
                        });
                    }
                    let def_rt = RuntimeType::Generic(td, params);
                    let rt_obj = stack.get_runtime_type(gc, def_rt);
                    push!(ObjectRef(rt_obj));
                }
                _ => return stack.throw_by_name(gc, "System.InvalidOperationException"),
            }
            Some(StepResult::InstructionStepped)
        }
        ("GetGenericArguments" | "get_GenericArguments", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);
            let target_type = stack.resolve_runtime_type(obj);
            let args = match target_type {
                RuntimeType::Generic(_, args) => args.clone(),
                _ => vec![],
            };

            // Check GC safe point before allocating type array
            stack.check_gc_safe_point();

            let type_type_td = stack.loader().corlib_type("System.Type");
            let type_type = ConcreteType::from(type_type_td);
            let mut vector = stack.current_context().new_vector(type_type, args.len());
            for (i, arg) in args.into_iter().enumerate() {
                // Check GC safe point periodically during loops with allocations
                // Check every 16 iterations
                if i % 16 == 0 {
                    stack.check_gc_safe_point();
                }
                let arg_obj = stack.get_runtime_type(gc, arg);
                arg_obj.write(&mut vector.get_mut()[(i * ObjectRef::SIZE)..]);
            }
            let obj = ObjectRef::new(gc, HeapStorage::Vec(vector));
            stack.register_new_object(&obj);
            push!(ObjectRef(obj));
            Some(StepResult::InstructionStepped)
        }
        ("GetTypeHandle" | "get_TypeHandle", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);

            let rth = stack.loader().corlib_type("System.RuntimeTypeHandle");
            let mut instance = stack.current_context().new_object(rth);
            obj.write(instance.instance_storage.get_field_mut_local(rth, "_value"));

            push!(ValueType(Box::new(instance)));
            Some(StepResult::InstructionStepped)
        }
        ("MakeGenericType", 1) => {
            pop_args!(stack, gc, [ObjectRef(parameters), ObjectRef(target)]);

            // Check GC safe point before potentially allocating generic type objects
            stack.check_gc_safe_point();

            let target_rt = stack.resolve_runtime_type(target);

            if let RuntimeType::Type(td) | RuntimeType::Generic(td, _) = target_rt {
                let param_objs = parameters.as_vector(|v| {
                    let mut result = vec![];
                    for i in 0..v.layout.length {
                        result.push(ObjectRef::read(&v.get()[(i * ObjectRef::SIZE)..]));
                    }
                    result
                });
                let mut new_generics = Vec::with_capacity(param_objs.len());
                for p_obj in param_objs {
                    new_generics.push(stack.resolve_runtime_type(p_obj).clone());
                }
                let new_rt = RuntimeType::Generic(td, new_generics);

                let rt_obj = stack.get_runtime_type(gc, new_rt);
                push!(ObjectRef(rt_obj));
            } else {
                return stack.throw_by_name(gc, "System.InvalidOperationException");
            }
            Some(StepResult::InstructionStepped)
        }
        ("CreateInstanceDefaultCtor", 2) => {
            pop!(); // skipCheck
            pop!(); // publicOnly
            pop_args!(stack, gc, [ObjectRef(target_obj)]);

            // Check GC safe point before object instantiation
            stack.check_gc_safe_point();

            let target_rt = stack.resolve_runtime_type(target_obj);

            let (td, type_generics) = match target_rt {
                RuntimeType::Type(td) => (td, vec![]),
                RuntimeType::Generic(td, args) => (td, args.clone()),
                _ => panic!("cannot create instance of {:?}", target_rt),
            };

            let type_generics_concrete: Vec<ConcreteType> = type_generics
                .iter()
                .map(|a| a.to_concrete(stack.loader()))
                .collect();
            let new_lookup = GenericLookup::new(type_generics_concrete);
            let new_ctx = stack.current_context().with_generics(&new_lookup);

            let instance = new_ctx.new_object(td);

            for m in &td.definition().methods {
                if m.runtime_special_name
                    && m.name == ".ctor"
                    && m.signature.instance
                    && m.signature.parameters.is_empty()
                {
                    let desc = MethodDescription {
                        parent: td,
                        method_resolution: td.resolution,
                        method: m,
                    };

                    stack.constructor_frame(
                        gc,
                        instance,
                        MethodInfo::new(desc, &new_lookup, stack.shared.clone()),
                        new_lookup,
                    );
                    return StepResult::InstructionStepped;
                }
            }

            panic!("could not find a parameterless constructor in {:?}", td)
        }
        _ => None,
    };

    result.unwrap_or_else(|| panic!("unimplemented runtime type intrinsic: {:?}", method))
}

pub fn runtime_type_handle_intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let method_name = &*method.method.name;
    let param_count = method.method.signature.parameters.len();

    match (method_name, param_count) {
        ("GetActivationInfo", 5) => {
            // static extern void GetActivationInfo(RuntimeTypeHandle type, out IntPtr pfnAllocator, out IntPtr allocatorFirstArg, out IntPtr pfnCtor, out bool ctorIsPublic);
            pop_args!(
                stack,
                gc,
                [
                    ManagedPtr(ctor_is_public),
                    ManagedPtr(pfn_ctor),
                    ManagedPtr(allocator_first_arg),
                    ManagedPtr(pfn_allocator)
                ]
            );

            let rt_obj = match stack.pop_stack(gc) {
                StackValue::ValueType(rth_handle) => {
                    ObjectRef::read(rth_handle.instance_storage.get_field_local(rth_handle.description, "_value"))
                }
                StackValue::ObjectRef(rt_obj) => rt_obj,
                v => panic!("invalid type on stack ({:?}), expected ValueType(RuntimeTypeHandle) or ObjectRef(RuntimeType)", v),
            };

            let rt = stack.resolve_runtime_type(rt_obj);
            let td = match rt {
                RuntimeType::Type(td) => td,
                RuntimeType::Generic(td, _) => td,
                _ => panic!("GetActivationInfo called on non-type: {:?}", rt),
            };

            // pfnAllocator = IntPtr.Zero
            stack.push_stack(gc, StackValue::NativeInt(0));
            unsafe {
                stack.pop_stack(gc).store(
                    pfn_allocator.value.as_ptr(),
                    dotnetdll::prelude::StoreType::IntPtr,
                )
            };

            // allocatorFirstArg = IntPtr.Zero
            stack.push_stack(gc, StackValue::NativeInt(0));
            unsafe {
                stack.pop_stack(gc).store(
                    allocator_first_arg.value.as_ptr(),
                    dotnetdll::prelude::StoreType::IntPtr,
                )
            };

            // Find default ctor
            let mut found_ctor = false;
            for m in td.definition().methods.iter() {
                if m.name == ".ctor" && m.signature.instance && m.signature.parameters.is_empty() {
                    let method_idx = stack.get_runtime_method_index(
                        MethodDescription {
                            parent: td,
                            method_resolution: td.resolution,
                            method: m,
                        },
                        if let RuntimeType::Generic(_, type_generics) = rt {
                            GenericLookup {
                                type_generics: type_generics
                                    .iter()
                                    .map(|t| t.to_concrete(stack.loader()))
                                    .collect(),
                                method_generics: vec![],
                            }
                        } else {
                            stack.empty_generics().clone()
                        },
                    );

                    stack.push_stack(gc, StackValue::NativeInt(method_idx as isize));
                    unsafe {
                        stack.pop_stack(gc).store(
                            pfn_ctor.value.as_ptr(),
                            dotnetdll::prelude::StoreType::IntPtr,
                        )
                    };

                    stack.push_stack(gc, StackValue::Int32(1));
                    unsafe {
                        stack.pop_stack(gc).store(
                            ctor_is_public.value.as_ptr(),
                            dotnetdll::prelude::StoreType::Int8,
                        )
                    };
                    found_ctor = true;
                    break;
                }
            }

            if !found_ctor {
                panic!("Could not find default constructor for {}", td.type_name());
            }

            StepResult::InstructionStepped
        }
        _ => panic!(
            "Unknown RuntimeTypeHandle intrinsic: {}.{}",
            method.parent.type_name(),
            method_name
        ),
    }
}

pub fn runtime_method_info_intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    macro_rules! push {
        ($($args:tt)*) => {
            vm_push!(stack, gc, $($args)*)
        };
    }

    let method_name = &*method.method.name;
    let param_count = method.method.signature.parameters.len();

    let result = match (method_name, param_count) {
        ("GetName" | "get_Name", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);
            let (method, _) = stack.resolve_runtime_method(obj);
            push!(string(&method.method.name));
            Some(StepResult::InstructionStepped)
        }
        ("GetDeclaringType" | "get_DeclaringType", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);
            let (method, _) = stack.resolve_runtime_method(obj);
            let rt_obj = stack.get_runtime_type(gc, RuntimeType::Type(method.parent));
            push!(ObjectRef(rt_obj));
            Some(StepResult::InstructionStepped)
        }
        ("GetMethodHandle" | "get_MethodHandle", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);

            let rmh = stack.loader().corlib_type("System.RuntimeMethodHandle");
            let mut instance = stack.current_context().new_object(rmh);
            obj.write(instance.instance_storage.get_field_mut_local(rmh, "_value"));

            push!(ValueType(Box::new(instance)));
            Some(StepResult::InstructionStepped)
        }
        _ => None,
    };

    result.expect("unimplemented method info intrinsic");
    StepResult::InstructionStepped
}

pub fn runtime_field_info_intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    macro_rules! push {
        ($($args:tt)*) => {
            vm_push!(stack, gc, $($args)*)
        };
    }

    let method_name = &*method.method.name;
    let param_count = method.method.signature.parameters.len();

    let result = match (method_name, param_count) {
        ("GetName" | "get_Name", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);
            let (field, _) = stack.resolve_runtime_field(obj);
            push!(string(&field.field.name));
            Some(StepResult::InstructionStepped)
        }
        ("GetDeclaringType" | "get_DeclaringType", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);
            let (field, _) = stack.resolve_runtime_field(obj);
            let rt_obj = stack.get_runtime_type(gc, RuntimeType::Type(field.parent));
            push!(ObjectRef(rt_obj));
            Some(StepResult::InstructionStepped)
        }
        ("GetFieldHandle" | "get_FieldHandle", 0) => {
            pop_args!(stack, gc, [ObjectRef(obj)]);

            let rfh = stack.loader().corlib_type("System.RuntimeFieldHandle");
            let mut instance = stack.current_context().new_object(rfh);
            obj.write(instance.instance_storage.get_field_mut_local(rfh, "_value"));

            push!(ValueType(Box::new(instance)));
            Some(StepResult::InstructionStepped)
        }
        _ => None,
    };

    result.expect("unimplemented field info intrinsic");
    StepResult::InstructionStepped
}

pub fn intrinsic_runtime_helpers_get_method_table<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let obj = vm_pop!(stack, gc);
    let object_type = match obj {
        StackValue::ObjectRef(ObjectRef(Some(h))) => ctx.get_heap_description(h),
        StackValue::ObjectRef(ObjectRef(None)) => {
            return stack.throw_by_name(gc, "System.NullReferenceException");
        }
        _ => panic!("invalid type on stack"),
    };

    let mt_ptr = object_type.definition_ptr().unwrap().as_ptr();
    vm_push!(stack, gc, NativeInt(mt_ptr as isize));
    StepResult::InstructionStepped
}

pub fn intrinsic_runtime_helpers_is_bitwise_equatable<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);
    let value = match &*layout {
        LayoutManager::Scalar(Scalar::ObjectRef) => false,
        LayoutManager::Scalar(_) => true,
        _ => false,
    };
    vm_push!(stack, gc, Int32(value as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_runtime_helpers_is_reference_or_contains_references<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let target = &generics.method_generics[0];
    let layout = type_layout(target.clone(), &ctx);
    vm_push!(stack, gc, Int32(layout.is_or_contains_refs() as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_runtime_helpers_run_class_constructor<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let arg = stack.peek_stack();
    let StackValue::ValueType(handle) = arg else {
        panic!(
            "RunClassConstructor expects a RuntimeTypeHandle, received {:?}",
            arg
        )
    };

    let target_obj = ObjectRef::read(
        handle
            .instance_storage
            .get_field_local(handle.description, "_value"),
    );
    let target_type = stack.resolve_runtime_type(target_obj);
    let target_ct = target_type.to_concrete(stack.loader());
    let target_desc = stack.loader().find_concrete_type(target_ct);

    if stack.initialize_static_storage(gc, target_desc, generics.clone()) {
        return StepResult::InstructionStepped;
    }

    // Initialization complete, pop the argument
    stack.pop_stack(gc);
    StepResult::InstructionStepped
}

pub fn intrinsic_activator_create_instance<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    let target_ct = generics.method_generics[0].clone();
    let target_td = stack.loader().find_concrete_type(target_ct.clone());
    let ctx = ResolutionContext::for_method(
        method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );

    if target_td.is_value_type(&ctx) {
        let instance = ctx.new_object(target_td);
        vm_push!(stack, gc, ValueType(Box::new(instance)));
        StepResult::InstructionStepped
    } else {
        let instance = ctx.new_object(target_td);
        let mut new_lookup = GenericLookup::default();
        if let BaseType::Type {
            source: TypeSource::Generic { parameters, .. },
            ..
        } = target_ct.get()
        {
            new_lookup.type_generics = parameters.clone();
        }

        for m in target_td.definition().methods.iter() {
            if m.name == ".ctor" && m.signature.instance && m.signature.parameters.is_empty() {
                let desc = MethodDescription {
                    parent: target_td,
                    method_resolution: target_td.resolution,
                    method: m,
                };

                stack.constructor_frame(
                    gc,
                    instance,
                    MethodInfo::new(desc, &new_lookup, stack.shared.clone()),
                    new_lookup,
                );
                return StepResult::InstructionStepped;
            }
        }

        panic!(
            "could not find a parameterless constructor in {:?}",
            target_td
        )
    }
}

pub fn intrinsic_get_from_handle<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ValueType(handle)]);
    let target = ObjectRef::read(
        handle
            .instance_storage
            .get_field_local(handle.description, "_value"),
    );
    vm_push!(stack, gc, ObjectRef(target));
    StepResult::InstructionStepped
}

pub fn intrinsic_type_handle_to_int_ptr<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ValueType(handle)]);
    let target = handle
        .instance_storage
        .get_field_local(handle.description, "_value");
    let val = usize::from_ne_bytes(target.try_into().unwrap());
    vm_push!(stack, gc, NativeInt(val as isize));
    StepResult::InstructionStepped
}

pub fn intrinsic_method_handle_get_function_pointer<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ValueType(handle)]);
    let method_obj = ObjectRef::read(
        handle
            .instance_storage
            .get_field_local(handle.description, "_value"),
    );
    let (method, lookup) = stack.resolve_runtime_method(method_obj);
    let index = stack.get_runtime_method_index(method, lookup.clone());
    vm_push!(stack, gc, NativeInt(index as isize));
    StepResult::InstructionStepped
}

pub fn intrinsic_type_get_is_value_type<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(o)]);
    let target = stack.resolve_runtime_type(o);
    let target_ct = target.to_concrete(stack.loader());
    let target_desc = stack.loader().find_concrete_type(target_ct);
    let ctx = ResolutionContext::for_method(
        _method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let value = target_desc.is_value_type(&ctx);
    vm_push!(stack, gc, Int32(value as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_type_get_is_enum<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(o)]);
    let target = stack.resolve_runtime_type(o);
    let value = match target {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => td.is_enum().is_some(),
        _ => false,
    };
    vm_push!(stack, gc, Int32(value as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_type_get_is_interface<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(o)]);
    let target = stack.resolve_runtime_type(o);
    let value = match target {
        RuntimeType::Type(td) | RuntimeType::Generic(td, _) => {
            matches!(td.definition().flags.kind, Kind::Interface)
        }
        _ => false,
    };
    vm_push!(stack, gc, Int32(value as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_type_op_equality<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(o2), ObjectRef(o1)]);
    vm_push!(stack, gc, Int32((o1 == o2) as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_type_op_inequality<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(o2), ObjectRef(o1)]);
    vm_push!(stack, gc, Int32((o1 != o2) as i32));
    StepResult::InstructionStepped
}

pub fn intrinsic_type_get_type_handle<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    _method: MethodDescription,
    generics: &GenericLookup,
) -> StepResult {
    pop_args!(stack, gc, [ObjectRef(obj)]);

    let rth = stack.loader().corlib_type("System.RuntimeTypeHandle");
    let ctx = ResolutionContext::for_method(
        _method,
        stack.loader(),
        generics,
        stack.shared.caches.clone(),
    );
    let mut instance = ctx.new_object(rth);
    obj.write(instance.instance_storage.get_field_mut_local(rth, "_value"));

    vm_push!(stack, gc, ValueType(Box::new(instance)));
    StepResult::InstructionStepped
}
