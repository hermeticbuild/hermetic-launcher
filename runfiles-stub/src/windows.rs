// Windows-specific implementation using Windows API
// Uses kernel32.dll functions

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
    unsafe { ExitProcess(1) }
}

// Windows API types
type DWORD = u32;
type BOOL = i32;
type HANDLE = *mut core::ffi::c_void;
type LPVOID = *mut core::ffi::c_void;
type LPCSTR = *const u8;
type LPSTR = *mut u8;

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const STD_OUTPUT_HANDLE: DWORD = 0xFFFFFFF5u32;
const GENERIC_READ: DWORD = 0x80000000;
const OPEN_EXISTING: DWORD = 3;
const FILE_ATTRIBUTE_NORMAL: DWORD = 0x80;
const INFINITE: DWORD = 0xFFFFFFFF;
const CREATE_UNICODE_ENVIRONMENT: DWORD = 0x00000400;

// STARTUPINFOW structure (wide char version for CreateProcessW)
#[repr(C)]
struct STARTUPINFOW {
    cb: DWORD,
    lpReserved: *mut u16,
    lpDesktop: *mut u16,
    lpTitle: *mut u16,
    dwX: DWORD,
    dwY: DWORD,
    dwXSize: DWORD,
    dwYSize: DWORD,
    dwXCountChars: DWORD,
    dwYCountChars: DWORD,
    dwFillAttribute: DWORD,
    dwFlags: DWORD,
    wShowWindow: u16,
    cbReserved2: u16,
    lpReserved2: *mut u8,
    hStdInput: HANDLE,
    hStdOutput: HANDLE,
    hStdError: HANDLE,
}

// PROCESS_INFORMATION structure
#[repr(C)]
struct PROCESS_INFORMATION {
    hProcess: HANDLE,
    hThread: HANDLE,
    dwProcessId: DWORD,
    dwThreadId: DWORD,
}

