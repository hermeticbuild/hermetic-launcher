// Windows backend: kernel32 (Win32) primitives, a `main` entry that parses the
// command line, and a CreateProcessW-based launch (spawn + wait + propagate exit code).
// Runtime args and executable-relative paths stay UTF-16; embedded args are
// decoded from UTF-8.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::common::Manifest;
use crate::run::{Launch, ResolvedArg};
use crate::runfiles::Runfiles;

// Windows API types
type DWORD = u32;
type BOOL = i32;
type HANDLE = *mut core::ffi::c_void;
type LPVOID = *mut core::ffi::c_void;

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const INVALID_FILE_ATTRIBUTES: DWORD = 0xFFFFFFFF;
const STD_OUTPUT_HANDLE: DWORD = 0xFFFFFFF5u32;
const GENERIC_READ: DWORD = 0x80000000;
const OPEN_EXISTING: DWORD = 3;
const FILE_ATTRIBUTE_NORMAL: DWORD = 0x80;
const INFINITE: DWORD = 0xFFFFFFFF;
const CREATE_UNICODE_ENVIRONMENT: DWORD = 0x00000400;
const ERROR_INSUFFICIENT_BUFFER: DWORD = 122;

// File sharing and memory-mapping parameters (for the runfiles manifest)
const FILE_SHARE_READ: DWORD = 0x00000001;
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
    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: DWORD,
        dwShareMode: DWORD,
        lpSecurityAttributes: LPVOID,
        dwCreationDisposition: DWORD,
        dwFlagsAndAttributes: DWORD,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    fn GetFileAttributesW(lpFileName: *const u16) -> DWORD;
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
    fn GetCurrentProcess() -> HANDLE;
    fn GetLastError() -> DWORD;
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

fn wide_path_exists(path: &[u16]) -> bool {
    let api_path = crate::native_path::windows_api_path(path);
    unsafe { GetFileAttributesW(api_path.as_ptr()) != INVALID_FILE_ATTRIBUTES }
}

pub fn utf8_path_exists(path: &str) -> bool {
    let wide: Vec<u16> = path.encode_utf16().collect();
    wide_path_exists(&wide)
}

pub fn executable_relative(executable: &[u16], fallback: &str) -> Option<ResolvedArg> {
    crate::native_path::windows_executable_relative(executable, fallback).map(ResolvedArg::Wide)
}

pub fn resolved_arg_exists(arg: &ResolvedArg) -> bool {
    match arg {
        ResolvedArg::Bytes(path) => match core::str::from_utf8(path) {
            Ok(path) => utf8_path_exists(path),
            Err(_) => false,
        },
        ResolvedArg::Wide(path) => wide_path_exists(path),
    }
}

// Path used to launch this process, via QueryFullProcessImageNameW (the
// documented API for a process's own image path). With dwFlags = 0 it returns a
// fully-qualified Win32 path, i.e. already absolute. Keep it in UTF-16 so
// executable-relative fallbacks preserve every code unit. None on failure.
fn launch_path() -> Option<Vec<u16>> {
    // This is the documented approximate upper bound for an extended-length
    // Windows path. Filesystem operations below use explicit verbatim paths;
    // process creation remains subject to the Windows command-line limit.
    const MAX_PATH_UNITS: usize = 32768;

    let mut capacity = 4096;
    loop {
        let mut wide = vec![0u16; capacity];
        let mut size: DWORD = wide.len() as DWORD;
        let ok = unsafe {
            QueryFullProcessImageNameW(GetCurrentProcess(), 0, wide.as_mut_ptr(), &mut size)
        };
        if ok != 0 {
            if size == 0 || size as usize > wide.len() {
                return None;
            }
            // `size` is the number of characters written, excluding the NUL
            // terminator.
            return Some(wide[..size as usize].to_vec());
        }
        if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || capacity == MAX_PATH_UNITS {
            return None;
        }
        capacity = core::cmp::min(capacity * 2, MAX_PATH_UNITS);
    }
}

// QueryFullProcessImageNameW already yields an absolute path, so the cwd is
// never needed to absolutize it; this exists only to satisfy the shared
// `absolutize` seam and is not expected to be called on Windows.
pub fn current_dir() -> Option<Vec<u8>> {
    None
}

// Environment variable lookup (two-call GetEnvironmentVariableW pattern).
pub fn get_env_var(name: &[u8]) -> Option<String> {
    unsafe {
        let mut name_with_null: Vec<u16> = name.iter().map(|&byte| byte as u16).collect();
        name_with_null.push(0);

        let size = GetEnvironmentVariableW(name_with_null.as_ptr(), core::ptr::null_mut(), 0);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u16; size as usize];
        let actual_size = GetEnvironmentVariableW(
            name_with_null.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as DWORD,
        );
        if actual_size > 0 && actual_size < buf.len() as DWORD {
            buf.truncate(actual_size as usize);
            String::from_utf16(&buf).ok()
        } else {
            None
        }
    }
}

