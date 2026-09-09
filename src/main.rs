use lagui::{DrawList, Quad, demo::Demo};
use std::ffi::{CString, c_char, c_void};

unsafe extern "C" {
    fn bg_run(
        context: *mut c_void,
        input: extern "C" fn(*mut c_void, u32, f32, f32),
        paint: extern "C" fn(*mut c_void, *mut Quad, usize, f32, f32) -> usize,
        shader: *const c_char,
        smoke: bool,
    ) -> i32;
}

extern "C" fn input(context: *mut c_void, kind: u32, x: f32, y: f32) {
    // SAFETY: bg_run only calls on the main thread, synchronously within main's lifetime.
    unsafe { &mut *context.cast::<Demo>() }.input(kind, x, y);
}
extern "C" fn paint(
    context: *mut c_void,
    data: *mut Quad,
    capacity: usize,
    width: f32,
    height: f32,
) -> usize {
    // SAFETY: the adapter provides aligned writable Quad storage of `capacity` elements,
    // exclusive for this callback. No Rust reference escapes the callback.
    let (demo, storage) = unsafe {
        (
            &*context.cast::<Demo>(),
            std::slice::from_raw_parts_mut(data, capacity),
        )
    };
    let mut list = DrawList::new(storage);
    if demo.paint(&mut list, width, height).is_err() {
        return 0;
    }
    list.commands().len()
}

fn main() {
    let smoke = std::env::args().any(|a| a == "--smoke");
    let mut demo = Demo::default();
    let shader = CString::new(include_str!("../native/quad.metal")).unwrap();
    // SAFETY: bg_run retains neither pointers nor callbacks after returning; all
    // interaction with Demo is serialized on the main thread.
    let result = unsafe {
        bg_run(
            (&mut demo as *mut Demo).cast(),
            input,
            paint,
            shader.as_ptr(),
            smoke,
        )
    };
    if result != 0 {
        std::process::exit(result);
    }
}
