use crate::{
    utils::decompose_type_source,
    value::{
        string::CLRString, ConcreteType, GenericLookup, HeapStorage, MethodDescription, Object,
        ObjectRef, ResolutionContext, StackValue, TypeDescription, Vector,
    },
    vm::{CallStack, GCHandle},
};
use dotnetdll::prelude::{BaseType, MethodType, TypeSource};
use std::{fmt::Debug, hash::Hash};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RuntimeType {
    Structure(crate::utils::ResolutionS, BaseType<Box<RuntimeType>>),
    TypeParameter {
        owner: TypeDescription,
        index: u16,
    },
    MethodParameter {
        owner: MethodDescription,
        index: u16,
    },
}

impl RuntimeType {
    pub fn resolution(&self) -> crate::utils::ResolutionS {
        match self {
            RuntimeType::Structure(res, _) => *res,
            RuntimeType::TypeParameter { owner, .. } => owner.resolution,
            RuntimeType::MethodParameter { owner, .. } => owner.resolution(),
        }
    }
}

impl From<RuntimeType> for ConcreteType {
    fn from(value: RuntimeType) -> Self {
        match value {
            RuntimeType::Structure(res, base) => {
                ConcreteType::new(res, base.map(|t| RuntimeType::into(*t)))
            }
            rest => todo!("convert {rest:?} to ConcreteType"),
        }
    }
}

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn get_runtime_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> ObjectRef<'gc> {
        if let Some(obj) = self.runtime_types.get(&target) {
            return *obj;
        }
        let rt = self.assemblies.corlib_type("DotnetRs.RuntimeType");
        let rt_obj = Object::new(rt, &self.current_context());
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
        self.register_new_object(&obj_ref);

        let index = self.runtime_types_list.len();
        self.runtime_types_list.push(target.clone());
        self.runtime_types.insert(target, obj_ref);

        obj_ref.as_object_mut(gc, |instance| {
            instance
                .instance_storage
                .get_field_mut("index")
                .copy_from_slice(&index.to_ne_bytes());
        });
        obj_ref
    }

    pub fn resolve_runtime_type(&self, obj: ObjectRef<'gc>) -> &RuntimeType {
        obj.as_object(|instance| {
            let mut ct = [0u8; ObjectRef::SIZE];
            ct.copy_from_slice(instance.instance_storage.get_field("index"));
            let index = usize::from_ne_bytes(ct);
            &self.runtime_types_list[index]
        })
    }

    pub fn make_runtime_type(&self, ctx: &ResolutionContext, t: &MethodType) -> RuntimeType {
        match t {
            MethodType::Base(b) => RuntimeType::Structure(
                ctx.resolution,
                b.clone().map(|t| Box::new(self.make_runtime_type(ctx, &t))),
            ),
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

    pub fn get_handle_for_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> Object<'gc> {
        let rth = self.assemblies.corlib_type("System.RuntimeTypeHandle");
        let mut instance = Object::new(rth, &self.current_context());
        let handle_location = instance.instance_storage.get_field_mut("_value");
        self.get_runtime_type(gc, target).write(handle_location);
        instance
    }

    pub fn get_runtime_method_index(&mut self, gc: GCHandle<'gc>, method: MethodDescription, lookup: GenericLookup) -> u16 {
        let idx = match self
            .runtime_methods
            .iter()
            .position(|(m, g)| *m == method && *g == lookup)
        {
            Some(i) => i,
            None => {
                self.runtime_methods.push((method, lookup));
                self.runtime_methods.len() - 1
            }
        };
        idx as u16
    }
}

