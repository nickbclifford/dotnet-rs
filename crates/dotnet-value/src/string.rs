use dotnet_utils::sync::Mutex;
use gc_arena::static_collect;
use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    ops::Deref,
    sync::{Arc, LazyLock},
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
macro_rules! with_vector {
    ($stack:expr, $value:expr, |$v:ident| $code:expr) => {{
        let value = $value;
        let obj = value.as_object_ref();
        if let Some(handle) = obj.0 {
            let _gc_scope = $crate::GcScopeGuard::enter(
                $stack.as_borrow_scope(),
                $stack.as_borrow_scope().gc_ready_token(),
            );
            let heap = handle.borrow();
            if let $crate::object::HeapStorage::Vec(ref $v) = heap.storage {
                $code
            } else {
                panic!(
                    "invalid type on stack, expected vector, received {:?}",
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
            let mut heap = $crate::ThreadSafeLock::borrow_mut(handle, &gc);
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

/// UTF-16 characters followed by one NUL terminator, which is not part of the logical string.
///
/// CoreCLR strings are NUL-terminated and CoreLib relies on it when it reads a pinned `char*`:
/// `String.GetNonRandomizedHashCode` reads characters in pairs, so for an odd length its last read
/// covers the terminator. Without one, that read left the allocation. Every logical view (`len`,
/// `Deref`, `as_mut_slice`, managed payload bounds) excludes the terminator.
#[derive(Clone)]
enum StringStorage {
    Owned(Vec<u16>),
    Interned(Arc<[u16]>),
}

impl StringStorage {
    /// The stored characters including the terminator.
    fn terminated(&self) -> &[u16] {
        match self {
            StringStorage::Owned(chars) => chars.as_slice(),
            StringStorage::Interned(chars) => chars.as_ref(),
        }
    }
}

#[derive(Clone)]
pub struct CLRString(StringStorage);
static_collect!(CLRString);

struct InternConfig {
    enabled: bool,
    max_entries: usize,
}

const STRING_INTERN_DEFAULT_MAX_ENTRIES: usize = 4096;

type StringInternerCache = HashMap<Arc<[u16]>, Arc<[u16]>>;

pub fn parse_env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        Err(_) => default,
    }
}

static INTERN_CONFIG: LazyLock<InternConfig> = LazyLock::new(|| {
    let enabled = parse_env_bool("DOTNET_STRING_INTERN_EXPERIMENT", true);

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

// The single-threaded synchronization facade is RefCell-backed and deliberately !Sync, so its
// cache must be thread-local. Multithreaded and loom builds retain one shared interner.
#[cfg(any(dotnet_value_multithreading, loom))]
static STRING_INTERNER: LazyLock<Mutex<StringInternerCache>> =
    LazyLock::new(|| Mutex::new(StringInternerCache::new()));

#[cfg(all(not(dotnet_value_multithreading), not(loom)))]
thread_local! {
    static STRING_INTERNER: Mutex<StringInternerCache> = Mutex::new(StringInternerCache::new());
}

fn intern_chars(chars: Vec<u16>, interner: &mut StringInternerCache) -> StringStorage {
    if let Some(existing) = interner.get(chars.as_slice()) {
        return StringStorage::Interned(Arc::clone(existing));
    }

    // The interner is an optimization only. Clearing it wholesale keeps it bounded under
    // high-cardinality workloads without a per-entry eviction path.
    if interner.len() >= INTERN_CONFIG.max_entries {
        interner.clear();
    }

    let interned = Arc::<[u16]>::from(chars.into_boxed_slice());
    interner.insert(Arc::clone(&interned), Arc::clone(&interned));
    StringStorage::Interned(interned)
}

fn maybe_intern(chars: Vec<u16>) -> StringStorage {
    if !INTERN_CONFIG.enabled {
        return StringStorage::Owned(chars);
    }

    #[cfg(any(dotnet_value_multithreading, loom))]
    {
        intern_chars(chars, &mut STRING_INTERNER.lock())
    }

    #[cfg(all(not(dotnet_value_multithreading), not(loom)))]
    {
        STRING_INTERNER.with(|interner| intern_chars(chars, &mut interner.lock()))
    }
}

impl CLRString {
    pub fn new(mut chars: Vec<u16>) -> Self {
        chars.push(0);
        Self(maybe_intern(chars))
    }

    pub fn len(&self) -> usize {
        self.0.terminated().len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.deref().is_empty()
    }

    pub fn size_bytes(&self) -> usize {
        size_of::<CLRString>() + self.len() * 2
    }

    pub fn as_string(&self) -> String {
        // Managed strings are not guaranteed valid UTF-16 (e.g. unpaired surrogates from
        // raw paths or untrusted input), so lossily replace rather than panic the VM.
        String::from_utf16_lossy(self.deref())
    }

    pub fn as_mut_slice(&mut self) -> &mut [u16] {
        let len = self.len();
        let chars = match &mut self.0 {
            StringStorage::Owned(chars) => chars.as_mut_slice(),
            StringStorage::Interned(chars) => Arc::make_mut(chars),
        };
        &mut chars[..len]
    }

    /// Returns the raw base of the UTF-16 backing allocation without creating a slice.
    /// The allocation holds `len()` characters followed by a NUL terminator.
    /// The address does not retain this string's owner or any borrow/lock, root the
    /// allocation, or grant write access; interned storage may be shared.
    pub(crate) fn raw_data_ptr(&self) -> *const u8 {
        match &self.0 {
            StringStorage::Owned(chars) => chars.as_ptr().cast(),
            StringStorage::Interned(chars) => Arc::as_ptr(chars).cast(),
        }
    }
}

impl Deref for CLRString {
    type Target = [u16];

    fn deref(&self) -> &Self::Target {
        let chars = self.0.terminated();
        &chars[..chars.len() - 1]
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

#[cfg(test)]
mod tests {
    use super::CLRString;

    /// The terminator CoreLib reads through a pinned `char*` follows the characters in the
    /// allocation, and no logical view includes it.
    fn assert_terminated(string: &CLRString, expected: &str) {
        let expected: Vec<u16> = expected.encode_utf16().collect();
        assert_eq!(&**string, expected.as_slice());
        assert_eq!(string.len(), expected.len());
        assert_eq!(
            string.size_bytes(),
            size_of::<CLRString>() + expected.len() * 2
        );
        let base = string.raw_data_ptr().cast::<u16>();
        // SAFETY: F10.RawMemoryAccessValid — the allocation holds `len()` characters and a
        // terminator, and `string` is borrowed for the read.
        let terminator = unsafe { base.wrapping_add(string.len()).read() };
        assert_eq!(terminator, 0);
    }

    #[test]
    fn strings_keep_a_nul_terminator_outside_their_length() {
        for text in ["", "A", "Id", "Zip", "Score", "Name12"] {
            assert_terminated(&CLRString::from(text), text);
            // Interned and owned copies compare and hash by their characters only.
            assert_eq!(CLRString::from(text), CLRString::from(text));
        }

        let mut string = CLRString::from("abc");
        let chars = string.as_mut_slice();
        assert_eq!(chars.len(), 3);
        chars[2] = u16::from(b'z');
        assert_terminated(&string, "abz");
    }
}
