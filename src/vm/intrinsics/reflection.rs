use crate::{
    assemblies::{AssemblyLoader, SUPPORT_ASSEMBLY},
    match_method,
    types::{
        TypeDescription, generics::{ConcreteType, GenericLookup},
        members::{FieldDescription, MethodDescription},
    },
    utils::{ResolutionS, decompose_type_source},
    value::object::{HeapStorage, Object, ObjectRef, Vector},
    vm::{CallStack, GCHandle, MethodInfo, StepResult, context::ResolutionContext},
    vm_expect_stack, vm_pop, vm_push,
};
use dotnetdll::prelude::{BaseType, MethodType, TypeSource};
use std::{fmt::Debug, hash::Hash};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RuntimeMethodSignature; // TODO

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RuntimeType {
    Void,
    Boolean,
    Char,
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Float32,
    Float64,
    IntPtr,
    UIntPtr,
    Object,
    String,
    Type(TypeDescription),
    Generic(TypeDescription, Vec<RuntimeType>),
    Vector(Box<RuntimeType>),
    Array(Box<RuntimeType>, u32),
    Pointer(Box<RuntimeType>),
    ByRef(Box<RuntimeType>),
    ValuePointer(Box<RuntimeType>, bool),
    FunctionPointer(RuntimeMethodSignature),
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
    pub fn resolution(&self, loader: &AssemblyLoader) -> ResolutionS {
        match self {
            RuntimeType::Void
            | RuntimeType::Boolean
            | RuntimeType::Char
            | RuntimeType::Int8
            | RuntimeType::UInt8
            | RuntimeType::Int16
            | RuntimeType::UInt16
            | RuntimeType::Int32
            | RuntimeType::UInt32
            | RuntimeType::Int64
            | RuntimeType::UInt64
            | RuntimeType::Float32
            | RuntimeType::Float64
            | RuntimeType::IntPtr
            | RuntimeType::UIntPtr
            | RuntimeType::Object
            | RuntimeType::String
            | RuntimeType::Vector(_)
            | RuntimeType::Array(_, _)
            | RuntimeType::Pointer(_)
            | RuntimeType::ByRef(_)
            | RuntimeType::ValuePointer(_, _)
            | RuntimeType::FunctionPointer(_) => loader.corlib_type("System.Object").resolution,
            RuntimeType::Type(td) => td.resolution,
            RuntimeType::Generic(td, _) => td.resolution,
            RuntimeType::TypeParameter { owner, .. } => owner.resolution,
            RuntimeType::MethodParameter { owner, .. } => owner.resolution(),
        }
    }

    pub fn get_name(&self) -> String {
        match self {
            RuntimeType::Void => "Void".to_string(),
            RuntimeType::Boolean => "Boolean".to_string(),
            RuntimeType::Char => "Char".to_string(),
            RuntimeType::Int8 => "SByte".to_string(),
            RuntimeType::UInt8 => "Byte".to_string(),
            RuntimeType::Int16 => "Int16".to_string(),
            RuntimeType::UInt16 => "UInt16".to_string(),
            RuntimeType::Int32 => "Int32".to_string(),
            RuntimeType::UInt32 => "UInt32".to_string(),
            RuntimeType::Int64 => "Int64".to_string(),
            RuntimeType::UInt64 => "UInt64".to_string(),
            RuntimeType::Float32 => "Single".to_string(),
            RuntimeType::Float64 => "Double".to_string(),
            RuntimeType::IntPtr => "IntPtr".to_string(),
            RuntimeType::UIntPtr => "UIntPtr".to_string(),
            RuntimeType::Object => "Object".to_string(),
            RuntimeType::String => "String".to_string(),
            RuntimeType::Type(td) | RuntimeType::Generic(td, _) => td.definition.name.to_string(),
            RuntimeType::Vector(t) => format!("{}[]", t.get_name()),
            RuntimeType::Array(t, rank) => {
                let commas = if *rank > 1 {
                    ",".repeat(*rank as usize - 1)
                } else {
                    "".to_string()
                };
                format!("{}[{}]", t.get_name(), commas)
            }
            RuntimeType::Pointer(t) => format!("{}*", t.get_name()),
            RuntimeType::ByRef(t) => format!("{}&", t.get_name()),
            RuntimeType::ValuePointer(t, _) => format!("{}*", t.get_name()),
            RuntimeType::TypeParameter { owner, index } => owner
                .definition
                .generic_parameters
                .get(*index as usize)
                .map(|p| p.name.to_string())
                .unwrap_or_else(|| format!("!{}", index)),
            RuntimeType::MethodParameter { index, .. } => format!("!!{}", index),
            RuntimeType::FunctionPointer(_) => "method*".to_string(),
        }
    }

    pub fn to_concrete(&self, loader: &AssemblyLoader) -> ConcreteType {
        let corlib_res = loader.corlib_type("System.Object").resolution;
        match self {
            RuntimeType::Void => ConcreteType::from(loader.corlib_type("System.Void")),
            RuntimeType::Boolean => ConcreteType::new(corlib_res, BaseType::Boolean),
            RuntimeType::Char => ConcreteType::new(corlib_res, BaseType::Char),
            RuntimeType::Int8 => ConcreteType::new(corlib_res, BaseType::Int8),
            RuntimeType::UInt8 => ConcreteType::new(corlib_res, BaseType::UInt8),
            RuntimeType::Int16 => ConcreteType::new(corlib_res, BaseType::Int16),
            RuntimeType::UInt16 => ConcreteType::new(corlib_res, BaseType::UInt16),
            RuntimeType::Int32 => ConcreteType::new(corlib_res, BaseType::Int32),
            RuntimeType::UInt32 => ConcreteType::new(corlib_res, BaseType::UInt32),
            RuntimeType::Int64 => ConcreteType::new(corlib_res, BaseType::Int64),
            RuntimeType::UInt64 => ConcreteType::new(corlib_res, BaseType::UInt64),
            RuntimeType::Float32 => ConcreteType::new(corlib_res, BaseType::Float32),
            RuntimeType::Float64 => ConcreteType::new(corlib_res, BaseType::Float64),
            RuntimeType::IntPtr => ConcreteType::new(corlib_res, BaseType::IntPtr),
            RuntimeType::UIntPtr => ConcreteType::new(corlib_res, BaseType::UIntPtr),
            RuntimeType::Object => ConcreteType::new(corlib_res, BaseType::Object),
            RuntimeType::String => ConcreteType::new(corlib_res, BaseType::String),
            RuntimeType::Type(td) => ConcreteType::from(*td),
            RuntimeType::Generic(td, args) => {
                let index = td
                    .resolution
                    .0
                    .type_definitions
                    .iter()
                    .position(|t| std::ptr::eq(t, td.definition))
                    .unwrap();
                let source = TypeSource::Generic {
                    base: dotnetdll::prelude::UserType::Definition(
                        td.resolution
                            .0
                            .type_definition_index(index)
                            .expect("invalid type definition"),
                    ),
                    parameters: args.iter().map(|a| a.to_concrete(loader)).collect(),
                };
                ConcreteType::new(
                    td.resolution,
                    BaseType::Type {
                        source,
                        value_kind: None,
                    },
                )
            }
            RuntimeType::Vector(t) => {
                ConcreteType::new(corlib_res, BaseType::Vector(vec![], t.to_concrete(loader)))
            }
            RuntimeType::Array(t, rank) => ConcreteType::new(
                corlib_res,
                BaseType::Array(
                    t.to_concrete(loader),
                    dotnetdll::binary::signature::encoded::ArrayShape {
                        rank: *rank as usize,
                        sizes: vec![],
                        lower_bounds: vec![],
                    },
                ),
            ),
            RuntimeType::Pointer(t) | RuntimeType::ByRef(t) | RuntimeType::ValuePointer(t, _) => {
                ConcreteType::new(
                    corlib_res,
                    BaseType::ValuePointer(vec![], Some(t.to_concrete(loader))),
                )
            }
            RuntimeType::TypeParameter { .. } | RuntimeType::MethodParameter { .. } => {
                ConcreteType::new(corlib_res, BaseType::Object)
            }
            rest => todo!("convert {rest:?} to ConcreteType"),
        }
    }
}

