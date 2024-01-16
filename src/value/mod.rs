pub enum StackValue {
    Int32(i32),
    Int64(i64),
    NativeInt(isize),
    NativeFloat(f64),
    ObjectRef(), // TODO
    UnmanagedPtr(usize),
    ManagedPtr() // TODO
}

