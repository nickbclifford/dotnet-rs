use crate::{
    utils::decompose_type_source,
    value::{
        string::CLRString, ConcreteType, Context, GenericLookup, HeapStorage, MethodDescription,
        Object, ObjectRef, StackValue, TypeDescription,
    },
    vm::{intrinsics::expect_stack, CallStack, GCHandle},
};
use dotnetdll::prelude::{BaseType, MethodType, ResolvedDebug, TypeSource};
use std::{
    fmt::Debug,
    hash::{Hash, Hasher},
};

fn is_generic(t: &MethodType) -> bool {
    match t {
        MethodType::Base(b) => match &**b {
            BaseType::Type { source, .. } => matches!(source, TypeSource::Generic { .. }),
            BaseType::Array(t, ..)
            | BaseType::Vector(_, t)
            | BaseType::ValuePointer(_, Some(t)) => is_generic(t),
            _ => false,
        },
        _ => true,
    }
}

#[derive(Clone)]
pub struct RuntimeType {
    pub target: MethodType,
    pub source: MethodDescription,
    pub generics: GenericLookup,
}
impl PartialEq for RuntimeType {
    fn eq(&self, other: &Self) -> bool {
        if is_generic(&self.target) || is_generic(&other.target) {
            self.target == other.target
                && self.source == other.source
                && self.generics == other.generics
        } else {
            self.target == other.target
        }
    }
}
impl Eq for RuntimeType {}
impl Hash for RuntimeType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if is_generic(&self.target) {
            self.source.hash(state);
            self.generics.hash(state);
        }
        self.target.hash(state);
    }
}
impl Debug for RuntimeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if is_generic(&self.target) {
            write!(f, "{:?}{:?}", self.target, self.generics)
        } else {
            write!(f, "{}", self.target.show(self.source.resolution().0))
        }
    }
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

