//! Test runner for runfiles-stub
//!
//! This test runner validates the runfiles-stub functionality by:
//! 1. Setting up a realistic runfiles tree with demo binaries and test data
//! 2. Creating a manifest file that matches the runfiles tree
//! 3. Using the finalizer to create stub binaries
//! 4. Running the stubs and validating their behavior
//!
//! Usage: test-runner --template <path> --finalizer <path> --v1-template <path>
//!     --v1-finalizer <path> --test-binaries <dir>
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
    /// Path to the published V1 runfiles-stub template
    v1_template_path: PathBuf,
    /// Path to the published V1 finalize-stub binary
    v1_finalizer_path: PathBuf,
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

fn assert_v1_artifact(path: &Path, label: &str) -> Result<(), String> {
    const V1_MARKER: &[u8] = b"@@RUNFILES_EXPORT_ENV@@";
    const V2_MARKER: &[u8] = b"@@RUNFILES_EXPORT_ENV@@V2";

    let data = fs::read(path)
        .map_err(|err| format!("Failed to read {label} {}: {err}", path.display()))?;
    let contains = |needle: &[u8]| data.windows(needle.len()).any(|window| window == needle);
    if !contains(V1_MARKER) || contains(V2_MARKER) {
        return Err(format!(
            "{label} {} is not a V1 compatibility artifact",
            path.display()
        ));
    }
    Ok(())
}

impl TestConfig {
    fn from_args() -> Result<Self, String> {
        let args: Vec<String> = env::args().collect();

        let mut template_path = None;
        let mut finalizer_path = None;
        let mut v1_template_path = None;
        let mut v1_finalizer_path = None;
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
                "--v1-template" => {
                    i += 1;
                    v1_template_path = Some(PathBuf::from(&args[i]));
                }
                "--v1-finalizer" => {
                    i += 1;
                    v1_finalizer_path = Some(PathBuf::from(&args[i]));
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
                    println!("Usage: test-runner --template <path> --finalizer <path> --v1-template <path> --v1-finalizer <path> --test-binaries <dir> [--work-dir <dir>]");
                    println!();
                    println!("Options:");
                    println!("  --template       Path to runfiles-stub template binary");
                    println!("  --finalizer      Path to finalize-stub binary");
                    println!("  --v1-template    Path to published V1 runfiles-stub template");
                    println!("  --v1-finalizer   Path to published V1 finalize-stub binary");
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
        let v1_template_path = resolve_runfile_path(
            &runfiles,
            v1_template_path.ok_or("--v1-template is required")?,
        );
        let v1_finalizer_path = resolve_runfile_path(
            &runfiles,
            v1_finalizer_path.ok_or("--v1-finalizer is required")?,
        );
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
        if !v1_template_path.exists() {
            return Err(format!(
                "Published V1 template not found: {}",
                v1_template_path.display()
            ));
        }
        if !v1_finalizer_path.exists() {
            return Err(format!(
                "Published V1 finalizer not found: {}",
                v1_finalizer_path.display()
            ));
        }
        assert_v1_artifact(&v1_template_path, "Published V1 template")?;
        assert_v1_artifact(&v1_finalizer_path, "Published V1 finalizer")?;
        if !test_binaries_dir.exists() {
            return Err(format!("Test binaries dir not found: {}", test_binaries_dir.display()));
        }

        Ok(Self {
            template_path,
            finalizer_path,
            v1_template_path,
            v1_finalizer_path,
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
    finalize_stub_with_fallbacks(config, output_path, args, transform_indices, &[], true)
}

fn finalize_stub_with_fallbacks(
    config: &TestConfig,
    output_path: &Path,
    args: &[&str],
    transform_indices: &[usize],
    fallbacks: &[(usize, &str)],
    export_runfiles_env: bool,
) -> Result<(), String> {
    finalize_stub_with_tools(
        &config.finalizer_path,
        &config.template_path,
        output_path,
        args,
        transform_indices,
        fallbacks,
        export_runfiles_env,
    )
}

fn finalize_stub_with_tools(
    finalizer_path: &Path,
    template_path: &Path,
    output_path: &Path,
    args: &[&str],
    transform_indices: &[usize],
    fallbacks: &[(usize, &str)],
    export_runfiles_env: bool,
) -> Result<(), String> {
    let mut cmd = Command::new(finalizer_path);
    cmd.arg("--template").arg(template_path);
    cmd.arg("--output").arg(output_path);

    // Add transform flags
    if !transform_indices.is_empty() {
        let transform_str: Vec<String> = transform_indices.iter().map(|i| i.to_string()).collect();
        cmd.arg("--transform").arg(transform_str.join(","));
    }

    for (index, path) in fallbacks {
        cmd.arg("--fallback").arg(format!("{}={}", index, path));
    }
    cmd.arg("--export-runfiles-env")
        .arg(export_runfiles_env.to_string());

    cmd.arg("--");

    // Add arguments
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd.output().map_err(|e| format!("Failed to run finalizer: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Finalizer {} failed: {}",
            finalizer_path.display(),
            stderr
        ));
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

fn run_successful_stub(command: &mut Command, context: &str) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|e| format!("Failed to run {}: {}", context, e))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "{} failed with status {}\nstdout: {}\nstderr: {}",
            context, output.status, stdout, stderr
        ));
    }
    Ok(stdout)
}

