use crate::{TypeDescription, generics::GenericLookup, resolution::ResolutionS};
use dotnetdll::prelude::{
    Field, ManagedMethod, Method, MethodIndex, MethodMemberIndex, MethodType, Resolution,
    ResolvedDebug, TypeIndex, body,
};
use gc_arena::static_collect;
use std::{
    fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MethodDescription {
    pub parent: TypeDescription,
    pub parent_generics: GenericLookup,
    pub method_resolution: ResolutionS,
    pub method_member_index: MethodMemberIndex,
}

static_collect!(MethodDescription);

impl MethodDescription {
    pub fn new(
        parent: TypeDescription,
        parent_generics: GenericLookup,
        method_resolution: ResolutionS,
        method_member_index: MethodMemberIndex,
    ) -> Self {
        Self {
            parent,
            parent_generics,
            method_resolution,
            method_member_index,
        }
    }

    /// Declaring-type generic argument of this callee, bounded by its declared arity.
    /// Surplus call-site lookup entries do not belong to the callee.
    pub fn own_type_arg<'a>(
        &self,
        generics: &'a GenericLookup,
        index: usize,
    ) -> Result<&'a crate::generics::ConcreteType, crate::error::TypeResolutionError> {
        let arity = self.parent.definition().generic_parameters.len();
        if index >= arity {
            return Err(crate::error::TypeResolutionError::GenericIndexOutOfBounds {
                index,
                length: arity,
            });
        }
        generics.type_arg(index)
    }

    /// Owned variant of [`Self::own_type_arg`].
    pub fn cloned_own_type_arg(
        &self,
        generics: &GenericLookup,
        index: usize,
    ) -> Result<crate::generics::ConcreteType, crate::error::TypeResolutionError> {
        self.own_type_arg(generics, index).cloned()
    }

    /// Method generic argument `index` of this callee, read from the call-site `generics`.
    ///
    /// The index is validated against the callee's own declared generic arity, so a callee that
    /// is not generic (or has fewer parameters) never observes arguments left over in the lookup
    /// from an enclosing generic method. Intrinsics that size raw memory operations from a
    /// generic argument must go through this accessor.
    pub fn own_method_arg<'a>(
        &self,
        generics: &'a GenericLookup,
        index: usize,
    ) -> Result<&'a crate::generics::ConcreteType, crate::error::TypeResolutionError> {
        let arity = self.method().generic_parameters.len();
        if index >= arity {
            return Err(crate::error::TypeResolutionError::GenericIndexOutOfBounds {
                index,
                length: arity,
            });
        }
        generics.method_arg(index)
    }

    /// Owned variant of [`Self::own_method_arg`].
    pub fn cloned_own_method_arg(
        &self,
        generics: &GenericLookup,
        index: usize,
    ) -> Result<crate::generics::ConcreteType, crate::error::TypeResolutionError> {
        self.own_method_arg(generics, index).cloned()
    }

    pub fn method(&self) -> &'static Method<'static> {
        let def = self.parent.definition();
        match self.method_member_index {
            MethodMemberIndex::Method(i) => &def.methods[i],
            MethodMemberIndex::PropertyGetter(i) => def.properties[i]
                .getter
                .as_ref()
                .expect("PropertyGetter index has no getter"),
            MethodMemberIndex::PropertySetter(i) => def.properties[i]
                .setter
                .as_ref()
                .expect("PropertySetter index has no setter"),
            MethodMemberIndex::PropertyOther { property, other } => {
                &def.properties[property].other[other]
            }
            MethodMemberIndex::EventAdd(i) => &def.events[i].add_listener,
            MethodMemberIndex::EventRemove(i) => &def.events[i].remove_listener,
            MethodMemberIndex::EventRaise(i) => def.events[i]
                .raise_event
                .as_ref()
                .expect("EventRaise index has no raise_event"),
            MethodMemberIndex::EventOther { event, other } => &def.events[event].other[other],
        }
    }

    pub fn resolution(&self) -> ResolutionS {
        self.method_resolution.clone()
    }

    /// Maps a `(TypeIndex, MethodMemberIndex)` pair to the corresponding [`MethodIndex`].
    ///
    /// `MethodIndex`'s fields are `pub(crate)` in `dotnetdll`, so it cannot be built directly.
    /// We rebuild it from `(TypeIndex, MethodMemberIndex)` using only public `Resolution`
    /// constructors. The indices originate from a valid resolution, so every lookup is expected
    /// to succeed.
    ///
    /// Exposed as an associated function so callers that already have a `&Resolution` and
    /// `TypeIndex` can decode a method signature without constructing a throwaway
    /// `MethodDescription` first.
    pub fn index_for(
        res: &'static Resolution<'static>,
        parent: TypeIndex,
        member: MethodMemberIndex,
    ) -> MethodIndex {
        match member {
            MethodMemberIndex::Method(i) => res
                .method_index(parent, i)
                .expect("method index out of range"),
            MethodMemberIndex::PropertyGetter(i) => {
                let p = res
                    .property_index(parent, i)
                    .expect("property index out of range");
                res.property_getter_index(p)
                    .expect("property has no getter")
            }
            MethodMemberIndex::PropertySetter(i) => {
                let p = res
                    .property_index(parent, i)
                    .expect("property index out of range");
                res.property_setter_index(p)
                    .expect("property has no setter")
            }
            MethodMemberIndex::PropertyOther { property, other } => {
                let p = res
                    .property_index(parent, property)
                    .expect("property index out of range");
                res.property_other_index(p, other)
                    .expect("property other index out of range")
            }
            MethodMemberIndex::EventAdd(i) => {
                let e = res
                    .event_index(parent, i)
                    .expect("event index out of range");
                res.event_add_index(e)
            }
            MethodMemberIndex::EventRemove(i) => {
                let e = res
                    .event_index(parent, i)
                    .expect("event index out of range");
                res.event_remove_index(e)
            }
            MethodMemberIndex::EventRaise(i) => {
                let e = res
                    .event_index(parent, i)
                    .expect("event index out of range");
                res.event_raise_index(e).expect("event has no raise method")
            }
            MethodMemberIndex::EventOther { event, other } => {
                let e = res
                    .event_index(parent, event)
                    .expect("event index out of range");
                res.event_other_index(e, other)
                    .expect("event other index out of range")
            }
        }
    }

    /// Reconstructs the `dotnetdll` [`MethodIndex`] for this method.
    pub fn method_index(&self) -> MethodIndex {
        Self::index_for(
            self.method_resolution.definition(),
            self.parent.index,
            self.method_member_index,
        )
    }

    /// Returns the decoded IL body for this method, or `None` if it has none.
    ///
    /// Delegates to [`Resolution::method_body`](Resolution::method_body), which
    /// works in both eager and lazy parse modes (in lazy mode the body is decoded and cached on
    /// first access). `None` covers abstract methods, methods with no IL (`rva == 0`), and — in
    /// lazy mode — a deferred IL decode error. Prefer this over `self.method().body`, which is
    /// always `None` when the owning resolution was parsed with `lazy_method_bodies`.
    pub fn body(&self) -> Option<&'static body::Method> {
        self.method_resolution
            .definition()
            .method_body(self.method_index())
            .ok()
    }

    /// Returns the decoded signature for this method.
    ///
    /// Delegates to [`Resolution::method_signature`](Resolution::method_signature),
    /// which works in both eager and lazy parse modes (in lazy mode the signature is decoded and
    /// cached on first access). Prefer this over `self.signature()`, which returns a
    /// placeholder when the owning resolution was parsed with `lazy_method_signatures`.
    pub fn signature(&self) -> &'static ManagedMethod<MethodType> {
        self.method_resolution
            .definition()
            .method_signature(self.method_index())
            .expect("failed to decode method signature")
    }
}

