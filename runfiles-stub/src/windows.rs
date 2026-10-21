// Windows backend: kernel32 (Win32) primitives, a `main` entry that parses the
// command line, and a CreateProcessW-based launch (spawn + wait + propagate exit code).
// Runtime args stay UTF-16; embedded args are widened from UTF-8.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::common::Manifest;
use crate::run::Launch;
use crate::runfiles::Runfiles;

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

// File sharing and memory-mapping parameters (for the runfiles manifest)
const FILE_SHARE_READ: DWORD = 0x00000001;
const FILE_SHARE_WRITE: DWORD = 0x00000002;
const FILE_SHARE_DELETE: DWORD = 0x00000004;
const PAGE_READONLY: DWORD = 0x02;
const FILE_MAP_READ: DWORD = 0x04;

// STARTUPINFOW structure (wide char version for CreateProcessW)
#[allow(non_snake_case)] // Win32 field names
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
#[allow(non_snake_case)] // Win32 field names
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
    fn GetFileSizeEx(hFile: HANDLE, lpFileSize: *mut i64) -> BOOL;
    fn CreateFileMappingA(
        hFile: HANDLE,
        lpFileMappingAttributes: LPVOID,
        flProtect: DWORD,
        dwMaximumSizeHigh: DWORD,
        dwMaximumSizeLow: DWORD,
        lpName: LPCSTR,
    ) -> HANDLE;
    fn MapViewOfFile(
        hFileMappingObject: HANDLE,
        dwDesiredAccess: DWORD,
        dwFileOffsetHigh: DWORD,
        dwFileOffsetLow: DWORD,
        dwNumberOfBytesToMap: usize,
    ) -> LPVOID;
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
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(lpszEnvironmentBlock: *mut u16) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn QueryFullProcessImageNameW(
        hProcess: HANDLE,
        dwFlags: DWORD,
        lpExeName: *mut u16,
        lpdwSize: *mut DWORD,
    ) -> BOOL;
}

// --- path semantics ---
pub const SEP: char = '\\';
pub const NEWLINE: &[u8] = b"\r\n";

// Windows does not materialize the runfiles symlink tree by default (creating
// symlinks requires privileges), so a sibling `<exe>.runfiles` / `RUNFILES_DIR`
// is sparse — only the manifest maps runfiles to real paths. Prefer the manifest
// at equal precedence; a directory-first choice would resolve to files that do
// not exist. See runfiles.rs `select_source`.
pub const PREFER_DIRECTORY_SOURCE: bool = false;

pub fn is_absolute(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 2
        && ((b[0].is_ascii_alphabetic() && b[1] == b':') || (b[0] == b'\\' && b[1] == b'\\'))
}

pub fn to_native_path(s: &str) -> String {
    s.replace('/', "\\")
}

// --- primitives ---
pub fn print(s: &[u8]) {
    unsafe {
        let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut written: DWORD = 0;
        WriteFile(stdout, s.as_ptr(), s.len() as DWORD, &mut written, core::ptr::null_mut());
    }
}

pub fn exit(code: i32) -> ! {
    unsafe { ExitProcess(code as u32) }
}

// Directory/file existence check by trying to open it (needs backup semantics for dirs).
pub fn path_exists(path: &[u8]) -> bool {
    const FILE_FLAG_BACKUP_SEMANTICS: DWORD = 0x02000000;
    unsafe {
        // Request no access and allow ordinary sharing: an existence probe
        // should not fail merely because another process has the path open.
        // https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-createfilea
        let handle = CreateFileA(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            core::ptr::null_mut(),
        );
        if handle != INVALID_HANDLE_VALUE {
            CloseHandle(handle);
            true
        } else {
            false
        }
    }
}

// Path used to launch this process, via QueryFullProcessImageNameW (the
// documented API for a process's own image path). With dwFlags = 0 it returns a
// fully-qualified Win32 path, i.e. already absolute. UTF-16 is narrowed to bytes
// the same way argv is elsewhere in this backend. None on failure.
fn launch_path() -> Option<Vec<u8>> {
    let mut wide = vec![0u16; 4096];
    let mut size: DWORD = wide.len() as DWORD;
    let ok = unsafe {
        QueryFullProcessImageNameW(GetCurrentProcess(), 0, wide.as_mut_ptr(), &mut size)
    };
    if ok == 0 || size == 0 || size as usize > wide.len() {
        return None;
    }
    // `size` is the number of characters written, excluding the NUL terminator.
    Some(wide[..size as usize].iter().map(|&w| (w & 0xFF) as u8).collect())
}