pub fn runtime_type_intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: GenericLookup,
) {
    macro_rules! pop {
        () => {
            vm_pop!(stack)
        };
    }
    macro_rules! push {
        ($($args:tt)*) => {
            vm_push!(stack, gc, $($args)*)
        };
    }

    // TODO: real signature checking
    match format!("{:?}", method).as_str() {
        "DotnetRs.Assembly DotnetRs.RuntimeType::GetAssembly()" => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());

            let target_type = stack.resolve_runtime_type(obj);
            let resolution = target_type.resolution();

            let value = match stack.runtime_asms.get(&resolution) {
                Some(o) => *o,
                None => {
                    let support_res = stack.assemblies.get_assembly(crate::resolve::SUPPORT_ASSEMBLY);
                    let definition = support_res.0.type_definitions
                        .iter()
                        .find(|a| a.type_name() == "DotnetRs.Assembly")
                        .expect("could not find DotnetRs.Assembly in support library");
                    let mut asm_handle = Object::new(
                        TypeDescription { resolution: support_res, definition },
                        &ResolutionContext::new(&generics, stack.assemblies, support_res),
                    );
                    let data = (resolution.as_raw() as usize).to_ne_bytes();
                    asm_handle.instance_storage.get_field_mut("resolution").copy_from_slice(&data);
                    let v = ObjectRef::new(gc, HeapStorage::Obj(asm_handle));
                    stack.runtime_asms.insert(resolution, v);
                    v
                }
            };
            push!(StackValue::ObjectRef(value));
        }
        "string DotnetRs.RuntimeType::GetNamespace()" => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let target_type = stack.resolve_runtime_type(obj);
            match target_type {
                RuntimeType::Structure(_, base) => {
                    if let BaseType::Type { source, .. } = base {
                        let (ut, _) = decompose_type_source(source);
                        let td = stack.assemblies.locate_type(target_type.resolution(), ut);
                        match td.definition.namespace.as_ref() {
                            None => push!(StackValue::null()),
                            Some(n) => push!(StackValue::string(gc, CLRString::from(n)))
                        }
                    } else {
                        push!(StackValue::null())
                    }
                }
                _ => push!(StackValue::null())
            }
        }
        "string DotnetRs.RuntimeType::GetName()" => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let target_type = stack.resolve_runtime_type(obj);
            let name = match target_type {
                RuntimeType::Structure(_, base) => {
                    match base {
                        BaseType::Type { source, .. } => {
                            let (ut, _) = decompose_type_source(source);
                            ut.type_name(target_type.resolution().0).to_string()
                        }
                        b => format!("{:?}", b)
                    }
                }
                RuntimeType::TypeParameter { index, owner } => {
                     owner.definition.generic_parameters[*index as usize].name.to_string()
                }
                RuntimeType::MethodParameter { index, owner: _ } => {
                     // TODO: find where generic parameters are on Method
                     format!("!!{}", index)
                }
            };
            push!(StackValue::string(gc, CLRString::from(name)));
        }
        "bool DotnetRs.RuntimeType::GetIsGenericType()" => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let target_type = stack.resolve_runtime_type(obj);
            let is_generic = match target_type {
                RuntimeType::Structure(_, BaseType::Type { source, .. }) => {
                    let (_, generics) = decompose_type_source(source);
                    !generics.is_empty()
                }
                _ => false
            };
            push!(StackValue::Int32(if is_generic { 1 } else { 0 }));
        }
        "System.Type DotnetRs.RuntimeType::GetGenericTypeDefinition()" => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let target_type = stack.resolve_runtime_type(obj);
            match target_type {
                RuntimeType::Structure(res, base) => {
                    if let BaseType::Type { source, value_kind } = base {
                        let (ut, _) = decompose_type_source(source);
                        let td = stack.assemblies.locate_type(*res, ut);
                        let n_params = td.definition.generic_parameters.len();
                        let mut params = Vec::with_capacity(n_params);
                        for i in 0..n_params {
                            params.push(Box::new(RuntimeType::TypeParameter {
                                owner: td,
                                index: i as u16,
                            }));
                        }
                        let def_rt = RuntimeType::Structure(*res, BaseType::Type {
                            source: TypeSource::Generic {
                                base: ut,
                                parameters: params,
                            },
                            value_kind: *value_kind,
                        });
                        let rt_obj = stack.get_runtime_type(gc, def_rt);
                        push!(StackValue::ObjectRef(rt_obj));
                    } else {
                        todo!("InvalidOperationException: not a generic type")
                    }
                }
                _ => todo!("InvalidOperationException: not a generic type")
            }
        }
        "System.Type[] DotnetRs.RuntimeType::GetGenericArguments()" => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let target_type = stack.resolve_runtime_type(obj);
            let args = match target_type {
                RuntimeType::Structure(_, base) => {
                    if let BaseType::Type { source, .. } = base {
                        let (_, generics) = decompose_type_source(source);
                        generics
                    } else {
                        vec![]
                    }
                }
                _ => vec![]
            };

            let type_type_td = stack.assemblies.corlib_type("System.Type");
            let type_type = ConcreteType::from(type_type_td);
            let mut vector = Vector::new(type_type, args.len(), &stack.current_context());
            for (i, arg) in args.into_iter().enumerate() {
                let rt = (*arg).clone();
                let arg_obj = stack.get_runtime_type(gc, rt);
                arg_obj.write(&mut vector.get_mut()[(i * ObjectRef::SIZE)..]);
            }
            push!(StackValue::ObjectRef(ObjectRef::new(gc, HeapStorage::Vec(vector))));
        }
        "valuetype [System.Runtime]System.RuntimeTypeHandle DotnetRs.RuntimeType::GetTypeHandle()" => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());

            let rth = stack.assemblies.corlib_type("System.RuntimeTypeHandle");
            let mut instance = Object::new(rth, &stack.current_context());
            obj.write(instance.instance_storage.get_field_mut("_value"));

            push!(StackValue::ValueType(Box::new(instance)));
        }
        "[System.Runtime]System.Type DotnetRs.RuntimeType::MakeGenericType([System.Runtime]System.Type[])" => {
            vm_expect_stack!(let ObjectRef(parameters) = pop!());
            vm_expect_stack!(let ObjectRef(target) = pop!());
            let target_rt = stack.resolve_runtime_type(target);

            if let RuntimeType::Structure(res, base) = target_rt {
                if let BaseType::Type { source, value_kind } = base {
                     let (ut, _) = decompose_type_source(source);

                     let param_objs = parameters.as_vector(|v| {
                         let mut result = vec![];
                         for i in 0..v.layout.length {
                             result.push(ObjectRef::read(&v.get()[(i * ObjectRef::SIZE)..]));
                         }
                         result
                     });
                     let mut new_generics = Vec::with_capacity(param_objs.len());
                     for p_obj in param_objs {
                         new_generics.push(Box::new(stack.resolve_runtime_type(p_obj).clone()));
                     }
                     let new_rt = RuntimeType::Structure(*res, BaseType::Type {
                         source: TypeSource::Generic {
                             base: ut,
                             parameters: new_generics,
                         },
                         value_kind: *value_kind,
                     });

                     let rt_obj = stack.get_runtime_type(gc, new_rt);
                     push!(StackValue::ObjectRef(rt_obj));
                } else {
                    todo!("MakeGenericType on non-type")
                }
            } else {
                todo!("MakeGenericType on non-structure")
            }
        }
        rest => todo!("reflection intrinsic {rest}"),
    }
    stack.increment_ip();
}
