// Platform-agnostic runfiles discovery and resolution. OS-specific behaviour
// (path separators, absolute-path detection, file I/O) is reached through the
// `platform` module so this logic is shared by every backend.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use crate::common::{cstr_len, Manifest};
use crate::platform;

pub enum Runfiles {
    Manifest {
        manifest: Manifest,
        path: String,
        logical_dir: Option<String>,
    },
    Directory {
        path: String,
    },
}

impl Runfiles {
    pub fn create(rt: &platform::RuntimeArgs) -> Option<Self> {
        // Environment-provided sources take precedence over sources discovered
        // next to the executable. Within a tier, `select_source` applies the
        // platform's directory-vs-manifest preference.
        if let Some(rf) = select_source(
            || {
                platform::get_env_var(b"RUNFILES_DIR")
                    .filter(|path| !path.is_empty() && dir_exists(path))
                    .map(|path| Self::Directory { path })
            },
            || {
                platform::get_env_var(b"RUNFILES_MANIFEST_FILE")
                    .filter(|path| !path.is_empty())
                    .and_then(|path| {
                        load_manifest(&path).map(|manifest| Self::Manifest {
                            manifest,
                            path,
                            logical_dir: None,
                        })
                    })
            },
        ) {
            return Some(rf);
        }

        // Locate runfiles next to the launching executable: the
        // <executable>.runfiles directory and the <executable>.runfiles_manifest
        // file, in the platform's preferred order. The executable path comes from
        // the OS (an absolute, non-symlink-resolved launch path), not from argv[0].
        if let Some(exe_path) = rt.executable_path() {
            let exe_len = cstr_len(&exe_path);
            if exe_len > 0 {
                // Convert the executable path to a string (if valid UTF-8).
                let exe_str = core::str::from_utf8(&exe_path[..exe_len]).ok()?;
                let runfiles_dir = String::from(exe_str) + ".runfiles";
                let manifest_path = String::from(exe_str) + ".runfiles_manifest";

                if let Some(rf) = select_source(
                    || {
                        dir_exists(&runfiles_dir).then(|| Self::Directory {
                            path: runfiles_dir.clone(),
                        })
                    },
                    || {
                        load_manifest(&manifest_path).map(|manifest| Self::Manifest {
                            manifest,
                            path: manifest_path.clone(),
                            // Preserve the logical path even though the sibling
                            // tree is absent; the manifest selected the actual
                            // executable.
                            logical_dir: Some(runfiles_dir.clone()),
                        })
                    },
                ) {
                    return Some(rf);
                }
            }
        }

        None
    }

    pub fn rlocation(&self, path: &str) -> Option<String> {
        // If path is absolute, don't resolve through runfiles.
        if platform::is_absolute(path) {
            return None;
        }

        match self {
            Self::Manifest { manifest, .. } => resolve_manifest(manifest, path),
            Self::Directory { path: dir } => Some(join_runfiles_path(dir, path)),
        }
    }

    pub fn argv0_rlocation(&self, path: &str) -> Option<String> {
        let dir = match self {
            Self::Directory { path } => path,
            Self::Manifest { logical_dir, .. } => logical_dir.as_ref()?,
        };
        Some(join_runfiles_path(dir, path))
    }

    pub fn manifest_path(&self) -> Option<&str> {
        match self {
            Self::Manifest { path, .. } => Some(path),
            Self::Directory { .. } => None,
        }
    }

    pub fn dir_path(&self) -> Option<&str> {
        match self {
            Self::Manifest { .. } => None,
            Self::Directory { path } => Some(path),
        }
    }
}

/// Choose between a directory and a manifest source at equal precedence.
///
/// Both are evaluated lazily — we never probe a directory or open a manifest we
/// do not end up selecting. The directory is preferred where the platform
/// materializes the runfiles tree; on Windows the tree is not materialized by
/// default, so the sibling directory is sparse and the manifest must win
/// (otherwise `rlocation`s resolve to files that do not exist). See
/// `platform::PREFER_DIRECTORY_SOURCE`.
fn select_source(
    directory: impl FnOnce() -> Option<Runfiles>,
    manifest: impl FnOnce() -> Option<Runfiles>,
) -> Option<Runfiles> {
    if platform::PREFER_DIRECTORY_SOURCE {
        directory().or_else(manifest)
    } else {
        manifest().or_else(directory)
    }
}

