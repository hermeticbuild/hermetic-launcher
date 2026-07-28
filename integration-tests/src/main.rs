//! Test runner for runfiles-stub
//!
//! This test runner validates the runfiles-stub functionality by:
//! 1. Setting up a realistic runfiles tree with demo binaries and test data
//! 2. Creating a manifest file that matches the runfiles tree
//! 3. Using the finalizer to create stub binaries
//! 4. Running the stubs and validating their behavior
//!
//! Usage: test-runner --template <path> --finalizer <path> --test-binaries <dir>
//!
//! The test runner automatically detects the current platform and creates
//! appropriate paths (Windows vs Unix style).

use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use runfiles::{rlocation, Runfiles};

/// Platform-specific path separator for manifest values
#[cfg(windows)]
const PATH_SEP: char = '\\';
#[cfg(not(windows))]
const PATH_SEP: char = '/';

/// Executable extension
#[cfg(windows)]
const EXE_EXT: &str = ".exe";
#[cfg(not(windows))]
const EXE_EXT: &str = "";

/// Workspace name used in runfiles paths
const WORKSPACE_NAME: &str = "_main";

/// Test configuration
struct TestConfig {
    /// Path to the runfiles-stub template binary
    template_path: PathBuf,
    /// Path to the finalize-stub binary
    finalizer_path: PathBuf,
    /// Directory containing test binaries (hash-file, add-numbers, etc.)
    test_binaries_dir: PathBuf,
    /// Working directory for test artifacts
    work_dir: PathBuf,
}

/// Runfiles setup for a test
struct RunfilesSetup {
    /// Root directory of the runfiles tree
    runfiles_dir: PathBuf,
    /// Path to the manifest file
    manifest_path: PathBuf,
    /// Mapping from rlocation paths to absolute paths
    entries: HashMap<String, PathBuf>,
}

fn resolve_runfile_path(runfiles: &Runfiles, path: PathBuf) -> PathBuf {
    if path.exists() || path.is_absolute() {
        return path;
    }

    let runfiles_key = path.to_string_lossy().replace('\\', "/");
    rlocation!(runfiles, runfiles_key.as_str()).unwrap_or(path)
}

/// argv[0] as print-env reported it: the first `|`-separated field of `ARGS:`.
#[cfg(unix)]
fn reported_argv0(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("ARGS:"))
        .and_then(|args| args.split('|').next())
}

fn environment_path<'a>(stdout: &'a str, name: &str) -> Option<&'a Path> {
    let prefix = format!("ENV:{}=", name);
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(Path::new)
}

impl TestConfig {
    fn from_args() -> Result<Self, String> {
        let args: Vec<String> = env::args().collect();

        let mut template_path = None;
        let mut finalizer_path = None;
        let mut test_binaries_dir = None;
        let mut work_dir = None;

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--template" => {
                    i += 1;
                    template_path = Some(PathBuf::from(&args[i]));
                }
                "--finalizer" => {
                    i += 1;
                    finalizer_path = Some(PathBuf::from(&args[i]));
                }
                "--test-binaries" => {
                    i += 1;
                    test_binaries_dir = Some(PathBuf::from(&args[i]));
                }
                "--work-dir" => {
                    i += 1;
                    work_dir = Some(PathBuf::from(&args[i]));
                }
                "--help" | "-h" => {
                    println!("Usage: test-runner --template <path> --finalizer <path> --test-binaries <dir> [--work-dir <dir>]");
                    println!();
                    println!("Options:");
                    println!("  --template       Path to runfiles-stub template binary");
                    println!("  --finalizer      Path to finalize-stub binary");
                    println!("  --test-binaries  Directory containing test binaries");
                    println!("  --work-dir       Working directory for test artifacts (default: temp dir)");
                    std::process::exit(0);
                }
                _ => {
                    return Err(format!("Unknown argument: {}", args[i]));
                }
            }
            i += 1;
        }

        let runfiles = Runfiles::create()
            .map_err(|err| format!("Failed to create runfiles resolver: {err}"))?;
        let template_path =
            resolve_runfile_path(&runfiles, template_path.ok_or("--template is required")?);
        let finalizer_path =
            resolve_runfile_path(&runfiles, finalizer_path.ok_or("--finalizer is required")?);
        let test_binaries_dir = resolve_runfile_path(
            &runfiles,
            test_binaries_dir.ok_or("--test-binaries is required")?,
        );
        let work_dir = work_dir
            .or_else(|| env::var_os("TEST_TMPDIR").map(|dir| PathBuf::from(dir).join("work")))
            .unwrap_or_else(|| env::temp_dir().join("runfiles-stub-tests"));

        // Validate paths exist
        if !template_path.exists() {
            return Err(format!("Template not found: {}", template_path.display()));
        }
        if !finalizer_path.exists() {
            return Err(format!("Finalizer not found: {}", finalizer_path.display()));
        }
        if !test_binaries_dir.exists() {
            return Err(format!("Test binaries dir not found: {}", test_binaries_dir.display()));
        }

        Ok(Self {
            template_path,
            finalizer_path,
            test_binaries_dir,
            work_dir,
        })
    }
}

impl RunfilesSetup {
    /// Create a new runfiles setup in the given directory
    fn new(base_dir: &Path, name: &str) -> std::io::Result<Self> {
        let runfiles_dir = base_dir.join(format!("{}.runfiles", name));
        // Keep manifest-mode tests independent from adjacent discovery. Tests
        // that exercise conventional sibling sources opt in explicitly.
        let manifest_path = base_dir.join(format!("{}.manifest", name));

        fs::create_dir_all(&runfiles_dir)?;

        Ok(Self {
            runfiles_dir,
            manifest_path,
            entries: HashMap::new(),
        })
    }

    /// Add a file to the runfiles tree
    fn add_file(&mut self, rlocation_path: &str, source_path: &Path) -> std::io::Result<()> {
        // Create the destination path in the runfiles tree
        let dest_path = self.runfiles_dir.join(rlocation_path.replace('/', &PATH_SEP.to_string()));

        // Create parent directories
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Copy the file
        fs::copy(source_path, &dest_path)?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&dest_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&dest_path, perms)?;
        }

        // Store the mapping
        self.entries.insert(rlocation_path.to_string(), dest_path);

        Ok(())
    }

    /// Add a file with content to the runfiles tree
    fn add_file_content(&mut self, rlocation_path: &str, content: &[u8]) -> std::io::Result<()> {
        let dest_path = self.runfiles_dir.join(rlocation_path.replace('/', &PATH_SEP.to_string()));

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&dest_path, content)?;

        self.entries.insert(rlocation_path.to_string(), dest_path);

        Ok(())
    }

    /// Write the manifest file
    fn write_manifest(&self) -> std::io::Result<()> {
        let mut file = File::create(&self.manifest_path)?;

        // Write the workspace marker (like Bazel does)
        writeln!(file, "{}/.runfile", WORKSPACE_NAME)?;

        // Write each entry
        for (rlocation_path, abs_path) in &self.entries {
            // Convert absolute path to platform-native format
            let abs_path_str = abs_path.to_string_lossy();

            // On Windows, manifest values use forward slashes in the Bazel convention
            // but we'll use the native format for compatibility
            #[cfg(windows)]
            let abs_path_str = abs_path_str.replace('\\', "/");

            writeln!(file, "{} {}", rlocation_path, abs_path_str)?;
        }

        Ok(())
    }

    /// Append raw `<source> <target>` manifest lines after the normal entries.
    /// Used to inject relative (symlink-style) targets that Bazel emits for
    /// unresolved symlinks, which `add_file`'s absolute entries can't represent.
    fn append_manifest_lines(&self, lines: &[(&str, &str)]) -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new().append(true).open(&self.manifest_path)?;
        for (source, target) in lines {
            writeln!(file, "{} {}", source, target)?;
        }
        Ok(())
    }

    /// Get the absolute path for an rlocation path
    fn get_path(&self, rlocation_path: &str) -> Option<&PathBuf> {
        self.entries.get(rlocation_path)
    }
}

