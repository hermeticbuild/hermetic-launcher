// Placeholder bytes patched by `finalize-stub`. These MUST keep their exact byte
// initializers, sizes, `#[used]`, and declaration order: the finalizer locates them
// by scanning the binary image for these byte patterns (and the nth 256-byte run of
// '@' for ARG0..ARG9). Only the link section name differs per OS, so the statics are
// declared once in a macro that is invoked per platform with the right section.
//
// They are `static mut` on purpose: that prevents the compiler from const-folding the
// template bytes into the code, so the patched values are actually read at runtime.

pub const ARG_SIZE: usize = 256;

macro_rules! define_placeholders {
    ($section:literal) => {
        #[used]
        #[link_section = $section]
        static mut ARGC_PLACEHOLDER: [u8; 32] = *b"@@RUNFILES_ARGC@@\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

        #[used]
        #[link_section = $section]
        static mut TRANSFORM_FLAGS: [u8; 32] = *b"@@RUNFILES_TRANSFORM_FLAGS@@\0\0\0\0";

        #[used]
        #[link_section = $section]
        static mut EXPORT_RUNFILES_ENV: [u8; 32] = *b"@@RUNFILES_EXPORT_ENV@@\0\0\0\0\0\0\0\0\0";

        #[used]
        #[link_section = $section]
        static mut ARG0_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];
        #[used]
        #[link_section = $section]
        static mut ARG1_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];
        #[used]
        #[link_section = $section]
        static mut ARG2_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];
        #[used]
        #[link_section = $section]
        static mut ARG3_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];
        #[used]
        #[link_section = $section]
        static mut ARG4_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];
        #[used]
        #[link_section = $section]
        static mut ARG5_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];
        #[used]
        #[link_section = $section]
        static mut ARG6_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];
        #[used]
        #[link_section = $section]
        static mut ARG7_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];
        #[used]
        #[link_section = $section]
        static mut ARG8_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];
        #[used]
        #[link_section = $section]
        static mut ARG9_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];
    };
}

#[cfg(target_os = "linux")]
define_placeholders!(".runfiles_stubs");
#[cfg(target_os = "macos")]
define_placeholders!("__DATA,__runfiles");
#[cfg(target_os = "windows")]
define_placeholders!(".runfiles");

// Read a placeholder as a byte slice. Uses a raw pointer (not `&STATIC`) to avoid
// forming a reference to a mutable static; the bytes are only read, never mutated.
#[inline]
fn read(ptr: *const u8, len: usize) -> &'static [u8] {
    unsafe { core::slice::from_raw_parts(ptr, len) }
}

pub fn argc() -> &'static [u8] {
    read(core::ptr::addr_of!(ARGC_PLACEHOLDER) as *const u8, 32)
}
pub fn transform_flags() -> &'static [u8] {
    read(core::ptr::addr_of!(TRANSFORM_FLAGS) as *const u8, 32)
}
pub fn export_runfiles_env() -> &'static [u8] {
    read(core::ptr::addr_of!(EXPORT_RUNFILES_ENV) as *const u8, 32)
}

pub fn arg(i: usize) -> &'static [u8] {
    let ptr = match i {
        0 => core::ptr::addr_of!(ARG0_PLACEHOLDER),
        1 => core::ptr::addr_of!(ARG1_PLACEHOLDER),
        2 => core::ptr::addr_of!(ARG2_PLACEHOLDER),
        3 => core::ptr::addr_of!(ARG3_PLACEHOLDER),
        4 => core::ptr::addr_of!(ARG4_PLACEHOLDER),
        5 => core::ptr::addr_of!(ARG5_PLACEHOLDER),
        6 => core::ptr::addr_of!(ARG6_PLACEHOLDER),
        7 => core::ptr::addr_of!(ARG7_PLACEHOLDER),
        8 => core::ptr::addr_of!(ARG8_PLACEHOLDER),
        _ => core::ptr::addr_of!(ARG9_PLACEHOLDER),
    };
    read(ptr as *const u8, ARG_SIZE)
}

/// True if the placeholder still holds its unpatched template value.
pub fn is_template_placeholder(placeholder: &[u8]) -> bool {
    if placeholder.len() < 17 {
        return false;
    }
    placeholder.starts_with(b"@@RUNFILES_")
}