impl<'gc, 'm: 'gc> CallStack<'gc, 'm> {
    pub fn get_runtime_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> ObjectRef<'gc> {
        if let Some(obj) = self.runtime.runtime_types.get(&target) {
            return *obj;
        }
        let rt = self.runtime.loader.corlib_type("DotnetRs.RuntimeType");
        let rt_obj = Object::new(rt, &self.current_context());
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
        self.register_new_object(&obj_ref);

        let index = self.runtime.runtime_types_list.len();
        self.runtime.runtime_types_list.push(target.clone());
        self.runtime.runtime_types.insert(target, obj_ref);

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
            let ct = instance.instance_storage.get_field("index");
            let index = usize::from_ne_bytes(ct.try_into().unwrap());
            &self.runtime.runtime_types_list[index]
        })
    }

    pub fn resolve_runtime_method(
        &self,
        obj: ObjectRef<'gc>,
    ) -> &(MethodDescription, GenericLookup) {
        obj.as_object(|instance| {
            let data = instance.instance_storage.get_field("index");
            let index = usize::from_ne_bytes(data.try_into().unwrap());
            &self.runtime.runtime_methods[index]
        })
    }

    pub fn resolve_runtime_field(&self, obj: ObjectRef<'gc>) -> &(FieldDescription, GenericLookup) {
        obj.as_object(|instance| {
            let data = instance.instance_storage.get_field("index");
            let index = usize::from_ne_bytes(data.try_into().unwrap());
            &self.runtime.runtime_fields[index]
        })
    }

    pub fn make_runtime_type(&self, ctx: &ResolutionContext, t: &MethodType) -> RuntimeType {
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

    pub fn get_handle_for_type(&mut self, gc: GCHandle<'gc>, target: RuntimeType) -> Object<'gc> {
        let rth = self.runtime.loader.corlib_type("System.RuntimeTypeHandle");
        let mut instance = Object::new(rth, &self.current_context());
        let handle_location = instance.instance_storage.get_field_mut("_value");
        self.get_runtime_type(gc, target).write(handle_location);
        instance
    }

    pub fn get_runtime_method_index(
        &mut self,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> u16 {
        let idx = match self
            .runtime
            .runtime_methods
            .iter()
            .position(|(m, g)| *m == method && *g == lookup)
        {
            Some(i) => i,
            None => {
                self.runtime.runtime_methods.push((method, lookup));
                self.runtime.runtime_methods.len() - 1
            }
        };
        idx as u16
    }

    pub fn get_runtime_field_index(
        &mut self,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> u16 {
        let idx = match self
            .runtime
            .runtime_fields
            .iter()
            .position(|(f, g)| *f == field && *g == lookup)
        {
            Some(i) => i,
            None => {
                self.runtime.runtime_fields.push((field, lookup));
                self.runtime.runtime_fields.len() - 1
            }
        };
        idx as u16
    }

    pub fn get_runtime_method_obj(
        &mut self,
        gc: GCHandle<'gc>,
        method: MethodDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        if let Some(obj) = self
            .runtime
            .runtime_method_objs
            .get(&(method, lookup.clone()))
        {
            return *obj;
        }

        let is_ctor = method.method.name == ".ctor" || method.method.name == ".cctor";
        let class_name = if is_ctor {
            "DotnetRs.ConstructorInfo"
        } else {
            "DotnetRs.MethodInfo"
        };

        let rt = self.runtime.loader.corlib_type(class_name);
        let rt_obj = Object::new(rt, &self.current_context());
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
        self.register_new_object(&obj_ref);

        let index = self.get_runtime_method_index(method, lookup.clone());

        obj_ref.as_object_mut(gc, |instance| {
            instance
                .instance_storage
                .get_field_mut("index")
                .copy_from_slice(&(index as usize).to_ne_bytes());
        });

        self.runtime
            .runtime_method_objs
            .insert((method, lookup), obj_ref);
        obj_ref
    }

    pub fn get_runtime_field_obj(
        &mut self,
        gc: GCHandle<'gc>,
        field: FieldDescription,
        lookup: GenericLookup,
    ) -> ObjectRef<'gc> {
        if let Some(obj) = self
            .runtime
            .runtime_field_objs
            .get(&(field, lookup.clone()))
        {
            return *obj;
        }

        let rt = self.runtime.loader.corlib_type("DotnetRs.FieldInfo");
        let rt_obj = Object::new(rt, &self.current_context());
        let obj_ref = ObjectRef::new(gc, HeapStorage::Obj(rt_obj));
        self.register_new_object(&obj_ref);

        let index = self.get_runtime_field_index(field, lookup.clone());

        obj_ref.as_object_mut(gc, |instance| {
            instance
                .instance_storage
                .get_field_mut("index")
                .copy_from_slice(&(index as usize).to_ne_bytes());
        });

        self.runtime
            .runtime_field_objs
            .insert((field, lookup), obj_ref);
        obj_ref
    }
}

pub fn runtime_type_intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: GenericLookup,
) -> StepResult {
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

    match_method!(method, {
        [DotnetRs.RuntimeType::GetAssembly()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());

            let target_type = stack.resolve_runtime_type(obj);
            let resolution = target_type.resolution(stack.runtime.loader);

            let value = match stack.runtime.runtime_asms.get(&resolution) {
                Some(o) => *o,
                None => {
                    let support_res = stack.runtime.loader.get_assembly(SUPPORT_ASSEMBLY);
                    let definition = support_res.0.type_definitions
                        .iter()
                        .find(|a| a.type_name() == "DotnetRs.Assembly")
                        .expect("could find DotnetRs.Assembly in support library");
                    let mut asm_handle = Object::new(
                        TypeDescription { resolution: support_res, definition },
                        &ResolutionContext::new(&generics, stack.runtime.loader, support_res),
                    );
                    let data = (resolution.as_raw() as usize).to_ne_bytes();
                    asm_handle.instance_storage.get_field_mut("resolution").copy_from_slice(&data);
                    let v = ObjectRef::new(gc, HeapStorage::Obj(asm_handle));
                    stack.register_new_object(&v);
                    stack.runtime.runtime_asms.insert(resolution, v);
                    v
                }
            };
            push!(ObjectRef(value));
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.RuntimeType::GetNamespace()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let target_type = stack.resolve_runtime_type(obj);
            match target_type {
                RuntimeType::Type(td) | RuntimeType::Generic(td, _) => {
                    match td.definition.namespace.as_ref() {
                        None => push!(null()),
                        Some(n) => push!(string(n)),
                    }
                }
                _ => push!(string("System")),
            }
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.RuntimeType::GetName()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let target_type = stack.resolve_runtime_type(obj);
            push!(string(target_type.get_name()));
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.RuntimeType::GetIsGenericType()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let target_type = stack.resolve_runtime_type(obj);
            let is_generic = matches!(target_type, RuntimeType::Generic(_, _));
            push!(Int32(if is_generic { 1 } else { 0 }));
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.RuntimeType::GetGenericTypeDefinition()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let target_type = stack.resolve_runtime_type(obj);
            match target_type {
                RuntimeType::Generic(td, _) => {
                    let n_params = td.definition.generic_parameters.len();
                    let mut params = Vec::with_capacity(n_params);
                    for i in 0..n_params {
                        params.push(RuntimeType::TypeParameter {
                            owner: *td,
                            index: i as u16,
                        });
                    }
                    let def_rt = RuntimeType::Generic(*td, params);
                    let rt_obj = stack.get_runtime_type(gc, def_rt);
                    push!(ObjectRef(rt_obj));
                }
                _ => return stack.throw_by_name(gc, "System.InvalidOperationException")
            }
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.RuntimeType::GetGenericArguments()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let target_type = stack.resolve_runtime_type(obj);
            let args = match target_type {
                RuntimeType::Generic(_, args) => args.clone(),
                _ => vec![]
            };

            let type_type_td = stack.runtime.loader.corlib_type("System.Type");
            let type_type = ConcreteType::from(type_type_td);
            let mut vector = Vector::new(type_type, args.len(), &stack.current_context());
            for (i, arg) in args.into_iter().enumerate() {
                let arg_obj = stack.get_runtime_type(gc, arg);
                arg_obj.write(&mut vector.get_mut()[(i * ObjectRef::SIZE)..]);
            }
            let obj = ObjectRef::new(gc, HeapStorage::Vec(vector));
            stack.register_new_object(&obj);
            push!(ObjectRef(obj));
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.RuntimeType::GetTypeHandle()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());

            let rth = stack.runtime.loader.corlib_type("System.RuntimeTypeHandle");
            let mut instance = Object::new(rth, &stack.current_context());
            obj.write(instance.instance_storage.get_field_mut("_value"));

            push!(ValueType(Box::new(instance)));
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.RuntimeType::MakeGenericType(any)] => {
            vm_expect_stack!(let ObjectRef(parameters) = pop!());
            vm_expect_stack!(let ObjectRef(target) = pop!());
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
                 let new_rt = RuntimeType::Generic(*td, new_generics);

                 let rt_obj = stack.get_runtime_type(gc, new_rt);
                 push!(ObjectRef(rt_obj));
            } else {
                return stack.throw_by_name(gc, "System.InvalidOperationException");
            }
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.RuntimeType::CreateInstanceDefaultCtor(any, any)] => {
            pop!(); // skipCheck
            pop!(); // publicOnly
            vm_expect_stack!(let ObjectRef(target_obj) = pop!());

            let target_rt = stack.resolve_runtime_type(target_obj);

            let (td, type_generics) = match target_rt {
                RuntimeType::Type(td) => (*td, vec![]),
                RuntimeType::Generic(td, args) => (*td, args.clone()),
                _ => panic!("cannot create instance of {:?}", target_rt),
            };

            let type_generics_concrete = type_generics
                .iter()
                .map(|a| a.to_concrete(stack.runtime.loader))
                .collect();
            let new_lookup = GenericLookup::new(type_generics_concrete);
            let new_ctx = stack.current_context().with_generics(&new_lookup);

            let instance = Object::new(td, &new_ctx);

            for m in &td.definition.methods {
                if m.runtime_special_name
                    && m.name == ".ctor"
                    && m.signature.instance
                    && m.signature.parameters.is_empty()
                {
                    let desc = MethodDescription {
                        parent: td,
                        method: m
                    };

                    stack.constructor_frame(
                        gc,
                        instance,
                        MethodInfo::new(desc, &new_lookup, stack.runtime.loader),
                        new_lookup,
                    );
                    return StepResult::InstructionStepped;
                }
            }

            panic!("could not find a parameterless constructor in {:?}", td)
        },
    }).expect("unimplemented runtime type intrinsic");
    StepResult::InstructionStepped
}