// Memory-map the manifest read-only. `path` is NUL-terminated by the caller.
pub fn load_manifest(path: &[u8]) -> Option<Manifest> {
    let path = if path.last() == Some(&0) {
        &path[..path.len() - 1]
    } else {
        path
    };
    let wide_path: Vec<u16> = core::str::from_utf8(path).ok()?.encode_utf16().collect();
    let api_path = crate::native_path::windows_api_path(&wide_path);

    unsafe {
        // FILE_SHARE_READ lets the child process (which inherits
        // RUNFILES_MANIFEST_FILE) open the same manifest for reading.
        let file = CreateFileW(
            api_path.as_ptr(),
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

        // CreateFileMappingW returns NULL (not INVALID_HANDLE_VALUE) on failure.
        let mapping = CreateFileMappingW(
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

    // Append "KEY=VALUE\0" as UTF-16.
    let push_var = |buf: &mut Vec<u16>, key: &[u8], value: &str| {
        for &b in key {
            buf.push(b as u16);
        }
        buf.push(b'=' as u16);
        buf.extend(value.encode_utf16());
        buf.push(0);
    };

    unsafe {
        let env_block = GetEnvironmentStringsW();
        if env_block.is_null() {
            // No parent environment: add runfiles vars in sorted order.
            if let Some(rf) = runfiles {
                if let Some(ref path) = rf.dir_path {
                    push_var(&mut buf, b"JAVA_RUNFILES", path);
                    push_var(&mut buf, b"RUNFILES_DIR", path);
                }
                if let Some(ref path) = rf.manifest_path {
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
                            if let Some(ref path) = rf.dir_path {
                                push_var(&mut buf, b"JAVA_RUNFILES", path);
                            }
                        }
                        java_inserted = true;
                    }
                    if !dir_inserted && var_comes_after(b"RUNFILES_DIR") {
                        if let Some(rf) = runfiles {
                            if let Some(ref path) = rf.dir_path {
                                push_var(&mut buf, b"RUNFILES_DIR", path);
                            }
                        }
                        dir_inserted = true;
                    }
                    if !manifest_inserted && var_comes_after(b"RUNFILES_MANIFEST_FILE") {
                        if let Some(rf) = runfiles {
                            if let Some(ref path) = rf.manifest_path {
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
                    if let Some(ref path) = rf.dir_path {
                        push_var(&mut buf, b"JAVA_RUNFILES", path);
                    }
                }
                if !dir_inserted {
                    if let Some(ref path) = rf.dir_path {
                        push_var(&mut buf, b"RUNFILES_DIR", path);
                    }
                }
                if !manifest_inserted {
                    if let Some(ref path) = rf.manifest_path {
                        push_var(&mut buf, b"RUNFILES_MANIFEST_FILE", path);
                    }
                }
            }

            FreeEnvironmentStringsW(env_block);
        }
    }

    // An empty environment still requires two NUL code units. Otherwise the
    // last entry already supplied the first terminator.
    if buf.is_empty() {
        buf.push(0);
    }
    buf.push(0);
    buf
}

// Parse a Windows command line into runtime arguments (excluding argv[0]).
// We avoid CommandLineToArgvW to skip the shell32.dll dependency.
fn parse_command_line(
    cmdline: *const u16,
    argv_out: &mut [*const u16; 128],
    argv_len_out: &mut [usize; 128],
) -> usize {
    unsafe {
        let mut pos = 0usize;
        let mut argc = 0usize;

        // Skip leading whitespace
        while *cmdline.add(pos) != 0
            && (*cmdline.add(pos) == b' ' as u16 || *cmdline.add(pos) == b'\t' as u16)
        {
            pos += 1;
        }

        // Skip argv[0] (executable path)
        let quoted = *cmdline.add(pos) == b'"' as u16;
        if quoted {
            pos += 1;
            while *cmdline.add(pos) != 0 && *cmdline.add(pos) != b'"' as u16 {
                pos += 1;
            }
            if *cmdline.add(pos) == b'"' as u16 {
                pos += 1;
            }
        } else {
            while *cmdline.add(pos) != 0
                && *cmdline.add(pos) != b' ' as u16
                && *cmdline.add(pos) != b'\t' as u16
            {
                pos += 1;
            }
        }

        // Parse remaining arguments
        while *cmdline.add(pos) != 0 && argc < 128 {
            while *cmdline.add(pos) != 0
                && (*cmdline.add(pos) == b' ' as u16 || *cmdline.add(pos) == b'\t' as u16)
            {
                pos += 1;
            }
            if *cmdline.add(pos) == 0 {
                break;
            }

            let arg_start = pos;
            let in_quotes = *cmdline.add(pos) == b'"' as u16;
            if in_quotes {
                pos += 1;
                while *cmdline.add(pos) != 0 && *cmdline.add(pos) != b'"' as u16 {
                    pos += 1;
                }
                argv_out[argc] = cmdline.add(arg_start + 1);
                argv_len_out[argc] = pos - arg_start - 1;
                if *cmdline.add(pos) == b'"' as u16 {
                    pos += 1;
                }
            } else {
                while *cmdline.add(pos) != 0
                    && *cmdline.add(pos) != b' ' as u16
                    && *cmdline.add(pos) != b'\t' as u16
                {
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

// --- runtime args & launch ---
pub struct RuntimeArgs {
    runtime_argv: [*const u16; 128],
    runtime_argv_len: [usize; 128],
    runtime_count: usize,
}

impl RuntimeArgs {
    /// Absolute path of the launching executable (QueryFullProcessImageNameW),
    /// converted to the UTF-8 representation used by runfiles self-location.
    /// Executable-relative fallbacks use `fallback_executable_path` instead.
    pub fn executable_path(&self) -> Option<Vec<u8>> {
        String::from_utf16(&launch_path()?)
            .ok()
            .map(String::into_bytes)
            .map(crate::common::absolutize)
    }

    pub fn fallback_executable_path(&self) -> Option<Vec<u16>> {
        launch_path()
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

pub fn launch(launch: &Launch, rt: &RuntimeArgs) -> ! {
    unsafe {
        let argc = launch.resolved.len();

        // Build the UTF-16 command line: embedded args (widened) + runtime args (native).
        let mut cmdline_wide: Vec<u16> = Vec::with_capacity(8192);
        let mut application_name = None;

        for (i, arg) in launch.resolved.iter().enumerate() {
            let wide: Vec<u16> = match arg {
                ResolvedArg::Bytes(bytes) => {
                    let bytes = if bytes.last() == Some(&0) {
                        &bytes[..bytes.len() - 1]
                    } else {
                        &bytes[..]
                    };
                    match core::str::from_utf8(bytes) {
                        Ok(value) => value.encode_utf16().collect(),
                        Err(_) => {
                            print(b"ERROR: Embedded argument is not valid UTF-8\r\n");
                            ExitProcess(1);
                        }
                    }
                }
                ResolvedArg::Wide(path) => {
                    if path.last() == Some(&0) {
                        path[..path.len() - 1].to_vec()
                    } else {
                        path.clone()
                    }
                }
            };

            if i == 0 {
                application_name =
                    crate::native_path::windows_extended_path(&wide).map(|mut path| {
                        path.push(0);
                        path
                    });
            }

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

        // A NULL environment inherits the caller's variables unchanged. When
        // export is enabled, replace or remove the three runfiles variables.
        let env_storage = if launch.export_runfiles_env {
            Some(build_runfiles_environ(launch.child_runfiles))
        } else {
            None
        };
        let envp = env_storage
            .as_ref()
            .map_or(core::ptr::null_mut(), |storage| {
                storage.as_ptr() as *mut core::ffi::c_void
            });
        let creation_flags = if launch.export_runfiles_env {
            CREATE_UNICODE_ENVIRONMENT
        } else {
            0
        };

        let mut si: STARTUPINFOW = core::mem::zeroed();
        si.cb = core::mem::size_of::<STARTUPINFOW>() as DWORD;
        let mut pi: PROCESS_INFORMATION = core::mem::zeroed();

        // Pass absolute resolved paths separately so CreateProcessW does not
        // apply its command-line module-name limit. A relative argv[0] must
        // remain command-line-only: non-NULL partial application names use the
        // current directory and do not search PATH or infer an .exe extension.
        // https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw
        let application_name = application_name
            .as_ref()
            .map_or(core::ptr::null(), |path| path.as_ptr());
        let success = CreateProcessW(
            application_name,
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
    let mut runtime_argv: [*const u16; 128] = [core::ptr::null(); 128];
    let mut runtime_argv_len: [usize; 128] = [0; 128];
    let runtime_count = parse_command_line(cmdline, &mut runtime_argv, &mut runtime_argv_len);
    crate::run::main(RuntimeArgs {
        runtime_argv,
        runtime_argv_len,
        runtime_count,
    })
}
