extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::panic::PanicInfo;
use talc::{ClaimOnOom, Span, Talc};

// Global allocator using talc with a static memory arena
// 8 MiB should be plenty for manifest parsing, path resolution, and environment handling
static mut ARENA: [u8; 8 * 1024 * 1024] = [0; 8 * 1024 * 1024];

// Simple wrapper for single-threaded use (no locking needed)
struct TalcAllocator(UnsafeCell<Talc<ClaimOnOom>>);
unsafe impl Sync for TalcAllocator {}

unsafe impl GlobalAlloc for TalcAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        (*self.0.get()).malloc(layout).map_or(core::ptr::null_mut(), |p| p.as_ptr())
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        (*self.0.get()).free(core::ptr::NonNull::new_unchecked(ptr), layout);
    }
}

#[global_allocator]
static ALLOCATOR: TalcAllocator = TalcAllocator(UnsafeCell::new(Talc::new(unsafe {
    ClaimOnOom::new(Span::from_array(core::ptr::addr_of!(ARENA).cast_mut()))
})));

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    exit(1);
}

// Compiler intrinsics (memcpy, memset)
#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *s.add(i) = c as u8;
        i += 1;
    }
    s
}

// glibc expects this symbol when linking without crt1.o/_start.
#[no_mangle]
pub static _IO_stdin_used: i32 = 0x20001;

#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let a = *s1.add(i);
        let b = *s2.add(i);
        if a != b {
            return a as i32 - b as i32;
        }
        i += 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn bcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    memcmp(s1, s2, n)
}

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    let mut len = 0;
    while *s.add(len) != 0 {
        len += 1;
    }
    len
}

// Syscall numbers - architecture specific
#[cfg(target_arch = "x86_64")]
mod syscall_numbers {
    pub const SYS_READ: usize = 0;
    pub const SYS_WRITE: usize = 1;
    pub const SYS_OPEN: usize = 2;
    pub const SYS_CLOSE: usize = 3;
    pub const SYS_ACCESS: usize = 21;
    pub const SYS_EXECVE: usize = 59;
    pub const SYS_EXIT: usize = 60;
}

#[cfg(target_arch = "aarch64")]
mod syscall_numbers {
    pub const SYS_READ: usize = 63;
    pub const SYS_WRITE: usize = 64;
    pub const SYS_OPENAT: usize = 56;  // openat is used on aarch64
    pub const SYS_CLOSE: usize = 57;
    pub const SYS_FACCESSAT: usize = 48;  // faccessat is used on aarch64
    pub const SYS_EXECVE: usize = 221;
    pub const SYS_EXIT: usize = 93;
    pub const AT_FDCWD: i32 = -100;  // Special fd for openat/faccessat to work like open/access
}

use syscall_numbers::*;

const O_RDONLY: i32 = 0;
const STDOUT: i32 = 1;

#[cfg(target_arch = "x86_64")]
fn exit(code: i32) -> ! {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_EXIT,
            in("rdi") code,
            options(noreturn)
        );
    }
}

#[cfg(target_arch = "aarch64")]
fn exit(code: i32) -> ! {
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") SYS_EXIT,
            in("x0") code,
            options(noreturn)
        );
    }
}

#[cfg(target_arch = "x86_64")]
fn write(fd: i32, buf: &[u8]) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_WRITE,
            in("rdi") fd,
            in("rsi") buf.as_ptr(),
            in("rdx") buf.len(),
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
fn write(fd: i32, buf: &[u8]) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") SYS_WRITE,
            in("x0") fd,
            in("x1") buf.as_ptr(),
            in("x2") buf.len(),
            lateout("x0") ret,
        );
    }
    ret
}

#[cfg(target_arch = "x86_64")]
fn open(path: &[u8]) -> i32 {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_OPEN,
            in("rdi") path.as_ptr(),
            in("rsi") O_RDONLY,
            in("rdx") 0,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
fn open(path: &[u8]) -> i32 {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") SYS_OPENAT,
            in("x0") AT_FDCWD,
            in("x1") path.as_ptr(),
            in("x2") O_RDONLY,
            in("x3") 0,
            lateout("x0") ret,
        );
    }
    ret
}

