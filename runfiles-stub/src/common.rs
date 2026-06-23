// Platform-agnostic helpers shared by every backend: small byte utilities and the
// memory-mapped runfiles MANIFEST parser.

extern crate alloc;

use alloc::vec::Vec;

use crate::platform;

/// Length of a NUL-terminated byte string stored in a fixed-size buffer:
/// the offset of the first NUL, or the full length if there is none.
pub fn cstr_len(s: &[u8]) -> usize {
    s.iter().position(|&b| b == 0).unwrap_or(s.len())
}

/// Make `path` absolute without resolving symlinks. An already-absolute path is
/// returned unchanged; a relative one is joined onto the current working
/// directory. Best-effort: if the cwd is unavailable, the path is returned as-is.
pub fn absolutize(path: Vec<u8>) -> Vec<u8> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if crate::native_path::unix_path_is_absolute(&path) {
        return path;
    }
    #[cfg(target_os = "windows")]
    if let Ok(s) = core::str::from_utf8(&path) {
        if platform::is_absolute(s) {
            return path;
        }
    }
    if let Some(mut cwd) = platform::current_dir() {
        if cwd.last().copied() != Some(platform::SEP as u8) {
            cwd.push(platform::SEP as u8);
        }
        cwd.extend_from_slice(&path);
        return cwd;
    }
    path
}

/// Print a decimal number to stdout (used in diagnostics).
#[allow(dead_code)] // only some backends print numeric diagnostics
pub fn print_number(mut n: usize) {
    if n == 0 {
        platform::print(b"0");
        return;
    }
    let mut buf = [0u8; 20]; // enough for 64-bit numbers
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        platform::print(&buf[i..i + 1]);
    }
}

// Memory-mapped manifest: a pointer/length pair into the mapped file. The
// manifest is scanned lazily on each lookup (O(n)), so no per-entry allocation
// is needed and arbitrarily large manifests are supported. The kernel caches
// the file's pages for us.
pub struct Manifest {
    ptr: *const u8,
    len: usize,
}

impl Manifest {
    /// Construct from a mapping that lives for the rest of the process.
    ///
    /// # Safety
    /// `ptr`/`len` must come from a successful read-only mapping that is leaked
    /// for the lifetime of the process (until execve/ExitProcess reclaims it).
    pub unsafe fn from_mapping(ptr: *const u8, len: usize) -> Self {
        Manifest { ptr, len }
    }

    #[inline]
    fn data(&self) -> &[u8] {
        // Safety: ptr/len come from a leaked, process-lifetime mapping (see `from_mapping`).
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }

    pub fn lookup(&self, key: &str) -> Option<&str> {
        let key_bytes = key.as_bytes();
        for (k, v) in ManifestLines::new(self.data()) {
            if k == key_bytes {
                return core::str::from_utf8(v).ok();
            }
        }
        None
    }

    /// Find the longest manifest entry whose key is a prefix of `path` at a '/' boundary.
    /// Returns (resolved_value, suffix) where suffix includes the leading '/'.
    pub fn prefix_lookup<'a, 'b>(&'a self, path: &'b str) -> Option<(&'a str, &'b str)> {
        let path_bytes = path.as_bytes();
        let mut best: Option<(&'a str, &'b str)> = None;
        let mut best_len: usize = 0;
        for (k, v) in ManifestLines::new(self.data()) {
            if path_bytes.len() > k.len()
                && k.len() > best_len
                && &path_bytes[..k.len()] == k
                && path_bytes[k.len()] == b'/'
            {
                // Values were owned UTF-8 Strings before; keep that guarantee by
                // only considering candidates whose value is valid UTF-8.
                if let Ok(value) = core::str::from_utf8(v) {
                    best_len = k.len();
                    best = Some((value, &path[k.len()..]));
                }
            }
        }
        best
    }
}

/// Iterator over `(key, value)` byte slices of a Bazel runfiles MANIFEST.
/// Replicates `str::lines()` + `split_once(' ')`: split on '\n', strip one
/// trailing '\r' (CRLF), and skip lines without a space (e.g. the
/// "<workspace>/.runfile" marker).
struct ManifestLines<'a> {
    rest: &'a [u8],
}

impl<'a> ManifestLines<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { rest: data }
    }
}

impl<'a> Iterator for ManifestLines<'a> {
    type Item = (&'a [u8], &'a [u8]);

    fn next(&mut self) -> Option<(&'a [u8], &'a [u8])> {
        while !self.rest.is_empty() {
            let (mut line, remainder) = match self.rest.iter().position(|&b| b == b'\n') {
                Some(nl) => (&self.rest[..nl], &self.rest[nl + 1..]),
                None => (self.rest, &self.rest[self.rest.len()..]),
            };
            self.rest = remainder;
            // Strip one trailing '\r' (handles CRLF line endings).
            if let Some((&b'\r', head)) = line.split_last() {
                line = head;
            }
            if let Some(sp) = line.iter().position(|&b| b == b' ') {
                return Some((&line[..sp], &line[sp + 1..]));
            }
            // No space -> skip this line and continue.
        }
        None
    }
}