pub fn runtime_type_intrinsic_call<'gc, 'm: 'gc>(
    gc: GCHandle<'gc>,
    stack: &mut CallStack<'gc, 'm>,
    method: MethodDescription,
    generics: GenericLookup,
) {
    macro_rules! pop {
        () => {
            stack.pop_stack()
        };
    }
    macro_rules! push {
        ($value:expr) => {
            stack.push_stack(gc, $value)
        };
    }

    // TODO: real signature checking
    match format!("{:?}", method).as_str() {
        "DotnetRs.Assembly DotnetRs.RuntimeType::GetAssembly()" => {
            expect_stack!(let ObjectRef(obj) = pop!());

            let target_type: RuntimeType = obj.try_into().unwrap();
            // https://learn.microsoft.com/en-us/dotnet/api/system.type.assembly?view=net-9.0#system-type-assembly
            // "If the current Type object represents a constructed generic type, this property returns
            //  the assembly that contains the generic type definition. For example, suppose you create
            //  an assembly named MyGenerics.dll that contains the generic type definition MyGenericStack<T>.
            //  If you create an instance of MyGenericStack<int> in another assembly, the Assembly property
            //  for the constructed type returns an Assembly object that represents MyGenerics.dll."
            // "Similarly, if the current Type object represents an unassigned generic parameter T,
            //  this property returns the assembly that contains the generic type that defines T."
            match &target_type.target {
                MethodType::Base(t) => {
                    let description = if let BaseType::Type { source, .. } = &**t {
                        let (ut, _) = decompose_type_source(source);
                        stack.assemblies.locate_type(target_type.source.resolution(), ut)
                    } else {
                        stack.assemblies.find_concrete_type(target_type.into())
                    };
                    let value = match stack.runtime_asms.get(&description.resolution) {
                        Some(o) => *o,
                        None => {
                            let resolution = obj.as_object(|i| i.description.resolution);
                            let definition = resolution.0.type_definitions
                                .iter()
                                .find(|a| a.type_name() == "DotnetRs.Assembly")
                                .unwrap();
                            let mut asm_handle = Object::new(TypeDescription { resolution, definition }, Context {
                                generics: &generics,
                                assemblies: stack.assemblies,
                                resolution,
                            });
                            let data = (description.resolution.as_raw() as usize).to_ne_bytes();
                            asm_handle.instance_storage.get_field_mut("resolution").copy_from_slice(&data);
                            let v = ObjectRef::new(gc, HeapStorage::Obj(asm_handle));
                            stack.runtime_asms.insert(description.resolution, v);
                            v
                        }
                    };
                    push!(StackValue::ObjectRef(value));
                },
                _ => todo!("get assembly for {target_type:?}")
            }
        }
        "string DotnetRs.RuntimeType::GetNamespace()" => {
            expect_stack!(let ObjectRef(obj) = pop!());
            let target_type: RuntimeType = obj.try_into().unwrap();
            // https://learn.microsoft.com/en-us/dotnet/api/system.type.namespace?view=net-9.0
            // "If the current Type represents a constructed generic type, this property returns
            //  the namespace that contains the generic type definition. Similarly, if the current Type
            //  represents a generic parameter T, this property returns the namespace that contains
            //  the generic type definition that defines T."
            // "If the current Type object represents a generic parameter and a generic type definition
            //  is not available, such as for a signature type returned by MakeGenericMethodParameter,
            // this property returns null."
            match &target_type.target {
                MethodType::Base(t) => {
                    let description = if let BaseType::Type { source, .. } = &**t {
                        let (ut, _) = decompose_type_source(source);
                        stack.assemblies.locate_type(target_type.source.resolution(), ut)
                    } else {
                        stack.assemblies.find_concrete_type(target_type.into())
                    };
                    match description.definition.namespace.as_ref() {
                        None => push!(StackValue::null()),
                        Some(n) => push!(StackValue::string(gc, CLRString::from(n)))
                    }
                }
                _ => todo!("get namespace for {target_type:?}")
            }
        }
        "string DotnetRs.RuntimeType::GetName()" => {
            // just the regular name, not the fully qualified name
            // https://learn.microsoft.com/en-us/dotnet/api/system.reflection.memberinfo.name?view=net-9.0#remarks
            expect_stack!(let ObjectRef(obj) = pop!());
            let target: RuntimeType = obj.try_into().unwrap();
            let target: ConcreteType = target.into();
            let target = stack.assemblies.find_concrete_type(target);
            push!(StackValue::string(gc, CLRString::from(target.definition.name.as_ref())));
        }
        "valuetype [System.Runtime]System.RuntimeTypeHandle DotnetRs.RuntimeType::GetTypeHandle()" => {
            expect_stack!(let ObjectRef(obj) = pop!());

            let rth = stack.assemblies.corlib_type("System.RuntimeTypeHandle");
            let mut instance = Object::new(rth, Context::with_generics(stack.current_context(), &generics));
            obj.write(instance.instance_storage.get_field_mut("_value"));

            push!(StackValue::ValueType(Box::new(instance)));
        }
        "[System.Runtime]System.Type DotnetRs.RuntimeType::MakeGenericType([System.Runtime]System.Type[])" => {
            expect_stack!(let ObjectRef(parameters) = pop!());
            expect_stack!(let ObjectRef(target) = pop!());
            let rt: RuntimeType = target.try_into().unwrap();
            let rt: ConcreteType = rt.into();
            match rt.get() {
                BaseType::Type { source, .. } => {
                    let (ut, _) = decompose_type_source(source);
                    let name = ut.type_name(rt.resolution().0);
                    let fragments: Vec<_> = name.split('`').collect();
                    if fragments.len() <= 1 {
                        todo!("ArgumentException: type is not generic")
                    }
                    let n_params: usize = fragments[1].parse().unwrap();
                    let mut params: Vec<RuntimeType> = Vec::with_capacity(n_params);
                    for i in 0..n_params {
                        parameters.as_vector(|v| {
                            let start = i * ObjectRef::SIZE;
                            let param = ObjectRef::read(&v.get()[start..]);
                            params.push(param.try_into().unwrap());
                        });
                    }
                    todo!("make RuntimeType for {name} with generics {params:?}");
                }
                _ => todo!("ArgumentException: cannot make generic type from {:?}", rt),
            }
        }
        x => panic!("unsupported intrinsic call to {}", x),
    }
    stack.increment_ip();
}
