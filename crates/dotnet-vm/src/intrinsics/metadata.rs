//! Intrinsic metadata and classification.
//!
//! This module provides a unified taxonomy for intrinsic methods, allowing
//! the VM to understand WHY a method is intrinsic and HOW it should be dispatched.
//!
//! ## Intrinsic Categories
//!
//! ### Static
//! Methods that are always implemented by the VM, with no BCL implementation.
//! These are typically marked with `internal_call` or provide VM-specific functionality.
//! Examples: GC.Collect, RuntimeHelpers.InitializeArray
//!
//! ### VirtualOverride
//! Methods where the runtime type determines dispatch. The base type may have a BCL
//! implementation, but derived types in the VM override it with intrinsic implementations.
//! Examples: System.Type methods overridden by DotnetRs.RuntimeType
//!
//! ### DirectIntercept
//! Methods that MUST bypass any BCL implementation for correctness or performance.
//! The VM intercepts these calls even when a BCL implementation exists.
//! Examples: String.Length (internal representation differs), Math functions (performance)
use crate::intrinsics::{INTRINSIC_ATTR, IntrinsicHandler, IntrinsicRegistry};
use dotnet_assemblies::AssemblyLoader;
use dotnet_types::members::MethodDescription;

/// Classification of intrinsic methods based on their dispatch behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrinsicKind {
    /// Always use VM implementation. No BCL code exists or is ignored.
    /// Typically used for VM-specific operations (GC, reflection internals, etc.)
    Static,

    /// Runtime type determines dispatch. Used when derived types override base intrinsics.
    /// The VM checks the actual runtime type of `this` to select the appropriate handler.
    VirtualOverride,

    /// Must intercept and bypass BCL implementation.
    /// Used when VM's internal representation differs or for performance-critical operations.
    DirectIntercept,
}

/// Metadata about an intrinsic method.
///
/// This provides documentation and classification for each intrinsic,
/// making the codebase more maintainable and easier to understand.
#[derive(Clone)]
pub struct IntrinsicMetadata {
    /// The kind of intrinsic, determining dispatch behavior
    pub kind: IntrinsicKind,

    /// The handler function that implements this intrinsic
    pub handler: IntrinsicHandler,

    /// Human-readable explanation of why this method is intrinsic.
    /// Should explain the technical reason (e.g., "VM internal representation",
    /// "performance optimization", "no BCL implementation available")
    pub reason: &'static str,

    /// Optional filter to distinguish between overloads.
    /// If present, this function must return true for the intrinsic to match.
    pub signature_filter: Option<fn(&MethodDescription) -> bool>,
}

impl IntrinsicMetadata {
    /// Creates new intrinsic metadata.
    pub const fn new(kind: IntrinsicKind, handler: IntrinsicHandler, reason: &'static str) -> Self {
        Self {
            kind,
            handler,
            reason,
            signature_filter: None,
        }
    }

    /// Creates new intrinsic metadata with a signature filter.
    pub const fn with_filter(
        kind: IntrinsicKind,
        handler: IntrinsicHandler,
        reason: &'static str,
        filter: fn(&MethodDescription) -> bool,
    ) -> Self {
        Self {
            kind,
            handler,
            reason,
            signature_filter: Some(filter),
        }
    }

    /// Creates metadata for a static intrinsic.
    pub const fn static_intrinsic(handler: IntrinsicHandler, reason: &'static str) -> Self {
        Self::new(IntrinsicKind::Static, handler, reason)
    }

    /// Creates metadata for a virtual override intrinsic.
    pub const fn virtual_override(handler: IntrinsicHandler, reason: &'static str) -> Self {
        Self::new(IntrinsicKind::VirtualOverride, handler, reason)
    }

    /// Creates metadata for a direct intercept intrinsic.
    pub const fn direct_intercept(handler: IntrinsicHandler, reason: &'static str) -> Self {
        Self::new(IntrinsicKind::DirectIntercept, handler, reason)
    }
}

/// Result of intrinsic classification.
pub type ClassificationResult = Option<IntrinsicMetadata>;

