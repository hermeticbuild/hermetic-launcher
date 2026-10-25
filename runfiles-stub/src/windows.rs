// Windows backend: kernel32 (Win32) primitives, a `main` entry that parses the
// command line, and a CreateProcessW-based launch (spawn + wait + propagate exit code).
// Runtime args stay UTF-16; text from the shared code is converted from UTF-8
// at each Win32 call.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::common::Manifest;
use crate::run::Launch;
use crate::runfiles::Runfiles;

// Windows API types
type DWORD = u32;
type BOOL = i32;
type HANDLE = *mut core::ffi::c_void;
type LPVOID = *mut core::ffi::c_void;

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const STD_OUTPUT_HANDLE: DWORD = 0xFFFFFFF5u32;
const GENERIC_READ: DWORD = 0x80000000;
const OPEN_EXISTING: DWORD = 3;
const FILE_ATTRIBUTE_NORMAL: DWORD = 0x80;
const INFINITE: DWORD = 0xFFFFFFFF;
const CREATE_UNICODE_ENVIRONMENT: DWORD = 0x00000400;
const MAX_PATH: usize = 260;

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
    fn GetFileSizeEx(hFile: HANDLE, lpFileSize: *mut i64) -> BOOL;
    fn CreateFileMappingW(
        hFile: HANDLE,
        lpFileMappingAttributes: LPVOID,
        flProtect: DWORD,
        dwMaximumSizeHigh: DWORD,
        dwMaximumSizeLow: DWORD,
        lpName: *const u16,
    ) -> HANDLE;
    fn MapViewOfFile(
        hFileMappingObject: HANDLE,
        dwDesiredAccess: DWORD,
        dwFileOffsetHigh: DWORD,
        dwFileOffsetLow: DWORD,
        dwNumberOfBytesToMap: usize,
    ) -> LPVOID;
    fn CloseHandle(hObject: HANDLE) -> BOOL;
    fn GetEnvironmentVariableW(lpName: *const u16, lpBuffer: *mut u16, nSize: DWORD) -> DWORD;
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
    fn GetModuleFileNameW(hModule: HANDLE, lpFilename: *mut u16, nSize: DWORD) -> DWORD;
    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: DWORD,
        dwShareMode: DWORD,
        lpSecurityAttributes: LPVOID,
        dwCreationDisposition: DWORD,
        dwFlagsAndAttributes: DWORD,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    fn GetFinalPathNameByHandleW(
        hFile: HANDLE,
        lpszFilePath: *mut u16,
        cchFilePath: DWORD,
        dwFlags: DWORD,
    ) -> DWORD;
    fn GetFullPathNameW(
        lpFileName: *const u16,
        nBufferLength: DWORD,
        lpBuffer: *mut u16,
        lpFilePart: *mut *mut u16,
    ) -> DWORD;
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

// --- UTF-16 ---
// The W APIs take UTF-16, while the shared code holds UTF-8 text.

// ASCII command-line and path syntax, as UTF-16 units.
const SPACE: u16 = b' ' as u16;
const TAB: u16 = b'\t' as u16;
const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const EQUALS: u16 = b'=' as u16;

// `s` as NUL-terminated UTF-16. No UTF-8 byte encodes more than one UTF-16
// unit, so `s.len() + 1` units always suffice.
fn to_wide(s: &str) -> Vec<u16> {
    let mut wide = Vec::with_capacity(s.len() + 1);
    wide.extend(s.encode_utf16());
    wide.push(0);
    wide
}

const VERBATIM_PREFIX: [u16; 4] = [BACKSLASH, BACKSLASH, b'?' as u16, BACKSLASH];

// `path` as a NUL-terminated UTF-16 path that the file APIs accept at any
// length. Without a long-path manifest, which the stub doesn't have, they
// reject a path of MAX_PATH or more unless it has the `\\?\` prefix. The
// prefix also turns off their normalization of `/`, `.` and `..`, so the path
// is normalized first.
fn long_path(path: &str) -> Vec<u16> {
    let wide = to_wide(path);
    let bytes = path.as_bytes();
    if bytes.len() < MAX_PATH || !(bytes[0].is_ascii_alphabetic() && bytes[1] == b':') {
        return wide;
    }
    // With too small a buffer, the result is the size needed, including the NUL.
    let size = unsafe {
        GetFullPathNameW(wide.as_ptr(), 0, core::ptr::null_mut(), core::ptr::null_mut())
    };
    if size == 0 {
        return wide;
    }
    let mut full = vec![0u16; VERBATIM_PREFIX.len() + size as usize];
    full[..VERBATIM_PREFIX.len()].copy_from_slice(&VERBATIM_PREFIX);
    // With a large enough buffer, the result is the length, excluding the NUL.
    let length = unsafe {
        GetFullPathNameW(
            wide.as_ptr(),
            size,
            full[VERBATIM_PREFIX.len()..].as_mut_ptr(),
            core::ptr::null_mut(),
        )
    };
    if length == 0 || length >= size {
        return wide;
    }
    full.truncate(VERBATIM_PREFIX.len() + length as usize);
    full.push(0);
    full
}

