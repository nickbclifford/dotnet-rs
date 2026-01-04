use std::sync::atomic::AtomicI32;
fn main() {
    let mut x = 0i32;
    let ptr = &mut x as *mut i32;
    let _atomic = unsafe { AtomicI32::from_ptr(ptr) };
}