// External Windows API functions (kernel32.dll)
extern "system" {
    fn ExitProcess(exit_code: u32) -> !;
    fn GetStdHandle(nStdHandle: DWORD) -> HANDLE;
    fn WriteFile(
        hFile: HANDLE,
        lpBuffer: *const u8,
        nNumberOfBytesToWrite: DWORD,
        lpNumberOfBytesWritten: *mut DWORD,
        lpOverlapped: LPVOID,
    ) -> BOOL;
    fn CreateFileA(
        lpFileName: LPCSTR,
        dwDesiredAccess: DWORD,
        dwShareMode: DWORD,
        lpSecurityAttributes: LPVOID,
        dwCreationDisposition: DWORD,
        dwFlagsAndAttributes: DWORD,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    fn ReadFile(
        hFile: HANDLE,
        lpBuffer: LPVOID,
        nNumberOfBytesToRead: DWORD,
        lpNumberOfBytesRead: *mut DWORD,
        lpOverlapped: LPVOID,
    ) -> BOOL;
    fn CloseHandle(hObject: HANDLE) -> BOOL;
    fn GetEnvironmentVariableA(lpName: LPCSTR, lpBuffer: LPSTR, nSize: DWORD) -> DWORD;
    fn CreateProcessW(
        lpApplicationName: *const u16,
        lpCommandLine: *mut u16,
        lpProcessAttributes: LPVOID,
        lpThreadAttributes: LPVOID,
        bInheritHandles: BOOL,
        dwCreationFlags: DWORD,
        lpEnvironment: LPVOID,
        lpCurrentDirectory: *const u16,
        lpStartupInfo: *mut STARTUPINFOW,
        lpProcessInformation: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn GetCommandLineW() -> *const u16;
    fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: DWORD) -> DWORD;
    fn GetExitCodeProcess(hProcess: HANDLE, lpExitCode: *mut DWORD) -> BOOL;
}

// We don't use CommandLineToArgvW to avoid shell32.dll dependency
// Instead we implement custom command-line parsing following Windows rules

// Parse Windows command line into arguments
// Returns number of arguments parsed (excluding argv[0])
// Stores argument pointers in output array
fn parse_command_line(
    cmdline: *const u16,
    argv_out: &mut [*const u16; 128],
    argv_len_out: &mut [usize; 128],
) -> usize {
    unsafe {
        let mut pos = 0usize;
        let mut argc = 0usize;

        // Skip leading whitespace
        while *cmdline.add(pos) != 0 && (*cmdline.add(pos) == b' ' as u16 || *cmdline.add(pos) == b'\t' as u16) {
            pos += 1;
        }

        // Skip argv[0] (executable path)
        let quoted = *cmdline.add(pos) == b'"' as u16;
        if quoted {
            pos += 1; // Skip opening quote
            while *cmdline.add(pos) != 0 && *cmdline.add(pos) != b'"' as u16 {
                pos += 1;
            }
            if *cmdline.add(pos) == b'"' as u16 {
                pos += 1; // Skip closing quote
            }
        } else {
            while *cmdline.add(pos) != 0 && *cmdline.add(pos) != b' ' as u16 && *cmdline.add(pos) != b'\t' as u16 {
                pos += 1;
            }
        }

        // Parse remaining arguments
        while *cmdline.add(pos) != 0 && argc < 128 {
            // Skip whitespace
            while *cmdline.add(pos) != 0 && (*cmdline.add(pos) == b' ' as u16 || *cmdline.add(pos) == b'\t' as u16) {
                pos += 1;
            }

            if *cmdline.add(pos) == 0 {
                break;
            }

            // Start of argument
            let arg_start = pos;
            let in_quotes = *cmdline.add(pos) == b'"' as u16;

            if in_quotes {
                pos += 1; // Skip opening quote
                // Find closing quote
                while *cmdline.add(pos) != 0 && *cmdline.add(pos) != b'"' as u16 {
                    pos += 1;
                }
                // Store argument (skip quotes in length calculation)
                argv_out[argc] = cmdline.add(arg_start + 1);
                argv_len_out[argc] = pos - arg_start - 1;

                if *cmdline.add(pos) == b'"' as u16 {
                    pos += 1; // Skip closing quote
                }
            } else {
                // Unquoted argument - find whitespace
                while *cmdline.add(pos) != 0 && *cmdline.add(pos) != b' ' as u16 && *cmdline.add(pos) != b'\t' as u16 {
                    pos += 1;
                }
                argv_out[argc] = cmdline.add(arg_start);
                argv_len_out[argc] = pos - arg_start;
            }

            argc += 1;
        }

        argc
    }
}

// String utilities
fn print(s: &[u8]) {
    unsafe {
        let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut written: DWORD = 0;
        WriteFile(
            stdout,
            s.as_ptr(),
            s.len() as DWORD,
            &mut written,
            core::ptr::null_mut(),
        );
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

// Environment variable reading - returns String
fn get_env_var(name: &[u8]) -> Option<String> {
    unsafe {
        // Ensure name is null-terminated
        let mut name_with_null = name.to_vec();
        name_with_null.push(0);

        // First call to get required size
        let size = GetEnvironmentVariableA(
            name_with_null.as_ptr(),
            core::ptr::null_mut(),
            0,
        );

        if size == 0 {
            return None;
        }

        // Allocate buffer and get value
        let mut buf = vec![0u8; size as usize];
        let actual_size = GetEnvironmentVariableA(
            name_with_null.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as DWORD,
        );

        if actual_size > 0 && actual_size < buf.len() as DWORD {
            buf.truncate(actual_size as usize);
            // Convert to String, returning None if not valid UTF-8
            String::from_utf8(buf).ok()
        } else {
            None
        }
    }
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
        // Ensure path is null-terminated
        let mut path_with_null = path.to_vec();
        path_with_null.push(0);

        let handle = CreateFileA(
            path_with_null.as_ptr(),
            GENERIC_READ,
            0,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        );

        if handle == INVALID_HANDLE_VALUE {
            return None;
        }

        // Read file into Vec, reading in chunks
        let mut file_data = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let mut bytes_read: DWORD = 0;
            let success = ReadFile(
                handle,
                chunk.as_mut_ptr() as LPVOID,
                chunk.len() as DWORD,
                &mut bytes_read,
                core::ptr::null_mut(),
            );
            if success == 0 || bytes_read == 0 {
                break;
            }
            file_data.extend_from_slice(&chunk[..bytes_read as usize]);
        }
        CloseHandle(handle);

        if file_data.is_empty() {
            return None;
        }

        // Convert to UTF-8 string for easier parsing
        let file_str = String::from_utf8(file_data).ok()?;

        let mut manifest = Manifest::new();

        for line in file_str.lines() {
            // lines() automatically strips \r\n endings
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
        // Step 1: Try RUNFILES_MANIFEST_FILE envvar first
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

        // Step 2: Try RUNFILES_DIR envvar
        if let Some(runfiles_dir) = get_env_var(b"RUNFILES_DIR") {
            if !runfiles_dir.is_empty() {
                return Some(Self {
                    mode: RunfilesMode::DirectoryBased(runfiles_dir.clone()),
                    manifest_path: None,
                    dir_path: Some(runfiles_dir),
                });
            }
        }

        // Step 3: Try to find runfiles next to the executable
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

                // Add null terminator for CreateFileA
                let mut dir_with_null = Vec::from(runfiles_dir.as_bytes());
                dir_with_null.push(0);

                // Check if directory exists by trying to open it
                unsafe {
                    const FILE_FLAG_BACKUP_SEMANTICS: DWORD = 0x02000000;  // Needed to open directories
                    let handle = CreateFileA(
                        dir_with_null.as_ptr(),
                        GENERIC_READ,
                        0,
                        core::ptr::null_mut(),
                        OPEN_EXISTING,
                        FILE_FLAG_BACKUP_SEMANTICS,
                        core::ptr::null_mut(),
                    );
                    if handle != INVALID_HANDLE_VALUE {
                        CloseHandle(handle);
                        return Some(Self {
                            mode: RunfilesMode::DirectoryBased(runfiles_dir.clone()),
                            manifest_path: None,
                            dir_path: Some(runfiles_dir),
                        });
                    }
                }
            }
        }

        None
    }

    fn rlocation(&self, path: &str) -> Option<String> {
        // If path is absolute (Windows: starts with drive letter or \\), don't resolve
        let path_bytes = path.as_bytes();
        if path_bytes.len() >= 2 && ((path_bytes[0].is_ascii_alphabetic() && path_bytes[1] == b':') || (path_bytes[0] == b'\\' && path_bytes[1] == b'\\')) {
            return None;
        }

        match &self.mode {
            RunfilesMode::ManifestBased(manifest) => {
                if let Some(resolved) = manifest.lookup(path) {
                    // Convert forward slashes to backslashes
                    return Some(resolved.replace('/', "\\"));
                }
                None
            }
            RunfilesMode::DirectoryBased(dir) => {
                let mut result = dir.clone();

                // Add separator if needed
                if !result.ends_with('\\') && !result.ends_with('/') {
                    result.push('\\');
                }

                // Append path, converting forward slashes to backslashes
                result.push_str(&path.replace('/', "\\"));

                Some(result)
            }
        }
    }
}

