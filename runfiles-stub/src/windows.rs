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
}

// --- path semantics ---
pub const SEP: char = '\\';
pub const NEWLINE: &[u8] = b"\r\n";

/// Real executable path for runfiles discovery. Not implemented on Windows:
/// the Windows backend launches via CreateProcessW and derives argv[0] from the
/// command line rather than a relative path, so the relative-argv[0] discovery
/// gap addressed on Unix doesn't arise here. Returns None to keep argv[0]-based
/// discovery (GetModuleFileNameW could be wired up later if needed).
pub fn executable_path() -> Option<alloc::vec::Vec<u8>> {
    None
}

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
        let handle = CreateFileA(
            path.as_ptr(),
            GENERIC_READ,
            0,
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

                // Skip existing runfiles vars (we re-insert our own).
                let prefixed = |needle: &[u8]| -> bool {
                    entry_len > needle.len()
                        && (0..needle.len()).all(|i| *entry_ptr.add(i) == needle[i] as u16)
                };
                let should_skip = prefixed(b"RUNFILES_MANIFEST_FILE=")
                    || prefixed(b"RUNFILES_DIR=")
                    || prefixed(b"JAVA_RUNFILES=");

                if !should_skip {
                    // Case-insensitive "does this entry sort after `target`?"
                    let var_comes_after = |target: &[u8]| -> bool {
                        let upper = |c: u16| if (b'a' as u16..=b'z' as u16).contains(&c) { c - 32 } else { c };
                        for i in 0..target.len().min(entry_len) {
                            let e = upper(*entry_ptr.add(i));
                            let t = upper(target[i] as u16);
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

    // Final NUL: with the last entry's NUL this forms the double-NUL block terminator.
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

// Extract argv[0] from the command line, narrowed to bytes (for runfiles fallback).
fn parse_argv0(cmdline: *const u16) -> Vec<u8> {
    let mut exe_path_buf = Vec::new();
    unsafe {
        let mut pos = 0usize;
        while *cmdline.add(pos) != 0
            && (*cmdline.add(pos) == b' ' as u16 || *cmdline.add(pos) == b'\t' as u16)
        {
            pos += 1;
        }
        let quoted = *cmdline.add(pos) == b'"' as u16;
        if quoted {
            pos += 1;
        }
        while exe_path_buf.len() < 1048576 && *cmdline.add(pos) != 0 {
            let wchar = *cmdline.add(pos);
            if quoted {
                if wchar == b'"' as u16 {
                    break;
                }
            } else if wchar == b' ' as u16 || wchar == b'\t' as u16 {
                break;
            }
            exe_path_buf.push((wchar & 0xFF) as u8);
            pos += 1;
        }
    }
    exe_path_buf
}

// --- runtime args & launch ---
pub struct RuntimeArgs {
    argv0_bytes: Vec<u8>,
    runtime_argv: [*const u16; 128],
    runtime_argv_len: [usize; 128],
    runtime_count: usize,
}

impl RuntimeArgs {
    /// argv[0] (the stub's own path) as bytes, for runfiles fallback discovery.
    pub fn program_path(&self) -> Option<&[u8]> {
        if self.argv0_bytes.is_empty() {
            None
        } else {
            Some(&self.argv0_bytes)
        }
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

        for (i, arg) in launch.resolved.iter().enumerate() {
            // Embedded args are NUL-terminated; widen without the trailing NUL.
            let bytes = if arg.last() == Some(&0) { &arg[..arg.len() - 1] } else { &arg[..] };
            let wide: Vec<u16> = bytes.iter().map(|&b| b as u16).collect();

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

        // NULL lpApplicationName + quoted executable in the command line (Bazel approach).
        let success = CreateProcessW(
            core::ptr::null(),
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
    let argv0_bytes = parse_argv0(cmdline);
    crate::run::main(RuntimeArgs {
        argv0_bytes,
        runtime_argv,
        runtime_argv_len,
        runtime_count,
    })
}