impl Debug for MethodDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.resolution().is_null() {
            return write!(
                f,
                "{}::{} (No Resolution)",
                self.parent.type_name(),
                self.method().name
            );
        }
        write!(
            f,
            "{}",
            self.signature().show_with_name(
                self.resolution().definition(),
                format!("{}::{}", self.parent.type_name(), self.method().name)
            )
        )
    }
}

#[derive(Clone)]
pub struct FieldDescription {
    pub parent: TypeDescription,
    pub field_resolution: ResolutionS,
    pub index: usize,
}

static_collect!(FieldDescription);

impl FieldDescription {
    pub const fn new(parent: TypeDescription, field_resolution: ResolutionS, index: usize) -> Self {
        Self {
            parent,
            field_resolution,
            index,
        }
    }

    pub fn field(&self) -> &'static Field<'static> {
        &self.parent.definition().fields[self.index]
    }

    pub fn resolution(&self) -> ResolutionS {
        self.field_resolution.clone()
    }
}

impl Debug for FieldDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.field().static_member {
            write!(f, "static ")?;
        }

        write!(
            f,
            "{} {}::{}",
            self.field()
                .return_type
                .show(self.resolution().definition()),
            self.parent.type_name(),
            self.field().name
        )?;

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod method_description_eq_hash_tests {
    //! Regression tests for the `MethodDescription` equality/hash collision risk
    //! in descriptor identity.
    //!
    //! Root cause: `PartialEq` and `Hash` for `MethodDescription` both exclude
    //! `parent` (the declaring type).  Two descriptors from *different* types that
    //! share the same `method_index`, `method_resolution`, and `parent_generics`
    //! will therefore compare equal and produce identical hashes, causing silent
    //! false cache-hits in reflection registries and VMT caches.
    use super::*;
    use crate::{TypeDescription, generics::GenericLookup, resolution::ResolutionS};
    use dotnetdll::prelude::{MethodMemberIndex, TypeIndex};
    use std::{
        collections::{HashMap, hash_map::DefaultHasher},
        hash::{Hash, Hasher},
        mem::size_of,
    };

    /// Construct a deterministic `TypeIndex` for test data through the crate's
    /// centralized compatibility helper. Repeating the byte preserves the
    /// historical test values while avoiding a second layout transmute.
    fn make_type_index(byte: u8) -> TypeIndex {
        crate::type_index_from_usize(usize::from_ne_bytes([byte; size_of::<usize>()]))
    }

    fn hash_of(desc: &MethodDescription) -> u64 {
        let mut h = DefaultHasher::new();
        desc.hash(&mut h);
        h.finish()
    }

    /// Two `TypeDescription` values with *different* `TypeIndex` bytes must not
    /// compare equal — this is the precondition that makes the collision test
    /// meaningful.
    #[test]
    fn type_descriptions_with_different_indices_are_not_equal() {
        let parent_a = TypeDescription::new(ResolutionS::NULL, make_type_index(0));
        let parent_b = TypeDescription::new(ResolutionS::NULL, make_type_index(1));
        assert_ne!(
            parent_a, parent_b,
            "precondition: distinct TypeIndex values must produce distinct TypeDescriptions"
        );
    }

    /// Two `MethodDescription` values whose `parent` fields refer to different
    /// declaring types must not compare equal.
    #[test]
    fn method_descriptions_with_different_parents_are_not_equal() {
        let parent_a = TypeDescription::new(ResolutionS::NULL, make_type_index(0));
        let parent_b = TypeDescription::new(ResolutionS::NULL, make_type_index(1));
        assert_ne!(parent_a, parent_b, "precondition: parents must differ");

        let method_a = MethodDescription::new(
            parent_a,
            GenericLookup::default(),
            ResolutionS::NULL,
            MethodMemberIndex::Method(0),
        );
        let method_b = MethodDescription::new(
            parent_b,
            GenericLookup::default(),
            ResolutionS::NULL,
            MethodMemberIndex::Method(0),
        );

        assert_ne!(
            method_a, method_b,
            "MethodDescription::eq must include `parent`; \
             methods from different types must not compare equal"
        );
    }

    #[derive(Default)]
    struct CollidingHasher;

    impl Hasher for CollidingHasher {
        fn finish(&self) -> u64 {
            0
        }

        fn write(&mut self, _bytes: &[u8]) {}
    }

    /// Unequal keys remain independently retrievable even when their hashes collide.
    #[allow(
        clippy::mutable_key_type,
        reason = "MethodDescription hashing and equality use descriptor identity fields; the metadata arena's mutex is not inspected"
    )]
    #[test]
    fn method_descriptions_with_different_parents_remain_distinct_on_hash_collision() {
        let parent_a = TypeDescription::new(ResolutionS::NULL, make_type_index(0));
        let parent_b = TypeDescription::new(ResolutionS::NULL, make_type_index(1));
        assert_ne!(parent_a, parent_b, "precondition: parents must differ");

        let method_a = MethodDescription::new(
            parent_a,
            GenericLookup::default(),
            ResolutionS::NULL,
            MethodMemberIndex::Method(0),
        );
        let method_b = MethodDescription::new(
            parent_b,
            GenericLookup::default(),
            ResolutionS::NULL,
            MethodMemberIndex::Method(0),
        );

        let mut methods =
            HashMap::with_hasher(std::hash::BuildHasherDefault::<CollidingHasher>::default());
        methods.insert(method_a.clone(), "first");
        methods.insert(method_b.clone(), "second");

        assert_eq!(methods.len(), 2);
        assert_eq!(methods.get(&method_a), Some(&"first"));
        assert_eq!(methods.get(&method_b), Some(&"second"));
    }

    #[test]
    fn equal_method_descriptions_hash_equally() {
        let method = MethodDescription::new(
            TypeDescription::new(ResolutionS::NULL, make_type_index(3)),
            GenericLookup::default(),
            ResolutionS::NULL,
            MethodMemberIndex::Method(2),
        );
        let same = method.clone();
        assert_eq!(method, same);
        assert_eq!(hash_of(&method), hash_of(&same));
    }
}

