#[cfg(test)]
mod tests {
    const INTRINSICS_PHF_RS: &str = include_str!(concat!(env!("OUT_DIR"), "/intrinsics_phf.rs"));
    const INTRINSICS_DISPATCH_RS: &str =
        include_str!(concat!(env!("OUT_DIR"), "/intrinsics_dispatch.rs"));
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
}