pub fn runtime_method_info_intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: GenericLookup,
) -> StepResult {
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

    match_method!(method, {
        [DotnetRs.MethodInfo::GetName()]
        | [DotnetRs.ConstructorInfo::GetName()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let (method, _) = stack.resolve_runtime_method(obj);
            push!(string(&method.method.name));
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.MethodInfo::GetDeclaringType()]
        | [DotnetRs.ConstructorInfo::GetDeclaringType()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let (method, _) = stack.resolve_runtime_method(obj);
            let rt_obj = stack.get_runtime_type(gc, RuntimeType::Type(method.parent));
            push!(ObjectRef(rt_obj));
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.MethodInfo::GetMethodHandle()]
        | [DotnetRs.ConstructorInfo::GetMethodHandle()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());

            let rmh = stack.runtime.loader.corlib_type("System.RuntimeMethodHandle");
            let mut instance = Object::new(rmh, &stack.current_context());
            obj.write(instance.instance_storage.get_field_mut("_value"));

            push!(ValueType(Box::new(instance)));
            Some(StepResult::InstructionStepped)
        },
    })
    .expect("unimplemented method info intrinsic");

    StepResult::InstructionStepped
}

pub fn runtime_field_info_intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    _generics: GenericLookup,
) -> StepResult {
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

    match_method!(method, {
        [DotnetRs.FieldInfo::GetName()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let (field, _) = stack.resolve_runtime_field(obj);
            push!(string(&field.field.name));
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.FieldInfo::GetDeclaringType()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());
            let (field, _) = stack.resolve_runtime_field(obj);
            let rt_obj = stack.get_runtime_type(gc, RuntimeType::Type(field.parent));
            push!(ObjectRef(rt_obj));
            Some(StepResult::InstructionStepped)
        },
        [DotnetRs.FieldInfo::GetFieldHandle()] => {
            vm_expect_stack!(let ObjectRef(obj) = pop!());

            let rfh = stack.runtime.loader.corlib_type("System.RuntimeFieldHandle");
            let mut instance = Object::new(rfh, &stack.current_context());
            obj.write(instance.instance_storage.get_field_mut("_value"));

            push!(ValueType(Box::new(instance)));
            Some(StepResult::InstructionStepped)
        },
    })
    .expect("unimplemented field info intrinsic");

    StepResult::InstructionStepped
}