fn dir_exists(path: &str) -> bool {
    // A trailing separator makes the existing-path probe directory-specific on
    // Unix and Windows: regular files cannot be traversed as directories.
    let mut path_with_separator = String::from(path);
    if !path.ends_with('/') && !path.ends_with(platform::SEP) {
        path_with_separator.push(platform::SEP);
    }
    let mut path_with_null = Vec::from(path_with_separator.as_bytes());
    path_with_null.push(0);
    platform::path_exists(&path_with_null)
}

fn load_manifest(path: &str) -> Option<Manifest> {
    let mut path_with_null = Vec::from(path.as_bytes());
    path_with_null.push(0);
    platform::load_manifest(&path_with_null)
}

fn join_runfiles_path(dir: &str, path: &str) -> String {
    let mut result = String::from(dir);
    if !result.ends_with('/') && !result.ends_with(platform::SEP) {
        result.push(platform::SEP);
    }
    result.push_str(&platform::to_native_path(path));
    result
}

/// Maximum number of relative manifest hops to follow before giving up. Bazel
/// emits only short symlink chains (e.g. `python` -> `python3` -> `../../interp`),
/// so a small bound is plenty and also breaks any accidental cycle.
const MAX_MANIFEST_HOPS: usize = 32;

/// Resolve a runfiles-relative `path` through a manifest.
///
/// A manifest line maps a runfiles-relative key (LHS) to a target (RHS) that
/// behaves exactly like a filesystem symlink target: an absolute RHS is the final
/// location, while a relative RHS is interpreted relative to the directory of its
/// key (POSIX symlink semantics). A relative target therefore names another
/// runfiles-relative path, which may itself be another manifest entry — Bazel
/// chains venv interpreter shims this way — so we follow the chain until it lands
/// on an absolute path.
fn resolve_manifest(manifest: &Manifest, path: &str) -> Option<String> {
    let mut key = String::from(path);
    for _ in 0..MAX_MANIFEST_HOPS {
        let value = match manifest.lookup(&key) {
            Some(v) => v,
            None => {
                // Prefix match for paths within a TreeArtifact: only the directory
                // is listed, the file beneath it is not. Such directory entries are
                // always absolute, so the joined path is the final location.
                if let Some((resolved_prefix, suffix)) = manifest.prefix_lookup(&key) {
                    let mut result = String::from(resolved_prefix);
                    result.push_str(suffix);
                    return Some(platform::to_native_path(&result));
                }
                return None;
            }
        };

        // An absolute target is the final location; hand it back natively.
        if platform::is_absolute(value) {
            return Some(platform::to_native_path(value));
        }

        // A relative target is a symlink relative to the key's directory. Resolve
        // it into a new runfiles-relative key and look that up in turn.
        key = join_relative(parent_dir(&key), value)?;
    }
    None
}

/// Directory portion of a forward-slash runfiles key, without the trailing slash.
/// `"a/b/c"` -> `"a/b"`; a key with no slash -> `""` (the runfiles root).
fn parent_dir(key: &str) -> &str {
    match key.rfind('/') {
        Some(i) => &key[..i],
        None => "",
    }
}

/// Resolve a relative symlink `target` against directory `base` — both
/// forward-slash, runfiles-relative — normalizing `.` and `..` components.
/// Returns the new runfiles-relative key, or `None` if it would escape the
/// runfiles root (a malformed entry we refuse rather than follow outside the tree).
fn join_relative(base: &str, target: &str) -> Option<String> {
    let mut stack: Vec<&str> = Vec::new();
    for comp in base.split('/').chain(target.split('/')) {
        match comp {
            "" | "." => {}
            // `pop()?` fails the whole resolution if `..` reaches above the root.
            ".." => {
                stack.pop()?;
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        return None;
    }
    let mut result = String::new();
    for (i, comp) in stack.iter().enumerate() {
        if i > 0 {
            result.push('/');
        }
        result.push_str(comp);
    }
    Some(result)
}
