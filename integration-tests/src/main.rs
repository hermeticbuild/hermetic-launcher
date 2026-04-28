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
        let manifest_path = base_dir.join(format!("{}.runfiles_manifest", name));

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

    // Create a stub with a .runfiles_manifest file next to it (not a directory)
    let stub_path = test_dir.join(format!("manifest_stub{}", EXE_EXT));
    let manifest_path = test_dir.join(format!("manifest_stub{}.runfiles_manifest", EXE_EXT));

    // Create the runfiles directory structure for the files
    let runfiles_dir = test_dir.join(format!("manifest_stub{}.runfiles", EXE_EXT));
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

    // Write the manifest file (key value pairs separated by space)
    let add_rlocation = format!("{}/bin/add-numbers{}", WORKSPACE_NAME, EXE_EXT);
    let manifest_content = format!("{} {}\n", add_rlocation, dest_binary.display());
    fs::write(&manifest_path, manifest_content)
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // Create the stub
    finalize_stub(
        config,
        &stub_path,
        &[&add_rlocation, "7", "8"],
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
        return Err(format!(
            "Stub failed with exit code {}.\nstdout: {}\nstderr: {}",
            exit_code, stdout, stderr
        ));
    }

    if !stdout.contains("SUM:15") {
        return Err(format!("Unexpected output: {}. Expected 'SUM:15'", stdout));
    }

    println!("    PASS");

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
        ("merge_json", test_merge_json),
        ("orchestrator_env_propagation", test_orchestrator_env_propagation),
        ("mixed_arguments", test_mixed_arguments),
        ("fallback_runfiles_dir", test_fallback_runfiles_dir),
        ("fallback_runfiles_manifest", test_fallback_runfiles_manifest),
        ("print_env", test_print_env),
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
