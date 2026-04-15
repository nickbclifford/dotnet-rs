use gc_arena::static_collect;
use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    ops::Deref,
    sync::{Arc, LazyLock, Mutex},
};

#[macro_export]
macro_rules! with_string {
    ($stack:expr, $value:expr, |$s:ident| $code:expr) => {{
        let value = $value;
        let obj = value.as_object_ref();
        if let Some(handle) = obj.0 {
            let _gc_scope = $crate::GcScopeGuard::enter(
                $stack.as_borrow_scope(),
                $stack.as_borrow_scope().gc_ready_token(),
            );
            let heap = handle.borrow();
            if let $crate::object::HeapStorage::Str(ref $s) = heap.storage {
                $code
            } else {
                panic!(
                    "invalid type on stack, expected string, received {:?}",
                    heap.storage
                )
            }
        } else {
            return $stack.throw_by_name_with_message(
                "System.NullReferenceException",
                "Object reference not set to an instance of an object.",
            );
        }
    }};
}

#[macro_export]
macro_rules! with_string_mut {
    ($stack:expr, $value:expr, |$s:ident| $code:expr) => {{
        let value = $value;
        let obj = value.as_object_ref();
        if let Some(handle) = obj.0 {
            let gc = $stack.gc_with_token(&$stack.no_active_borrows_token());
            let _gc_scope = $crate::GcScopeGuard::enter(
                $stack.as_borrow_scope(),
                $stack.as_borrow_scope().gc_ready_token(),
            );
            let mut heap = handle.borrow_mut(&gc);
            if let $crate::object::HeapStorage::Str(ref mut $s) = heap.storage {
                $code
            } else {
                panic!(
                    "invalid type on stack, expected string, received {:?}",
                    heap.storage
                )
            }
        } else {
            return $stack.throw_by_name_with_message(
                "System.NullReferenceException",
                "Object reference not set to an instance of an object.",
            );
        }
    }};
}

#[derive(Clone)]
enum StringStorage {
    Owned(Vec<u16>),
    Interned(Arc<[u16]>),
}

#[derive(Clone)]
pub struct CLRString(StringStorage);
static_collect!(CLRString);

struct InternConfig {
    enabled: bool,
    max_entries: usize,
}

const STRING_INTERN_DEFAULT_MAX_ENTRIES: usize = 4096;

type StringInternerMap = HashMap<Arc<[u16]>, Arc<[u16]>>;

static INTERN_CONFIG: LazyLock<InternConfig> = LazyLock::new(|| {
    let enabled = match std::env::var("DOTNET_STRING_INTERN_EXPERIMENT") {
        Ok(raw) => {
            let trimmed = raw.trim();
            trimmed.eq_ignore_ascii_case("1")
                || trimmed.eq_ignore_ascii_case("true")
                || trimmed.eq_ignore_ascii_case("yes")
                || trimmed.eq_ignore_ascii_case("on")
        }
        Err(_) => false,
    };

    let max_entries = match std::env::var("DOTNET_STRING_INTERN_MAX_ENTRIES") {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(v) if v > 0 => v,
            _ => STRING_INTERN_DEFAULT_MAX_ENTRIES,
        },
        Err(_) => STRING_INTERN_DEFAULT_MAX_ENTRIES,
    };

    InternConfig {
        enabled,
        max_entries,
    }
});

static STRING_INTERNER: LazyLock<Mutex<StringInternerMap>> =
    LazyLock::new(|| Mutex::new(StringInternerMap::new()));

fn maybe_intern(chars: Vec<u16>) -> StringStorage {
    if !INTERN_CONFIG.enabled {
        return StringStorage::Owned(chars);
    }

    let mut interner = STRING_INTERNER
        .lock()
        .expect("string interner lock poisoned");
    if let Some(existing) = interner.get(chars.as_slice()) {
        return StringStorage::Interned(Arc::clone(existing));
    }

    if interner.len() >= INTERN_CONFIG.max_entries {
        interner.clear();
    }

    let interned = Arc::<[u16]>::from(chars.into_boxed_slice());
    interner.insert(Arc::clone(&interned), Arc::clone(&interned));
    StringStorage::Interned(interned)
}

impl CLRString {
    pub fn new(chars: Vec<u16>) -> Self {
        Self(maybe_intern(chars))
    }

    pub fn len(&self) -> usize {
        self.deref().len()
    }

    pub fn is_empty(&self) -> bool {
        self.deref().is_empty()
    }

    pub fn size_bytes(&self) -> usize {
        size_of::<CLRString>() + self.len() * 2
    }

    pub fn as_string(&self) -> String {
        String::from_utf16(self.deref()).unwrap()
    }

    pub fn as_mut_slice(&mut self) -> &mut [u16] {
        match &mut self.0 {
            StringStorage::Owned(chars) => chars.as_mut_slice(),
            StringStorage::Interned(chars) => Arc::make_mut(chars),
        }
    }
}

impl Deref for CLRString {
    type Target = [u16];

    fn deref(&self) -> &Self::Target {
        match &self.0 {
            StringStorage::Owned(chars) => chars.as_slice(),
            StringStorage::Interned(chars) => chars.as_ref(),
        }
    }
}

impl PartialEq for CLRString {
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl Debug for CLRString {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_string())
    }
}

impl<T: AsRef<str>> From<T> for CLRString {
    fn from(s: T) -> Self {
        Self::new(s.as_ref().encode_utf16().collect())
    }
}
