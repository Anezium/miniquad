#![allow(dead_code, non_snake_case)]
mod native {
    pub mod gl {
        pub type GLboolean = u8;
        pub type GLclampf = f32;
        pub type GLenum = u32;
        pub type GLint = i32;
        pub type GLsizei = i32;
        pub type GLuint = u32;
        pub const GL_RGBA: u32 = 0x1908;
        pub const GL_RGBA8: u32 = 0x8058;
        pub const GL_TEXTURE_2D: u32 = 0x0DE1;
        pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
        pub const GL_FALSE: u32 = 0;
        pub unsafe fn glRenderbufferStorage(_: u32, _: u32, _: i32, _: i32) {}
        pub unsafe fn glVertexAttribPointer(
            _: u32,
            _: i32,
            _: u32,
            _: u8,
            _: i32,
            _: *const std::ffi::c_void,
        ) {
        }
    }
}
mod sys {
    pub unsafe fn vglGetProcAddress(_: *const i8) -> *mut std::ffi::c_void {
        std::ptr::null_mut()
    }
}
#[path = "../src/native/vita/gl_loader.rs"]
mod gl_loader;
thread_local! { static LAST: std::cell::Cell<i32> = const { std::cell::Cell::new(0) }; }
#[no_mangle]
pub extern "C" fn glTexImage2D(
    _: u32,
    _: i32,
    fmt: i32,
    _: i32,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
    _: *const std::ffi::c_void,
) {
    LAST.with(|v| v.set(fmt));
}
fn upload(w: i32, h: i32, ptr: *const std::ffi::c_void, fmt: u32) -> i32 {
    let proc = gl_loader::get_proc_address("glTexImage2D").unwrap();
    let f: unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const std::ffi::c_void) =
        unsafe { std::mem::transmute(proc) };
    unsafe { f(0x0DE1, 0, 0x1908, w, h, 0, fmt, 0x1401, ptr) };
    LAST.with(|v| v.get())
}
#[test]
fn scoped_dxt_and_dynamic_passthrough() {
    let p = std::ptr::dangling::<u8>() as _;
    assert_eq!(upload(256, 128, p, 0x1908), 0x1908);
    assert_eq!(
        gl_loader::with_dxt5_uploads(|| upload(256, 128, p, 0x1908)),
        0x83f3
    );
    assert_eq!(upload(256, 128, p, 0x1908), 0x1908);
}
#[test]
fn unsafe_sizes_null_and_other_formats_passthrough() {
    let p = std::ptr::dangling::<u8>() as _;
    gl_loader::with_dxt5_uploads(|| {
        for (w, h) in [(2, 2), (0, 256), (-1, 256), (120, 256)] {
            assert_eq!(upload(w, h, p, 0x1908), 0x1908);
        }
        assert_eq!(upload(256, 256, std::ptr::null(), 0x1908), 0x1908);
        assert_eq!(upload(256, 256, p, 0x1903), 0x1908);
    });
}
#[test]
fn nested_and_unwind_restore_scope() {
    let p = std::ptr::dangling::<u8>() as _;
    gl_loader::with_dxt5_uploads(|| {
        let _ = std::panic::catch_unwind(|| gl_loader::with_dxt5_uploads(|| panic!("expected")));
        assert_eq!(upload(256, 256, p, 0x1908), 0x83f3);
    });
    let _ = std::panic::catch_unwind(|| gl_loader::with_dxt5_uploads(|| panic!("expected")));
    assert_eq!(upload(256, 256, p, 0x1908), 0x1908);
}
#[test]
fn scope_is_thread_local() {
    gl_loader::with_dxt5_uploads(|| {
        std::thread::spawn(|| {
            assert_eq!(
                upload(256, 256, std::ptr::dangling::<u8>() as _, 0x1908),
                0x1908
            )
        })
        .join()
        .unwrap();
    });
}