// UTF-16 from Windows as UTF-8. None if it holds an unpaired surrogate, which
// UTF-8 cannot represent.
fn from_wide(wide: &[u16]) -> Option<String> {
    char::decode_utf16(wide.iter().copied()).collect::<Result<_, _>>().ok()
}

// The text of a byte string from the shared code, without its trailing NUL.
// None if it is not UTF-8, as only text converts to UTF-16.
fn utf8(s: &[u8]) -> Option<&str> {
    core::str::from_utf8(s.strip_suffix(&[0]).unwrap_or(s)).ok()
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
pub fn path_exists(path: &str) -> bool {
    const FILE_FLAG_BACKUP_SEMANTICS: DWORD = 0x02000000;
    let path = long_path(path);
    // Request no access and allow ordinary sharing: an existence probe
    // should not fail merely because another process has the path open.
    // https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-createfilew
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            core::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return false;
    }
    unsafe { CloseHandle(handle) };
    true
}

// Path used to launch this process: the absolute path CreateProcess was given,
// with symlinks unresolved, so a stub started through a symlink finds the
// runfiles beside the symlink. (QueryFullProcessImageNameW would resolve them.)
// None on failure, truncation, or if the path is not valid UTF-16.
fn launch_path() -> Option<String> {
    let mut wide = vec![0u16; 4096];
    // A null module means the executable. The result is the number of
    // characters written, excluding the NUL, or the buffer size if truncated.
    let len = unsafe {
        GetModuleFileNameW(core::ptr::null_mut(), wide.as_mut_ptr(), wide.len() as DWORD)
    } as usize;
    if len == 0 || len >= wide.len() {
        return None;
    }
    from_wide(&wide[..len])
}

// GetModuleFileNameW already yields an absolute path, so the cwd is
// never needed to absolutize it; this exists only to satisfy the shared
// `absolutize` seam and is not expected to be called on Windows.
pub fn current_dir() -> Option<Vec<u8>> {
    None
}

// Environment variable lookup (two-call GetEnvironmentVariableW pattern).
pub fn get_env_var(name: &str) -> Option<String> {
    let name = to_wide(name);
    let size = unsafe { GetEnvironmentVariableW(name.as_ptr(), core::ptr::null_mut(), 0) };
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u16; size as usize];
    let actual_size =
        unsafe { GetEnvironmentVariableW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as DWORD) };
    if actual_size > 0 && actual_size < buf.len() as DWORD {
        from_wide(&buf[..actual_size as usize])
    } else {
        None
    }
}

// Memory-map the manifest read-only.
pub fn load_manifest(path: &str) -> Option<Manifest> {
    let path = to_wide(path);
    // FILE_SHARE_READ lets the child process (which inherits
    // RUNFILES_MANIFEST_FILE) open the same manifest for reading.
    let file = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        )
    };
    if file == INVALID_HANDLE_VALUE {
        return None;
    }

    let mut size: i64 = 0;
    if unsafe { GetFileSizeEx(file, &mut size) } == 0 || size <= 0 {
        unsafe { CloseHandle(file) };
        return None;
    }
    let len = size as usize;

    // CreateFileMappingW returns NULL (not INVALID_HANDLE_VALUE) on failure.
    let mapping = unsafe {
        CreateFileMappingW(file, core::ptr::null_mut(), PAGE_READONLY, 0, 0, core::ptr::null())
    };
    if mapping.is_null() {
        unsafe { CloseHandle(file) };
        return None;
    }

    // The view keeps its own reference to the section; both handles can be closed.
    let view = unsafe {
        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
        CloseHandle(mapping);
        CloseHandle(file);
        view
    };
    if view.is_null() {
        return None;
    }

    // Leak the view: ExitProcess reclaims it.
    Some(unsafe { Manifest::from_mapping(view as *const u8, len) })
}