#[cfg(target_arch = "x86_64")]
fn read(fd: i32, buf: &mut [u8]) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_READ,
            in("rdi") fd,
            in("rsi") buf.as_ptr(),
            in("rdx") buf.len(),
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
fn read(fd: i32, buf: &mut [u8]) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") SYS_READ,
            in("x0") fd,
            in("x1") buf.as_ptr(),
            in("x2") buf.len(),
            lateout("x0") ret,
        );
    }
    ret
}

#[cfg(target_arch = "x86_64")]
fn close(fd: i32) {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_CLOSE,
            in("rdi") fd,
            lateout("rax") _,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
}

#[cfg(target_arch = "aarch64")]
fn close(fd: i32) {
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") SYS_CLOSE,
            in("x0") fd,
            lateout("x0") _,
        );
    }
}

// Check if a path exists using access() syscall with F_OK (0)
#[cfg(target_arch = "x86_64")]
fn path_exists(path: &[u8]) -> bool {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_ACCESS,
            in("rdi") path.as_ptr(),
            in("rsi") 0i32,  // F_OK = 0 (check existence)
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    ret == 0
}

#[cfg(target_arch = "aarch64")]
fn path_exists(path: &[u8]) -> bool {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") SYS_FACCESSAT,
            in("x0") AT_FDCWD,
            in("x1") path.as_ptr(),
            in("x2") 0i32,  // F_OK = 0 (check existence)
            in("x3") 0i32,  // flags = 0
            lateout("x0") ret,
        );
    }
    ret == 0
}

#[cfg(target_arch = "x86_64")]
fn execve(filename: *const u8, argv: *const *const u8, envp: *const *const u8) -> i32 {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_EXECVE,
            in("rdi") filename,
            in("rsi") argv,
            in("rdx") envp,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
fn execve(filename: *const u8, argv: *const *const u8, envp: *const *const u8) -> i32 {
    let ret: i32;
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") SYS_EXECVE,
            in("x0") filename,
            in("x1") argv,
            in("x2") envp,
            lateout("x0") ret,
        );
    }
    ret
}

// String utilities
fn print(s: &[u8]) {
    write(STDOUT, s);
}

fn str_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for i in 0..a.len() {
        if a[i] != b[i] {
            return false;
        }
    }
    true
}

fn str_starts_with(haystack: &[u8], needle: &[u8]) -> bool {
    if haystack.len() < needle.len() {
        return false;
    }
    str_eq(&haystack[..needle.len()], needle)
}

fn find_byte(haystack: &[u8], needle: u8) -> Option<usize> {
    for i in 0..haystack.len() {
        if haystack[i] == needle {
            return Some(i);
        }
    }
    None
}

// Environment variable reading - returns the value as a String
// Returns None if the variable doesn't exist or isn't valid UTF-8
fn get_env_var(name: &[u8]) -> Option<String> {
    let fd = open(b"/proc/self/environ\0");
    if fd < 0 {
        return None;
    }

    // Read environment into Vec
    let mut environ_data = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let bytes_read = read(fd, &mut chunk);
        if bytes_read <= 0 {
            break;
        }
        environ_data.extend_from_slice(&chunk[..bytes_read as usize]);
    }
    close(fd);

    if environ_data.is_empty() {
        return None;
    }

    let mut pos = 0;

    while pos < environ_data.len() {
        let start = pos;
        while pos < environ_data.len() && environ_data[pos] != 0 {
            pos += 1;
        }

        let entry = &environ_data[start..pos];
        if let Some(eq_pos) = find_byte(entry, b'=') {
            let key = &entry[..eq_pos];
            let value = &entry[eq_pos + 1..];

            if str_eq(key, name) {
                // Convert to String, returning None if not valid UTF-8
                return String::from_utf8(value.to_vec()).ok();
            }
        }

        pos += 1;
    }

    None
}

// Manifest entry using String for UTF-8 paths
struct ManifestEntry {
    key: String,
    value: String,
}

struct Manifest {
    entries: Vec<ManifestEntry>,
}

impl Manifest {
    fn new() -> Self {
        Self { entries: Vec::new() }
    }