fn assert_argv0(stdout: &str, expected: &str, context: &str) -> Result<(), String> {
    let actual = stdout
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("ARGS:"))
        .and_then(|args| args.split('|').next());
    if actual != Some(expected) {
        return Err(format!(
            "{} selected the wrong executable path\nexpected argv0: {}\nstdout: {}",
            context, expected, stdout
        ));
    }
    Ok(())
}

fn assert_runfiles_env(
    stdout: &str,
    expected: &[(&str, Option<&Path>)],
    context: &str,
) -> Result<(), String> {
    for (variable, value) in expected {
        let expected = value.map_or_else(
            || format!("ENV:{}=<unset>", variable),
            |path| format!("ENV:{}={}", variable, path.display()),
        );
        if !stdout.contains(&expected) {
            return Err(format!(
                "{} has the wrong runfiles environment\nexpected: {}\nstdout: {}",
                context, expected, stdout
            ));
        }
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

    // The adjacent logical runfiles tree intentionally does not exist. The
    // manifest maps the executable elsewhere, while argv[0] retains this path
    // so runfiles-aware children can recover their logical identity.
    let runfiles_dir = test_dir.join(format!("manifest_stub{}.runfiles", EXE_EXT));
    let print_env_binary = config
        .test_binaries_dir
        .join(format!("print-env{}", EXE_EXT));

    // Write the manifest file (key value pairs separated by space)
    let print_env_rlocation = format!("{}/bin/print-env{}", WORKSPACE_NAME, EXE_EXT);
    let manifest_content = format!("{} {}\n", print_env_rlocation, print_env_binary.display());
    fs::write(&manifest_path, manifest_content)
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

    // Create the stub
    finalize_stub(config, &stub_path, &[&print_env_rlocation], &[0])?;

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

    #[cfg(unix)]
    let expected_argv0 = runfiles_dir.join(&print_env_rlocation);
    #[cfg(windows)]
    let expected_argv0 = print_env_binary;
    assert_argv0(
        &stdout,
        &expected_argv0.to_string_lossy(),
        "adjacent manifest argv0",
    )?;

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

fn test_executable_fallback_selection(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: executable_fallback_selection");

    let test_dir = config.work_dir.join("test_executable_fallback_selection");
    // The Windows launcher obtains this path as UTF-16. A code point above
    // U+00FF catches the old low-byte narrowing before fallback resolution.
    let stub_dir = test_dir.join("launcher-東京");
    let other_cwd = test_dir.join("other-cwd");
    fs::create_dir_all(&stub_dir)
        .map_err(|e| format!("Failed to create {}: {}", stub_dir.display(), e))?;
    fs::create_dir_all(&other_cwd)
        .map_err(|e| format!("Failed to create {}: {}", other_cwd.display(), e))?;

    let print_env_binary = config
        .test_binaries_dir
        .join(format!("print-env{}", EXE_EXT));
    let key = format!("{}/bin/print-env{}", WORKSPACE_NAME, EXE_EXT);
    let mut primary = RunfilesSetup::new(&test_dir, "primary-東京")
        .map_err(|e| format!("Failed to create runfiles: {}", e))?;
    primary
        .add_file(&key, &print_env_binary)
        .map_err(|e| format!("Failed to add primary executable: {}", e))?;
    primary
        .write_manifest()
        .map_err(|e| format!("Failed to write primary manifest: {}", e))?;
    let fallback_arg = format!("../primary-東京.runfiles/{}", key);

    let stub_path = stub_dir.join(format!("stub{}", EXE_EXT));
    finalize_stub_with_fallbacks(
        config,
        &stub_path,
        &[&key],
        &[0],
        &[(0, &fallback_arg)],
        false,
    )?;

    let fallback_argv0 = format!(
        "{}{}{}",
        stub_dir.display(),
        PATH_SEP,
        fallback_arg.replace('/', &PATH_SEP.to_string())
    );
    let primary_argv0 = format!(
        "{}{}{}",
        primary.runfiles_dir.display(),
        PATH_SEP,
        key.replace('/', &PATH_SEP.to_string())
    );
    let stale_runfiles = test_dir.join("does-not-exist.runfiles");
    let runfiles_file = test_dir.join("not-a-runfiles-directory");
    fs::write(&runfiles_file, b"not a directory")
        .map_err(|e| format!("Failed to write non-directory runfiles fixture: {}", e))?;
    let stale_manifest = test_dir.join("stale.runfiles_manifest");
    fs::write(
        &stale_manifest,
        format!("{} {}\n", key, test_dir.join("missing-primary").display()),
    )
    .map_err(|e| format!("Failed to write stale manifest: {}", e))?;
    let cases = [
        ("stale RUNFILES_DIR", None, fallback_argv0.as_str()),
        (
            "non-directory RUNFILES_DIR",
            Some(("RUNFILES_DIR", runfiles_file.as_path())),
            fallback_argv0.as_str(),
        ),
        (
            "stale manifest target",
            Some(("RUNFILES_MANIFEST_FILE", stale_manifest.as_path())),
            fallback_argv0.as_str(),
        ),
        (
            "primary runfile",
            Some(("RUNFILES_DIR", primary.runfiles_dir.as_path())),
            primary_argv0.as_str(),
        ),
        (
            "primary manifest",
            Some(("RUNFILES_MANIFEST_FILE", primary.manifest_path.as_path())),
            primary_argv0.as_str(),
        ),
    ];
    for (name, runfiles_setting, expected_argv0) in cases {
        let mut command = Command::new(&stub_path);
        let missing_manifest = test_dir.join("missing-manifest");
        let mut expected_runfiles_dir = stale_runfiles.as_path();
        let mut expected_manifest = missing_manifest.as_path();
        command
            .env_clear()
            .current_dir(&other_cwd)
            .env("RUNFILES_DIR", &stale_runfiles)
            .env("RUNFILES_MANIFEST_FILE", &missing_manifest)
            .env("JAVA_RUNFILES", &stale_runfiles);
        if let Some((name, value)) = runfiles_setting {
            command.env(name, value);
            if name == "RUNFILES_DIR" {
                expected_runfiles_dir = value;
            } else if name == "RUNFILES_MANIFEST_FILE" {
                expected_manifest = value;
            }
        }
        let stdout = run_successful_stub(&mut command, name)?;
        assert_argv0(&stdout, expected_argv0, name)?;
        assert_runfiles_env(
            &stdout,
            &[
                ("RUNFILES_DIR", Some(expected_runfiles_dir)),
                ("RUNFILES_MANIFEST_FILE", Some(expected_manifest)),
                ("JAVA_RUNFILES", Some(stale_runfiles.as_path())),
            ],
            name,
        )?;
    }

    #[cfg(unix)]
    {
        let linked_stub_dir = test_dir.join("linked-launcher");
        std::os::unix::fs::symlink(&stub_dir, &linked_stub_dir)
            .map_err(|e| format!("Failed to create launcher directory symlink: {}", e))?;
        let mut command = Command::new(linked_stub_dir.join(format!("stub{}", EXE_EXT)));
        command.env_clear().current_dir(&other_cwd);
        let stdout = run_successful_stub(&mut command, "symlinked launcher fallback")?;
        let linked_argv0 = format!(
            "{}{}{}",
            linked_stub_dir.display(),
            PATH_SEP,
            fallback_arg.replace('/', &PATH_SEP.to_string())
        );
        assert_argv0(&stdout, &linked_argv0, "symlinked launcher fallback")?;
    }

    let exporting_stub = stub_dir.join(format!("exporting-stub{}", EXE_EXT));
    finalize_stub_with_fallbacks(
        config,
        &exporting_stub,
        &[&key],
        &[0],
        &[(0, &fallback_arg)],
        true,
    )?;
    let mut exporting_command = Command::new(&exporting_stub);
    exporting_command
        .env_clear()
        .env("RUNFILES_DIR", &runfiles_file)
        .env("RUNFILES_MANIFEST_FILE", test_dir.join("missing-manifest"));
    let stdout = run_successful_stub(
        &mut exporting_command,
        "exporting fallback stub without runfiles",
    )?;
    assert_argv0(
        &stdout,
        &fallback_argv0,
        "exporting fallback stub without runfiles",
    )?;
    assert_runfiles_env(
        &stdout,
        &[
            ("RUNFILES_DIR", None),
            ("RUNFILES_MANIFEST_FILE", None),
            ("JAVA_RUNFILES", None),
        ],
        "exporting fallback stub without runfiles",
    )?;

    let mut unicode_primary_command = Command::new(&exporting_stub);
    unicode_primary_command
        .env_clear()
        .env("RUNFILES_DIR", &primary.runfiles_dir);
    let stdout = run_successful_stub(
        &mut unicode_primary_command,
        "exporting fallback stub with Unicode primary runfiles",
    )?;
    assert_argv0(
        &stdout,
        &primary_argv0,
        "exporting fallback stub with Unicode primary runfiles",
    )?;
    assert_runfiles_env(
        &stdout,
        &[
            ("RUNFILES_DIR", Some(primary.runfiles_dir.as_path())),
            ("JAVA_RUNFILES", Some(primary.runfiles_dir.as_path())),
        ],
        "exporting fallback stub with Unicode primary runfiles",
    )?;

    // A transformed absolute argument does not use runfiles and has no
    // fallback. Preserve the existing export contract: the discovered context
    // still replaces the child's inherited runfiles variables.
    let absolute_stub = stub_dir.join(format!("absolute-stub{}", EXE_EXT));
    let absolute_program = print_env_binary.to_string_lossy();
    finalize_stub_with_fallbacks(
        config,
        &absolute_stub,
        &[absolute_program.as_ref()],
        &[0],
        &[],
        true,
    )?;
    let mut absolute_command = Command::new(&absolute_stub);
    absolute_command
        .env_clear()
        .env("RUNFILES_DIR", &primary.runfiles_dir);
    let stdout = run_successful_stub(
        &mut absolute_command,
        "transformed absolute executable with runfiles",
    )?;
    assert_argv0(
        &stdout,
        absolute_program.as_ref(),
        "transformed absolute executable with runfiles",
    )?;
    assert_runfiles_env(
        &stdout,
        &[
            ("RUNFILES_DIR", Some(primary.runfiles_dir.as_path())),
            ("JAVA_RUNFILES", Some(primary.runfiles_dir.as_path())),
        ],
        "transformed absolute executable with runfiles",
    )?;

    // A valid but unrelated context must not leak into a child selected through
    // executable-relative fallback. A nested launcher would otherwise mistake
    // this manifest for its own runfiles and disable usable physical paths.
    let child_manifest = test_dir.join("child-東京.runfiles_manifest");
    fs::write(
        &child_manifest,
        format!(
            "{}/child-only {}\n",
            WORKSPACE_NAME,
            print_env_binary.display()
        ),
    )
    .map_err(|e| format!("Failed to write child-only manifest: {}", e))?;
    let mut child_manifest_command = Command::new(&exporting_stub);
    child_manifest_command
        .env_clear()
        .env("RUNFILES_MANIFEST_FILE", &child_manifest);
    let stdout = run_successful_stub(
        &mut child_manifest_command,
        "fallback stub with child-only runfiles",
    )?;
    assert_argv0(
        &stdout,
        &fallback_argv0,
        "fallback stub with child-only runfiles",
    )?;
    assert_runfiles_env(
        &stdout,
        &[
            ("RUNFILES_DIR", None),
            ("RUNFILES_MANIFEST_FILE", None),
            ("JAVA_RUNFILES", None),
        ],
        "fallback stub with child-only runfiles",
    )?;

    println!(
        "    PASS (Unicode path, inherited no-export env, fallback export, primary runfile, and unrelated-manifest scrub)"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn test_non_utf8_executable_relative_fallback(config: &TestConfig) -> Result<(), String> {
    use std::fmt::Write as _;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::PermissionsExt;

    println!("  Running test: non_utf8_executable_relative_fallback");

    let test_dir = config
        .work_dir
        .join("test_non_utf8_executable_relative_fallback");
    let mut directory_name = b"launcher-".to_vec();
    directory_name.push(0xff);
    let stub_dir = test_dir.join(std::ffi::OsString::from_vec(directory_name));
    let fallback_dir = stub_dir.join("fallback");
    let other_cwd = test_dir.join("other-cwd");
    fs::create_dir_all(&fallback_dir)
        .map_err(|e| format!("Failed to create non-UTF-8 launcher directory: {}", e))?;
    fs::create_dir_all(&other_cwd)
        .map_err(|e| format!("Failed to create {}: {}", other_cwd.display(), e))?;

    let opaque_binary = config.test_binaries_dir.join("print-opaque-argv");
    let fallback_binary = fallback_dir.join("print-opaque-argv");
    fs::copy(&opaque_binary, &fallback_binary)
        .map_err(|e| format!("Failed to copy opaque argv binary: {}", e))?;
    let mut permissions = fs::metadata(&fallback_binary)
        .map_err(|e| format!("Failed to stat opaque argv binary: {}", e))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fallback_binary, permissions)
        .map_err(|e| format!("Failed to make opaque argv binary executable: {}", e))?;

    let finalized = test_dir.join("finalized-stub");
    finalize_stub_with_fallbacks(
        config,
        &finalized,
        &["_main/missing-opaque-primary"],
        &[0],
        &[(0, "fallback/print-opaque-argv")],
        false,
    )?;
    let stub_path = stub_dir.join("stub");
    fs::rename(&finalized, &stub_path)
        .map_err(|e| format!("Failed to move launcher into non-UTF-8 directory: {}", e))?;

    let mut command = Command::new(&stub_path);
    command.env_clear().current_dir(&other_cwd);
    let stdout = run_successful_stub(&mut command, "non-UTF-8 launcher fallback")?;

    let mut expected_hex = String::new();
    for byte in fallback_binary.as_os_str().as_bytes() {
        write!(&mut expected_hex, "{byte:02x}")
            .map_err(|e| format!("Failed to format expected argv[0]: {}", e))?;
    }
    let expected = format!("ARGV0_HEX:{expected_hex}");
    if !stdout.contains(&expected) {
        return Err(format!(
            "Executable-relative fallback narrowed the non-UTF-8 launcher path\nexpected: {}\nstdout: {}",
            expected, stdout
        ));
    }

    println!("    PASS");
    Ok(())
}

#[cfg(windows)]
fn test_windows_extended_paths(config: &TestConfig) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    println!("  Running test: windows_extended_paths");

    let test_dir = config.work_dir.join("test_windows_extended_paths");
    let stub_dir = test_dir
        .join(format!("launcher-東京-{}", "a".repeat(180)))
        .join(format!("nested-{}", "b".repeat(100)));
    fs::create_dir_all(&stub_dir)
        .map_err(|e| format!("Failed to create long launcher directory: {}", e))?;
    if stub_dir.as_os_str().encode_wide().count() <= 260 {
        return Err(format!(
            "Windows extended-path fixture did not exceed MAX_PATH: {}",
            stub_dir.display()
        ));
    }

    let source_binary = config.test_binaries_dir.join("print-env.exe");
    let fallback_binary = stub_dir.join("fallback-東京").join("print-env.exe");
    let primary_binary = stub_dir.join("primary-主要").join("print-env.exe");
    for destination in [&fallback_binary, &primary_binary] {
        fs::create_dir_all(destination.parent().unwrap())
            .map_err(|e| format!("Failed to create {}: {}", destination.display(), e))?;
        fs::copy(&source_binary, destination)
            .map_err(|e| format!("Failed to copy {}: {}", destination.display(), e))?;
    }

    let assert_path = |stdout: &str, expected: &Path, context: &str| -> Result<(), String> {
        let actual = stdout
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("ARGS:"))
            .and_then(|args| args.split('|').next())
            .ok_or_else(|| format!("{} did not print argv[0]\nstdout: {}", context, stdout))?;
        let actual = if let Some(rest) = actual.strip_prefix("\\\\?\\UNC\\") {
            format!("\\\\{rest}")
        } else if let Some(rest) = actual.strip_prefix("\\\\?\\") {
            rest.to_string()
        } else {
            actual.to_string()
        };
        let expected = expected.to_string_lossy();
        if !actual.eq_ignore_ascii_case(&expected) {
            return Err(format!(
                "{} selected the wrong long path\nexpected argv0: {}\nactual argv0: {}\nstdout: {}",
                context, expected, actual, stdout
            ));
        }
        Ok(())
    };

    let fallback_stub = stub_dir.join("fallback-stub.exe");
    finalize_stub_with_fallbacks(
        config,
        &fallback_stub,
        &["_main/missing-long-primary.exe"],
        &[0],
        &[(0, "fallback-東京/print-env.exe")],
        false,
    )?;
    let mut fallback_command = Command::new(&fallback_stub);
    fallback_command.env_clear();
    let stdout = run_successful_stub(&mut fallback_command, "long Windows fallback")?;
    assert_path(&stdout, &fallback_binary, "long Windows fallback")?;

    let key = "_main/bin/print-env.exe";
    let manifest_path = stub_dir.join("long-primary.runfiles_manifest");
    fs::write(
        &manifest_path,
        format!(
            "{} {}\n",
            key,
            primary_binary.to_string_lossy().replace('\\', "/")
        ),
    )
    .map_err(|e| format!("Failed to write long manifest: {}", e))?;
    let manifest_stub = stub_dir.join("manifest-stub.exe");
    finalize_stub_with_fallbacks(
        config,
        &manifest_stub,
        &[key],
        &[0],
        &[(0, "fallback-東京/print-env.exe")],
        false,
    )?;
    let mut manifest_command = Command::new(&manifest_stub);
    manifest_command
        .env_clear()
        .env("RUNFILES_MANIFEST_FILE", &manifest_path);
    let stdout = run_successful_stub(&mut manifest_command, "long Windows manifest primary")?;
    assert_path(&stdout, &primary_binary, "long Windows manifest primary")?;

    println!("    PASS (fallback, manifest, existence check, and CreateProcessW)");
    Ok(())
}

fn test_transformed_data_argument_falls_back(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: transformed_data_argument_falls_back");

    let test_dir = config.work_dir.join("test_data_argument_fallback");
    let stub_dir = test_dir.join("launcher");
    fs::create_dir_all(&stub_dir)
        .map_err(|e| format!("Failed to create {}: {}", stub_dir.display(), e))?;
    let fallback_arg = "../fallback/input.txt";
    let fallback_path = test_dir.join("fallback").join("input.txt");
    fs::create_dir_all(fallback_path.parent().unwrap())
        .map_err(|e| format!("Failed to create fallback data directory: {}", e))?;
    fs::write(&fallback_path, b"Hello, World!\n")
        .map_err(|e| format!("Failed to write fallback data: {}", e))?;

    let hash_binary = config
        .test_binaries_dir
        .join(format!("hash-file{}", EXE_EXT));
    let hash_key = format!("{}/bin/hash-file{}", WORKSPACE_NAME, EXE_EXT);
    let mut primary = RunfilesSetup::new(&test_dir, "primary")
        .map_err(|e| format!("Failed to create primary runfiles: {}", e))?;
    primary
        .add_file(&hash_key, &hash_binary)
        .map_err(|e| format!("Failed to add primary hash executable: {}", e))?;
    let missing_data_key = format!("{}/missing/input.txt", WORKSPACE_NAME);
    let stub_path = stub_dir.join(format!("stub{}", EXE_EXT));
    finalize_stub_with_fallbacks(
        config,
        &stub_path,
        &[&hash_key, &missing_data_key],
        &[0, 1],
        &[(1, fallback_arg)],
        false,
    )?;

    let missing_runfiles = Command::new(&stub_path)
        .env_clear()
        .output()
        .map_err(|e| format!("Failed to run incomplete-fallback stub: {}", e))?;
    let missing_stdout = String::from_utf8_lossy(&missing_runfiles.stdout);
    if missing_runfiles.status.success()
        || !missing_stdout.contains("Failed to initialize runfiles")
    {
        return Err(format!(
            "Launcher accepted incomplete fallback coverage without runfiles\nstdout: {}\nstderr: {}",
            missing_stdout,
            String::from_utf8_lossy(&missing_runfiles.stderr)
        ));
    }

    let mut command = Command::new(&stub_path);
    command
        .env_clear()
        .env("RUNFILES_DIR", &primary.runfiles_dir);
    let stdout = run_successful_stub(&mut command, "data-argument fallback stub")?;
    let expected_hash = "c98c24b677eff44860afea6f493bbaec5bb1c4cbb209c6fc2bbb47f66ff2ad31";
    if stdout.trim().to_lowercase() != format!("sha256:{}", expected_hash) {
        return Err(format!(
            "Transformed data argument did not select its executable-relative fallback\nstdout: {}",
            stdout
        ));
    }

    println!("    PASS");
    Ok(())
}

fn test_no_transformed_arguments(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: no_transformed_arguments");

    let test_dir = config.work_dir.join("test_no_transformed_arguments");
    fs::create_dir_all(&test_dir)
        .map_err(|e| format!("Failed to create {}: {}", test_dir.display(), e))?;
    let print_env_binary = config
        .test_binaries_dir
        .join(format!("print-env{}", EXE_EXT));
    let print_env_binary = print_env_binary.to_string_lossy();

    let exporting_stub = test_dir.join(format!("exporting{}", EXE_EXT));
    finalize_stub_with_fallbacks(
        config,
        &exporting_stub,
        &[print_env_binary.as_ref()],
        &[],
        &[],
        true,
    )?;
    let output = Command::new(&exporting_stub)
        .env_clear()
        .output()
        .map_err(|e| format!("Failed to run no-transform exporting stub: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() || !stdout.contains("Failed to initialize runfiles") {
        return Err(format!(
            "No-transform exporting stub ran without runfiles\nstdout: {}\nstderr: {}",
            stdout,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let non_exporting_stub = test_dir.join(format!("non-exporting{}", EXE_EXT));
    finalize_stub_with_fallbacks(
        config,
        &non_exporting_stub,
        &[print_env_binary.as_ref()],
        &[],
        &[],
        false,
    )?;
    let mut command = Command::new(&non_exporting_stub);
    let inherited_runfiles = test_dir.join("inherited.runfiles");
    let inherited_manifest = test_dir.join("inherited.runfiles_manifest");
    command
        .env_clear()
        .env("RUNFILES_DIR", &inherited_runfiles)
        .env("RUNFILES_MANIFEST_FILE", &inherited_manifest)
        .env("JAVA_RUNFILES", &inherited_runfiles);
    let stdout = run_successful_stub(&mut command, "no-transform non-exporting stub")?;
    assert_runfiles_env(
        &stdout,
        &[
            ("RUNFILES_DIR", Some(inherited_runfiles.as_path())),
            ("RUNFILES_MANIFEST_FILE", Some(inherited_manifest.as_path())),
            ("JAVA_RUNFILES", Some(inherited_runfiles.as_path())),
        ],
        "no-transform non-exporting stub",
    )?;

    println!("    PASS");
    Ok(())
}

fn test_finalizer_rejects_invalid_fallbacks(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: finalizer_rejects_invalid_fallbacks");

    let cases: &[(&str, &[&str], &[&str], &str)] = &[
        ("invalid", &["invalid"], &["arg0"], "expected N=PATH"),
        (
            "duplicate",
            &["0=first", "0=second"],
            &["arg0"],
            "fallback argument 0 is declared more than once",
        ),
        (
            "non-transformed",
            &["1=path"],
            &["arg0", "arg1"],
            "fallback argument 1 is not marked for runfiles transformation",
        ),
        (
            "out-of-range",
            &["10=path"],
            &["arg0"],
            "fallback argument index 10 exceeds the maximum index 9",
        ),
        (
            "absolute POSIX path",
            &["0=/absolute/path"],
            &["arg0"],
            "fallback argument 0 path \"/absolute/path\" must be executable-relative",
        ),
        (
            "absolute Windows drive path",
            &["0=C:\\absolute\\path"],
            &["arg0"],
            "fallback argument 0 path \"C:\\\\absolute\\\\path\" must be executable-relative",
        ),
        (
            "drive-relative Windows path",
            &["0=C:relative\\path"],
            &["arg0"],
            "fallback argument 0 path \"C:relative\\\\path\" must be executable-relative",
        ),
        (
            "absolute Windows UNC path",
            &["0=\\\\server\\share\\tool"],
            &["arg0"],
            "fallback argument 0 path \"\\\\\\\\server\\\\share\\\\tool\" must be executable-relative",
        ),
        (
            "absolute embedded argument",
            &["0=relative/path"],
            &["/absolute/program"],
            "fallback argument 0 cannot be attached to absolute embedded argument \"/absolute/program\"",
        ),
    ];
    for &(name, fallbacks, args, expected) in cases {
        let mut command = Command::new(&config.finalizer_path);
        command
            .arg("--template")
            .arg(&config.template_path)
            .arg("--transform")
            .arg("0");
        for fallback in fallbacks {
            command.arg("--fallback").arg(fallback);
        }
        let output = command
            .arg("--")
            .args(args)
            .output()
            .map_err(|e| format!("Failed to run {} finalizer case: {}", name, e))?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() {
            return Err(format!("Finalizer accepted {} fallback", name));
        }
        if !stderr.contains(expected) {
            return Err(format!(
                "Finalizer rejected {} fallback with the wrong diagnostic\nexpected: {}\nstderr: {}",
                name, expected, stderr
            ));
        }
    }

    let boundary_output = config.work_dir.join(format!("boundary-stub{}", EXE_EXT));
    let boundary_fallback = "x".repeat(254);
    let boundary = Command::new(&config.finalizer_path)
        .arg("--template")
        .arg(&config.template_path)
        .arg("--output")
        .arg(&boundary_output)
        .arg("--transform")
        .arg("0")
        .arg("--fallback")
        .arg(format!("0={}", boundary_fallback))
        .arg("--")
        .arg("a")
        .output()
        .map_err(|e| format!("Failed to run exact-boundary finalizer case: {}", e))?;
    if !boundary.status.success() {
        return Err(format!(
            "Finalizer rejected the exact 256-byte argument/fallback boundary: {}",
            String::from_utf8_lossy(&boundary.stderr),
        ));
    }

    let overflow = Command::new(&config.finalizer_path)
        .arg("--template")
        .arg(&config.template_path)
        .arg("--transform")
        .arg("0")
        .arg("--fallback")
        .arg(format!("0={}", "x".repeat(255)))
        .arg("--")
        .arg("a")
        .output()
        .map_err(|e| format!("Failed to run overflow finalizer case: {}", e))?;
    let overflow_stderr = String::from_utf8_lossy(&overflow.stderr);
    if overflow.status.success()
        || !overflow_stderr
            .contains("ARG0 and its fallback require 257 bytes; maximum combined size is 256 bytes")
    {
        return Err(format!(
            "Finalizer did not enforce the combined slot boundary\nstderr: {}",
            overflow_stderr,
        ));
    }

    let old_fallback = Command::new(&config.finalizer_path)
        .arg("--template")
        .arg(&config.v1_template_path)
        .arg("--transform")
        .arg("0")
        .arg("--fallback")
        .arg("0=fallback")
        .arg("--")
        .arg("_main/missing")
        .output()
        .map_err(|e| format!("Failed to run published-V1-template fallback case: {}", e))?;
    let old_fallback_stderr = String::from_utf8_lossy(&old_fallback.stderr);
    if old_fallback.status.success()
        || !old_fallback_stderr.contains(
            "template does not support executable-relative fallbacks; use a V2 runfiles stub template",
        )
    {
        return Err(format!(
            "Finalizer did not reject fallback metadata for the published V1 template\nstderr: {}",
            old_fallback_stderr,
        ));
    }

    println!("    PASS (syntax, boundaries, absolutes, and template capability)");
    Ok(())
}

fn test_finalizer_version_compatibility(config: &TestConfig) -> Result<(), String> {
    println!("  Running test: finalizer_version_compatibility");

    let test_dir = config.work_dir.join("test_finalizer_version_compatibility");
    let print_env_binary = config
        .test_binaries_dir
        .join(format!("print-env{}", EXE_EXT));
    let print_env_key = format!("{}/bin/print-env{}", WORKSPACE_NAME, EXE_EXT);
    let mut runfiles = RunfilesSetup::new(&test_dir, "compat")
        .map_err(|e| format!("Failed to create compatibility runfiles: {}", e))?;
    runfiles
        .add_file(&print_env_key, &print_env_binary)
        .map_err(|e| format!("Failed to add compatibility executable: {}", e))?;
    let expected_argv0 = runfiles
        .get_path(&print_env_key)
        .ok_or("Compatibility executable missing from runfiles fixture")?
        .to_string_lossy()
        .into_owned();
    let stale_manifest = test_dir.join("inherited.runfiles_manifest");
    let inherited_java = test_dir.join("inherited-java.runfiles");
    let combinations = [
        (
            "published V1 finalizer with V2 template",
            "v1-finalizer-v2-template",
            config.v1_finalizer_path.as_path(),
            config.template_path.as_path(),
        ),
        (
            "V2 finalizer with published V1 template",
            "v2-finalizer-v1-template",
            config.finalizer_path.as_path(),
            config.v1_template_path.as_path(),
        ),
    ];

    for export_runfiles_env in [false, true] {
        let suffix = if export_runfiles_env {
            "export"
        } else {
            "preserve"
        };
        for (label, output_name, finalizer, template) in combinations {
            let output = test_dir.join(format!("{output_name}-{suffix}{}", EXE_EXT));
            finalize_stub_with_tools(
                finalizer,
                template,
                &output,
                &[&print_env_key],
                &[0],
                &[],
                export_runfiles_env,
            )?;

            let context = format!("{} with export={}", label, export_runfiles_env);
            let mut command = Command::new(&output);
            command
                .env_clear()
                .env("RUNFILES_DIR", &runfiles.runfiles_dir)
                .env("RUNFILES_MANIFEST_FILE", &stale_manifest)
                .env("JAVA_RUNFILES", &inherited_java);
            let stdout = run_successful_stub(&mut command, &context)?;
            assert_argv0(&stdout, &expected_argv0, &context)?;
            let expected_manifest = if export_runfiles_env {
                None
            } else {
                Some(stale_manifest.as_path())
            };
            let expected_java = if export_runfiles_env {
                runfiles.runfiles_dir.as_path()
            } else {
                inherited_java.as_path()
            };
            assert_runfiles_env(
                &stdout,
                &[
                    ("RUNFILES_DIR", Some(runfiles.runfiles_dir.as_path())),
                    ("RUNFILES_MANIFEST_FILE", expected_manifest),
                    ("JAVA_RUNFILES", Some(expected_java)),
                ],
                &context,
            )?;
        }
    }

    println!("    PASS (published V1/V2 execution, transforms, and export settings)");
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
    let manifest_argv0 = runfiles
        .get_path(&print_env_rlocation)
        .ok_or_else(|| format!("Missing manifest entry for {}", print_env_rlocation))?
        .to_string_lossy();
    assert_argv0(&stdout, &manifest_argv0, "manifest runfiles argv0")?;

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
        ("merge_json", test_merge_json),
        ("orchestrator_env_propagation", test_orchestrator_env_propagation),
        ("mixed_arguments", test_mixed_arguments),
        ("fallback_runfiles_dir", test_fallback_runfiles_dir),
        ("fallback_runfiles_manifest", test_fallback_runfiles_manifest),
        ("run_runfiles_discovery", test_run_runfiles_discovery),
        ("relative_manifest_symlinks", test_relative_manifest_symlinks),
        (
            "executable_fallback_selection",
            test_executable_fallback_selection,
        ),
        (
            "transformed_data_argument_falls_back",
            test_transformed_data_argument_falls_back,
        ),
        ("no_transformed_arguments", test_no_transformed_arguments),
        (
            "finalizer_rejects_invalid_fallbacks",
            test_finalizer_rejects_invalid_fallbacks,
        ),
        (
            "finalizer_version_compatibility",
            test_finalizer_version_compatibility,
        ),
        ("print_env", test_print_env),
        ("large_manifest", test_large_manifest),
    ];
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let mut tests = tests;
    #[cfg(target_os = "linux")]
    tests.push((
        "non_utf8_executable_relative_fallback",
        test_non_utf8_executable_relative_fallback,
    ));
    #[cfg(windows)]
    tests.push(("windows_extended_paths", test_windows_extended_paths));

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
