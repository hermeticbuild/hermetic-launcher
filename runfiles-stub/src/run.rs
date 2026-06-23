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
use crate::runfiles::{path_exists, Runfiles};

pub enum ResolvedArg {
    /// Native path bytes on Unix; UTF-8 runfiles and embedded arguments on Windows.
    Bytes(Vec<u8>),
    /// A path obtained from a native Windows API. Keeping it wide preserves
    /// executable locations that cannot be represented by narrowing UTF-16.
    #[cfg(target_os = "windows")]
    Wide(Vec<u16>),
}

/// Everything the backend needs to launch the target, in OS-neutral form.
/// `resolved[0]` is the program to execute; native code units are NUL-terminated.
pub struct Launch<'a> {
    pub resolved: &'a [ResolvedArg],
    /// Runfiles-relative argv[0] (used by Unix to preserve symlink walks; ignored on Windows).
    #[allow(dead_code)] // unused on Windows (CreateProcessW derives argv[0] from the command line)
    pub argv0_override: Option<&'a [u8]>,
    /// Replace inherited runfiles variables when true; preserve them when false.
    pub export_runfiles_env: bool,
    /// The related runfiles context to export. None with export enabled scrubs
    /// inherited runfiles variables after fallback selected physical paths.
    pub child_runfiles: Option<&'a Runfiles>,
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

    // Always try runfiles first when an argument is transformed. A launcher
    // without transformed arguments can still require runfiles solely to export
    // them. Otherwise initialization may fail only when every transformed
    // argument can fall back.
    let argc_mask = if argc >= 32 { 0xFFFFFFFF } else { (1u32 << argc) - 1 };
    let transformed_args = transform_flags & argc_mask;
    let mut fallback_args = 0u32;
    for i in 0..argc {
        if cstr_len(placeholders::fallback(i)) != 0 {
            fallback_args |= 1 << i;
        }
    }
    let needs_transform = transformed_args != 0;
    let requires_runfiles = if needs_transform {
        (transformed_args & !fallback_args) != 0
    } else {
        export_runfiles_env
    };

    let runfiles = if needs_transform || export_runfiles_env {
        match Runfiles::create(&rt) {
            Some(rf) => Some(rf),
            None if requires_runfiles => {
                eline(b"ERROR: Failed to initialize runfiles");
                eline(b"Set RUNFILES_DIR or RUNFILES_MANIFEST_FILE, or ensure <executable>.runfiles/ directory exists");
                platform::exit(1);
            }
            None => None,
        }
    } else {
        None
    };

    // Keep the OS-reported executable path in its native representation.
    // Fallback joining must preserve both non-UTF-8 Unix bytes and UTF-16
    // Windows paths rather than narrowing them through a shared String.
    let executable_path = if fallback_args != 0 {
        rt.fallback_executable_path()
    } else {
        None
    };

    // Resolve each embedded argument (NUL-terminated). resolved[0] is the program.
    let mut resolved: Vec<ResolvedArg> = Vec::with_capacity(argc);
    let mut arg0_from_runfiles = false;
    let mut arg0_fallback_selected = false;
    let mut any_arg_from_runfiles = false;
    let mut any_fallback_selected = false;
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
        let has_fallback = (fallback_args & (1 << i)) != 0;

        // A fallback-enabled argument accepts its runfiles candidate only when
        // that selected path exists. Otherwise an absent directory entry would
        // suppress the usable fallback and fail later in the child or exec call.
        let mut resolved_from_runfiles = false;
        let mut resolved_arg = if should_transform {
            let runfiles_path = runfiles.as_ref().and_then(|rf| {
                core::str::from_utf8(arg_slice)
                    .ok()
                    .and_then(|s| rf.rlocation(s))
            });
            let runfiles_path = runfiles_path.filter(|path| !has_fallback || path_exists(path));
            if let Some(resolved_str) = runfiles_path {
                resolved_from_runfiles = true;
                ResolvedArg::Bytes(Vec::from(resolved_str.as_bytes()))
            } else if has_fallback {
                let fallback_data = placeholders::fallback(i);
                let fallback_len = cstr_len(fallback_data);
                let fallback = core::str::from_utf8(&fallback_data[..fallback_len]).ok();
                let fallback_path = executable_path.as_deref().and_then(|executable| {
                    let fallback = fallback?;
                    if fallback.is_empty() {
                        return None;
                    }
                    platform::executable_relative(executable, fallback)
                });
                // The fallback becomes the selected executable or data argument,
                // so reject it here rather than hand a nonexistent path to the
                // child and obscure why fallback resolution failed.
                match fallback_path.filter(platform::resolved_arg_exists) {
                    Some(path) => {
                        if i == 0 {
                            arg0_fallback_selected = true;
                        }
                        any_fallback_selected = true;
                        path
                    }
                    None => {
                        platform::print(b"ERROR: No usable runfiles or executable-relative fallback for argument ");
                        platform::print(&[b'0' + i as u8]);
                        platform::print(platform::NEWLINE);
                        platform::exit(1);
                    }
                }
            } else {
                ResolvedArg::Bytes(arg_slice.to_vec())
            }
        } else {
            ResolvedArg::Bytes(arg_slice.to_vec())
        };
        if i == 0 {
            arg0_from_runfiles = resolved_from_runfiles;
        }
        any_arg_from_runfiles |= resolved_from_runfiles;
        match &mut resolved_arg {
            ResolvedArg::Bytes(bytes) => bytes.push(0),
            #[cfg(target_os = "windows")]
            ResolvedArg::Wide(wide) => wide.push(0),
        }
        resolved.push(resolved_arg);
    }

    // Preserve argv[0] as a runfiles-relative path when runfiles selected the
    // executable. Adjacent manifest discovery also records its logical
    // <launcher>.runfiles directory so tools can walk that path to locate the
    // runfiles tree even when the manifest maps execution to another location.
    // A fallback must instead use its fully resolved executable path.
    let argv0_override: Option<Vec<u8>> = if arg0_from_runfiles {
        runfiles
            .as_ref()
            .and_then(|rf| rf.dir_path.as_deref())
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
            })
    } else {
        None
    };

    let launch = Launch {
        resolved: &resolved,
        argv0_override: argv0_override.as_deref(),
        export_runfiles_env,
        // Data arguments can come from the parent runfiles without making that
        // context belong to an executable selected by adjacent fallback.
        child_runfiles: if export_runfiles_env
            && !arg0_fallback_selected
            && (!any_fallback_selected || any_arg_from_runfiles)
        {
            runfiles.as_ref()
        } else {
            None
        },
    };
    platform::launch(&launch, &rt)
}
