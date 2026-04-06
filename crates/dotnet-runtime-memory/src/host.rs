/// Host hook for relaxed-ordering reads required by memory finalization paths.
pub trait MemoryOrderingHost {
    fn tracer_enabled_relaxed(&self) -> bool;
}

/// Host hook used by memory services to emit finalization tracing without
/// depending on VM-global state types.
pub trait MemorySharedStateHost: MemoryOrderingHost {
    fn trace_gc_resurrection(&self, indent: usize, obj_type_name: &str, addr: usize);
}
