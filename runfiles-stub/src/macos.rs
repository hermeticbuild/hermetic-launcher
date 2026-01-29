// macOS-specific implementation using libc
// Unlike Linux version, this uses libc and can link with libsystem

use core::panic::PanicInfo;

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

// Environment variable reading via the environ pointer
fn get_env_var(name: &[u8], buf: &mut [u8]) -> Option<usize> {
    unsafe {
        let mut env_ptr = environ;

        // Iterate through environment variables
        while !(*env_ptr).is_null() {
            let entry_ptr = *env_ptr;

            // Find the length of this environment variable string
            let mut len = 0;
            while *entry_ptr.add(len) != 0 {
                len += 1;
                if len > 4096 {  // Safety limit
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
                    let copy_len = value.len().min(buf.len());
                    buf[..copy_len].copy_from_slice(&value[..copy_len]);
                    return Some(copy_len);
                }
            }

            env_ptr = env_ptr.add(1);
        }
    }

    None
}

// Manifest entry storage
const MAX_ENTRIES: usize = 1024;
const MAX_PATH_LEN: usize = 256;

struct ManifestEntry {
    key: [u8; MAX_PATH_LEN],
    key_len: usize,
    value: [u8; MAX_PATH_LEN],
    value_len: usize,
}

struct Manifest {
    entries: [ManifestEntry; MAX_ENTRIES],
    count: usize,
}

impl Manifest {
    fn new() -> Self {
        const EMPTY_ENTRY: ManifestEntry = ManifestEntry {
            key: [0; MAX_PATH_LEN],
            key_len: 0,
            value: [0; MAX_PATH_LEN],
            value_len: 0,
        };

        Self {
            entries: [EMPTY_ENTRY; MAX_ENTRIES],
            count: 0,
        }
    }

    fn add_entry(&mut self, key: &[u8], value: &[u8]) {
        if self.count >= MAX_ENTRIES {
            return;
        }

        let entry = &mut self.entries[self.count];
        let key_len = key.len().min(MAX_PATH_LEN);
        let value_len = value.len().min(MAX_PATH_LEN);

        entry.key[..key_len].copy_from_slice(&key[..key_len]);
        entry.key_len = key_len;
        entry.value[..value_len].copy_from_slice(&value[..value_len]);
        entry.value_len = value_len;

        self.count += 1;
    }