    fn add_entry(&mut self, key: &str, value: &str) {
        self.entries.push(ManifestEntry {
            key: String::from(key),
            value: String::from(value),
        });
    }

    fn lookup(&self, key: &str) -> Option<&str> {
        for entry in &self.entries {
            if entry.key == key {
                return Some(&entry.value);
            }
        }
        None
    }

    /// Find the longest manifest entry whose key is a prefix of `path` at a '/' boundary.
    /// Returns (resolved_value, suffix) where suffix includes the leading '/'.
    fn prefix_lookup<'a, 'b>(&'a self, path: &'b str) -> Option<(&'a str, &'b str)> {
        let mut best: Option<(&str, &str)> = None;
        let mut best_len: usize = 0;
        for entry in &self.entries {
            let key = &entry.key;
            if path.len() > key.len()
                && path.starts_with(key.as_str())
                && path.as_bytes()[key.len()] == b'/'
                && key.len() > best_len
            {
                best_len = key.len();
                best = Some((&entry.value, &path[key.len()..]));
            }
        }
        best
    }
}

// Load manifest file
fn load_manifest(path: &[u8]) -> Option<Manifest> {
    let fd = open(path);
    if fd < 0 {
        return None;
    }

    // Read file into Vec, reading in chunks
    let mut file_data = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let bytes_read = read(fd, &mut chunk);
        if bytes_read <= 0 {
            break;
        }
        file_data.extend_from_slice(&chunk[..bytes_read as usize]);
    }
    close(fd);

    if file_data.is_empty() {
        return None;
    }

    // Convert to UTF-8 string for easier parsing
    let file_str = String::from_utf8(file_data).ok()?;

    let mut manifest = Manifest::new();

    for line in file_str.lines() {
        if let Some((key, value)) = line.split_once(' ') {
            manifest.add_entry(key, value);
        }
    }

    Some(manifest)
}

// Runfiles implementation using String for UTF-8 paths
enum RunfilesMode {
    ManifestBased(Manifest),
    DirectoryBased(String),
}

struct Runfiles {
    mode: RunfilesMode,
    // Paths for environment variables (when export_runfiles_env is true)
    manifest_path: Option<String>, // RUNFILES_MANIFEST_FILE
    dir_path: Option<String>,      // RUNFILES_DIR and JAVA_RUNFILES
}

impl Runfiles {
    fn create(executable_path: Option<&[u8]>) -> Option<Self> {
        // Try RUNFILES_MANIFEST_FILE first
        if let Some(manifest_path) = get_env_var(b"RUNFILES_MANIFEST_FILE") {
            if !manifest_path.is_empty() {
                // Create null-terminated path for load_manifest
                let mut path_with_null = Vec::from(manifest_path.as_bytes());
                path_with_null.push(0);

                if let Some(manifest) = load_manifest(&path_with_null) {
                    return Some(Self {
                        mode: RunfilesMode::ManifestBased(manifest),
                        manifest_path: Some(manifest_path),
                        dir_path: None,
                    });
                }
            }
        }

        // Try RUNFILES_DIR
        if let Some(runfiles_dir) = get_env_var(b"RUNFILES_DIR") {
            if !runfiles_dir.is_empty() {
                return Some(Self {
                    mode: RunfilesMode::DirectoryBased(runfiles_dir.clone()),
                    manifest_path: None,
                    dir_path: Some(runfiles_dir),
                });
            }
        }

        // Try to find runfiles next to the executable
        // Check for <executable>.runfiles_manifest file (preferred)
        // Then check for <executable>.runfiles directory
        if let Some(exe_path) = executable_path {
            let exe_len = str_len(exe_path);
            if exe_len > 0 {
                // Convert executable path to string (if valid UTF-8)
                let exe_str = core::str::from_utf8(&exe_path[..exe_len]).ok()?;

                // Try <executable>.runfiles_manifest file first
                let manifest_file_path = String::from(exe_str) + ".runfiles_manifest";

                // Add null terminator for syscall
                let mut manifest_path_with_null = Vec::from(manifest_file_path.as_bytes());
                manifest_path_with_null.push(0);

                // Try to load the manifest file
                if let Some(manifest) = load_manifest(&manifest_path_with_null) {
                    // Also determine the runfiles directory for RUNFILES_DIR envvar
                    let dir_path = String::from(exe_str) + ".runfiles";

                    return Some(Self {
                        mode: RunfilesMode::ManifestBased(manifest),
                        manifest_path: Some(manifest_file_path),
                        dir_path: Some(dir_path),
                    });
                }

                // Try <executable>.runfiles directory
                let runfiles_dir = String::from(exe_str) + ".runfiles";

                // Add null terminator for path_exists syscall
                let mut dir_with_null = Vec::from(runfiles_dir.as_bytes());
                dir_with_null.push(0);

                // Check if directory exists using access() syscall
                if path_exists(&dir_with_null) {
                    return Some(Self {
                        mode: RunfilesMode::DirectoryBased(runfiles_dir.clone()),
                        manifest_path: None,
                        dir_path: Some(runfiles_dir),
                    });
                }
            }
        }

        None
    }