// Build the UTF-16 environment block (sorted, double-NUL terminated) with the runfiles
// variables merged in. Returns an owned Vec the caller keeps alive while CreateProcessW
// reads it. Previously this used a 128 KB `static mut`; a heap Vec removes the mutable
// static, the fixed size cap, and the abort-on-overflow path, while preserving the exact
// sorted-insertion ordering. Windows requires the block sorted alphabetically by name;
// GetEnvironmentStringsW() already returns it sorted, so we insert our vars in place.
fn build_runfiles_environ(runfiles: Option<&Runfiles>) -> Vec<u16> {
    let dir = runfiles.and_then(Runfiles::dir_path);
    let manifest = runfiles.and_then(Runfiles::manifest_path);
    // In sorted order. Inherited values of these are removed; a var without a
    // value for this launch is not exported at all.
    let vars =
        [("JAVA_RUNFILES", dir), ("RUNFILES_DIR", dir), ("RUNFILES_MANIFEST_FILE", manifest)];
    let mut to_insert = vars.iter().peekable();

    let mut buf: Vec<u16> = Vec::with_capacity(8192);
    // Append "NAME=VALUE\0" for a var that has a value.
    let push_var = |buf: &mut Vec<u16>, &(name, value): &(&str, Option<&str>)| {
        if let Some(value) = value {
            buf.extend(name.encode_utf16());
            buf.push(EQUALS);
            buf.extend(value.encode_utf16());
            buf.push(0);
        }
    };

    let inherited = inherited_environment();
    for entry in inherited.split(|&c| c == 0).filter(|entry| !entry.is_empty()) {
        // The name ends at the first `=` after its first character, since
        // hidden per-drive variables such as `=C:` start with one.
        let name_len = entry[1..].iter().position(|&c| c == EQUALS).map_or(entry.len(), |i| i + 1);
        let name = &entry[..name_len];
        if vars.iter().any(|&(var, _)| cmp_env_name(name, var).is_eq()) {
            continue;
        }
        while let Some(var) = to_insert.next_if(|&&(var, _)| cmp_env_name(name, var).is_gt()) {
            push_var(&mut buf, var);
        }
        buf.extend_from_slice(entry);
        buf.push(0);
    }
    // Append any runfiles vars that sort after every existing entry.
    for var in to_insert {
        push_var(&mut buf, var);
    }

    // Final NUL: with the last entry's NUL this forms the double-NUL block terminator.
    buf.push(0);
    buf
}

// A copy of this process's environment block: `NAME=VALUE` entries, each
// followed by a NUL. Empty if the environment is unavailable.
fn inherited_environment() -> Vec<u16> {
    // A pointer to a snapshot of the environment that we must free, laid out as
    // `NAME=VALUE\0NAME=VALUE\0...\0`: each entry ends with a NUL, and one more
    // NUL ends the block. NULL on failure.
    let block = unsafe { GetEnvironmentStringsW() };
    if block.is_null() {
        return Vec::new();
    }
    // Skip entry by entry until an empty one, which ends the block.
    let mut len = 0;
    while unsafe { *block.add(len) } != 0 {
        while unsafe { *block.add(len) } != 0 {
            len += 1;
        }
        len += 1; // the entry's NUL
    }
    let copy = unsafe { core::slice::from_raw_parts(block, len) }.to_vec();
    unsafe { FreeEnvironmentStringsW(block) };
    copy
}

