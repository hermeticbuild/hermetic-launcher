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

        // The ten argument placeholders as one contiguous 2D array rather than ten
        // separate statics. `arg()` indexes it with pointer arithmetic, which lowers
        // to a single PC-relative `adrp+add`; ten distinct statics made the compiler
        // materialize a table of ten absolute addresses, and under PIE (mandatory on
        // arm64 macOS) that table needs load-time rebasing — which would force a
        // writable, file-backed `__DATA` page back into existence. The bytes on disk
        // (2560 contiguous '@') are identical either way, so the finalizer's
        // 256-byte-run scan is unaffected.
        #[used]
        #[link_section = $section]
        static mut ARGS: [[u8; ARG_SIZE]; 10] = [[b'@'; ARG_SIZE]; 10];
    };
}

#[cfg(target_os = "linux")]
define_placeholders!(".runfiles_stubs");
// Read-only `__TEXT` section: the placeholders are only ever read at runtime (the
// finalizer patches them on disk), so keeping them in `__TEXT` avoids a writable,
// load-time `__DATA` file page. See macos.rs for the no-libc rationale.
#[cfg(target_os = "macos")]
define_placeholders!("__TEXT,__runfiles");
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
    // Clamp to the last slot, matching the previous per-index behaviour. Indexing
    // the single ARGS array lowers to a PC-relative address (no rebased pointer
    // table); see the note on ARGS above.
    let idx = if i < 10 { i } else { 9 };
    let base = core::ptr::addr_of!(ARGS) as *const u8;
    let ptr = unsafe { base.add(idx * ARG_SIZE) };
    read(ptr, ARG_SIZE)
}

/// True if the placeholder still holds its unpatched template value.
pub fn is_template_placeholder(placeholder: &[u8]) -> bool {
    if placeholder.len() < 17 {
        return false;
    }
    placeholder.starts_with(b"@@RUNFILES_")
}
