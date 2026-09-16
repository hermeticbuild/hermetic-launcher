// Placeholder bytes patched by `finalize-stub`. These MUST keep their exact byte
// initializers, sizes, `#[used]`, and declaration order: the finalizer locates them
// by scanning the binary image for these byte patterns and the contiguous argument
// storage described by the capabilities record. Only the link section name differs
// per OS, so the statics are declared once in a macro with the right section.
//
// They are `static mut` on purpose: that prevents the compiler from const-folding the
// template bytes into the code, so the patched values are actually read at runtime.

#[cfg(not(feature = "large"))]
pub const ARG_SIZE: usize = 256;
#[cfg(feature = "large")]
pub const ARG_SIZE: usize = 4096;
#[cfg(not(feature = "large"))]
pub const MAX_ARGS: usize = 10;
#[cfg(feature = "large")]
pub const MAX_ARGS: usize = 40;

// Keep the tiny stub's arithmetic and generated code unchanged.
#[cfg(not(feature = "large"))]
pub type TransformFlags = u32;
#[cfg(feature = "large")]
pub type TransformFlags = u64;

// Versioned, NUL-padded ASCII metadata, generated from the same compile-time
// constants as the storage and runtime. No runtime formatting code is linked.
const fn capabilities() -> [u8; 64] {
    let mut out = [0; 64];
    let parts: [&[u8]; 3] = [b"@@RUNFILES_CAPS@@v1;args=", b";size=", b";flags="];
    let values = [MAX_ARGS, ARG_SIZE, TransformFlags::BITS as usize];
    let mut pos = 0;
    let mut part = 0;
    while part < parts.len() {
        let mut i = 0;
        while i < parts[part].len() {
            out[pos] = parts[part][i];
            pos += 1;
            i += 1;
        }
        let mut divisor = 1;
        while values[part] / divisor >= 10 {
            divisor *= 10;
        }
        while divisor > 0 {
            out[pos] = b'0' + ((values[part] / divisor) % 10) as u8;
            pos += 1;
            divisor /= 10;
        }
        part += 1;
    }
    out
}

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

        // Argument placeholders as one contiguous 2D array rather than separate
        // statics. `arg()` indexes it with pointer arithmetic, which lowers
        // to a single PC-relative `adrp+add`; ten distinct statics made the compiler
        // materialize a table of ten absolute addresses, and under PIE (mandatory on
        // arm64 macOS) that table needs load-time rebasing — which would force a
        // writable, file-backed `__DATA` page back into existence. The bytes on disk
        // for the tiny variant (2560 contiguous '@') remain unchanged.
        #[used]
        #[link_section = $section]
        static mut ARGS: [[u8; ARG_SIZE]; MAX_ARGS] = [[b'@'; ARG_SIZE]; MAX_ARGS];

        #[used]
        #[link_section = $section]
        static CAPABILITIES: [u8; 64] = capabilities();
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
    let idx = if i < MAX_ARGS { i } else { MAX_ARGS - 1 };
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
