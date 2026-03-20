use crate::{TypeDescription, generics::GenericLookup, resolution::ResolutionS};
use dotnetdll::prelude::{Field, Method, MethodMemberIndex, ResolvedDebug};
use gc_arena::static_collect;
use std::{
    fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
};

#[derive(Clone)]
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
            self.method().signature.show_with_name(
                self.resolution().definition(),
                format!("{}::{}", self.parent.type_name(), self.method().name)
            )
        )
    }
}

impl PartialEq for MethodDescription {
    fn eq(&self, other: &Self) -> bool {
        self.parent == other.parent
            && self.method_member_index == other.method_member_index
            && self.method_resolution == other.method_resolution
            && self.parent_generics == other.parent_generics
    }
}

impl Eq for MethodDescription {}

impl Hash for MethodDescription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parent.hash(state);
        self.method_member_index.hash(state);
        self.method_resolution.hash(state);
        self.parent_generics.hash(state);
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
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
        mem::size_of,
    };

    /// Construct a `TypeIndex` from raw bytes.  `TypeIndex` has no public
    /// constructor so we replicate the transmute trick used in `generics.rs`
    /// tests and `comparer.rs` helpers.
    fn make_type_index(byte: u8) -> TypeIndex {
        // SAFETY: TypeIndex is a plain integer wrapper; any bit pattern is valid.
        unsafe {
            std::mem::transmute::<[u8; size_of::<TypeIndex>()], TypeIndex>(
                [byte; size_of::<TypeIndex>()],
            )
        }
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
    fn method_descriptions_with_different_parents_falsely_compare_equal() {
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

    /// The same two descriptors must also hash differently, ensuring
    /// `HashMap<MethodDescription, _>` and `HashSet<MethodDescription>`
    /// cannot alias entries from different declaring types.
    #[test]
    fn method_descriptions_with_different_parents_produce_identical_hashes() {
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
            hash_of(&method_a),
            hash_of(&method_b),
            "MethodDescription::hash must include `parent`; \
             methods from different declaring types must produce different hashes"
        );
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
