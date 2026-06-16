// Platform-agnostic runfiles discovery and resolution. OS-specific behaviour
// (path separators, absolute-path detection, file I/O) is reached through the
// `platform` module so this logic is shared by every backend.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use crate::common::{cstr_len, Manifest};
use crate::platform;

pub enum RunfilesMode {
    ManifestBased(Manifest),
    DirectoryBased(String),
}

pub struct Runfiles {
    mode: RunfilesMode,
    // Paths for environment variables (when export_runfiles_env is true)
    pub manifest_path: Option<String>, // RUNFILES_MANIFEST_FILE
    pub dir_path: Option<String>,      // RUNFILES_DIR and JAVA_RUNFILES
}

impl Runfiles {
    pub fn create(rt: &platform::RuntimeArgs) -> Option<Self> {
        // Try RUNFILES_MANIFEST_FILE first
        if let Some(manifest_path) = platform::get_env_var(b"RUNFILES_MANIFEST_FILE") {
            if !manifest_path.is_empty() {
                // Create null-terminated path for load_manifest
                let mut path_with_null = Vec::from(manifest_path.as_bytes());
                path_with_null.push(0);

                if let Some(manifest) = platform::load_manifest(&path_with_null) {
                    return Some(Self {
                        mode: RunfilesMode::ManifestBased(manifest),
                        manifest_path: Some(manifest_path),
                        dir_path: None,
                    });
                }
            }
        }

        // Try RUNFILES_DIR
        if let Some(runfiles_dir) = platform::get_env_var(b"RUNFILES_DIR") {
            if !runfiles_dir.is_empty() {
                return Some(Self {
                    mode: RunfilesMode::DirectoryBased(runfiles_dir.clone()),
                    manifest_path: None,
                    dir_path: Some(runfiles_dir),
                });
            }
        }

        // Locate runfiles next to the launching executable:
        // <executable>.runfiles_manifest file first (preferred), then
        // <executable>.runfiles directory. The executable path comes from the OS
        // (an absolute, non-symlink-resolved launch path), not from argv[0].
        if let Some(exe_path) = rt.executable_path() {
            let exe_len = cstr_len(&exe_path);
            if exe_len > 0 {
                // Convert the executable path to a string (if valid UTF-8).
                let exe_str = core::str::from_utf8(&exe_path[..exe_len]).ok()?;

                // Try <executable>.runfiles_manifest file first
                let manifest_file_path = String::from(exe_str) + ".runfiles_manifest";

                // Add null terminator for the file open
                let mut manifest_path_with_null = Vec::from(manifest_file_path.as_bytes());
                manifest_path_with_null.push(0);

                if let Some(manifest) = platform::load_manifest(&manifest_path_with_null) {
                    // Also determine the runfiles directory for RUNFILES_DIR envvar
                    let dir_path = String::from(exe_str) + ".runfiles";

                    return Some(Self {
                        mode: RunfilesMode::ManifestBased(manifest),
                        manifest_path: Some(manifest_file_path),
                        dir_path: Some(dir_path),
                    });
                }

                // Try <executable>.runfiles directory
                let runfiles_dir = String::from(exe_str) + ".runfiles";

                // Add null terminator for the existence check
                let mut dir_with_null = Vec::from(runfiles_dir.as_bytes());
                dir_with_null.push(0);

                if platform::path_exists(&dir_with_null) {
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

    pub fn rlocation(&self, path: &str) -> Option<String> {
        // If path is absolute, don't resolve through runfiles.
        if platform::is_absolute(path) {
            return None;
        }

        match &self.mode {
            RunfilesMode::ManifestBased(manifest) => {
                if let Some(resolved) = manifest.lookup(path) {
                    return Some(platform::to_native_path(resolved));
                }
                // Prefix match for paths within TreeArtifacts
                if let Some((resolved_prefix, suffix)) = manifest.prefix_lookup(path) {
                    let mut result = String::from(resolved_prefix);
                    result.push_str(suffix);
                    return Some(platform::to_native_path(&result));
                }
                None
            }
            RunfilesMode::DirectoryBased(dir) => {
                let mut result = dir.clone();
                // Add separator if needed.
                if !result.ends_with('/') && !result.ends_with(platform::SEP) {
                    result.push(platform::SEP);
                }
                result.push_str(&platform::to_native_path(path));
                Some(result)
            }
        }
    }
}