// Order an environment variable name against an ASCII `var` as Windows sorts
// the environment block: by code unit, ignoring case.
fn cmp_env_name(name: &[u16], var: &str) -> Ordering {
    let upper = |c: u16| u8::try_from(c).map_or(c, |b| b.to_ascii_uppercase().into());
    name.iter().map(|&c| upper(c)).cmp(var.encode_utf16().map(upper))
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
    /// Absolute path of the launching executable (`GetModuleFileNameW`),
    /// for runfiles self-location. Independent of the command-line argv[0].
    pub fn executable_path(&self) -> Option<Vec<u8>> {
        launch_path().map(|path| crate::common::absolutize(path.into_bytes()))
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
fn append_arg(cmdline: &mut Vec<u16>, arg: impl Iterator<Item = u16> + Clone, force_quotes: bool) {
    // Quote when the argument would otherwise re-split or vanish: any space or tab
    // (parse_command_line and the CRT both split on both), or an empty argument
    // (which would disappear entirely without surrounding quotes).
    let quote =
        force_quotes || arg.clone().next().is_none() || arg.clone().any(|c| c == SPACE || c == TAB);
    if quote {
        cmdline.push(QUOTE);
    }
    let mut backslashes: usize = 0;
    for c in arg {
        if c == BACKSLASH {
            backslashes += 1;
        } else {
            if c == QUOTE {
                // Emit 2*backslashes+1 backslashes (the run was already pushed below
                // on prior iterations; push backslashes+1 more) to escape the quote.
                for _ in 0..=backslashes {
                    cmdline.push(BACKSLASH);
                }
            }
            backslashes = 0;
        }
        cmdline.push(c);
    }
    if quote {
        // Double any trailing backslashes so they don't escape the closing quote.
        for _ in 0..backslashes {
            cmdline.push(BACKSLASH);
        }
        cmdline.push(QUOTE);
    }
}

// The NUL-terminated UTF-16 path of the file that `path` names, through any
// symlinks. None if `path` names no file.
fn final_path(path: &str) -> Option<Vec<u16>> {
    const CAPACITY: DWORD = 4096;
    // Windows can start a program from a path of MAX_PATH or more only with
    // the `\\?\` prefix. From 247 characters, it also fails to load DLLs with
    // an embedded manifest, such as CPython's, from beside the program
    // (STATUS_NAME_TOO_LONG) unless the program was started with the prefix.
    const LONGEST_UNPREFIXED: usize = 246;
    let path = long_path(path);
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut wide = vec![0u16; CAPACITY as usize];
    // Flags 0: the normalized path with its drive letter, prefixed `\\?\`.
    let size = unsafe { GetFinalPathNameByHandleW(handle, wide.as_mut_ptr(), CAPACITY, 0) };
    unsafe { CloseHandle(handle) };
    // `size` excludes the NUL when the path fits and includes it when it doesn't.
    if size == 0 || size >= CAPACITY {
        return None;
    }
    wide.truncate(size as usize);
    // Drop the `\\?\` prefix from a drive path short enough not to need it,
    // so the program sees the path it would have been started by.
    let start = if wide.starts_with(&VERBATIM_PREFIX)
        && wide.get(VERBATIM_PREFIX.len() + 1) == Some(&u16::from(b':'))
        && wide.len() - VERBATIM_PREFIX.len() <= LONGEST_UNPREFIXED
    {
        VERBATIM_PREFIX.len()
    } else {
        0
    };
    let mut result = wide.split_off(start);
    result.push(0);
    Some(result)
}

pub fn launch(launch: &Launch, rt: &RuntimeArgs) -> ! {
    // Windows takes arguments as UTF-16, so they must be text.
    let args: Vec<&str> = launch
        .resolved
        .iter()
        .enumerate()
        .map(|(i, arg)| {
            utf8(arg).unwrap_or_else(|| {
                print(b"ERROR: Argument ");
                print(&[b'0' + i as u8]);
                print(b" is not valid UTF-8\r\n");
                exit(1)
            })
        })
        .collect();

    // As on Unix, the program sees its runfiles path as argv[0] while the
    // resolved path selects the image, so programs that locate files from
    // argv[0] (such as Python finding a venv's pyvenv.cfg) find them in the
    // runfiles tree. Windows rarely materializes that tree, so the
    // runfiles path is used only where it exists; otherwise argv[0] stays
    // the resolved path, a path that works.
    let argv0_override = launch.argv0_override.and_then(utf8).filter(|path| path_exists(path));

    // Windows looks for a program's DLLs beside the path it was started by,
    // so a program started through a runfiles symlink, such as a Python
    // venv's python.exe, can't find its DLLs. Start the file the symlink
    // points to, as Unix's execve does.
    let program = args[0];
    let final_program = if is_absolute(program) { final_path(program) } else { None };
    let application_name = final_program.or_else(|| argv0_override.map(|_| to_wide(program)));

    // Build the UTF-16 command line: embedded args (encoded) + runtime args (native).
    let mut cmdline_wide: Vec<u16> = Vec::with_capacity(8192);

    // Quote arg0 always (Bazel launcher.cc convention); others as needed.
    append_arg(&mut cmdline_wide, argv0_override.unwrap_or(program).encode_utf16(), true);
    for arg in &args[1..] {
        cmdline_wide.push(SPACE);
        append_arg(&mut cmdline_wide, arg.encode_utf16(), false);
    }
    for i in 0..rt.runtime_count {
        let arg =
            unsafe { core::slice::from_raw_parts(rt.runtime_argv[i], rt.runtime_argv_len[i]) };
        cmdline_wide.push(SPACE);
        append_arg(&mut cmdline_wide, arg.iter().copied(), false);
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

    let mut si: STARTUPINFOW = unsafe { core::mem::zeroed() };
    si.cb = core::mem::size_of::<STARTUPINFOW>() as DWORD;
    let mut pi: PROCESS_INFORMATION = unsafe { core::mem::zeroed() };

    // Without an application name, a NULL lpApplicationName + quoted
    // executable in the command line (Bazel approach), which keeps
    // CreateProcessW's search for the executable and its default `.exe`
    // extension.
    let success = unsafe {
        CreateProcessW(
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
        )
    };
    if success == 0 {
        print(b"ERROR: CreateProcess failed\r\n");
        exit(1);
    }

    unsafe {
        WaitForSingleObject(pi.hProcess, INFINITE);
        let mut exit_code: DWORD = 0;
        GetExitCodeProcess(pi.hProcess, &mut exit_code);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        ExitProcess(exit_code)
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