// Environment building for export mode
// Windows environments can be large (32KB+), use 128KB to be safe
const MAX_ENV_SIZE: usize = 131072;

// External Windows API function for environment access
extern "system" {
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(lpszEnvironmentBlock: *mut u16) -> BOOL;
}

static mut MODIFIED_ENV_DATA: [u16; MAX_ENV_SIZE / 2] = [0; MAX_ENV_SIZE / 2];

fn build_runfiles_environ(runfiles: Option<&Runfiles>) -> *mut core::ffi::c_void {
    unsafe {
        // Windows requires environment variables to be sorted alphabetically
        // GetEnvironmentStringsW() already returns sorted environment
        // We need to maintain sorted order when adding our variables

        let mut data_pos = 0usize;
        let max_pos = MODIFIED_ENV_DATA.len();

        // Helper to check bounds before writing
        let check_bounds = |pos: usize, needed: usize| -> bool {
            pos + needed <= max_pos
        };

        // Copy existing environment and insert runfiles vars in correct sorted position
        let env_block = GetEnvironmentStringsW();
        if env_block.is_null() {
            // No parent environment, just add runfiles vars in sorted order
            let mut add_env = |key: &[u8], value: &str| -> bool {
                let value_bytes = value.as_bytes();
                let total_len = key.len() + 1 + value_bytes.len() + 1; // key + '=' + value + '\0'
                if !check_bounds(data_pos, total_len) {
                    return false;
                }

                for &b in key {
                    MODIFIED_ENV_DATA[data_pos] = b as u16;
                    data_pos += 1;
                }
                MODIFIED_ENV_DATA[data_pos] = b'=' as u16;
                data_pos += 1;
                for &b in value_bytes {
                    MODIFIED_ENV_DATA[data_pos] = b as u16;
                    data_pos += 1;
                }
                MODIFIED_ENV_DATA[data_pos] = 0;
                data_pos += 1;
                true
            };

            if let Some(rf) = runfiles {
                if let Some(ref path) = rf.dir_path {
                    if !add_env(b"JAVA_RUNFILES", path) {
                        print(b"ERROR: Failed to add JAVA_RUNFILES to environment\r\n");
                        print(b"Environment buffer limit exceeded. Total size limit: ");
                        print_number(MAX_ENV_SIZE);
                        print(b" bytes\r\n");
                        ExitProcess(1);
                    }
                    if !add_env(b"RUNFILES_DIR", path) {
                        print(b"ERROR: Failed to add RUNFILES_DIR to environment\r\n");
                        print(b"Environment buffer limit exceeded. Total size limit: ");
                        print_number(MAX_ENV_SIZE);
                        print(b" bytes\r\n");
                        ExitProcess(1);
                    }
                }
                if let Some(ref path) = rf.manifest_path {
                    if !add_env(b"RUNFILES_MANIFEST_FILE", path) {
                        print(b"ERROR: Failed to add RUNFILES_MANIFEST_FILE to environment\r\n");
                        print(b"Environment buffer limit exceeded. Total size limit: ");
                        print_number(MAX_ENV_SIZE);
                        print(b" bytes\r\n");
                        ExitProcess(1);
                    }
                }
            }
        } else {
            // Iterate through existing environment and insert runfiles vars at correct position
            let mut pos = 0;
            let mut java_runfiles_inserted = false;
            let mut runfiles_dir_inserted = false;
            let mut runfiles_manifest_inserted = false;
            let mut env_dropped = false;

            loop {
                let entry_start = pos;
                while *env_block.add(pos) != 0 {
                    pos += 1;
                    if pos > 65536 { break; } // safety: MAX_ENV_SIZE / 2
                }

                let entry_len = pos - entry_start;
                if entry_len == 0 { break; }

                let entry_ptr = env_block.add(entry_start);

                // Check if we should skip existing runfiles vars
                let should_skip =
                    (entry_len > 23 && {
                        let mut matches = true;
                        for i in 0..23 {
                            if *entry_ptr.add(i) != b"RUNFILES_MANIFEST_FILE="[i] as u16 {
                                matches = false;
                                break;
                            }
                        }
                        matches
                    }) ||
                    (entry_len > 13 && {
                        let mut matches = true;
                        for i in 0..13 {
                            if *entry_ptr.add(i) != b"RUNFILES_DIR="[i] as u16 {
                                matches = false;
                                break;
                            }
                        }
                        matches
                    }) ||
                    (entry_len > 14 && {
                        let mut matches = true;
                        for i in 0..14 {
                            if *entry_ptr.add(i) != b"JAVA_RUNFILES="[i] as u16 {
                                matches = false;
                                break;
                            }
                        }
                        matches
                    });

                if !should_skip {
                    // Helper to compare var name with a target name (case-insensitive, stops at '=')
                    let var_comes_after = |target: &[u8]| -> bool {
                        for i in 0..target.len().min(entry_len) {
                            let entry_char = *entry_ptr.add(i);
                            let target_char = target[i] as u16;

                            // Convert both to uppercase for case-insensitive comparison
                            let entry_upper = if entry_char >= b'a' as u16 && entry_char <= b'z' as u16 {
                                entry_char - 32
                            } else {
                                entry_char
                            };
                            let target_upper = if target_char >= b'a' as u16 && target_char <= b'z' as u16 {
                                target_char - 32
                            } else {
                                target_char
                            };

                            if entry_upper != target_upper {
                                return entry_upper > target_upper;
                            }
                        }
                        entry_len > target.len()
                    };

                    // Insert JAVA_RUNFILES if needed
                    if !java_runfiles_inserted && var_comes_after(b"JAVA_RUNFILES") {
                        if let Some(rf) = runfiles {
                            if let Some(ref path) = rf.dir_path {
                                let path_bytes = path.as_bytes();
                                let total_len = 14 + path_bytes.len() + 1; // "JAVA_RUNFILES=" + value + '\0'
                                if !check_bounds(data_pos, total_len) {
                                    env_dropped = true;
                                } else {
                                    for &b in b"JAVA_RUNFILES=" {
                                        MODIFIED_ENV_DATA[data_pos] = b as u16;
                                        data_pos += 1;
                                    }
                                    for &b in path_bytes {
                                        MODIFIED_ENV_DATA[data_pos] = b as u16;
                                        data_pos += 1;
                                    }
                                    MODIFIED_ENV_DATA[data_pos] = 0;
                                    data_pos += 1;
                                }
                            }
                        }
                        java_runfiles_inserted = true;
                    }

                    // Insert RUNFILES_DIR if needed
                    if !runfiles_dir_inserted && var_comes_after(b"RUNFILES_DIR") {
                        if let Some(rf) = runfiles {
                            if let Some(ref path) = rf.dir_path {
                                let path_bytes = path.as_bytes();
                                let total_len = 13 + path_bytes.len() + 1; // "RUNFILES_DIR=" + value + '\0'
                                if !check_bounds(data_pos, total_len) {
                                    env_dropped = true;
                                } else {
                                    for &b in b"RUNFILES_DIR=" {
                                        MODIFIED_ENV_DATA[data_pos] = b as u16;
                                        data_pos += 1;
                                    }
                                    for &b in path_bytes {
                                        MODIFIED_ENV_DATA[data_pos] = b as u16;
                                        data_pos += 1;
                                    }
                                    MODIFIED_ENV_DATA[data_pos] = 0;
                                    data_pos += 1;
                                }
                            }
                        }
                        runfiles_dir_inserted = true;
                    }

                    // Insert RUNFILES_MANIFEST_FILE if needed
                    if !runfiles_manifest_inserted && var_comes_after(b"RUNFILES_MANIFEST_FILE") {
                        if let Some(rf) = runfiles {
                            if let Some(ref path) = rf.manifest_path {
                                let path_bytes = path.as_bytes();
                                let total_len = 23 + path_bytes.len() + 1; // "RUNFILES_MANIFEST_FILE=" + value + '\0'
                                if !check_bounds(data_pos, total_len) {
                                    env_dropped = true;
                                } else {
                                    for &b in b"RUNFILES_MANIFEST_FILE=" {
                                        MODIFIED_ENV_DATA[data_pos] = b as u16;
                                        data_pos += 1;
                                    }
                                    for &b in path_bytes {
                                        MODIFIED_ENV_DATA[data_pos] = b as u16;
                                        data_pos += 1;
                                    }
                                    MODIFIED_ENV_DATA[data_pos] = 0;
                                    data_pos += 1;
                                }
                            }
                        }
                        runfiles_manifest_inserted = true;
                    }

                    // Copy this environment variable
                    if data_pos + entry_len + 1 <= MODIFIED_ENV_DATA.len() {
                        for i in 0..entry_len {
                            MODIFIED_ENV_DATA[data_pos + i] = *entry_ptr.add(i);
                        }
                        MODIFIED_ENV_DATA[data_pos + entry_len] = 0;
                        data_pos += entry_len + 1;
                    } else {
                        env_dropped = true;
                    }
                }

                pos += 1;
            }

            // Add any remaining runfiles vars that weren't inserted yet
            if !java_runfiles_inserted {
                if let Some(rf) = runfiles {
                    if let Some(ref path) = rf.dir_path {
                        let path_bytes = path.as_bytes();
                        let total_len = 14 + path_bytes.len() + 1;
                        if !check_bounds(data_pos, total_len) {
                            env_dropped = true;
                        } else {
                            for &b in b"JAVA_RUNFILES=" {
                                MODIFIED_ENV_DATA[data_pos] = b as u16;
                                data_pos += 1;
                            }
                            for &b in path_bytes {
                                MODIFIED_ENV_DATA[data_pos] = b as u16;
                                data_pos += 1;
                            }
                            MODIFIED_ENV_DATA[data_pos] = 0;
                            data_pos += 1;
                        }
                    }
                }
            }
            if !runfiles_dir_inserted {
                if let Some(rf) = runfiles {
                    if let Some(ref path) = rf.dir_path {
                        let path_bytes = path.as_bytes();
                        let total_len = 13 + path_bytes.len() + 1;
                        if !check_bounds(data_pos, total_len) {
                            env_dropped = true;
                        } else {
                            for &b in b"RUNFILES_DIR=" {
                                MODIFIED_ENV_DATA[data_pos] = b as u16;
                                data_pos += 1;
                            }
                            for &b in path_bytes {
                                MODIFIED_ENV_DATA[data_pos] = b as u16;
                                data_pos += 1;
                            }
                            MODIFIED_ENV_DATA[data_pos] = 0;
                            data_pos += 1;
                        }
                    }
                }
            }
            if !runfiles_manifest_inserted {
                if let Some(rf) = runfiles {
                    if let Some(ref path) = rf.manifest_path {
                        let path_bytes = path.as_bytes();
                        let total_len = 23 + path_bytes.len() + 1;
                        if !check_bounds(data_pos, total_len) {
                            env_dropped = true;
                        } else {
                            for &b in b"RUNFILES_MANIFEST_FILE=" {
                                MODIFIED_ENV_DATA[data_pos] = b as u16;
                                data_pos += 1;
                            }
                            for &b in path_bytes {
                                MODIFIED_ENV_DATA[data_pos] = b as u16;
                                data_pos += 1;
                            }
                            MODIFIED_ENV_DATA[data_pos] = 0;
                            data_pos += 1;
                        }
                    }
                }
            }

            // Check if any environment variables were dropped
            if env_dropped {
                FreeEnvironmentStringsW(env_block);
                print(b"ERROR: Failed to copy all environment variables\r\n");
                print(b"Environment buffer limit exceeded. Total size limit: ");
                print_number(MAX_ENV_SIZE);
                print(b" bytes\r\n");
                print(b"Current usage: ");
                print_number(data_pos * 2); // *2 because it's u16 array
                print(b" bytes\r\n");
                print(b"Consider reducing the number or size of environment variables.\r\n");
                ExitProcess(1);
            }

            FreeEnvironmentStringsW(env_block);
        }

        // Add double null terminator to mark end of environment block
        if data_pos < MODIFIED_ENV_DATA.len() {
            MODIFIED_ENV_DATA[data_pos] = 0;
            data_pos += 1;
        }
        if data_pos < MODIFIED_ENV_DATA.len() {
            MODIFIED_ENV_DATA[data_pos] = 0;
        }

        MODIFIED_ENV_DATA.as_mut_ptr() as *mut core::ffi::c_void
    }
}

