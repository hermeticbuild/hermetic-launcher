// macOS-specific implementation using libc
// Unlike Linux version, this uses libc and can link with libsystem

extern crate alloc;

use alloc::string::String;
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
    unsafe { exit(1) }
}

// External libc functions
extern "C" {
    fn exit(code: i32) -> !;
    fn write(fd: i32, buf: *const u8, count: usize) -> isize;
    fn open(path: *const u8, flags: i32, ...) -> i32;
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
    fn close(fd: i32) -> i32;
    fn access(path: *const u8, mode: i32) -> i32;
    fn execve(path: *const u8, argv: *const *const u8, envp: *const *const u8) -> i32;

    // Access to errno - macOS provides this via __error()
    // Returns a pointer to the thread-local errno variable
    fn __error() -> *mut i32;

    // Access to environment - macOS provides this
    static mut environ: *const *const u8;
}

// Get the current errno value
fn get_errno() -> i32 {
    unsafe { *__error() }
}

// Check if a path exists using access() with F_OK
fn path_exists(path: &[u8]) -> bool {
    unsafe {
        access(path.as_ptr(), 0) == 0  // F_OK = 0
    }
}

// File open flags
const O_RDONLY: i32 = 0;
const STDOUT: i32 = 1;

// String utilities
fn print(s: &[u8]) {
    unsafe {
        write(STDOUT, s.as_ptr(), s.len());
    }
}

fn print_number(mut n: usize) {
    let mut buf = [0u8; 20]; // Enough for 64-bit numbers
    let mut i = 0;

    if n == 0 {
        print(b"0");
        return;
    }

    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }

    // Print in reverse order
    while i > 0 {
        i -= 1;
        print(&buf[i..i+1]);
    }
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

// Environment variable reading via the environ pointer - returns String
fn get_env_var(name: &[u8]) -> Option<String> {
    unsafe {
        let mut env_ptr = environ;

        // Iterate through environment variables
        while !(*env_ptr).is_null() {
            let entry_ptr = *env_ptr;

            // Find the length of this environment variable string
            let mut len = 0;
            while *entry_ptr.add(len) != 0 {
                len += 1;
                if len > 1048576 {  // Safety limit: 1MB
                    break;
                }
            }

            // Convert to slice
            let entry = core::slice::from_raw_parts(entry_ptr, len);

            // Look for '=' separator
            if let Some(eq_pos) = find_byte(entry, b'=') {
                let key = &entry[..eq_pos];
                let value = &entry[eq_pos + 1..];

                if str_eq(key, name) {
                    // Convert to String, returning None if not valid UTF-8
                    return String::from_utf8(value.to_vec()).ok();
                }
            }

            env_ptr = env_ptr.add(1);
        }
    }

    None
}

// Manifest entry using String for UTF-8 paths (Bazel-generated)
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
}

