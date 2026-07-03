#[cfg(test)]
mod tests {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };

    const INTRINSICS_PHF_RS: &str = include_str!(concat!(env!("OUT_DIR"), "/intrinsics_phf.rs"));
    const INTRINSICS_DISPATCH_RS: &str =
        include_str!(concat!(env!("OUT_DIR"), "/intrinsics_dispatch.rs"));

    fn signature_hash(normalized_signature: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        normalized_signature.hash(&mut hasher);
        hasher.finish()
    }
    #[test]
    fn generated_intrinsics_phf_contains_p1_blocker_keys() {
        assert!(
            INTRINSICS_PHF_RS
                .contains("M:System.Runtime.CompilerServices.RuntimeHelpers::InitializeArray#2:S"),
            "Generated intrinsic PHF table is missing RuntimeHelpers.InitializeArray registration key"
        );
        assert!(
            INTRINSICS_PHF_RS.contains("M:System.Environment::get_CurrentManagedThreadId#0:S"),
            "Generated intrinsic PHF table is missing Environment.CurrentManagedThreadId registration key"
        );
    }
    #[test]
    fn generated_intrinsics_dispatch_contains_p1_blocker_handlers() {
        assert!(
            INTRINSICS_DISPATCH_RS
            .contains("dotnet_intrinsics_span::conversions::intrinsic_runtime_helpers_initialize_array(ctx, method, generics)"),
            "Generated intrinsic dispatch table is missing RuntimeHelpers.InitializeArray handler arm"
        );
        assert!(
            INTRINSICS_DISPATCH_RS
            .contains("crate::intrinsics::gc::intrinsic_environment_current_managed_thread_id(ctx, method, generics)"),
            "Generated intrinsic dispatch table is missing Environment.CurrentManagedThreadId handler arm"
        );
    }

    #[test]
    fn generated_intrinsic_table_keeps_non_primitive_overload_filter_boundary() {
        // Non-primitive overloads stay classified by build-time registration while
        // argument-shape validation remains a runtime-handler concern.
        assert!(
            INTRINSICS_PHF_RS.contains("M:System.Array::GetValue#2:I"),
            "Generated intrinsic PHF table is missing System.Array.GetValue arity-2 instance key"
        );

        let hash = signature_hash("Object System.Array::GetValue(Int32[])");
        let expected_filter =
            format!("dotnet_intrinsics_core::array_ops::intrinsic_array_get_value_filter_{hash:x}");
        assert!(
            INTRINSICS_PHF_RS.contains(&expected_filter),
            "Generated intrinsic PHF table is missing expected filter symbol for non-primitive int[] overload"
        );
    }
}