impl PartialEq for FieldDescription {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.parent == other.parent
    }
}

impl Eq for FieldDescription {}

impl Hash for FieldDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.parent.hash(state);
    }
}

#[cfg(test)]
mod generic_slot_tests {
    use super::*;
    use crate::{error::TypeResolutionError, generics::ConcreteType, resolution::MetadataArena};
    use dotnetdll::prelude::{
        Accessibility, BaseType, CallingConvention, MethodSignature, Module, ReturnType,
        TypeDefinition,
    };
    use dotnetdll::resolved::generic::Generic;
    use std::sync::Arc;

    fn method(type_arity: usize, method_arity: usize) -> MethodDescription {
        let mut resolution = Resolution::new(Module::new("slots.dll"));
        let mut ty = TypeDefinition::new(None, "Slots");
        for _ in 0..type_arity {
            ty.generic_parameters.push(Generic::new("T"));
        }
        let index = resolution.push_type_definition(ty);
        let signature = MethodSignature {
            instance: false,
            explicit_this: false,
            calling_convention: CallingConvention::Default,
            parameters: vec![],
            return_type: ReturnType(vec![], None),
            varargs: None,
        };
        let mut method = Method::new(Accessibility::Public, signature, "Run", None);
        for _ in 0..method_arity {
            method.generic_parameters.push(Generic::new("M"));
        }
        resolution.push_method(index, method);
        let ptr = Box::into_raw(Box::new(resolution));
        let arena = Arc::new(MetadataArena::new());
        // SAFETY: F10.RawAllocationOwnership — ptr was created by Box::into_raw and the arena owns it.
        unsafe { arena.add_resolution(ptr) };
        let res = ResolutionS::new(ptr, arena);
        MethodDescription::new(
            TypeDescription::new(res.clone(), index),
            GenericLookup::default(),
            res,
            MethodMemberIndex::Method(0),
        )
    }