// QueryFullProcessImageNameW already yields an absolute path, so the cwd is
// never needed to absolutize it; this exists only to satisfy the shared
// `absolutize` seam and is not expected to be called on Windows.
pub fn current_dir() -> Option<Vec<u8>> {
    None
}

// Environment variable lookup (two-call GetEnvironmentVariableA pattern).
pub fn get_env_var(name: &[u8]) -> Option<String> {
    unsafe {
        let mut name_with_null = name.to_vec();
        name_with_null.push(0);

        let size = GetEnvironmentVariableA(name_with_null.as_ptr(), core::ptr::null_mut(), 0);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let actual_size =
            GetEnvironmentVariableA(name_with_null.as_ptr(), buf.as_mut_ptr(), buf.len() as DWORD);
        if actual_size > 0 && actual_size < buf.len() as DWORD {
            buf.truncate(actual_size as usize);
            String::from_utf8(buf).ok()
        } else {
            None
        }
    }
}

// Memory-map the manifest read-only. `path` is NUL-terminated by the caller.
pub fn load_manifest(path: &[u8]) -> Option<Manifest> {
    unsafe {
        // FILE_SHARE_READ lets the child process (which inherits
        // RUNFILES_MANIFEST_FILE) open the same manifest for reading.
        let file = CreateFileA(
            path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        );
        if file == INVALID_HANDLE_VALUE {
            return None;
        }

        let mut size: i64 = 0;
        if GetFileSizeEx(file, &mut size) == 0 || size <= 0 {
            CloseHandle(file);
            return None;
        }
        let len = size as usize;

        // CreateFileMappingA returns NULL (not INVALID_HANDLE_VALUE) on failure.
        let mapping = CreateFileMappingA(
            file,
            core::ptr::null_mut(),
            PAGE_READONLY,
            0,
            0,
            core::ptr::null(),
        );
        if mapping.is_null() {
            CloseHandle(file);
            return None;
        }

        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
        // The view keeps its own reference to the section; both handles can be closed.
        CloseHandle(mapping);
        CloseHandle(file);
        if view.is_null() {
            return None;
        }

        // Leak the view: ExitProcess reclaims it.
        Some(Manifest::from_mapping(view as *const u8, len))
    }
}