// Load manifest file
fn load_manifest(path: &[u8]) -> Option<Manifest> {
    unsafe {
        let fd = open(path.as_ptr(), O_RDONLY);
        if fd < 0 {
            return None;
        }

        // Read file into Vec, reading in chunks
        let mut file_data = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let bytes_read = read(fd, chunk.as_mut_ptr(), chunk.len());
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
}

// Runfiles implementation using String for dynamic path storage
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
            let exe_len = strlen(exe_path);
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
                    // The directory is <executable>.runfiles
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

// Build modified environment with runfiles variables using Vec
// Returns (data_vec, pointers_vec) - caller must keep data_vec alive while pointers are used
fn build_runfiles_environ(runfiles: Option<&Runfiles>) -> (Vec<u8>, Vec<*const u8>) {
    let mut env_data = Vec::new();
    let mut env_ptrs = Vec::new();

    // Helper to add an environment variable to env_data and record offset
    let add_env_var = |data: &mut Vec<u8>, ptrs: &mut Vec<*const u8>, name: &[u8], value: &str| {
        let start_pos = data.len();
        data.extend_from_slice(name);
        data.push(b'=');
        data.extend_from_slice(value.as_bytes());
        data.push(0); // null terminator

        // Temporarily store offset, fix up later
        ptrs.push(start_pos as *const u8);
    };

    // Add runfiles environment variables first
    if let Some(rf) = runfiles {
        if let Some(ref path) = rf.manifest_path {
            add_env_var(&mut env_data, &mut env_ptrs, b"RUNFILES_MANIFEST_FILE", path);
        }
        if let Some(ref path) = rf.dir_path {
            add_env_var(&mut env_data, &mut env_ptrs, b"RUNFILES_DIR", path);
            add_env_var(&mut env_data, &mut env_ptrs, b"JAVA_RUNFILES", path);
        }
    }

    // Copy existing environment, filtering out runfiles vars
    unsafe {
        let mut env_ptr = environ;
        while !(*env_ptr).is_null() {
            let entry_ptr = *env_ptr;

            // Find length of this entry
            let mut len = 0;
            while *entry_ptr.add(len) != 0 {
                len += 1;
                if len > 1048576 {  // Safety limit: 1MB
                    break;
                }
            }

            let entry = core::slice::from_raw_parts(entry_ptr, len);

            // Check if this is a runfiles variable we should skip
            let should_skip = str_starts_with(entry, b"RUNFILES_MANIFEST_FILE=")
                || str_starts_with(entry, b"RUNFILES_DIR=")
                || str_starts_with(entry, b"JAVA_RUNFILES=");

            if !should_skip {
                let start_pos = env_data.len();
                env_data.extend_from_slice(entry);
                env_data.push(0); // null terminator
                env_ptrs.push(start_pos as *const u8); // Temporarily store offset
            }

            env_ptr = env_ptr.add(1);
        }
    }

    // Fix up all the pointers to point to actual addresses in env_data
    let base_ptr = env_data.as_ptr();
    for ptr in env_ptrs.iter_mut() {
        let offset = *ptr as usize;
        *ptr = unsafe { base_ptr.add(offset) };
    }

    // Null-terminate the pointer array
    env_ptrs.push(core::ptr::null());

    (env_data, env_ptrs)
}

// Placeholders for stub runner (will be replaced in final binary)
const ARG_SIZE: usize = 256;

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARGC_PLACEHOLDER: [u8; 32] = *b"@@RUNFILES_ARGC@@\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

#[used]
#[link_section = "__DATA,__runfiles"]
static mut TRANSFORM_FLAGS: [u8; 32] = *b"@@RUNFILES_TRANSFORM_FLAGS@@\0\0\0\0";

#[used]
#[link_section = "__DATA,__runfiles"]
static mut EXPORT_RUNFILES_ENV: [u8; 32] = *b"@@RUNFILES_EXPORT_ENV@@\0\0\0\0\0\0\0\0\0";

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARG0_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARG1_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARG2_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARG3_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARG4_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARG5_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARG6_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARG7_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARG8_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = "__DATA,__runfiles"]
static mut ARG9_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

// Get the length of a null-terminated string
fn strlen(s: &[u8]) -> usize {
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

#[no_mangle]
pub extern "C" fn main(runtime_argc: i32, runtime_argv: *const *const u8) -> ! {
    unsafe {
        // Check if ARGC is still a placeholder
        if is_template_placeholder(&ARGC_PLACEHOLDER) {
            print(b"ERROR: This is a template stub runner.\n");
            print(b"You must finalize it by replacing the placeholders before use.\n");
            print(b"The ARGC_PLACEHOLDER has not been replaced.\n");
            exit(1);
        }

        // Parse argc from placeholder
        let argc_str = &ARGC_PLACEHOLDER;
        let argc_len = strlen(argc_str);
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
        let flags_len = strlen(flags_str);
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

        // Parse export_runfiles_env flag (defaults to true)
        let export_str = &EXPORT_RUNFILES_ENV;
        let export_len = strlen(export_str);
        let export_runfiles_env = if !is_template_placeholder(export_str) && export_len > 0 {
            // Parse as "1" (true) or "0" (false)
            export_str[0] != b'0'
        } else {
            true // Default to true
        };

        // Check if any arguments need transformation
        let argc_mask = if argc >= 32 {
            0xFFFFFFFF
        } else {
            (1u32 << argc) - 1
        };
        let needs_transform = (transform_flags & argc_mask) != 0;
        let needs_runfiles = needs_transform || export_runfiles_env;

        // Get executable path from runtime argv[0] for runfiles fallback
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
        let mut resolved_paths: Vec<Vec<u8>> = Vec::with_capacity(128);

        // Resolve embedded arguments
        for i in 0..argc {
            let arg_data = arg_placeholders[i];
            let arg_len = strlen(arg_data);

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
            for i in 1..runtime_argc as usize {
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
            // Return environ directly wrapped in expected format
            let mut ptrs = Vec::new();
            let mut env_ptr = environ;
            while !(*env_ptr).is_null() {
                ptrs.push(*env_ptr);
                env_ptr = env_ptr.add(1);
            }
            ptrs.push(core::ptr::null());
            (Vec::new(), ptrs)
        };

        // Execute the target program
        let ret = execve(executable, resolved_ptrs.as_ptr(), env_ptrs.as_ptr());

        // If execve returns, it failed
        // On macOS, libc's execve() returns -1 on failure and sets errno
        // We need to read errno to get the actual error code
        let errno = get_errno();
        print(b"ERROR: execve failed with errno ");
        print_number(errno as usize);
        print(b" (return code ");
        print_number((-ret) as usize);
        print(b")\n");
        exit(1);
    }
}