    #[test]
    fn generic_slots_reject_surplus_and_missing_arguments() {
        let empty = method(0, 0);
        let one = method(1, 1);
        let int = ConcreteType::new(one.resolution(), BaseType::Int32);
        let lookup = GenericLookup::from_arcs(
            vec![int.clone(), int.clone()].into(),
            vec![int.clone(), int.clone()].into(),
        );
        assert!(matches!(
            empty.own_type_arg(&lookup, 0),
            Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: 0,
                length: 0
            })
        ));
        assert!(matches!(
            empty.own_method_arg(&lookup, 0),
            Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: 0,
                length: 0
            })
        ));
        assert_eq!(one.cloned_own_type_arg(&lookup, 0).unwrap(), int);
        assert_eq!(one.cloned_own_method_arg(&lookup, 0).unwrap(), int);
        assert!(matches!(
            one.own_type_arg(&lookup, 1),
            Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: 1,
                length: 1
            })
        ));
        assert!(matches!(
            one.own_method_arg(&lookup, 1),
            Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: 1,
                length: 1
            })
        ));
        let missing = GenericLookup::default();
        assert!(matches!(
            one.own_type_arg(&missing, 0),
            Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: 0,
                length: 0
            })
        ));
        assert!(matches!(
            one.own_method_arg(&missing, 0),
            Err(TypeResolutionError::GenericIndexOutOfBounds {
                index: 0,
                length: 0
            })
        ));
    }
}