// Build the UTF-16 environment block (sorted, double-NUL terminated) with the runfiles
// variables merged in. Returns an owned Vec the caller keeps alive while CreateProcessW
// reads it. Previously this used a 128 KB `static mut`; a heap Vec removes the mutable
// static, the fixed size cap, and the abort-on-overflow path, while preserving the exact
// sorted-insertion ordering. Windows requires the block sorted alphabetically by name;
// GetEnvironmentStringsW() already returns it sorted, so we insert our vars in place.
fn build_runfiles_environ(runfiles: Option<&Runfiles>) -> Vec<u16> {
    let mut buf: Vec<u16> = Vec::with_capacity(8192);

    // Append "KEY=VALUE\0", widening bytes to UTF-16.
    let push_var = |buf: &mut Vec<u16>, key: &[u8], value: &str| {
        for &b in key {
            buf.push(b as u16);
        }
        buf.push(b'=' as u16);
        for &b in value.as_bytes() {
            buf.push(b as u16);
        }
        buf.push(0);
    };

    unsafe {
        let env_block = GetEnvironmentStringsW();
        if env_block.is_null() {
            // No parent environment: add runfiles vars in sorted order.
            if let Some(rf) = runfiles {
                if let Some(path) = rf.dir_path() {
                    push_var(&mut buf, b"JAVA_RUNFILES", path);
                    push_var(&mut buf, b"RUNFILES_DIR", path);
                }
                if let Some(path) = rf.manifest_path() {
                    push_var(&mut buf, b"RUNFILES_MANIFEST_FILE", path);
                }
            }
        } else {
            let mut pos = 0;
            let mut java_inserted = false;
            let mut dir_inserted = false;
            let mut manifest_inserted = false;

            loop {
                let entry_start = pos;
                while *env_block.add(pos) != 0 {
                    pos += 1;
                }
                let entry_len = pos - entry_start;
                if entry_len == 0 {
                    break;
                }
                let entry_ptr = env_block.add(entry_start);

                // Remove inherited runfiles variables before optionally inserting
                // the values selected for this launch.
                let ascii_upper = |unit: u16| {
                    if (b'a' as u16..=b'z' as u16).contains(&unit) {
                        unit - 32
                    } else {
                        unit
                    }
                };
                let prefixed_case_insensitive = |needle: &[u8]| -> bool {
                    entry_len >= needle.len()
                        && (0..needle.len()).all(|i| {
                            ascii_upper(*entry_ptr.add(i)) == ascii_upper(needle[i] as u16)
                        })
                };
                let should_skip = prefixed_case_insensitive(b"RUNFILES_MANIFEST_FILE=")
                    || prefixed_case_insensitive(b"RUNFILES_DIR=")
                    || prefixed_case_insensitive(b"JAVA_RUNFILES=");

                if !should_skip {
                    // Case-insensitive "does this entry sort after `target`?"
                    let var_comes_after = |target: &[u8]| -> bool {
                        for i in 0..target.len().min(entry_len) {
                            let e = ascii_upper(*entry_ptr.add(i));
                            let t = ascii_upper(target[i] as u16);
                            if e != t {
                                return e > t;
                            }
                        }
                        entry_len > target.len()
                    };

                    if !java_inserted && var_comes_after(b"JAVA_RUNFILES") {
                        if let Some(rf) = runfiles {
                            if let Some(path) = rf.dir_path() {
                                push_var(&mut buf, b"JAVA_RUNFILES", path);
                            }
                        }
                        java_inserted = true;
                    }
                    if !dir_inserted && var_comes_after(b"RUNFILES_DIR") {
                        if let Some(rf) = runfiles {
                            if let Some(path) = rf.dir_path() {
                                push_var(&mut buf, b"RUNFILES_DIR", path);
                            }
                        }
                        dir_inserted = true;
                    }
                    if !manifest_inserted && var_comes_after(b"RUNFILES_MANIFEST_FILE") {
                        if let Some(rf) = runfiles {
                            if let Some(path) = rf.manifest_path() {
                                push_var(&mut buf, b"RUNFILES_MANIFEST_FILE", path);
                            }
                        }
                        manifest_inserted = true;
                    }

                    // Copy this environment variable.
                    for i in 0..entry_len {
                        buf.push(*entry_ptr.add(i));
                    }
                    buf.push(0);
                }

                pos += 1;
            }

            // Append any runfiles vars that sort after every existing entry.
            if let Some(rf) = runfiles {
                if !java_inserted {
                    if let Some(path) = rf.dir_path() {
                        push_var(&mut buf, b"JAVA_RUNFILES", path);
                    }
                }
                if !dir_inserted {
                    if let Some(path) = rf.dir_path() {
                        push_var(&mut buf, b"RUNFILES_DIR", path);
                    }
                }
                if !manifest_inserted {
                    if let Some(path) = rf.manifest_path() {
                        push_var(&mut buf, b"RUNFILES_MANIFEST_FILE", path);
                    }
                }
            }

            FreeEnvironmentStringsW(env_block);
        }
    }

    // Final NUL: with the last entry's NUL this forms the double-NUL block terminator.
    buf.push(0);
    buf
}