    fn rlocation(&self, path: &str) -> Option<String> {
        // If path is absolute, don't resolve through runfiles
        if path.starts_with('/') {
            return None;
        }

        match &self.mode {
            RunfilesMode::ManifestBased(manifest) => {
                if let Some(resolved) = manifest.lookup(path) {
                    return Some(String::from(resolved));
                }
                // Prefix match for paths within TreeArtifacts
                if let Some((resolved_prefix, suffix)) = manifest.prefix_lookup(path) {
                    let mut result = String::from(resolved_prefix);
                    result.push_str(suffix);
                    return Some(result);
                }
                None
            }
            RunfilesMode::DirectoryBased(dir) => {
                let mut result = dir.clone();

                // Add separator if needed
                if !result.ends_with('/') {
                    result.push('/');
                }

                // Append the path
                result.push_str(path);

                Some(result)
            }
        }
    }
}

// Placeholders for stub runner (will be replaced in final binary)
// Each placeholder uses a distinctive pattern starting with @@RUNFILES_
const ARG_SIZE: usize = 256;

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARGC_PLACEHOLDER: [u8; 32] = *b"@@RUNFILES_ARGC@@\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

#[used]
#[link_section = ".runfiles_stubs"]
static mut TRANSFORM_FLAGS: [u8; 32] = *b"@@RUNFILES_TRANSFORM_FLAGS@@\0\0\0\0";

#[used]
#[link_section = ".runfiles_stubs"]
static mut EXPORT_RUNFILES_ENV: [u8; 32] = *b"@@RUNFILES_EXPORT_ENV@@\0\0\0\0\0\0\0\0\0";

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARG0_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARG1_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARG2_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARG3_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARG4_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARG5_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARG6_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARG7_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARG8_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles_stubs"]
static mut ARG9_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

// Get the length of a null-terminated string (Rust-style, takes slice)
fn str_len(s: &[u8]) -> usize {
    let mut len = 0;
    while len < s.len() && s[len] != 0 {
        len += 1;
    }
    len
}

// Check if placeholder is still in template state
fn is_template_placeholder(placeholder: &[u8]) -> bool {
    if placeholder.len() < 17 {
        return false;
    }
    str_starts_with(placeholder, b"@@RUNFILES_")
}

// Read and parse environment variables from /proc/self/environ
// Returns a tuple of (data_vec, pointers_vec) where pointers point into data_vec
fn read_environ() -> (Vec<u8>, Vec<*const u8>) {
    // Read environment from /proc/self/environ
    let fd = open(b"/proc/self/environ\0");
    if fd < 0 {
        // If we can't read environ, return empty environment
        return (Vec::new(), vec![core::ptr::null()]);
    }

    // Read into Vec
    let mut environ_data = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let bytes_read = read(fd, &mut chunk);
        if bytes_read <= 0 {
            break;
        }
        environ_data.extend_from_slice(&chunk[..bytes_read as usize]);
    }
    close(fd);

    if environ_data.is_empty() {
        return (Vec::new(), vec![core::ptr::null()]);
    }

    // Parse environment variables (null-separated entries)
    let mut env_ptrs = Vec::new();
    let mut pos = 0;
    let data_len = environ_data.len();

    while pos < data_len {
        // Skip empty entries
        if environ_data[pos] == 0 {
            pos += 1;
            continue;
        }

        // Mark start of this environment variable
        env_ptrs.push(unsafe { environ_data.as_ptr().add(pos) });

        // Find the end (null byte)
        while pos < data_len && environ_data[pos] != 0 {
            pos += 1;
        }

        // Move past the null byte
        pos += 1;
    }

    // Null-terminate the pointer array
    env_ptrs.push(core::ptr::null());

    (environ_data, env_ptrs)
}

