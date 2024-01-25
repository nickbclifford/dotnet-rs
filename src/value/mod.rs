#[derive(Clone, Debug)]
pub enum StackValue {
    Int32(i32),
    Int64(i64),
    NativeInt(isize),
    NativeFloat(f64),
    ObjectRef(ObjectRef),
    UnmanagedPtr(usize),
    ManagedPtr(*mut u8) // TODO
}

// TODO: this will eventually be a GC'd reference
pub type ObjectRef = Option<Object>;
#[derive(Clone, Debug)]
pub struct Object {

}
