// Platform-agnostic launcher flow, shared by every backend.
//
// Each backend's entry point gathers the process's runtime arguments into a
// `platform::RuntimeArgs` and calls `run::main`. This function parses the patched
// placeholders, resolves the embedded arguments through runfiles, computes the
// runfiles-relative argv[0], and hands a `Launch` to `platform::launch`, which performs
// the OS-specific marshalling and process replacement/spawn (and never returns).

extern crate alloc;

use alloc::vec::Vec;

use crate::common::cstr_len;
use crate::placeholders::{self, is_template_placeholder};
use crate::platform;
use crate::runfiles::Runfiles;

/// Everything the backend needs to launch the target, in OS-neutral form.
/// `resolved[0]` is the program to execute; bytes are NUL-terminated.
pub struct Launch<'a> {
    pub resolved: &'a [Vec<u8>],
    /// Runfiles-relative argv[0] (used by Unix to preserve symlink walks; ignored on Windows).
    #[allow(dead_code)] // unused on Windows (CreateProcessW derives argv[0] from the command line)
    pub argv0_override: Option<&'a [u8]>,
    pub runfiles: Option<&'a Runfiles>,
    pub export_env: bool,
}

/// Print `msg` followed by the platform newline.
fn eline(msg: &[u8]) {
    platform::print(msg);
    platform::print(platform::NEWLINE);
}

pub fn main(rt: platform::RuntimeArgs) -> ! {
    // Reject an un-finalized template.
    if is_template_placeholder(placeholders::argc()) {
        eline(b"ERROR: This is a template stub runner.");
        eline(b"You must finalize it by replacing the placeholders before use.");
        eline(b"The ARGC_PLACEHOLDER has not been replaced.");
        platform::exit(1);
    }

    // Parse argc (decimal, 1..=10).
    let argc_str = placeholders::argc();
    let argc_len = cstr_len(argc_str);
    if argc_len == 0 {
        eline(b"ERROR: ARGC is empty");
        platform::exit(1);
    }
    let mut argc: usize = 0;
    for &c in &argc_str[..argc_len] {
        if c.is_ascii_digit() {
            argc = argc * 10 + (c - b'0') as usize;
        } else {
            eline(b"ERROR: ARGC contains non-digit characters");
            platform::exit(1);
        }
    }
    if argc == 0 || argc > 10 {
        eline(b"ERROR: Invalid argc (must be 1-10)");
        platform::exit(1);
    }

    // Parse transform flags (decimal bitmask of which args to resolve).
    let flags_str = placeholders::transform_flags();
    let flags_len = cstr_len(flags_str);
    let mut transform_flags: u32 = 0;
    if !is_template_placeholder(flags_str) && flags_len > 0 {
        for &c in &flags_str[..flags_len] {
            if c.is_ascii_digit() {
                transform_flags = transform_flags * 10 + (c - b'0') as u32;
            } else {
                eline(b"ERROR: TRANSFORM_FLAGS contains non-digit characters");
                platform::exit(1);
            }
        }
    }
    // If flags are unset, default to transforming all args.
    if flags_len == 0 || is_template_placeholder(flags_str) {
        transform_flags = 0xFFFFFFFF;
    }

    // Parse export-runfiles-env flag (defaults to true).
    let export_str = placeholders::export_runfiles_env();
    let export_len = cstr_len(export_str);
    let export_runfiles_env = if !is_template_placeholder(export_str) && export_len > 0 {
        export_str[0] != b'0'
    } else {
        true
    };

    // Decide whether runfiles are needed at all.
    let argc_mask = if argc >= 32 { 0xFFFFFFFF } else { (1u32 << argc) - 1 };
    let needs_transform = (transform_flags & argc_mask) != 0;
    let needs_runfiles = needs_transform || export_runfiles_env;

    // Resolve the path used to locate `<exe>.runfiles`. argv[0] is the usual
    // source, but `bazel run` (and any caller that execs us with a relative
    // argv[0] from an unrelated cwd) makes a relative argv[0] useless for
    // discovery — `<argv[0]>.runfiles` joined to cwd points nowhere. In that
    // case fall back to the real executable path (/proc/self/exe,
    // _NSGetExecutablePath). RUNFILES_DIR / RUNFILES_MANIFEST_FILE, when set,
    // still take precedence inside Runfiles::create.
    let argv0 = rt.program_path();
    let resolved_exe: Option<Vec<u8>> = match argv0 {
        Some(p) if !p.is_empty() && p[0] == b'/' => None, // absolute argv[0] is fine
        _ => platform::executable_path(),
    };
    let executable_path = resolved_exe.as_deref().or(argv0);

    let runfiles = if needs_runfiles {
        match Runfiles::create(executable_path) {
            Some(rf) => Some(rf),
            None => {
                eline(b"ERROR: Failed to initialize runfiles");
                eline(b"Set RUNFILES_DIR or RUNFILES_MANIFEST_FILE, or ensure <executable>.runfiles/ directory exists");
                platform::exit(1);
            }
        }
    } else {
        None
    };

    // Resolve each embedded argument (NUL-terminated). resolved[0] is the program.
    let mut resolved: Vec<Vec<u8>> = Vec::with_capacity(argc);
    for i in 0..argc {
        let arg_data = placeholders::arg(i);
        let arg_len = cstr_len(arg_data);
        if arg_len == 0 {
            platform::print(b"ERROR: Argument ");
            platform::print(&[b'0' + i as u8]);
            eline(b" is empty");
            platform::exit(1);
        }
        let arg_slice = &arg_data[..arg_len];
        let should_transform = (transform_flags & (1 << i)) != 0;

        // Resolve through runfiles when marked and possible; otherwise use as-is.
        let mut bytes = if should_transform {
            match runfiles.as_ref().and_then(|rf| {
                core::str::from_utf8(arg_slice).ok().and_then(|s| rf.rlocation(s))
            }) {
                Some(resolved_str) => Vec::from(resolved_str.as_bytes()),
                None => arg_slice.to_vec(),
            }
        } else {
            arg_slice.to_vec()
        };
        bytes.push(0);
        resolved.push(bytes);
    }

    // Preserve argv[0] as a runfiles-relative path so tools that walk argv[0]'s
    // parent symlinks (e.g. aspect_rules_py's venv_shim) can locate .runfiles. The
    // fully-resolved path is still used as the program to execute.
    let argv0_override: Option<Vec<u8>> = runfiles
        .as_ref()
        .and_then(|rf| rf.dir_path.as_ref())
        .and_then(|dir| {
            let arg0 = placeholders::arg(0);
            let arg0_len = cstr_len(arg0);
            if arg0_len == 0 {
                return None;
            }
            let mut path = Vec::from(dir.as_bytes());
            if !dir.ends_with('/') {
                path.push(b'/');
            }
            path.extend_from_slice(&arg0[..arg0_len]);
            path.push(0);
            Some(path)
        });

    let launch = Launch {
        resolved: &resolved,
        argv0_override: argv0_override.as_deref(),
        runfiles: runfiles.as_ref(),
        export_env: export_runfiles_env,
    };
    platform::launch(&launch, &rt)
}