// Build modified environment with runfiles variables
// Returns (data_vec, pointers_vec) - caller must keep data_vec alive while pointers are used
fn build_runfiles_environ(runfiles: Option<&Runfiles>) -> (Vec<u8>, Vec<*const u8>) {
    let (base_data, base_ptrs) = read_environ();

    // If no runfiles info, just return base environment
    let rf = match runfiles {
        Some(r) => r,
        None => return (base_data, base_ptrs),
    };

    let mut env_data = Vec::new();
    let mut env_ptrs = Vec::new();

    // Helper to add an environment variable to env_data and record pointer
    let add_env_var = |data: &mut Vec<u8>, ptrs: &mut Vec<*const u8>, name: &[u8], value: &str| {
        let start_pos = data.len();
        data.extend_from_slice(name);
        data.push(b'=');
        data.extend_from_slice(value.as_bytes());
        data.push(0); // null terminator

        // We'll fix up pointers after building the data
        ptrs.push(start_pos as *const u8); // Temporarily store offset, fix up later
    };

    // Add runfiles environment variables first
    if let Some(ref path) = rf.manifest_path {
        add_env_var(&mut env_data, &mut env_ptrs, b"RUNFILES_MANIFEST_FILE", path);
    }

    if let Some(ref path) = rf.dir_path {
        add_env_var(&mut env_data, &mut env_ptrs, b"RUNFILES_DIR", path);
        add_env_var(&mut env_data, &mut env_ptrs, b"JAVA_RUNFILES", path);
    }

    // Copy existing environment (skip runfiles vars that we're setting)
    // We need to iterate through the base data directly
    let mut pos = 0;
    while pos < base_data.len() {
        // Skip empty entries
        if base_data[pos] == 0 {
            pos += 1;
            continue;
        }

        // Find the end of this entry
        let start = pos;
        while pos < base_data.len() && base_data[pos] != 0 {
            pos += 1;
        }

        let env_entry = &base_data[start..pos];

        // Skip if this is a runfiles var we're replacing
        let is_runfiles_var = env_entry.starts_with(b"RUNFILES_MANIFEST_FILE=")
            || env_entry.starts_with(b"RUNFILES_DIR=")
            || env_entry.starts_with(b"JAVA_RUNFILES=");

        if !is_runfiles_var {
            let start_pos = env_data.len();
            env_data.extend_from_slice(env_entry);
            env_data.push(0); // null terminator
            env_ptrs.push(start_pos as *const u8); // Temporarily store offset
        }

        // Move past the null byte
        pos += 1;
    }

    // Now fix up all the pointers to point to actual addresses in env_data
    let base_ptr = env_data.as_ptr();
    for ptr in env_ptrs.iter_mut() {
        let offset = *ptr as usize;
        *ptr = unsafe { base_ptr.add(offset) };
    }

    // Null-terminate the pointer array
    env_ptrs.push(core::ptr::null());

    (env_data, env_ptrs)
}

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".global _start",
    "_start:",
    "mov rdi, rsp",                 // Pass stack pointer as first argument
    "call _start_rust",             // Call the actual start function
);

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    ".global _start",
    "_start:",
    "mov x0, sp",                   // Pass stack pointer as first argument
    "b _start_rust",                // Jump to the actual start function
);

