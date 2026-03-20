mod integration_tests_impl;

#[cfg(target_os = "linux")]
#[unsafe(no_mangle)]
pub extern "C" fn __lsan_default_suppressions() -> *const core::ffi::c_char {
    // Integration fixtures exercise .NET/native interop paths that call through libffi.
    // LeakSanitizer reports persistent process-lifetime allocations under ffi_call_unix64.
    // Suppress this known external path so ASan task runs can focus on actionable VM leaks.
    static SUPPRESSIONS: &[u8] = b"leak:ffi_call_unix64\n\0";
    SUPPRESSIONS.as_ptr().cast()
}