/// Classifies a method to determine if it's intrinsic and what kind.
///
/// This is the single source of truth for intrinsic detection. It checks
/// all possible sources in a consistent order:
/// 1. Registry → Returns associated metadata (primary source)
/// 2. `internal_call` flag → Static intrinsic
/// 3. IntrinsicAttribute → Requires registry lookup for metadata
///
/// # Arguments
/// * `method` - The method to classify
/// * `loader` - Assembly loader for attribute resolution
/// * `registry` - Intrinsic registry for dynamic intrinsics (set to None during bootstrap)
///
/// # Returns
/// `Some(IntrinsicMetadata)` if the method is intrinsic, `None` otherwise.
pub fn classify_intrinsic(
    method: MethodDescription,
    loader: &AssemblyLoader,
    registry: Option<&IntrinsicRegistry>,
) -> ClassificationResult {
    // 1. Check registry first if available (most efficient and now source of truth)
    if let Some(registry) = registry
        && let Some(metadata) = registry.get_metadata(&method)
    {
        return Some(metadata);
    }

    // 2. Check internal_call flag - fall back to generic static if registry didn't have it
    if method.method.internal_call {
        // We still check registry.get(&method) in case it's registered without full metadata
        if let Some(registry) = registry
            && let Some(handler) = registry.get(&method)
        {
            return Some(IntrinsicMetadata::static_intrinsic(
                handler,
                "Marked as internal_call in metadata",
            ));
        }
        return None;
    }

    // 3. Check for IntrinsicAttribute
    for a in &method.method.attributes {
        if let Ok(ctor) = loader.locate_attribute(method.resolution(), a)
            && ctor.parent.type_name() == INTRINSIC_ATTR
        {
            if let Some(registry) = registry
                && let Some(handler) = registry.get(&method)
            {
                // For attributed intrinsics without full metadata, default to DirectIntercept
                return Some(IntrinsicMetadata::direct_intercept(
                    handler,
                    "Marked with IntrinsicAttribute",
                ));
            }
            return None;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intrinsic_kind_categories() {
        // Test that we have the three expected categories
        assert_ne!(IntrinsicKind::Static, IntrinsicKind::VirtualOverride);
        assert_ne!(IntrinsicKind::Static, IntrinsicKind::DirectIntercept);
        assert_ne!(
            IntrinsicKind::VirtualOverride,
            IntrinsicKind::DirectIntercept
        );
    }

    #[test]
    fn test_metadata_constructors() {
        use crate::StepResult;

        // Create a dummy handler for testing
        let dummy_handler: IntrinsicHandler = |_, _, _| StepResult::Continue;

        // Test static constructor
        let static_meta = IntrinsicMetadata::static_intrinsic(dummy_handler, "test reason");
        assert_eq!(static_meta.kind, IntrinsicKind::Static);
        assert_eq!(static_meta.reason, "test reason");

        // Test virtual override constructor
        let virtual_meta = IntrinsicMetadata::virtual_override(dummy_handler, "virtual reason");
        assert_eq!(virtual_meta.kind, IntrinsicKind::VirtualOverride);
        assert_eq!(virtual_meta.reason, "virtual reason");

        // Test direct intercept constructor
        let intercept_meta = IntrinsicMetadata::direct_intercept(dummy_handler, "intercept reason");
        assert_eq!(intercept_meta.kind, IntrinsicKind::DirectIntercept);
        assert_eq!(intercept_meta.reason, "intercept reason");
    }

    #[test]
    fn test_intrinsic_classification_categories() {
        // This test verifies that the three intrinsic categories are clearly distinguished
        // in the IntrinsicKind enum. Each category has a specific purpose:
        //
        // 1. DirectIntercept: Must bypass BCL for correctness
        //    - Example: String.get_Length (VM's internal representation differs)
        //    - Example: Unsafe memory operations (must respect VM GC)
        //
        // 2. Static: VM-only implementation (no BCL equivalent)
        //    - Example: GC.Collect (VM-specific functionality)
        //    - Example: Threading primitives (VM-managed)
        //
        // 3. VirtualOverride: Runtime type determines dispatch
        //    - Example: RuntimeType overriding System.Type methods
        //    - Tested via virtual dispatch integration

        // Verify that the three categories are distinct
        assert_ne!(IntrinsicKind::DirectIntercept, IntrinsicKind::Static);
        assert_ne!(
            IntrinsicKind::DirectIntercept,
            IntrinsicKind::VirtualOverride
        );
        assert_ne!(IntrinsicKind::Static, IntrinsicKind::VirtualOverride);

        // Note: More comprehensive testing of classification logic would require
        // mocking MethodDescription objects with static references, which is
        // complex for unit tests. The actual classification is tested via
        // integration tests that exercise real intrinsic methods.
    }

    #[test]
    fn test_intrinsic_metadata_reason_field() {
        // Verify that metadata includes human-readable reasons for intrinsics
        use crate::StepResult;

        let dummy_handler: IntrinsicHandler = |_, _, _| StepResult::Continue;

        // Test that each constructor creates metadata with the correct reason
        let direct_meta = IntrinsicMetadata::direct_intercept(
            dummy_handler,
            "Must bypass BCL for memory layout correctness",
        );
        assert_eq!(direct_meta.kind, IntrinsicKind::DirectIntercept);
        assert_eq!(
            direct_meta.reason,
            "Must bypass BCL for memory layout correctness"
        );

        let static_meta =
            IntrinsicMetadata::static_intrinsic(dummy_handler, "VM-only implementation");
        assert_eq!(static_meta.kind, IntrinsicKind::Static);
        assert_eq!(static_meta.reason, "VM-only implementation");

        let virtual_meta = IntrinsicMetadata::virtual_override(
            dummy_handler,
            "Runtime type-specific implementation",
        );
        assert_eq!(virtual_meta.kind, IntrinsicKind::VirtualOverride);
        assert_eq!(virtual_meta.reason, "Runtime type-specific implementation");
    }

    #[test]
    fn test_intrinsic_metadata_handler_storage() {
        // Verify that metadata correctly stores handler function pointers
        use crate::StepResult;

        let handler1: IntrinsicHandler = |_, _, _| StepResult::Continue;
        let handler2: IntrinsicHandler = |_, _, _| StepResult::MethodThrew;

        let meta1 = IntrinsicMetadata::static_intrinsic(handler1, "test1");
        let meta2 = IntrinsicMetadata::static_intrinsic(handler2, "test2");

        // Handler function pointers should be stored correctly
        // (comparing function pointers directly)
        assert_eq!(
            meta1.handler as usize, handler1 as usize,
            "Handler should match original"
        );
        assert_ne!(
            meta1.handler as usize, meta2.handler as usize,
            "Different handlers should have different addresses"
        );
    }
}