// Parse a Windows command line into runtime arguments (excluding argv[0]).
// We avoid CommandLineToArgvW to skip the shell32.dll dependency, but follow its
// (MSVCRT) rules so each argument is recovered exactly as the caller meant it:
//   * arguments are separated by runs of spaces/tabs outside double quotes;
//   * 2n backslashes followed by `"` yield n backslashes and toggle quoting;
//   * 2n+1 backslashes followed by `"` yield n backslashes and a literal `"`;
//   * backslashes not followed by `"` are literal;
//   * inside a quoted section, `""` yields a literal `"` and stays quoted.
// Unescaping needs to rewrite the text, so the arguments are copied back to back into
// `storage` and `ranges[i]` holds the (offset, length) of argument i within it.
// `storage` must not be reallocated after this returns.
fn parse_command_line(
    cmdline: *const u16,
    storage: &mut Vec<u16>,
    ranges: &mut [(usize, usize); 128],
) -> usize {
    const SPACE: u16 = b' ' as u16;
    const TAB: u16 = b'\t' as u16;
    const QUOTE: u16 = b'"' as u16;
    const BACKSLASH: u16 = b'\\' as u16;

    unsafe {
        let mut pos = 0usize;
        let mut argc = 0usize;

        // Skip leading whitespace
        while *cmdline.add(pos) != 0 && (*cmdline.add(pos) == SPACE || *cmdline.add(pos) == TAB) {
            pos += 1;
        }

        // Skip argv[0] (executable path): no backslash escapes, quotes just group.
        let quoted = *cmdline.add(pos) == QUOTE;
        if quoted {
            pos += 1;
            while *cmdline.add(pos) != 0 && *cmdline.add(pos) != QUOTE {
                pos += 1;
            }
            if *cmdline.add(pos) == QUOTE {
                pos += 1;
            }
        } else {
            while *cmdline.add(pos) != 0 && *cmdline.add(pos) != SPACE && *cmdline.add(pos) != TAB {
                pos += 1;
            }
        }

        // Unescaping never grows the text, so the remaining command line bounds the storage.
        let mut remaining = 0usize;
        while *cmdline.add(pos + remaining) != 0 {
            remaining += 1;
        }
        storage.reserve(remaining);

        // Parse remaining arguments
        while *cmdline.add(pos) != 0 && argc < 128 {
            while *cmdline.add(pos) != 0 && (*cmdline.add(pos) == SPACE || *cmdline.add(pos) == TAB) {
                pos += 1;
            }
            if *cmdline.add(pos) == 0 {
                break;
            }

            let arg_start = storage.len();
            let mut in_quotes = false;
            loop {
                let c = *cmdline.add(pos);
                if c == 0 || (!in_quotes && (c == SPACE || c == TAB)) {
                    break;
                }
                if c == BACKSLASH {
                    let mut backslashes = 0usize;
                    while *cmdline.add(pos) == BACKSLASH {
                        backslashes += 1;
                        pos += 1;
                    }
                    if *cmdline.add(pos) == QUOTE {
                        for _ in 0..backslashes / 2 {
                            storage.push(BACKSLASH);
                        }
                        if backslashes % 2 == 1 {
                            // Escaped quote: literal `"`.
                            storage.push(QUOTE);
                            pos += 1;
                        }
                        // Even run: the quote itself is handled on the next iteration.
                    } else {
                        for _ in 0..backslashes {
                            storage.push(BACKSLASH);
                        }
                    }
                } else if c == QUOTE {
                    pos += 1;
                    if in_quotes && *cmdline.add(pos) == QUOTE {
                        // `""` inside quotes: literal `"`, still quoted.
                        storage.push(QUOTE);
                        pos += 1;
                    } else {
                        in_quotes = !in_quotes;
                    }
                } else {
                    storage.push(c);
                    pos += 1;
                }
            }
            ranges[argc] = (arg_start, storage.len() - arg_start);
            argc += 1;
        }
        argc
    }
}

// --- runtime args & launch ---
pub struct RuntimeArgs {
    // Owns the unescaped argument text that `runtime_argv` points into.
    _storage: Vec<u16>,
    runtime_argv: [*const u16; 128],
    runtime_argv_len: [usize; 128],
    runtime_count: usize,
}

impl RuntimeArgs {
    /// Absolute path of the launching executable (QueryFullProcessImageNameW),
    /// for runfiles self-location. Independent of the command-line argv[0].
    pub fn executable_path(&self) -> Option<Vec<u8>> {
        launch_path().map(crate::common::absolutize)
    }
}

// Append one argument to a Windows command line using MSVCRT/CommandLineToArgvW
// quoting rules, so the child re-parses each argument exactly as intended:
//   * wrap the argument in double quotes when it needs them (space, tab, or empty)
//     or `force_quotes` is set (used for argv[0] per the Bazel launcher.cc
//     convention);
//   * double every run of backslashes that immediately precedes a double quote
//     (including the closing quote we add), and escape embedded double quotes.
// Without this, embedded `"` characters and trailing `\` (common in Windows paths
// like `C:\dir\`) corrupt or merge arguments.
fn append_arg(cmdline: &mut Vec<u16>, arg: &[u16], force_quotes: bool) {
    // Quote when the argument would otherwise re-split or vanish: any space or tab
    // (parse_command_line and the CRT both split on both), or an empty argument
    // (which would disappear entirely without surrounding quotes).
    let quote = force_quotes
        || arg.is_empty()
        || arg.iter().any(|&c| c == b' ' as u16 || c == b'\t' as u16);
    if quote {
        cmdline.push(b'"' as u16);
    }
    let mut backslashes: usize = 0;
    for &c in arg {
        if c == b'\\' as u16 {
            backslashes += 1;
        } else {
            if c == b'"' as u16 {
                // Emit 2*backslashes+1 backslashes (the run was already pushed below
                // on prior iterations; push backslashes+1 more) to escape the quote.
                for _ in 0..=backslashes {
                    cmdline.push(b'\\' as u16);
                }
            }
            backslashes = 0;
        }
        cmdline.push(c);
    }
    if quote {
        // Double any trailing backslashes so they don't escape the closing quote.
        for _ in 0..backslashes {
            cmdline.push(b'\\' as u16);
        }
        cmdline.push(b'"' as u16);
    }
}