/// Finalize a stub binary
fn finalize_stub(
    config: &TestConfig,
    output_path: &Path,
    args: &[&str],
    transform_indices: &[usize],
) -> Result<(), String> {
    let mut cmd = Command::new(&config.finalizer_path);
    cmd.arg("--template").arg(&config.template_path);
    cmd.arg("--output").arg(output_path);

    // Add transform flags
    if !transform_indices.is_empty() {
        let transform_str: Vec<String> = transform_indices.iter().map(|i| i.to_string()).collect();
        cmd.arg("--transform").arg(transform_str.join(","));
    }

    cmd.arg("--");

    // Add arguments
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd.output().map_err(|e| format!("Failed to run finalizer: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Finalizer failed: {}", stderr));
    }

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(output_path)
            .map_err(|e| format!("Failed to get permissions: {}", e))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(output_path, perms)
            .map_err(|e| format!("Failed to set permissions: {}", e))?;
    }

    Ok(())
}

/// Run a stub and capture its output
fn run_stub(
    stub_path: &Path,
    runfiles_setup: &RunfilesSetup,
    extra_args: &[&str],
    use_manifest: bool,
) -> Result<(String, String, i32), String> {
    let mut cmd = Command::new(stub_path);

    // Set runfiles environment
    if use_manifest {
        cmd.env("RUNFILES_MANIFEST_FILE", &runfiles_setup.manifest_path);
        cmd.env_remove("RUNFILES_DIR");
    } else {
        cmd.env("RUNFILES_DIR", &runfiles_setup.runfiles_dir);
        cmd.env_remove("RUNFILES_MANIFEST_FILE");
    }

    // Add extra arguments
    for arg in extra_args {
        cmd.arg(arg);
    }

    let output = cmd.output().map_err(|e| format!("Failed to run stub: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((stdout, stderr, exit_code))
}

/// Test: Basic hash-file invocation
fn test_hash_file(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: hash_file");

    let test_dir = config.work_dir.join("test_hash_file");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    // Create runfiles setup
    let mut runfiles = RunfilesSetup::new(&test_dir, "hash_stub")
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;

    // Add the hash-file binary
    let hash_binary = config.test_binaries_dir.join(format!("hash-file{}", EXE_EXT));
    runfiles.add_file(&format!("{}/bin/hash-file{}", WORKSPACE_NAME, EXE_EXT), &hash_binary)
        .map_err(|e| format!("Failed to add hash-file: {}", e))?;

    // Add a test data file
    let test_content = b"Hello, World!\n";
    runfiles.add_file_content(&format!("{}/data/test.txt", WORKSPACE_NAME), test_content)
        .map_err(|e| format!("Failed to add test.txt: {}", e))?;

    // Write manifest
    runfiles.write_manifest()
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // Create finalized stub
    let stub_path = test_dir.join(format!("hash_stub{}", EXE_EXT));
    let hash_rlocation = format!("{}/bin/hash-file{}", WORKSPACE_NAME, EXE_EXT);
    let data_rlocation = format!("{}/data/test.txt", WORKSPACE_NAME);

    finalize_stub(
        config,
        &stub_path,
        &[&hash_rlocation, &data_rlocation],
        &[0, 1], // Transform both arguments
    )?;

    // Test with manifest
    let (stdout, stderr, exit_code) = run_stub(&stub_path, &runfiles, &[], true)?;

    if exit_code != 0 {
        return Err(format!("Stub failed with exit code {}: {}", exit_code, stderr));
    }

    // Verify output contains expected hash
    // SHA256 of "Hello, World!\n"
    let expected_hash = "sha256:c98c24b677eff44860afea6f493bbaec5bb1c4cbb209c6fc2bbb47f66ff2ad31";
    if !stdout.to_lowercase().contains(&expected_hash[7..20]) {
        return Err(format!("Unexpected output: {}. Expected hash containing '{}'", stdout, &expected_hash[7..20]));
    }

    // Test with directory-based runfiles
    let (_stdout2, stderr2, exit_code2) = run_stub(&stub_path, &runfiles, &[], false)?;

    if exit_code2 != 0 {
        return Err(format!("Stub (dir mode) failed with exit code {}: {}", exit_code2, stderr2));
    }

    println!("    PASS (manifest mode)");
    println!("    PASS (directory mode)");

    Ok(())
}

/// Test: add-numbers with runtime arguments
fn test_add_numbers_runtime_args(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: add_numbers_runtime_args");

    let test_dir = config.work_dir.join("test_add_numbers");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    let mut runfiles = RunfilesSetup::new(&test_dir, "add_stub")
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;

    // Add the add-numbers binary
    let add_binary = config.test_binaries_dir.join(format!("add-numbers{}", EXE_EXT));
    runfiles.add_file(&format!("{}/bin/add-numbers{}", WORKSPACE_NAME, EXE_EXT), &add_binary)
        .map_err(|e| format!("Failed to add add-numbers: {}", e))?;

    runfiles.write_manifest()
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // Create stub that only embeds the binary path (arguments come at runtime)
    let stub_path = test_dir.join(format!("add_stub{}", EXE_EXT));
    let add_rlocation = format!("{}/bin/add-numbers{}", WORKSPACE_NAME, EXE_EXT);

    finalize_stub(
        config,
        &stub_path,
        &[&add_rlocation],
        &[0], // Only transform the binary path
    )?;

    // Run with runtime arguments
    let (stdout, stderr, exit_code) = run_stub(&stub_path, &runfiles, &["10", "20", "30"], true)?;

    if exit_code != 0 {
        return Err(format!("Stub failed with exit code {}: {}", exit_code, stderr));
    }

    if !stdout.contains("SUM:60") {
        return Err(format!("Unexpected output: {}. Expected 'SUM:60'", stdout));
    }

    println!("    PASS");

    Ok(())
}

/// Test: runfiles source selection respects provenance, then prefers directories.
fn test_runfiles_source_precedence(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: runfiles_source_precedence");

    // The launcher prefers a directory at equal precedence where Bazel
    // materializes the runfiles tree; on Windows the tree is sparse, so the
    // manifest wins. Mirror that choice when asserting the selected source.
    let prefer_directory = !cfg!(windows);

    let test_dir = config.work_dir.join("test_runfiles_source_precedence");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    let stub_name = format!("precedence_stub{}", EXE_EXT);
    let mut runfiles = RunfilesSetup::new(&test_dir, &stub_name)
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;
    runfiles.manifest_path = test_dir.join(format!("{}.runfiles_manifest", stub_name));
    let executable_rlocation = format!("{}/bin/tool{}", WORKSPACE_NAME, EXE_EXT);
    let print_env_binary = config.test_binaries_dir.join(format!("print-env{}", EXE_EXT));
    runfiles
        .add_file(&executable_rlocation, &print_env_binary)
        .map_err(|e| format!("Failed to add directory executable: {}", e))?;

    let add_binary = config.test_binaries_dir.join(format!("add-numbers{}", EXE_EXT));
    let manifest_target = add_binary.to_string_lossy();
    #[cfg(windows)]
    let manifest_target = manifest_target.replace('\\', "/");
    fs::write(
        &runfiles.manifest_path,
        format!("{} {}\n", executable_rlocation, manifest_target),
    )
    .map_err(|e| format!("Failed to write conflicting manifest: {}", e))?;

    let stub_path = test_dir.join(&stub_name);
    finalize_stub(
        config,
        &stub_path,
        &[&executable_rlocation, "7", "8"],
        &[0],
    )?;

    // Both environment sources have the same precedence. Where the runfiles tree
    // is materialized the directory owns both resolution and the exported
    // environment; on Windows the tree is sparse, so the manifest wins.
    let mut command = Command::new(&stub_path);
    command
        .env("RUNFILES_DIR", &runfiles.runfiles_dir)
        .env("RUNFILES_MANIFEST_FILE", &runfiles.manifest_path)
        .env_remove("JAVA_RUNFILES");
    let output = command
        .output()
        .map_err(|e| format!("Failed to run stub with both runfiles variables: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The directory maps `tool` to print-env (prints ARGC/ENV), the manifest to
    // add-numbers (prints SUM), so the output identifies which source was chosen.
    let selected_expected = if prefer_directory {
        output.status.success()
            && stdout.contains("ARGC:3")
            && environment_path(&stdout, "RUNFILES_DIR") == Some(runfiles.runfiles_dir.as_path())
            && environment_path(&stdout, "JAVA_RUNFILES") == Some(runfiles.runfiles_dir.as_path())
            && stdout.contains("ENV:RUNFILES_MANIFEST_FILE=<unset>")
    } else {
        output.status.success() && stdout.contains("SUM:15") && !stdout.contains("ARGC:3")
    };
    if !selected_expected {
        return Err(format!(
            "Same-precedence environment sources did not select the {} source.\nstdout: {}\nstderr: {}",
            if prefer_directory { "directory" } else { "manifest" },
            stdout, stderr
        ));
    }

    // An environment manifest outranks an adjacent directory, so the manifest's
    // conflicting add-numbers entry must be selected as the sole source.
    let mut command = Command::new(&stub_path);
    command
        .env_remove("RUNFILES_DIR")
        .env_remove("JAVA_RUNFILES")
        .env("RUNFILES_MANIFEST_FILE", &runfiles.manifest_path);
    let output = command
        .output()
        .map_err(|e| format!("Failed to run stub with environment manifest: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() || !stdout.contains("SUM:15") {
        return Err(format!(
            "Environment manifest did not outrank adjacent directory.\nstdout: {}\nstderr: {}",
            stdout, stderr
        ));
    }

    // An invalid environment directory is not a source. Select the environment
    // manifest and scrub stale directory variables from the child.
    let manifest_export_path = test_dir.join("manifest_export.manifest");
    let print_env_target = print_env_binary.to_string_lossy();
    #[cfg(windows)]
    let print_env_target = print_env_target.replace('\\', "/");
    fs::write(
        &manifest_export_path,
        format!("{} {}\n", executable_rlocation, print_env_target),
    )
    .map_err(|e| format!("Failed to write manifest export fixture: {}", e))?;
    let invalid_dir = test_dir.join("not_a_directory");
    fs::write(&invalid_dir, b"not a directory")
        .map_err(|e| format!("Failed to write invalid directory fixture: {}", e))?;
    let mut command = Command::new(&stub_path);
    #[cfg(not(windows))]
    command
        .env("RUNFILES_DIR", &invalid_dir)
        .env("JAVA_RUNFILES", "stale")
        .env("RUNFILES_MANIFEST_FILE", &manifest_export_path);
    #[cfg(windows)]
    command
        .env("Runfiles_Dir", &invalid_dir)
        .env("Java_Runfiles", "stale")
        .env("Runfiles_Manifest_File", &manifest_export_path);
    let output = command
        .output()
        .map_err(|e| format!("Failed to inspect manifest-selected environment: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        || environment_path(&stdout, "RUNFILES_MANIFEST_FILE")
            != Some(manifest_export_path.as_path())
        || !stdout.contains("ENV:RUNFILES_DIR=<unset>")
        || !stdout.contains("ENV:JAVA_RUNFILES=<unset>")
    {
        return Err(format!(
            "Manifest selection did not scrub stale directory state.\nstdout: {}\nstderr: {}",
            stdout, stderr
        ));
    }

    // With no environment source, the adjacent directory and manifest have equal
    // precedence: the directory wins where the tree is materialized, the manifest
    // on Windows, and the winner is exported consistently.
    let mut command = Command::new(&stub_path);
    command
        .env_remove("RUNFILES_DIR")
        .env_remove("RUNFILES_MANIFEST_FILE")
        .env_remove("JAVA_RUNFILES");
    let output = command
        .output()
        .map_err(|e| format!("Failed to run stub with adjacent sources: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let selected_expected = if prefer_directory {
        output.status.success()
            && stdout.contains("ARGC:3")
            && environment_path(&stdout, "RUNFILES_DIR") == Some(runfiles.runfiles_dir.as_path())
            && environment_path(&stdout, "JAVA_RUNFILES") == Some(runfiles.runfiles_dir.as_path())
            && stdout.contains("ENV:RUNFILES_MANIFEST_FILE=<unset>")
    } else {
        output.status.success() && stdout.contains("SUM:15") && !stdout.contains("ARGC:3")
    };
    if !selected_expected {
        return Err(format!(
            "Adjacent {} source did not win the tie.\nstdout: {}\nstderr: {}",
            if prefer_directory { "directory" } else { "manifest" },
            stdout, stderr,
        ));
    }

    // Source selection is global, not per key: a key that only the NON-selected
    // source can resolve must not fall through to it.
    //
    // Case A: both sources come from the environment, so the platform preference
    // picks the winner. Embed a key that lives ONLY in the loser — any
    // fall-through would resolve it and betray the mixing.
    {
        let a_stub_name = format!("no_fallthrough_env_stub{}", EXE_EXT);
        let a_stub = test_dir.join(&a_stub_name);
        let a_manifest = test_dir.join("no_fallthrough_env.runfiles_manifest");
        // Manifest RHS values use forward slashes (Bazel's Windows convention);
        // harmless on Unix, where paths carry no backslashes.
        let add_target = add_binary.to_string_lossy().replace('\\', "/");

        let embedded_key = if prefer_directory {
            // Directory wins; put the embedded key only in the manifest.
            let key = format!("{}/bin/only_in_manifest{}", WORKSPACE_NAME, EXE_EXT);
            fs::write(&a_manifest, format!("{} {}\n", key, add_target))
                .map_err(|e| format!("Failed to write no-fallthrough manifest: {}", e))?;
            key
        } else {
            // Manifest wins; reuse `tool` (present only in the directory, as
            // print-env) and give the manifest an unrelated entry so it loads but
            // cannot resolve the key.
            let decoy = format!("{}/bin/decoy{}", WORKSPACE_NAME, EXE_EXT);
            fs::write(&a_manifest, format!("{} {}\n", decoy, add_target))
                .map_err(|e| format!("Failed to write no-fallthrough manifest: {}", e))?;
            executable_rlocation.clone()
        };
        finalize_stub(config, &a_stub, &[&embedded_key, "7", "8"], &[0])?;

        let mut command = Command::new(&a_stub);
        command
            .env("RUNFILES_DIR", &runfiles.runfiles_dir)
            .env("RUNFILES_MANIFEST_FILE", &a_manifest)
            .env_remove("JAVA_RUNFILES");
        let output = command
            .output()
            .map_err(|e| format!("Failed to test no-fallthrough environment sources: {}", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // The loser's happy-path signature must never appear: SUM from the
        // manifest's add-numbers, or ARGC from the directory's print-env.
        let loser_ran = if prefer_directory {
            stdout.contains("SUM:15")
        } else {
            stdout.contains("ARGC")
        };
        if output.status.success() || loser_ran {
            return Err(format!(
                "Selected source fell through to the other (environment sources).\nstdout: {}\nstderr: {}",
                stdout, stderr
            ));
        }
    }

    // Case B: RUNFILES_DIR is the only environment source (the manifest is merely
    // adjacent), so the environment directory wins on every platform; a key only
    // the adjacent manifest holds must not fall through to it.
    {
        let missing_rlocation = format!("{}/bin/missing{}", WORKSPACE_NAME, EXE_EXT);
        let missing_stub_name = format!("missing_stub{}", EXE_EXT);
        let missing_stub = test_dir.join(&missing_stub_name);
        let missing_manifest =
            test_dir.join(format!("{}.runfiles_manifest", missing_stub_name));
        let add_target = add_binary.to_string_lossy().replace('\\', "/");
        fs::write(&missing_manifest, format!("{} {}\n", missing_rlocation, add_target))
            .map_err(|e| format!("Failed to write no-fallthrough adjacent fixture: {}", e))?;
        finalize_stub(config, &missing_stub, &[&missing_rlocation, "7", "8"], &[0])?;

        let mut command = Command::new(&missing_stub);
        command
            .env("RUNFILES_DIR", &runfiles.runfiles_dir)
            .env_remove("RUNFILES_MANIFEST_FILE")
            .env_remove("JAVA_RUNFILES");
        let output = command
            .output()
            .map_err(|e| format!("Failed to test no-fallthrough adjacent manifest: {}", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() || stdout.contains("SUM:15") {
            return Err(format!(
                "Environment directory fell through to the adjacent manifest.\nstdout: {}\nstderr: {}",
                stdout, stderr
            ));
        }
    }

    println!("    PASS (environment precedence, then platform source preference)");
    Ok(())
}

/// Test: merge-json with two data files
fn test_merge_json(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: merge_json");

    let test_dir = config.work_dir.join("test_merge_json");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    let mut runfiles = RunfilesSetup::new(&test_dir, "merge_stub")
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;

    // Add the merge-json binary
    let merge_binary = config.test_binaries_dir.join(format!("merge-json{}", EXE_EXT));
    runfiles.add_file(&format!("{}/bin/merge-json{}", WORKSPACE_NAME, EXE_EXT), &merge_binary)
        .map_err(|e| format!("Failed to add merge-json: {}", e))?;

    // Add JSON data files
    runfiles.add_file_content(
        &format!("{}/data/base.json", WORKSPACE_NAME),
        br#"{"name": "test", "value": 1, "keep": true}"#,
    ).map_err(|e| format!("Failed to add base.json: {}", e))?;

    runfiles.add_file_content(
        &format!("{}/data/override.json", WORKSPACE_NAME),
        br#"{"value": 42, "extra": "field"}"#,
    ).map_err(|e| format!("Failed to add override.json: {}", e))?;

    runfiles.write_manifest()
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // Create stub with all arguments embedded
    let stub_path = test_dir.join(format!("merge_stub{}", EXE_EXT));
    let merge_rlocation = format!("{}/bin/merge-json{}", WORKSPACE_NAME, EXE_EXT);
    let base_rlocation = format!("{}/data/base.json", WORKSPACE_NAME);
    let override_rlocation = format!("{}/data/override.json", WORKSPACE_NAME);

    finalize_stub(
        config,
        &stub_path,
        &[&merge_rlocation, &base_rlocation, &override_rlocation],
        &[0, 1, 2], // Transform all arguments
    )?;

    let (stdout, stderr, exit_code) = run_stub(&stub_path, &runfiles, &[], true)?;

    if exit_code != 0 {
        return Err(format!("Stub failed with exit code {}: {}", exit_code, stderr));
    }

    // Verify merged output
    if !stdout.contains("MERGED:") {
        return Err(format!("Unexpected output format: {}", stdout));
    }
    if !stdout.contains("\"value\":42") && !stdout.contains("\"value\": 42") {
        return Err(format!("Merge didn't override value: {}", stdout));
    }
    if !stdout.contains("\"keep\":true") && !stdout.contains("\"keep\": true") {
        return Err(format!("Merge lost 'keep' field: {}", stdout));
    }
    if !stdout.contains("\"extra\"") {
        return Err(format!("Merge lost 'extra' field: {}", stdout));
    }

    println!("    PASS");

    Ok(())
}

/// Test: orchestrator calling hash-file (environment propagation)
fn test_orchestrator_env_propagation(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: orchestrator_env_propagation");

    let test_dir = config.work_dir.join("test_orchestrator");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    let mut runfiles = RunfilesSetup::new(&test_dir, "orch_stub")
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;

    // Add binaries
    let orchestrator_binary = config.test_binaries_dir.join(format!("orchestrator{}", EXE_EXT));
    let hash_binary = config.test_binaries_dir.join(format!("hash-file{}", EXE_EXT));

    runfiles.add_file(&format!("{}/bin/orchestrator{}", WORKSPACE_NAME, EXE_EXT), &orchestrator_binary)
        .map_err(|e| format!("Failed to add orchestrator: {}", e))?;
    runfiles.add_file(&format!("{}/bin/hash-file{}", WORKSPACE_NAME, EXE_EXT), &hash_binary)
        .map_err(|e| format!("Failed to add hash-file: {}", e))?;

    // Add test data
    runfiles.add_file_content(
        &format!("{}/data/sample.txt", WORKSPACE_NAME),
        b"Sample content for hashing",
    ).map_err(|e| format!("Failed to add sample.txt: {}", e))?;

    runfiles.write_manifest()
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // First, test env-check to verify environment variables are exported
    let env_stub_path = test_dir.join(format!("env_check_stub{}", EXE_EXT));
    let orch_rlocation = format!("{}/bin/orchestrator{}", WORKSPACE_NAME, EXE_EXT);

    finalize_stub(
        config,
        &env_stub_path,
        &[&orch_rlocation, "env-check"],
        &[0], // Only transform the binary path
    )?;

    let (stdout, stderr, exit_code) = run_stub(&env_stub_path, &runfiles, &[], true)?;

    if exit_code != 0 {
        return Err(format!(
            "Env check failed with exit code {}\nStdout: {}\nStderr: {}",
            exit_code, stdout, stderr
        ));
    }

    // Verify environment variables are propagated
    if !stdout.contains("RUNFILES_MANIFEST_FILE=") || stdout.contains("RUNFILES_MANIFEST_FILE=<unset>") {
        return Err(format!(
            "RUNFILES_MANIFEST_FILE not propagated correctly\nFull stdout:\n{}\nStderr:\n{}",
            stdout, stderr
        ));
    }

    println!("    PASS (env propagation)");

    // Now test hash-and-report which calls hash-file binary
    let hash_stub_path = test_dir.join(format!("hash_and_report_stub{}", EXE_EXT));
    let hash_rlocation = format!("{}/bin/hash-file{}", WORKSPACE_NAME, EXE_EXT);
    let data_rlocation = format!("{}/data/sample.txt", WORKSPACE_NAME);

    // Get absolute paths for the orchestrator command
    let hash_abs_path = runfiles.get_path(&hash_rlocation).unwrap();
    let data_abs_path = runfiles.get_path(&data_rlocation).unwrap();

    finalize_stub(
        config,
        &hash_stub_path,
        &[
            &orch_rlocation,
            "hash-and-report",
            &hash_abs_path.to_string_lossy(),
            &data_abs_path.to_string_lossy(),
        ],
        &[0], // Only transform the orchestrator path
    )?;

    let (stdout, stderr, exit_code) = run_stub(&hash_stub_path, &runfiles, &[], true)?;

    if exit_code != 0 {
        return Err(format!(
            "Hash-and-report failed with exit code {}\nStdout: {}\nStderr: {}",
            exit_code, stdout, stderr
        ));
    }

    if !stdout.contains("ORCHESTRATOR:HASH_RESULT:SHA256:") {
        return Err(format!(
            "Unexpected hash-and-report output (missing ORCHESTRATOR:HASH_RESULT:SHA256:)\nStdout:\n{}\nStderr:\n{}",
            stdout, stderr
        ));
    }

    println!("    PASS (hash-and-report)");

    Ok(())
}

/// Test: Mixed transformed and literal arguments
fn test_mixed_arguments(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: mixed_arguments");

    let test_dir = config.work_dir.join("test_mixed_args");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    let mut runfiles = RunfilesSetup::new(&test_dir, "mixed_stub")
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;

    // Add the add-numbers binary
    let add_binary = config.test_binaries_dir.join(format!("add-numbers{}", EXE_EXT));
    runfiles.add_file(&format!("{}/bin/add-numbers{}", WORKSPACE_NAME, EXE_EXT), &add_binary)
        .map_err(|e| format!("Failed to add add-numbers: {}", e))?;

    runfiles.write_manifest()
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // Create stub where only arg 0 is transformed (binary path)
    // but args 1 and 2 are literal values
    let stub_path = test_dir.join(format!("mixed_stub{}", EXE_EXT));
    let add_rlocation = format!("{}/bin/add-numbers{}", WORKSPACE_NAME, EXE_EXT);

    finalize_stub(
        config,
        &stub_path,
        &[&add_rlocation, "100", "200"],
        &[0], // Only transform the binary path, not the numbers
    )?;

    let (stdout, stderr, exit_code) = run_stub(&stub_path, &runfiles, &[], true)?;

    if exit_code != 0 {
        return Err(format!("Stub failed with exit code {}: {}", exit_code, stderr));
    }

    if !stdout.contains("SUM:300") {
        return Err(format!("Unexpected output: {}. Expected 'SUM:300'", stdout));
    }

    println!("    PASS");

    Ok(())
}

/// Test: Fallback runfiles directory discovery
fn test_fallback_runfiles_dir(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: fallback_runfiles_dir");

    let test_dir = config.work_dir.join("test_fallback");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    // Create a stub with a .runfiles directory next to it
    let stub_path = test_dir.join(format!("fallback_stub{}", EXE_EXT));
    let runfiles_dir = test_dir.join(format!("fallback_stub{}.runfiles", EXE_EXT));

    fs::create_dir_all(&runfiles_dir).map_err(|e| format!("Failed to create runfiles dir: {}", e))?;

    // Add files directly to runfiles directory
    let binary_dir = runfiles_dir.join(WORKSPACE_NAME).join("bin");
    fs::create_dir_all(&binary_dir).map_err(|e| format!("Failed to create binary dir: {}", e))?;

    let add_binary = config.test_binaries_dir.join(format!("add-numbers{}", EXE_EXT));
    let dest_binary = binary_dir.join(format!("add-numbers{}", EXE_EXT));
    fs::copy(&add_binary, &dest_binary).map_err(|e| format!("Failed to copy binary: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest_binary)
            .map_err(|e| format!("Failed to get permissions: {}", e))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest_binary, perms)
            .map_err(|e| format!("Failed to set permissions: {}", e))?;
    }

    // Create the stub
    let add_rlocation = format!("{}/bin/add-numbers{}", WORKSPACE_NAME, EXE_EXT);

    finalize_stub(
        config,
        &stub_path,
        &[&add_rlocation, "5", "10"],
        &[0],
    )?;

    // Run WITHOUT setting any environment variables
    let mut cmd = Command::new(&stub_path);
    cmd.env_remove("RUNFILES_DIR");
    cmd.env_remove("RUNFILES_MANIFEST_FILE");

    let output = cmd.output().map_err(|e| format!("Failed to run stub: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);

    if exit_code != 0 {
        return Err(format!("Stub failed with exit code {}: {}", exit_code, stderr));
    }

    if !stdout.contains("SUM:15") {
        return Err(format!("Unexpected output: {}. Expected 'SUM:15'", stdout));
    }

    println!("    PASS");

    Ok(())
}

/// Test: Fallback runfiles_manifest file discovery
fn test_fallback_runfiles_manifest(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: fallback_runfiles_manifest");

    let test_dir = config.work_dir.join("test_fallback_manifest");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    // Create a stub with only a .runfiles_manifest file next to it.
    let stub_path = test_dir.join(format!("manifest_stub{}", EXE_EXT));
    let manifest_path = test_dir.join(format!("manifest_stub{}.runfiles_manifest", EXE_EXT));

    // The logical sibling tree intentionally does not exist. The manifest maps
    // execution elsewhere, while argv[0] retains the logical runfiles identity.
    let runfiles_dir = test_dir.join(format!("manifest_stub{}.runfiles", EXE_EXT));
    let print_env_binary = config
        .test_binaries_dir
        .join(format!("print-env{}", EXE_EXT));

    // Write the manifest file (key value pairs separated by space)
    let print_env_rlocation = format!("{}/bin/print-env{}", WORKSPACE_NAME, EXE_EXT);
    let manifest_content = format!(
        "{} {}\n",
        print_env_rlocation,
        print_env_binary.display()
    );
    fs::write(&manifest_path, manifest_content)
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // Create the stub
    finalize_stub(
        config,
        &stub_path,
        &[&print_env_rlocation],
        &[0],
    )?;

    // Run WITHOUT setting any environment variables
    let mut cmd = Command::new(&stub_path);
    cmd.env_remove("RUNFILES_DIR");
    cmd.env_remove("RUNFILES_MANIFEST_FILE");
    cmd.env_remove("JAVA_RUNFILES");

    let output = cmd.output().map_err(|e| format!("Failed to run stub: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);

    if exit_code != 0 {
        return Err(format!(
            "Stub failed with exit code {}.\nstdout: {}\nstderr: {}",
            exit_code, stdout, stderr
        ));
    }
    if environment_path(&stdout, "RUNFILES_MANIFEST_FILE") != Some(manifest_path.as_path())
        || !stdout.contains("ENV:RUNFILES_DIR=<unset>")
        || !stdout.contains("ENV:JAVA_RUNFILES=<unset>")
    {
        return Err(format!(
            "Adjacent manifest was not exported as the sole runfiles source.\nstdout: {}",
            stdout
        ));
    }

    #[cfg(unix)]
    {
        let expected = runfiles_dir.join(&print_env_rlocation);
        if reported_argv0(&stdout) != Some(expected.to_string_lossy().as_ref()) {
            return Err(format!(
                "Adjacent manifest did not preserve logical argv[0].\nexpected: {}\nstdout: {}",
                expected.display(),
                stdout
            ));
        }
    }

    println!("    PASS");

    Ok(())
}

/// Regression test: runfiles discovery under `bazel run` semantics.
///
/// Both the path used to launch the executable and argv[0] may be relative;
/// argv[0] may also be completely fake. The stub should use reliable,
/// operating-system specific APIs to find its own path and not rely on argv[0].
///
/// `bazel test` masks the bug because it pre-sets RUNFILES_DIR, and the other
/// fallback tests above invoke the stub by its *absolute* path, so neither
/// exercises this code path. See https://github.com/aspect-build/rules_py/pull/1113.
fn test_run_runfiles_discovery(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: run_runfiles_discovery");

    let test_dir = config.work_dir.join("test_run_discovery");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    // A stub with its <stub>.runfiles directory next to it, like a built binary.
    let stub_name = format!("run_disc_stub{}", EXE_EXT);
    let stub_path = test_dir.join(&stub_name);
    let runfiles_dir = test_dir.join(format!("{}.runfiles", stub_name));

    // Place the target binary inside the runfiles tree.
    let binary_dir = runfiles_dir.join(WORKSPACE_NAME).join("bin");
    fs::create_dir_all(&binary_dir).map_err(|e| format!("Failed to create binary dir: {}", e))?;
    let add_binary = config.test_binaries_dir.join(format!("add-numbers{}", EXE_EXT));
    let dest_binary = binary_dir.join(format!("add-numbers{}", EXE_EXT));
    fs::copy(&add_binary, &dest_binary).map_err(|e| format!("Failed to copy binary: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest_binary)
            .map_err(|e| format!("Failed to get permissions: {}", e))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest_binary, perms)
            .map_err(|e| format!("Failed to set permissions: {}", e))?;
    }

    let add_rlocation = format!("{}/bin/add-numbers{}", WORKSPACE_NAME, EXE_EXT);
    finalize_stub(config, &stub_path, &[&add_rlocation, "5", "10"], &[0])?;

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // The working directory `bazel run` leaves us in: inside the runfiles tree.
        let cwd_inside_runfiles = runfiles_dir.join(WORKSPACE_NAME);

        // Execute the release artifact itself by a relative path, not merely
        // with a relative argv[0]. This covers the optimized macOS stub's cwd
        // lookup while retaining the argv[0] shape used by `bazel run`.
        let relative_stub_path = PathBuf::from("../..").join(&stub_name);
        let mut cmd = Command::new(&relative_stub_path);
        cmd.arg0(format!("bazel-bin/{}", stub_name));
        cmd.current_dir(&cwd_inside_runfiles);
        cmd.env_remove("RUNFILES_DIR");
        cmd.env_remove("RUNFILES_MANIFEST_FILE");
        cmd.env_remove("JAVA_RUNFILES");
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("Failed to run stub: {}", e))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if child
                .try_wait()
                .map_err(|e| format!("Failed to wait for stub: {}", e))?
                .is_some()
            {
                break;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Stub timed out when executed by relative path {}",
                    relative_stub_path.display()
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to collect stub output: {}", e))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let exit_code = output.status.code().unwrap_or(-1);

        if exit_code != 0 {
            return Err(format!(
                "Stub failed with exit code {} (rules_py #1113 regression: relative executable \
                 and argv[0] with cwd inside the runfiles tree).\nstdout: {}\nstderr: {}",
                exit_code, stdout, stderr
            ));
        }
        if !stdout.contains("SUM:15") {
            return Err(format!("Unexpected output: {}. Expected 'SUM:15'", stdout));
        }

        println!("    PASS (relative executable and argv[0], cwd inside runfiles)");
    }

    #[cfg(not(unix))]
    {
        // The relative-argv[0] failure mode is Unix-specific; on Windows the
        // launcher derives runfiles from the command-line argv[0], which Bazel
        // passes as an absolute path. The setup above still keeps the case
        // compiling and the stub finalized.
        let _ = &stub_path;
        println!("    SKIP (Unix-only reproduction)");
    }

    Ok(())
}

/// Regression test: manifest entries with relative (symlink-style) targets.
///
/// Bazel writes an unresolved symlink's `readlink` target into the manifest
/// verbatim, so a manifest target can be relative — interpreted, like any symlink
/// target, relative to the directory of its own key. aspect_rules_py's venv chains
/// these: the entrypoint `bin/python` is a relative symlink to a sibling
/// `bin/python3`, which is itself a relative symlink up to the real interpreter.
/// The launcher must resolve such relative targets (re-looking-up each hop) rather
/// than feeding the raw `../../..` string to execve.
/// See https://github.com/lucidsoftware/rules_py_hermetic_launcher_repro.
fn test_relative_manifest_symlinks(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: relative_manifest_symlinks");

    let test_dir = config.work_dir.join("test_relative_symlinks");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    let mut runfiles = RunfilesSetup::new(&test_dir, "relsym_stub")
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;

    // The real interpreter lives in a separate (canonical) repo directory and is
    // listed with an absolute target, exactly as Bazel does for the interpreter.
    let add_binary = config.test_binaries_dir.join(format!("add-numbers{}", EXE_EXT));
    let interp_rlocation = format!("interpreter_repo/bin/add-numbers{}", EXE_EXT);
    runfiles
        .add_file(&interp_rlocation, &add_binary)
        .map_err(|e| format!("Failed to add interpreter: {}", e))?;

    runfiles
        .write_manifest()
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // Two relative-target hops, mirroring the venv interpreter shim chain. The
    // finalized entrypoint is `python`; resolution must follow both hops:
    //   _main/venv/bin/python  -> python3                                  (sibling)
    //   _main/venv/bin/python3 -> ../../../interpreter_repo/bin/add-numbers (up to repo)
    // landing on the absolute interpreter entry written above. The three `..`
    // segments pop the three key dirs (_main, venv, bin) back to the runfiles root.
    let entry_key = format!("{}/venv/bin/python", WORKSPACE_NAME);
    let sibling_key = format!("{}/venv/bin/python3", WORKSPACE_NAME);
    runfiles
        .append_manifest_lines(&[
            // python -> sibling python3 (relative, single component)
            (&entry_key, "python3"),
            // python3 -> up three dirs into the interpreter repo (relative, with ..)
            (
                &sibling_key,
                &format!("../../../interpreter_repo/bin/add-numbers{}", EXE_EXT),
            ),
        ])
        .map_err(|e| format!("Failed to append relative entries: {}", e))?;

    // Finalize a stub whose entrypoint is the chained relative symlink `python`.
    let stub_path = test_dir.join(format!("relsym_stub{}", EXE_EXT));
    finalize_stub(config, &stub_path, &[&entry_key, "21", "21"], &[0])?;

    // Manifest mode is the path that fed execve the raw "../../.." string before
    // the fix; this must now resolve through both hops to the real interpreter.
    let (stdout, stderr, exit_code) = run_stub(&stub_path, &runfiles, &[], true)?;
    if exit_code != 0 {
        return Err(format!(
            "Relative-symlink stub failed with exit code {} (relative manifest target \
             fed to execve unresolved).\nstdout: {}\nstderr: {}",
            exit_code, stdout, stderr
        ));
    }
    if !stdout.contains("SUM:42") {
        return Err(format!("Unexpected output: {}. Expected 'SUM:42'", stdout));
    }

    println!("    PASS (two relative-target hops, manifest mode)");

    Ok(())
}

/// Build a tree whose manifest sends `_main/wrapper/bin/tool` out of its own
/// directory, so the program's physical and logical paths differ. `materialize`
/// controls whether that logical entry exists in the tree, as it would in a tree
/// Bazel actually built.
fn wrapper_runfiles(
    config: &TestConfig,
    test_dir: &Path,
    materialize: bool,
) -> Result<(RunfilesSetup, String), String> {
    let mut runfiles = RunfilesSetup::new(test_dir, "parent")
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;
    let target_rlocation = format!("tool_repo/bin/print-env{}", EXE_EXT);
    runfiles
        .add_file(
            &target_rlocation,
            &config.test_binaries_dir.join(format!("print-env{}", EXE_EXT)),
        )
        .map_err(|e| format!("Failed to add target: {}", e))?;
    runfiles
        .write_manifest()
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // A relative target keeps the manifest portable and still lands outside the
    // entry's own directory.
    let entry_key = format!("{}/wrapper/bin/tool", WORKSPACE_NAME);
    runfiles
        .append_manifest_lines(&[(&entry_key, &format!("../../../{}", target_rlocation))])
        .map_err(|e| format!("Failed to append relative entry: {}", e))?;

    if materialize {
        let entry = runfiles
            .runfiles_dir
            .join(entry_key.replace('/', &PATH_SEP.to_string()));
        fs::create_dir_all(entry.parent().unwrap())
            .map_err(|e| format!("Failed to create entry dir: {}", e))?;
        fs::write(&entry, b"").map_err(|e| format!("Failed to materialize entry: {}", e))?;
    }

    Ok((runfiles, entry_key))
}

/// Regression test: an environment manifest whose tree is still on disk must keep
/// argv[0] on the program's runfiles path, not the physical location the manifest
/// resolved to. See https://github.com/aspect-build/rules_py/issues/1378.
fn test_env_manifest_logical_argv0(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: env_manifest_logical_argv0");

    let test_dir = config.work_dir.join("test_env_manifest_argv0");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    // The tree is the parent's; the stub has none of its own, so the environment is
    // the only source the launcher can select.
    let (mut runfiles, entry_key) = wrapper_runfiles(config, &test_dir, true)?;

    let stub_path = test_dir.join(format!("child_stub{}", EXE_EXT));
    finalize_stub(config, &stub_path, &[&entry_key], &[0])?;

    let written_manifest = runfiles.manifest_path.clone();

    // Both layouts Bazel writes a manifest in name the same tree.
    for (layout, manifest_path) in [
        (
            "<binary>.runfiles_manifest",
            runfiles.runfiles_dir.with_extension("runfiles_manifest"),
        ),
        ("<tree>/MANIFEST", runfiles.runfiles_dir.join("MANIFEST")),
    ] {
        fs::copy(&written_manifest, &manifest_path)
            .map_err(|e| format!("Failed to place {} manifest: {}", layout, e))?;
        runfiles.manifest_path = manifest_path;

        let (stdout, stderr, exit_code) = run_stub(&stub_path, &runfiles, &[], true)?;
        if exit_code != 0 {
            return Err(format!(
                "Stub failed with exit code {} for {}.\nstdout: {}\nstderr: {}",
                exit_code, layout, stdout, stderr
            ));
        }

        // Windows derives argv[0] from the command line; the override is Unix-only.
        #[cfg(unix)]
        {
            let expected_argv0 = runfiles
                .runfiles_dir
                .join(entry_key.replace('/', &PATH_SEP.to_string()));
            if reported_argv0(&stdout) != Some(expected_argv0.to_string_lossy().as_ref()) {
                return Err(format!(
                    "Environment manifest ({}) did not preserve logical argv[0]; the \
                     child was launched as its physical path.\nexpected: {}\nstdout: {}",
                    layout,
                    expected_argv0.display(),
                    stdout
                ));
            }
        }
    }

    println!("    PASS (logical argv[0] preserved for both manifest layouts)");

    Ok(())
}

/// Regression test: an inferred tree that was never materialized must not be used
/// for argv[0]. `<tree>/MANIFEST` is the case that hides this — opening the manifest
/// already proves its parent directory exists, so probing the directory says nothing
/// about whether the tree was built. Overriding argv[0] from it would name a path
/// that does not exist, which is worse than leaving the physical path alone.
fn test_sparse_inferred_tree_keeps_physical_argv0(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: sparse_inferred_tree_keeps_physical_argv0");

    let test_dir = config.work_dir.join("test_sparse_inferred_tree");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    // Same tree, except the logical entry is absent — a sparse or unbuilt tree.
    let (mut runfiles, entry_key) = wrapper_runfiles(config, &test_dir, false)?;

    let stub_path = test_dir.join(format!("sparse_stub{}", EXE_EXT));
    finalize_stub(config, &stub_path, &[&entry_key], &[0])?;

    let in_tree_manifest = runfiles.runfiles_dir.join("MANIFEST");
    fs::copy(&runfiles.manifest_path, &in_tree_manifest)
        .map_err(|e| format!("Failed to place in-tree manifest: {}", e))?;
    runfiles.manifest_path = in_tree_manifest;

    let (stdout, stderr, exit_code) = run_stub(&stub_path, &runfiles, &[], true)?;
    if exit_code != 0 {
        return Err(format!(
            "Stub failed with exit code {}.\nstdout: {}\nstderr: {}",
            exit_code, stdout, stderr
        ));
    }

    #[cfg(unix)]
    {
        let physical = runfiles
            .runfiles_dir
            .join(format!("tool_repo/bin/print-env{}", EXE_EXT).replace('/', &PATH_SEP.to_string()));
        if reported_argv0(&stdout) != Some(physical.to_string_lossy().as_ref()) {
            return Err(format!(
                "argv[0] was overridden from a tree that was never materialized.\n\
                 expected the physical path: {}\nstdout: {}",
                physical.display(),
                stdout
            ));
        }
    }

    println!("    PASS (sparse inferred tree left argv[0] physical)");

    Ok(())
}

/// Regression test: a relative `RUNFILES_MANIFEST_FILE` names a relative tree, and
/// a relative argv[0] only holds while the child keeps the launcher's working
/// directory. The manifest's own targets are absolute, so the physical path is
/// strictly the safer identity here.
fn test_relative_env_manifest_keeps_physical_argv0(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: relative_env_manifest_keeps_physical_argv0");

    let test_dir = config.work_dir.join("test_relative_env_manifest");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    // The tree is fully materialized: only the relative manifest path is at issue.
    let (runfiles, entry_key) = wrapper_runfiles(config, &test_dir, true)?;

    let stub_path = test_dir.join(format!("relative_manifest_stub{}", EXE_EXT));
    finalize_stub(config, &stub_path, &[&entry_key], &[0])?;

    // `RUNFILES_MANIFEST_FILE` resolved against the launcher's cwd, as the README's
    // invocation example writes it.
    fs::copy(&runfiles.manifest_path, test_dir.join("parent.runfiles_manifest"))
        .map_err(|e| format!("Failed to place sibling manifest: {}", e))?;
    let output = Command::new(&stub_path)
        .current_dir(&test_dir)
        .env("RUNFILES_MANIFEST_FILE", "parent.runfiles_manifest")
        .env_remove("RUNFILES_DIR")
        .output()
        .map_err(|e| format!("Failed to run stub: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "Stub failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    #[cfg(unix)]
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let physical = runfiles
            .runfiles_dir
            .join(format!("tool_repo/bin/print-env{}", EXE_EXT).replace('/', &PATH_SEP.to_string()));
        if reported_argv0(&stdout) != Some(physical.to_string_lossy().as_ref()) {
            return Err(format!(
                "argv[0] was overridden from a relative manifest path, so it only \
                 resolves from the launcher's working directory.\n\
                 expected the physical path: {}\nstdout: {}",
                physical.display(),
                stdout
            ));
        }
    }

    println!("    PASS (relative env manifest left argv[0] physical)");

    Ok(())
}

/// Test: print-env to verify environment and argument passing
fn test_print_env(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: print_env");

    let test_dir = config.work_dir.join("test_print_env");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    let mut runfiles = RunfilesSetup::new(&test_dir, "print_env_stub")
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;

    // Add the print-env binary
    let print_env_binary = config.test_binaries_dir.join(format!("print-env{}", EXE_EXT));
    runfiles.add_file(&format!("{}/bin/print-env{}", WORKSPACE_NAME, EXE_EXT), &print_env_binary)
        .map_err(|e| format!("Failed to add print-env: {}", e))?;

    runfiles.write_manifest()
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // Create stub with some embedded arguments and test runtime args too
    let stub_path = test_dir.join(format!("print_env_stub{}", EXE_EXT));
    let print_env_rlocation = format!("{}/bin/print-env{}", WORKSPACE_NAME, EXE_EXT);

    finalize_stub(
        config,
        &stub_path,
        &[&print_env_rlocation, "--embedded-flag", "embedded-value"],
        &[0], // Only transform the binary path
    )?;

    // Test with manifest mode and runtime arguments
    let (stdout, stderr, exit_code) = run_stub(
        &stub_path,
        &runfiles,
        &["--runtime-flag", "runtime-value"],
        true, // Use manifest
    )?;

    if exit_code != 0 {
        return Err(format!("Stub failed with exit code {}: {}", exit_code, stderr));
    }

    #[cfg(unix)]
    {
        let expected = runfiles
            .get_path(&print_env_rlocation)
            .ok_or_else(|| format!("Missing manifest entry for {}", print_env_rlocation))?;
        if reported_argv0(&stdout) != Some(expected.to_string_lossy().as_ref()) {
            return Err(format!(
                "Environment manifest changed argv[0].\nexpected: {}\nstdout: {}",
                expected.display(),
                stdout
            ));
        }
    }

    // Verify embedded arguments are passed
    if !stdout.contains("--embedded-flag") {
        return Err(format!("Missing embedded flag in output: {}", stdout));
    }
    if !stdout.contains("embedded-value") {
        return Err(format!("Missing embedded value in output: {}", stdout));
    }

    // Verify runtime arguments are passed
    if !stdout.contains("--runtime-flag") {
        return Err(format!("Missing runtime flag in output: {}", stdout));
    }
    if !stdout.contains("runtime-value") {
        return Err(format!("Missing runtime value in output: {}", stdout));
    }

    // Verify RUNFILES_MANIFEST_FILE is set (since we used manifest mode)
    if !stdout.contains("RUNFILES_MANIFEST_FILE=") || stdout.contains("RUNFILES_MANIFEST_FILE=<unset>") {
        return Err(format!("RUNFILES_MANIFEST_FILE should be set: {}", stdout));
    }

    // Verify argument count (binary + 2 embedded + 2 runtime = 5)
    if !stdout.contains("ARGC:5") {
        return Err(format!("Expected ARGC:5 but got: {}", stdout));
    }

    println!("    PASS (manifest mode with embedded + runtime args)");

    // Test with directory mode
    let (stdout2, stderr2, exit_code2) = run_stub(
        &stub_path,
        &runfiles,
        &["dir-mode-arg"],
        false, // Use directory
    )?;

    if exit_code2 != 0 {
        return Err(format!("Stub (dir mode) failed with exit code {}: {}", exit_code2, stderr2));
    }

    // Verify RUNFILES_DIR is set in directory mode
    if !stdout2.contains("RUNFILES_DIR=") || stdout2.contains("RUNFILES_DIR=<unset>") {
        return Err(format!("RUNFILES_DIR should be set in directory mode: {}", stdout2));
    }

    println!("    PASS (directory mode)");

    Ok(())
}

/// Regression test for issue #35: a multi-megabyte manifest must not OOM.
///
/// The previous implementation copied the entire manifest into a growable `Vec`
/// and re-allocated every line into owned `String`s, all served from the stub's
/// 8 MiB static arena. Manifests larger than ~3 MiB exhausted the arena and the
/// stub aborted with a silent `exit(1)`. The mmap-based loader scans the file in
/// place with zero allocation, so arbitrarily large manifests work.
fn test_large_manifest(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: large_manifest");

    let test_dir = config.work_dir.join("test_large_manifest");
    fs::create_dir_all(&test_dir).map_err(|e| format!("Failed to create test dir: {}", e))?;

    let mut runfiles = RunfilesSetup::new(&test_dir, "large_stub")
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;

    // The single real entry we actually resolve.
    let add_binary = config.test_binaries_dir.join(format!("add-numbers{}", EXE_EXT));
    runfiles
        .add_file(&format!("{}/bin/add-numbers{}", WORKSPACE_NAME, EXE_EXT), &add_binary)
        .map_err(|e| format!("Failed to add add-numbers: {}", e))?;

    // Write the normal manifest (workspace marker + the real entry), then append
    // a large block of unused entries to push the file well past the old 8 MiB
    // arena. These keys are never looked up and the values point nowhere; only
    // the file's size matters for reproducing the OOM.
    runfiles
        .write_manifest()
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    {
        // ~100k lines averaging ~84 bytes => ~8 MiB of padding.
        let mut blob = String::with_capacity(9 * 1024 * 1024);
        for i in 0..100_000u32 {
            blob.push_str(&format!(
                "{ws}/pad/unused_entry_number_{i:08} /tmp/nonexistent/padding/path/value_{i:08}\n",
                ws = WORKSPACE_NAME,
            ));
        }
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(&runfiles.manifest_path)
            .map_err(|e| format!("Failed to open manifest for append: {}", e))?;
        f.write_all(blob.as_bytes())
            .map_err(|e| format!("Failed to append padding: {}", e))?;
    }

    let manifest_size = fs::metadata(&runfiles.manifest_path)
        .map_err(|e| format!("Failed to stat manifest: {}", e))?
        .len();
    if manifest_size <= 5 * 1024 * 1024 {
        return Err(format!(
            "Padded manifest too small ({} bytes); expected > 5 MiB",
            manifest_size
        ));
    }

    // Finalize a stub that resolves the real binary plus two literal numbers.
    let stub_path = test_dir.join(format!("large_stub{}", EXE_EXT));
    let add_rlocation = format!("{}/bin/add-numbers{}", WORKSPACE_NAME, EXE_EXT);
    finalize_stub(config, &stub_path, &[&add_rlocation, "40", "2"], &[0])?;

    // Manifest mode: this is the path that OOM'd before the fix.
    let (stdout, stderr, exit_code) = run_stub(&stub_path, &runfiles, &[], true)?;
    if exit_code != 0 {
        return Err(format!(
            "Large-manifest stub failed with exit code {} (issue #35 regression).\nManifest size: {} bytes\nstdout: {}\nstderr: {}",
            exit_code, manifest_size, stdout, stderr
        ));
    }
    if !stdout.contains("SUM:42") {
        return Err(format!("Unexpected output: {}. Expected 'SUM:42'", stdout));
    }

    println!("    PASS ({} byte manifest, manifest mode)", manifest_size);

    Ok(())
}

fn main() -> ExitCode {
    println!("=== Runfiles Stub Test Suite ===");
    println!();

    let config = match TestConfig::from_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!("Use --help for usage information");
            return ExitCode::from(1);
        }
    };

    // Clean and recreate work directory
    if config.work_dir.exists() {
        if let Err(e) = fs::remove_dir_all(&config.work_dir) {
            eprintln!("Warning: Failed to clean work dir: {}", e);
        }
    }
    if let Err(e) = fs::create_dir_all(&config.work_dir) {
        eprintln!("Error: Failed to create work dir: {}", e);
        return ExitCode::from(1);
    }

    println!("Configuration:");
    println!("  Template:      {}", config.template_path.display());
    println!("  Finalizer:     {}", config.finalizer_path.display());
    println!("  Test binaries: {}", config.test_binaries_dir.display());
    println!("  Work dir:      {}", config.work_dir.display());
    println!();

    let tests: Vec<(&str, fn(&TestConfig) -> Result<(), String>)> = vec![
        ("hash_file", test_hash_file),
        ("add_numbers_runtime_args", test_add_numbers_runtime_args),
        ("runfiles_source_precedence", test_runfiles_source_precedence),
        ("merge_json", test_merge_json),
        ("orchestrator_env_propagation", test_orchestrator_env_propagation),
        ("mixed_arguments", test_mixed_arguments),
        ("fallback_runfiles_dir", test_fallback_runfiles_dir),
        ("fallback_runfiles_manifest", test_fallback_runfiles_manifest),
        ("run_runfiles_discovery", test_run_runfiles_discovery),
        ("relative_manifest_symlinks", test_relative_manifest_symlinks),
        ("env_manifest_logical_argv0", test_env_manifest_logical_argv0),
        (
            "sparse_inferred_tree_keeps_physical_argv0",
            test_sparse_inferred_tree_keeps_physical_argv0,
        ),
        (
            "relative_env_manifest_keeps_physical_argv0",
            test_relative_env_manifest_keeps_physical_argv0,
        ),
        ("print_env", test_print_env),
        ("large_manifest", test_large_manifest),
    ];

    let mut passed = 0;
    let mut failed = 0;

    println!("Running {} tests...", tests.len());
    println!();

    for (_name, test_fn) in &tests {
        match test_fn(&config) {
            Ok(()) => {
                passed += 1;
            }
            Err(e) => {
                println!("  FAILED: {}", e);
                failed += 1;
            }
        }
    }

    println!();
    println!("=== Results ===");
    println!("Passed: {}", passed);
    println!("Failed: {}", failed);
    println!();

    if failed > 0 {
        ExitCode::from(1)
    } else {
        println!("All tests passed!");
        ExitCode::SUCCESS
    }
}