    fn lookup(&self, key: &[u8]) -> Option<&[u8]> {
        for i in 0..self.count {
            let entry = &self.entries[i];
            if str_eq(&entry.key[..entry.key_len], key) {
                return Some(&entry.value[..entry.value_len]);
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

        let mut file_buf = [0u8; 65536];
        let bytes_read = read(fd, file_buf.as_mut_ptr(), file_buf.len());
        close(fd);

        if bytes_read <= 0 {
            return None;
        }

        let mut manifest = Manifest::new();
        let data = &file_buf[..bytes_read as usize];
        let mut pos = 0;

        while pos < data.len() {
            let line_start = pos;
            while pos < data.len() && data[pos] != b'\n' {
                pos += 1;
            }

            let line = &data[line_start..pos];

            if let Some(space_pos) = find_byte(line, b' ') {
                let key = &line[..space_pos];
                let value = &line[space_pos + 1..];
                manifest.add_entry(key, value);
            }

            pos += 1;
        }

        Some(manifest)
    }
}

// Runfiles implementation
enum RunfilesMode {
    ManifestBased(Manifest),
    DirectoryBased([u8; MAX_PATH_LEN], usize),
}

struct Runfiles {
    mode: RunfilesMode,
    // Paths for environment variables (when export_runfiles_env is true)
    manifest_path: Option<([u8; MAX_PATH_LEN], usize)>, // RUNFILES_MANIFEST_FILE
    dir_path: Option<([u8; MAX_PATH_LEN], usize)>,      // RUNFILES_DIR and JAVA_RUNFILES
}

impl Runfiles {
    fn create(executable_path: Option<&[u8]>) -> Option<Self> {
        let mut manifest_path = [0u8; MAX_PATH_LEN];

        // Try RUNFILES_MANIFEST_FILE first
        if let Some(len) = get_env_var(b"RUNFILES_MANIFEST_FILE", &mut manifest_path) {
            if len > 0 {
                let mut path_with_null = [0u8; MAX_PATH_LEN + 1];
                path_with_null[..len].copy_from_slice(&manifest_path[..len]);

                if let Some(manifest) = load_manifest(&path_with_null[..len + 1]) {
                    return Some(Self {
                        mode: RunfilesMode::ManifestBased(manifest),
                        manifest_path: Some((manifest_path, len)),
                        dir_path: None,
                    });
                }
            }
        }

        // Try RUNFILES_DIR
        let mut runfiles_dir = [0u8; MAX_PATH_LEN];
        if let Some(len) = get_env_var(b"RUNFILES_DIR", &mut runfiles_dir) {
            if len > 0 {
                return Some(Self {
                    mode: RunfilesMode::DirectoryBased(runfiles_dir, len),
                    manifest_path: None,
                    dir_path: Some((runfiles_dir, len)),
                });
            }
        }

        // Try to find runfiles next to the executable
        // Check for <executable>.runfiles_manifest file (preferred)
        // Then check for <executable>.runfiles directory
        if let Some(exe_path) = executable_path {
            let exe_len = strlen(exe_path);
            if exe_len > 0 {
                // Try <executable>.runfiles_manifest file first
                if exe_len + 19 < MAX_PATH_LEN {  // +19 for ".runfiles_manifest\0"
                    let mut manifest_file_path = [0u8; MAX_PATH_LEN + 1];

                    // Copy executable path
                    manifest_file_path[..exe_len].copy_from_slice(&exe_path[..exe_len]);

                    // Append ".runfiles_manifest" (18 characters)
                    manifest_file_path[exe_len..exe_len + 18].copy_from_slice(b".runfiles_manifest");
                    let manifest_file_len = exe_len + 18;

                    // Try to load the manifest file
                    if let Some(manifest) = load_manifest(&manifest_file_path[..manifest_file_len + 1]) {
                        // Also determine the runfiles directory for RUNFILES_DIR envvar
                        // The directory is <executable>.runfiles
                        let mut dir_path = [0u8; MAX_PATH_LEN];
                        let dir_len = if exe_len + 9 < MAX_PATH_LEN {
                            dir_path[..exe_len].copy_from_slice(&exe_path[..exe_len]);
                            dir_path[exe_len..exe_len + 9].copy_from_slice(b".runfiles");
                            exe_len + 9
                        } else {
                            0
                        };

                        let mut manifest_path_without_null = [0u8; MAX_PATH_LEN];
                        manifest_path_without_null[..manifest_file_len].copy_from_slice(&manifest_file_path[..manifest_file_len]);

                        return Some(Self {
                            mode: RunfilesMode::ManifestBased(manifest),
                            manifest_path: Some((manifest_path_without_null, manifest_file_len)),
                            dir_path: if dir_len > 0 { Some((dir_path, dir_len)) } else { None },
                        });
                    }
                }

                // Try <executable>.runfiles directory
                if exe_len + 10 < MAX_PATH_LEN {  // +10 for ".runfiles\0"
                    let mut runfiles_dir = [0u8; MAX_PATH_LEN];

                    // Copy executable path
                    runfiles_dir[..exe_len].copy_from_slice(&exe_path[..exe_len]);

                    // Append ".runfiles"
                    runfiles_dir[exe_len..exe_len + 9].copy_from_slice(b".runfiles");
                    runfiles_dir[exe_len + 9] = 0; // null terminator

                    // Check if directory exists using access() syscall
                    if path_exists(&runfiles_dir[..exe_len + 10]) {
                        return Some(Self {
                            mode: RunfilesMode::DirectoryBased(runfiles_dir, exe_len + 9),
                            manifest_path: None,
                            dir_path: Some((runfiles_dir, exe_len + 9)),
                        });
                    }
                }
            }
        }

        None
    }

    fn rlocation(&self, path: &[u8]) -> Option<[u8; MAX_PATH_LEN]> {
        // If path is absolute, don't resolve through runfiles
        if path.len() > 0 && path[0] == b'/' {
            return None;
        }

        match &self.mode {
            RunfilesMode::ManifestBased(manifest) => {
                if let Some(resolved) = manifest.lookup(path) {
                    let mut result = [0u8; MAX_PATH_LEN];
                    let len = resolved.len().min(MAX_PATH_LEN);
                    result[..len].copy_from_slice(&resolved[..len]);
                    return Some(result);
                }
                None
            }
            RunfilesMode::DirectoryBased(dir, dir_len) => {
                let mut result = [0u8; MAX_PATH_LEN];
                let mut pos = 0;

                // Copy directory
                let copy_len = (*dir_len).min(MAX_PATH_LEN);
                result[..copy_len].copy_from_slice(&dir[..copy_len]);
                pos += copy_len;

                // Add separator if needed
                if pos < MAX_PATH_LEN && pos > 0 && result[pos - 1] != b'/' {
                    result[pos] = b'/';
                    pos += 1;
                }

                // Copy path
                let path_len = path.len().min(MAX_PATH_LEN - pos);
                result[pos..pos + path_len].copy_from_slice(&path[..path_len]);

                Some(result)
            }
        }
    }
}

// Environment building for export mode
// These limits are based on macOS's ARG_MAX, which defines the maximum
// combined size of argv + envp that can be passed to execve(). Modern macOS
// systems (macOS 26.1+) have a 1 MiB limit, older kernels have 256 KiB.
// This limit is fixed (not dynamic like Linux).
// See: sysctl kern.argmax or getconf ARG_MAX
// Reference: https://gist.github.com/malt3/c1439aa16208a74194accb025ab1cc5b
const MAX_ENV_SIZE: usize = 1048576;  // 1 MiB - matches modern macOS ARG_MAX limit
const MAX_ENV_VARS: usize = 1024;     // Max number of environment variables

static mut MODIFIED_ENV_DATA: [u8; MAX_ENV_SIZE] = [0; MAX_ENV_SIZE];
static mut MODIFIED_ENV_PTRS: [*const u8; MAX_ENV_VARS + 1] = [core::ptr::null(); MAX_ENV_VARS + 1];

fn build_runfiles_environ(runfiles: Option<&Runfiles>) -> *const *const u8 {
    unsafe {
        let mut data_pos = 0usize;
        let mut ptr_idx = 0usize;

        // Helper to add an environment variable
        let mut add_env_var = |key: &[u8], value: &[u8]| {
            if ptr_idx >= MAX_ENV_VARS {
                return false;
            }

            let entry_len = key.len() + 1 + value.len() + 1; // "KEY=VALUE\0"
            if data_pos + entry_len > MAX_ENV_SIZE {
                return false;
            }

            let entry_start = data_pos;

            // Copy key
            MODIFIED_ENV_DATA[data_pos..data_pos + key.len()].copy_from_slice(key);
            data_pos += key.len();

            // Add '='
            MODIFIED_ENV_DATA[data_pos] = b'=';
            data_pos += 1;

            // Copy value
            MODIFIED_ENV_DATA[data_pos..data_pos + value.len()].copy_from_slice(value);
            data_pos += value.len();

            // Null terminate
            MODIFIED_ENV_DATA[data_pos] = 0;
            data_pos += 1;

            // Store pointer
            MODIFIED_ENV_PTRS[ptr_idx] = MODIFIED_ENV_DATA.as_ptr().add(entry_start);
            ptr_idx += 1;

            true
        };

        // Add RUNFILES_MANIFEST_FILE if we have it
        if let Some(rf) = runfiles {
            if let Some((ref path, len)) = rf.manifest_path {
                if !add_env_var(b"RUNFILES_MANIFEST_FILE", &path[..len]) {
                    print(b"ERROR: Failed to add RUNFILES_MANIFEST_FILE to environment\n");
                    print(b"Environment buffer limit exceeded. Total size limit: ");
                    print_number(MAX_ENV_SIZE);
                    print(b" bytes, max variables: ");
                    print_number(MAX_ENV_VARS);
                    print(b"\n");
                    exit(1);
                }
            }
        }

        // Add RUNFILES_DIR if we have it
        if let Some(rf) = runfiles {
            if let Some((ref path, len)) = rf.dir_path {
                if !add_env_var(b"RUNFILES_DIR", &path[..len]) {
                    print(b"ERROR: Failed to add RUNFILES_DIR to environment\n");
                    print(b"Environment buffer limit exceeded. Total size limit: ");
                    print_number(MAX_ENV_SIZE);
                    print(b" bytes, max variables: ");
                    print_number(MAX_ENV_VARS);
                    print(b"\n");
                    exit(1);
                }
                if !add_env_var(b"JAVA_RUNFILES", &path[..len]) {
                    print(b"ERROR: Failed to add JAVA_RUNFILES to environment\n");
                    print(b"Environment buffer limit exceeded. Total size limit: ");
                    print_number(MAX_ENV_SIZE);
                    print(b" bytes, max variables: ");
                    print_number(MAX_ENV_VARS);
                    print(b"\n");
                    exit(1);
                }
            }
        }

        // Copy existing environment, filtering out runfiles vars
        let mut env_ptr = environ;
        let mut env_dropped = false;
        while !(*env_ptr).is_null() && ptr_idx < MAX_ENV_VARS {
            let entry_ptr = *env_ptr;

            // Find length of this entry
            let mut len = 0;
            while *entry_ptr.add(len) != 0 && len < 4096 {
                len += 1;
            }

            let entry = core::slice::from_raw_parts(entry_ptr, len);

            // Check if this is a runfiles variable we should skip
            let should_skip = str_starts_with(entry, b"RUNFILES_MANIFEST_FILE=")
                || str_starts_with(entry, b"RUNFILES_DIR=")
                || str_starts_with(entry, b"JAVA_RUNFILES=");

            if !should_skip {
                // Copy this environment variable
                if data_pos + len + 1 <= MAX_ENV_SIZE {
                    MODIFIED_ENV_DATA[data_pos..data_pos + len].copy_from_slice(entry);
                    MODIFIED_ENV_DATA[data_pos + len] = 0;

                    MODIFIED_ENV_PTRS[ptr_idx] = MODIFIED_ENV_DATA.as_ptr().add(data_pos);
                    ptr_idx += 1;

                    data_pos += len + 1;
                } else {
                    env_dropped = true;
                }
            }

            env_ptr = env_ptr.add(1);
        }

        // Check if any environment variables were dropped
        if env_dropped {
            print(b"ERROR: Failed to copy all environment variables\n");
            print(b"Environment buffer limit exceeded. Total size limit: ");
            print_number(MAX_ENV_SIZE);
            print(b" bytes, max variables: ");
            print_number(MAX_ENV_VARS);
            print(b"\n");
            print(b"Current usage: ");
            print_number(data_pos);
            print(b" bytes, ");
            print_number(ptr_idx);
            print(b" variables\n");
            print(b"Consider reducing the number or size of environment variables.\n");
            exit(1);
        }

        // Null-terminate the pointer array
        MODIFIED_ENV_PTRS[ptr_idx] = core::ptr::null();

        MODIFIED_ENV_PTRS.as_ptr()
    }
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
            let exe_len = strlen(core::slice::from_raw_parts(argv0_ptr, MAX_PATH_LEN));
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

        // Storage for resolved paths (embedded args + runtime args)
        // We need space for embedded args (up to 10) plus runtime args (runtime_argc - 1, excluding stub path)
        let mut resolved_paths: [[u8; MAX_PATH_LEN]; 128] = [[0; MAX_PATH_LEN]; 128];
        let mut resolved_ptrs: [*const u8; 129] = [core::ptr::null(); 129];
        let mut total_argc = 0usize;

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

            if should_transform {
                // Try to resolve through runfiles
                if let Some(ref rf) = runfiles {
                    if let Some(resolved) = rf.rlocation(arg_slice) {
                        resolved_paths[i] = resolved;
                    } else {
                        // If not found in runfiles, use the path as-is
                        let copy_len = arg_len.min(MAX_PATH_LEN);
                        resolved_paths[i][..copy_len].copy_from_slice(&arg_slice[..copy_len]);
                    }
                } else {
                    // Use path as-is
                    let copy_len = arg_len.min(MAX_PATH_LEN);
                    resolved_paths[i][..copy_len].copy_from_slice(&arg_slice[..copy_len]);
                }
            } else {
                // Use path as-is without transformation
                let copy_len = arg_len.min(MAX_PATH_LEN);
                resolved_paths[i][..copy_len].copy_from_slice(&arg_slice[..copy_len]);
            }

            resolved_ptrs[i] = resolved_paths[i].as_ptr();
        }
        total_argc = argc;

        // Append runtime arguments (skip argv[0] which is the stub itself)
        if runtime_argc > 1 {
            for i in 1..runtime_argc as usize {
                if total_argc >= 128 {
                    print(b"ERROR: Too many total arguments (embedded + runtime > 128)\n");
                    exit(1);
                }

                // Get runtime argument
                let runtime_arg_ptr = *runtime_argv.add(i);

                // Find length of runtime argument
                let mut arg_len = 0;
                while *runtime_arg_ptr.add(arg_len) != 0 && arg_len < MAX_PATH_LEN {
                    arg_len += 1;
                }

                // Copy runtime argument to resolved_paths
                let copy_len = arg_len.min(MAX_PATH_LEN);
                let runtime_arg_slice = core::slice::from_raw_parts(runtime_arg_ptr, copy_len);
                resolved_paths[total_argc][..copy_len].copy_from_slice(runtime_arg_slice);

                resolved_ptrs[total_argc] = resolved_paths[total_argc].as_ptr();
                total_argc += 1;
            }
        }

        // NULL-terminate the argv array
        resolved_ptrs[total_argc] = core::ptr::null();

        // Get the executable path (first argument)
        let executable = resolved_ptrs[0];

        // Build environment with runfiles variables if export is enabled
        let envp = if export_runfiles_env {
            build_runfiles_environ(runfiles.as_ref())
        } else {
            environ
        };

        // Execute the target program
        let ret = execve(executable, resolved_ptrs.as_ptr(), envp);

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