// Widen a byte string, dropping a trailing NUL.
// TODO: Decode UTF-8 here instead of widening each byte, which garbles
// non-ASCII paths and arguments. Paths read from Windows are narrowed by
// keeping each UTF-16 unit's low byte, and the environment block is widened
// the same way, so all three need to change together.
fn widen(arg: &[u8]) -> Vec<u16> {
    let bytes = if arg.last() == Some(&0) { &arg[..arg.len() - 1] } else { arg };
    bytes.iter().map(|&b| b as u16).collect()
}

pub fn launch(launch: &Launch, rt: &RuntimeArgs) -> ! {
    unsafe {
        let argc = launch.resolved.len();

        // As on Unix, the program sees its runfiles path as argv[0] while the
        // resolved path selects the image, so programs that locate files from
        // argv[0] (such as Python finding a venv's pyvenv.cfg) find them in the
        // runfiles tree. Windows rarely materializes that tree, so the
        // runfiles path is used only where it exists; otherwise argv[0] stays
        // the resolved path, a path that works.
        let argv0_override = launch.argv0_override.filter(|path| path_exists(path));
        let application_name: Option<Vec<u16>> = argv0_override.map(|_| {
            let mut wide = widen(&launch.resolved[0]);
            wide.push(0);
            wide
        });

        // Build the UTF-16 command line: embedded args (widened) + runtime args (native).
        let mut cmdline_wide: Vec<u16> = Vec::with_capacity(8192);

        for (i, arg) in launch.resolved.iter().enumerate() {
            // Embedded args are NUL-terminated; widen without the trailing NUL.
            let wide = match argv0_override {
                Some(override0) if i == 0 => widen(override0),
                _ => widen(arg),
            };

            // Quote arg0 always (Bazel launcher.cc convention); others as needed.
            append_arg(&mut cmdline_wide, &wide, i == 0);
            if i < argc - 1 || rt.runtime_count > 0 {
                cmdline_wide.push(b' ' as u16);
            }
        }

        for i in 0..rt.runtime_count {
            let arg = core::slice::from_raw_parts(rt.runtime_argv[i], rt.runtime_argv_len[i]);
            append_arg(&mut cmdline_wide, arg, false);
            if i < rt.runtime_count - 1 {
                cmdline_wide.push(b' ' as u16);
            }
        }

        cmdline_wide.push(0);

        // Build the environment with runfiles vars if export is enabled.
        // `env_storage` keeps the block alive while CreateProcessW reads it.
        let env_storage: Vec<u16>;
        let envp = if launch.export_env {
            env_storage = build_runfiles_environ(launch.runfiles);
            env_storage.as_ptr() as *mut core::ffi::c_void
        } else {
            core::ptr::null_mut()
        };
        let creation_flags = if launch.export_env { CREATE_UNICODE_ENVIRONMENT } else { 0 };

        let mut si: STARTUPINFOW = core::mem::zeroed();
        si.cb = core::mem::size_of::<STARTUPINFOW>() as DWORD;
        let mut pi: PROCESS_INFORMATION = core::mem::zeroed();

        // Without a runfiles argv[0], a NULL lpApplicationName + quoted executable in
        // the command line (Bazel approach), which keeps CreateProcessW's search for
        // the executable and its default `.exe` extension.
        let success = CreateProcessW(
            application_name.as_ref().map_or(core::ptr::null(), |name| name.as_ptr()),
            cmdline_wide.as_mut_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            1,
            creation_flags,
            envp,
            core::ptr::null(),
            &mut si,
            &mut pi,
        );
        if success == 0 {
            print(b"ERROR: CreateProcess failed\r\n");
            ExitProcess(1);
        }

        WaitForSingleObject(pi.hProcess, INFINITE);
        let mut exit_code: DWORD = 0;
        GetExitCodeProcess(pi.hProcess, &mut exit_code);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        ExitProcess(exit_code);
    }
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    let cmdline = unsafe { GetCommandLineW() };
    let mut storage: Vec<u16> = Vec::new();
    let mut ranges = [(0usize, 0usize); 128];
    let runtime_count = parse_command_line(cmdline, &mut storage, &mut ranges);
    // `storage` is fully built, so its heap buffer (and these pointers) stay put when it moves.
    let mut runtime_argv: [*const u16; 128] = [core::ptr::null(); 128];
    let mut runtime_argv_len: [usize; 128] = [0; 128];
    for i in 0..runtime_count {
        runtime_argv[i] = unsafe { storage.as_ptr().add(ranges[i].0) };
        runtime_argv_len[i] = ranges[i].1;
    }
    crate::run::main(RuntimeArgs {
        _storage: storage,
        runtime_argv,
        runtime_argv_len,
        runtime_count,
    })
}