#[no_mangle]
pub extern "C" fn _start_rust(initial_sp: *const usize) -> ! {
    unsafe {
        // Stack layout: [sp] = argc, [sp + 8] = argv[0], [sp + 16] = argv[1], ...
        let runtime_argc = *initial_sp;
        let runtime_argv = (initial_sp as usize + 8) as *const *const u8;

        // Check if ARGC is still a placeholder
        if is_template_placeholder(&ARGC_PLACEHOLDER) {
            print(b"ERROR: This is a template stub runner.\n");
            print(b"You must finalize it by replacing the placeholders before use.\n");
            print(b"The ARGC_PLACEHOLDER has not been replaced.\n");
            exit(1);
        }

        // Parse argc from placeholder
        let argc_str = &ARGC_PLACEHOLDER;
        let argc_len = str_len(argc_str);
        if argc_len == 0 {
            print(b"ERROR: ARGC is empty\n");
            exit(1);
        }

        // Parse argc as decimal number
        let mut argc: usize = 0;
        for i in 0..argc_len {
            let c = argc_str[i];
            if c >= b'0' && c <= b'9' {
                argc = argc * 10 + (c - b'0') as usize;
            } else {
                print(b"ERROR: ARGC contains non-digit characters\n");
                exit(1);
            }
        }

        if argc == 0 || argc > 10 {
            print(b"ERROR: Invalid argc (must be 1-10)\n");
            exit(1);
        }

        // Parse transform flags (bitmask of which args to transform)
        let flags_str = &TRANSFORM_FLAGS;
        let flags_len = str_len(flags_str);
        let mut transform_flags: u32 = 0;

        if !is_template_placeholder(flags_str) && flags_len > 0 {
            // Parse as decimal number (bitmask)
            for i in 0..flags_len {
                let c = flags_str[i];
                if c >= b'0' && c <= b'9' {
                    transform_flags = transform_flags * 10 + (c - b'0') as u32;
                } else {
                    print(b"ERROR: TRANSFORM_FLAGS contains non-digit characters\n");
                    exit(1);
                }
            }
        }
        // If flags not set, default to transforming all args
        if flags_len == 0 || is_template_placeholder(flags_str) {
            transform_flags = 0xFFFFFFFF; // Transform all by default
        }

        // Parse export_runfiles_env flag
        let export_env_str = &EXPORT_RUNFILES_ENV;
        let export_env_len = str_len(export_env_str);
        let export_runfiles_env = if !is_template_placeholder(export_env_str) && export_env_len > 0 {
            export_env_str[0] == b'1'
        } else {
            true // Default to true if not set
        };

        // Check if any arguments need transformation
        // Create a mask for only the arguments we have (argc args)
        let argc_mask = if argc >= 32 {
            0xFFFFFFFF
        } else {
            (1u32 << argc) - 1
        };
        let needs_transform = (transform_flags & argc_mask) != 0;
        let needs_runfiles = needs_transform || export_runfiles_env;

        // Get executable path from runtime argv[0] (the stub's actual path) for runfiles fallback
        let executable_path = if runtime_argc > 0 {
            let argv0_ptr = *runtime_argv;
            let mut exe_len = 0;
            // Safety limit of 1MB to prevent infinite loop
            while *argv0_ptr.add(exe_len) != 0 && exe_len < 1048576 {
                exe_len += 1;
            }
            if exe_len > 0 {
                Some(core::slice::from_raw_parts(argv0_ptr, exe_len))
            } else {
                None
            }
        } else {
            None
        };

        // Initialize runfiles only if needed
        let runfiles = if needs_runfiles {
            if let Some(rf) = Runfiles::create(executable_path) {
                Some(rf)
            } else {
                print(b"ERROR: Failed to initialize runfiles\n");
                print(b"Set RUNFILES_DIR or RUNFILES_MANIFEST_FILE, or ensure <executable>.runfiles/ directory exists\n");
                exit(1);
            }
        } else {
            None
        };

        // Get arg placeholders
        let arg_placeholders: [&[u8; ARG_SIZE]; 10] = [
            &ARG0_PLACEHOLDER,
            &ARG1_PLACEHOLDER,
            &ARG2_PLACEHOLDER,
            &ARG3_PLACEHOLDER,
            &ARG4_PLACEHOLDER,
            &ARG5_PLACEHOLDER,
            &ARG6_PLACEHOLDER,
            &ARG7_PLACEHOLDER,
            &ARG8_PLACEHOLDER,
            &ARG9_PLACEHOLDER,
        ];

        // Use Vec for dynamic path storage - no fixed size limits
        // We store as Vec<u8> since we need null-terminated bytes for execve
        let mut resolved_paths: Vec<Vec<u8>> = Vec::with_capacity(128);

        // Resolve embedded arguments
        for i in 0..argc {
            let arg_data = arg_placeholders[i];
            let arg_len = str_len(arg_data);

            if arg_len == 0 {
                print(b"ERROR: Argument ");
                let digit = [b'0' + i as u8];
                print(&digit);
                print(b" is empty\n");
                exit(1);
            }

            let arg_slice = &arg_data[..arg_len];

            // Check if this argument should be transformed
            let should_transform = (transform_flags & (1 << i)) != 0;

            let resolved = if should_transform {
                // Try to resolve through runfiles (which we know exists if we need transformation)
                if let Some(ref rf) = runfiles {
                    // Convert argument to &str for rlocation (Bazel args are UTF-8)
                    if let Ok(arg_str) = core::str::from_utf8(arg_slice) {
                        if let Some(resolved_str) = rf.rlocation(arg_str) {
                            // Convert back to bytes with null terminator
                            let mut path = Vec::from(resolved_str.as_bytes());
                            path.push(0);
                            path
                        } else {
                            // If not found in runfiles, use the path as-is
                            let mut path = arg_slice.to_vec();
                            path.push(0);
                            path
                        }
                    } else {
                        // Not valid UTF-8, use as-is
                        let mut path = arg_slice.to_vec();
                        path.push(0);
                        path
                    }
                } else {
                    // This should never happen - we checked needs_runfiles before
                    // But use path as-is for safety
                    let mut path = arg_slice.to_vec();
                    path.push(0);
                    path
                }
            } else {
                // Use path as-is without transformation
                let mut path = arg_slice.to_vec();
                path.push(0);
                path
            };

            resolved_paths.push(resolved);
        }

        // Append runtime arguments (skip argv[0] which is the stub itself)
        if runtime_argc > 1 {
            for i in 1..runtime_argc {
                // Get runtime argument
                let runtime_arg_ptr = *runtime_argv.add(i);

                // Find length of runtime argument (scan until null, with safety limit)
                let mut arg_len = 0;
                while *runtime_arg_ptr.add(arg_len) != 0 {
                    arg_len += 1;
                    // Safety limit to prevent infinite loop on malformed input
                    if arg_len > 1048576 {
                        print(b"ERROR: Runtime argument exceeds 1MB limit\n");
                        exit(1);
                    }
                }

                // Copy runtime argument (include null terminator)
                let runtime_arg_slice = core::slice::from_raw_parts(runtime_arg_ptr, arg_len + 1);
                resolved_paths.push(runtime_arg_slice.to_vec());
            }
        }

        // Build pointer array from the resolved paths
        let mut resolved_ptrs: Vec<*const u8> = Vec::with_capacity(resolved_paths.len() + 1);
        for path in &resolved_paths {
            resolved_ptrs.push(path.as_ptr());
        }
        // NULL-terminate the argv array
        resolved_ptrs.push(core::ptr::null());

        // Get the executable path (first argument)
        let executable = resolved_ptrs[0];

        // Build environment (with runfiles vars if export_runfiles_env is true)
        // We need to keep the env_data alive until execve
        let (_env_data, env_ptrs) = if export_runfiles_env {
            build_runfiles_environ(runfiles.as_ref())
        } else {
            read_environ()
        };

        // Execute the target program
        let ret = execve(executable, resolved_ptrs.as_ptr(), env_ptrs.as_ptr());

        // If execve returns, it failed
        print(b"ERROR: execve failed with code ");
        let digit = if ret < 0 {
            print(b"-");
            (-ret) as u8 + b'0'
        } else {
            ret as u8 + b'0'
        };
        print(&[digit]);
        print(b"\n");
        exit(1);
    }
}