// Placeholders for stub runner (will be replaced in final binary)
const ARG_SIZE: usize = 256;

#[used]
#[link_section = ".runfiles"]
static mut ARGC_PLACEHOLDER: [u8; 32] = *b"@@RUNFILES_ARGC@@\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

#[used]
#[link_section = ".runfiles"]
static mut TRANSFORM_FLAGS: [u8; 32] = *b"@@RUNFILES_TRANSFORM_FLAGS@@\0\0\0\0";

#[used]
#[link_section = ".runfiles"]
static mut EXPORT_RUNFILES_ENV: [u8; 32] = *b"@@RUNFILES_EXPORT_ENV@@\0\0\0\0\0\0\0\0\0";

#[used]
#[link_section = ".runfiles"]
static mut ARG0_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles"]
static mut ARG1_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles"]
static mut ARG2_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles"]
static mut ARG3_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles"]
static mut ARG4_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles"]
static mut ARG5_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles"]
static mut ARG6_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles"]
static mut ARG7_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles"]
static mut ARG8_PLACEHOLDER: [u8; ARG_SIZE] = [b'@'; ARG_SIZE];

#[used]
#[link_section = ".runfiles"]
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
pub extern "C" fn main() -> ! {
    unsafe {
        // Get command line
        let cmdline = GetCommandLineW();

        // Parse runtime arguments using custom parser (no shell32.dll needed)
        let mut runtime_argv: [*const u16; 128] = [core::ptr::null(); 128];
        let mut runtime_argv_len: [usize; 128] = [0; 128];
        let runtime_args_count = parse_command_line(cmdline, &mut runtime_argv, &mut runtime_argv_len);

        // Check if ARGC is still a placeholder
        if is_template_placeholder(&ARGC_PLACEHOLDER) {
            print(b"ERROR: This is a template stub runner.\r\n");
            print(b"You must finalize it by replacing the placeholders before use.\r\n");
            print(b"The ARGC_PLACEHOLDER has not been replaced.\r\n");
            ExitProcess(1);
        }

        // Parse argc from placeholder
        let argc_str = &ARGC_PLACEHOLDER;
        let argc_len = strlen(argc_str);
        if argc_len == 0 {
            print(b"ERROR: ARGC is empty\r\n");
            ExitProcess(1);
        }

        // Parse argc as decimal number
        let mut argc: usize = 0;
        for i in 0..argc_len {
            let c = argc_str[i];
            if c >= b'0' && c <= b'9' {
                argc = argc * 10 + (c - b'0') as usize;
            } else {
                print(b"ERROR: ARGC contains non-digit characters\r\n");
                ExitProcess(1);
            }
        }

        if argc == 0 || argc > 10 {
            print(b"ERROR: Invalid argc (must be 1-10)\r\n");
            ExitProcess(1);
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
                    print(b"ERROR: TRANSFORM_FLAGS contains non-digit characters\r\n");
                    ExitProcess(1);
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

        // Parse argv[0] from command line manually
        // Command line format: either "path\to\exe" args... or path\to\exe args...
        // We extract the first token (argv[0]) for runfiles fallback
        let mut exe_path_buf = Vec::new();
        let mut pos = 0usize;

        // Skip leading whitespace
        while *cmdline.add(pos) != 0 && (*cmdline.add(pos) == b' ' as u16 || *cmdline.add(pos) == b'\t' as u16) {
            pos += 1;
        }

        // Check if first char is a quote
        let quoted = *cmdline.add(pos) == b'"' as u16;
        if quoted {
            pos += 1; // Skip opening quote
        }

        // Extract argv[0] (with 1MB safety limit)
        while exe_path_buf.len() < 1048576 && *cmdline.add(pos) != 0 {
            let wchar = *cmdline.add(pos);

            // Check for end of argv[0]
            if quoted {
                if wchar == b'"' as u16 {
                    break; // End of quoted string
                }
            } else {
                if wchar == b' ' as u16 || wchar == b'\t' as u16 {
                    break; // End of unquoted string
                }
            }

            // Simple UTF-16 to ASCII conversion
            exe_path_buf.push((wchar & 0xFF) as u8);
            pos += 1;
        }

        let executable_path: Option<&[u8]> = if !exe_path_buf.is_empty() {
            Some(&exe_path_buf)
        } else {
            None
        };

        // Initialize runfiles only if needed
        let runfiles = if needs_runfiles {
            if let Some(rf) = Runfiles::create(executable_path) {
                Some(rf)
            } else {
                print(b"ERROR: Failed to initialize runfiles\r\n");
                print(b"Set RUNFILES_DIR or RUNFILES_MANIFEST_FILE, or ensure <executable>.runfiles\\ directory exists\r\n");
                ExitProcess(1);
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
                print(b" is empty\r\n");
                ExitProcess(1);
            }

            let arg_slice = &arg_data[..arg_len];

            // Check if this argument should be transformed
            let should_transform = (transform_flags & (1 << i)) != 0;

            let resolved = if should_transform {
                // Try to resolve through runfiles
                if let Some(ref rf) = runfiles {
                    // Convert argument to &str for rlocation (Bazel args are UTF-8)
                    if let Ok(arg_str) = core::str::from_utf8(arg_slice) {
                        if let Some(resolved_str) = rf.rlocation(arg_str) {
                            // Convert back to bytes
                            Vec::from(resolved_str.as_bytes())
                        } else {
                            // If not found in runfiles, use the path as-is
                            arg_slice.to_vec()
                        }
                    } else {
                        // Not valid UTF-8, use as-is
                        arg_slice.to_vec()
                    }
                } else {
                    // Use path as-is
                    arg_slice.to_vec()
                }
            } else {
                // Use path as-is without transformation
                arg_slice.to_vec()
            };

            resolved_paths.push(resolved);
        }

        // Build command line for CreateProcessW (UTF-16)
        // Command line includes embedded args + runtime args
        let mut cmdline_wide: Vec<u16> = Vec::with_capacity(8192);

        // Add embedded arguments (convert from UTF-8 to UTF-16)
        for i in 0..argc {
            let arg_slice = &resolved_paths[i];

            // Always quote the first argument (executable path) following Bazel's approach
            // For other arguments, only quote if they contain spaces
            let needs_quotes = i == 0 || find_byte(arg_slice, b' ').is_some();

            if needs_quotes {
                cmdline_wide.push(b'"' as u16);
            }

            // Convert UTF-8 to UTF-16 and copy
            for &b in arg_slice {
                cmdline_wide.push(b as u16);
            }

            if needs_quotes {
                cmdline_wide.push(b'"' as u16);
            }

            // Add space between arguments
            if i < argc - 1 || runtime_args_count > 0 {
                cmdline_wide.push(b' ' as u16);
            }
        }

        // Add runtime arguments (already UTF-16, just copy)
        for i in 0..runtime_args_count {
            let runtime_arg = runtime_argv[i];
            let arg_len = runtime_argv_len[i];

            // Check if we need quotes (scan for spaces)
            let mut needs_quotes = false;
            for j in 0..arg_len {
                if *runtime_arg.add(j) == b' ' as u16 {
                    needs_quotes = true;
                    break;
                }
            }

            if needs_quotes {
                cmdline_wide.push(b'"' as u16);
            }

            // Copy wide string
            for j in 0..arg_len {
                cmdline_wide.push(*runtime_arg.add(j));
            }

            if needs_quotes {
                cmdline_wide.push(b'"' as u16);
            }

            // Add space between arguments (except after last)
            if i < runtime_args_count - 1 {
                cmdline_wide.push(b' ' as u16);
            }
        }

        // Null-terminate command line
        cmdline_wide.push(0);

        // Build environment with runfiles variables if export is enabled
        let envp = if export_runfiles_env {
            build_runfiles_environ(runfiles.as_ref())
        } else {
            core::ptr::null_mut()
        };

        // Create the process
        let mut si: STARTUPINFOW = core::mem::zeroed();
        si.cb = core::mem::size_of::<STARTUPINFOW>() as DWORD;
        let mut pi: PROCESS_INFORMATION = core::mem::zeroed();

        // Determine creation flags
        // If we have a UTF-16 environment block, we need CREATE_UNICODE_ENVIRONMENT
        let creation_flags = if export_runfiles_env {
            CREATE_UNICODE_ENVIRONMENT
        } else {
            0
        };

        // Use NULL for lpApplicationName and quote the executable in the command line
        // This follows Bazel's launcher.cc approach
        let success = CreateProcessW(
            core::ptr::null(),          // Application name (NULL - parsed from command line)
            cmdline_wide.as_mut_ptr(),  // Command line (UTF-16) - quoted executable + args
            core::ptr::null_mut(),      // Process attributes
            core::ptr::null_mut(),      // Thread attributes
            1,                          // Inherit handles
            creation_flags,             // Creation flags (with CREATE_UNICODE_ENVIRONMENT if needed)
            envp,                       // Environment
            core::ptr::null(),          // Current directory
            &mut si,
            &mut pi,
        );

        if success == 0 {
            print(b"ERROR: CreateProcess failed\r\n");
            ExitProcess(1);
        }

        // Wait for the child process to complete
        WaitForSingleObject(pi.hProcess, INFINITE);

        // Get the child process's exit code
        let mut exit_code: DWORD = 0;
        GetExitCodeProcess(pi.hProcess, &mut exit_code);

        // Close handles
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);

        // Exit with the child process's exit code
        ExitProcess(exit_code);
    }
}
