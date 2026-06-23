use clap::{ArgAction, Parser};
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process;

const ARG_SIZE: usize = 256;
const ARGC_SIZE: usize = 32;
const UNSET_ENV_SIZE: usize = 256;

/// Finalize a runfiles stub template with actual arguments
#[derive(Parser)]
#[command(name = "finalize-stub")]
#[command(version, about, long_about = None)]
#[command(after_help = "EXAMPLES:\n  \
    # Transform only arg0:\n  \
    finalize-stub --template template --transform 0 --output finalized -- arg0 --flag value\n\n  \
    # Transform arg0 and arg2 (repeated flag):\n  \
    finalize-stub --template template --transform 0 --transform 2 --output output -- arg0 arg1 arg2\n\n  \
    # Transform arg0 and arg2 (comma-separated):\n  \
    finalize-stub --template template --transform 0,2 --output output -- arg0 arg1 arg2\n\n  \
    # No transforms (all arguments are literals):\n  \
    finalize-stub --template template --output output -- /absolute/path --flag")]
struct Cli {
    /// Path to template runfiles-stub binary
    #[arg(short, long, required = true)]
    template: String,

    /// Write output to file (default: stdout)
    #[arg(short, long)]
    output: Option<String>,

    /// Argument indices to transform (0-9). Can be specified multiple times or comma-separated.
    /// If not specified, no arguments are transformed by default.
    #[arg(long, action = ArgAction::Append, value_delimiter = ',', value_parser = clap::value_parser!(u32).range(0..10))]
    transform: Vec<u32>,

    /// Export runfiles environment variables (RUNFILES_DIR, RUNFILES_MANIFEST_FILE, JAVA_RUNFILES) to the executed process
    #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
    export_runfiles_env: bool,

    /// Environment variable names to remove before starting the executed process
    #[arg(long = "unset-env", action = ArgAction::Append, allow_hyphen_values = true)]
    unset_environment: Vec<String>,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Arguments to embed in the stub (argv[0], argv[1], ...)
    #[arg(required = true)]
    args: Vec<String>,
}

fn find_pattern(data: &[u8], pattern: &[u8]) -> Option<usize> {
    data.windows(pattern.len())
        .position(|window| window == pattern)
}

fn find_nth_pattern(data: &[u8], pattern: &[u8], n: usize) -> Option<usize> {
    let mut count = 0;
    let mut pos = 0;

    while pos < data.len() {
        if let Some(offset) = find_pattern(&data[pos..], pattern) {
            if count == n {
                return Some(pos + offset);
            }
            count += 1;
            // Skip past the entire matched pattern to avoid overlapping matches
            pos += offset + pattern.len();
        } else {
            break;
        }
    }
    None
}

fn replace_at(data: &mut [u8], offset: usize, new_value: &[u8], fixed_size: usize) -> Result<(), String> {
    if new_value.len() > fixed_size {
        return Err(format!(
            "Value too long: {} bytes > {} bytes max",
            new_value.len(),
            fixed_size
        ));
    }

    // Zero out the entire region
    for i in 0..fixed_size {
        data[offset + i] = 0;
    }

    // Copy new value
    data[offset..offset + new_value.len()].copy_from_slice(new_value);

    Ok(())
}

fn finalize_stub(template_path: &str, output_path: Option<&str>, argv: &[String], transform_flags: u32, export_runfiles_env: bool, unset_environment: &[String], verbose: bool) -> Result<(), String> {
    if argv.is_empty() {
        return Err("At least one argument (argv[0]) is required".to_string());
    }

    if argv.len() > 10 {
        return Err("Maximum 10 arguments supported (argv[0] to argv[9])".to_string());
    }

    let mut unset_environment_data = Vec::new();
    for name in unset_environment {
        if name.is_empty() {
            return Err("Environment variable name to unset cannot be empty".to_string());
        }
        if name.as_bytes().contains(&b'=') {
            return Err(format!("Environment variable name to unset cannot contain '=': {name}"));
        }
        if !unset_environment_data.is_empty() {
            unset_environment_data.push(0);
        }
        unset_environment_data.extend_from_slice(name.as_bytes());
        if unset_environment_data.len() > UNSET_ENV_SIZE {
            return Err(format!(
                "Environment variable names to unset require {} bytes; maximum is {} bytes",
                unset_environment_data.len(), UNSET_ENV_SIZE
            ));
        }
    }

    // Prevent overwriting the input file
    if let Some(output) = output_path {
        let template_canon = fs::canonicalize(template_path)
            .map_err(|e| format!("Failed to resolve template path: {}", e))?;
        let output_canon = fs::canonicalize(output).ok();

        if output_canon.as_ref() == Some(&template_canon) {
            return Err("Output path cannot be the same as template path (would overwrite input)".to_string());
        }
    }

    // Read template
    let mut data = fs::read(template_path)
        .map_err(|e| format!("Failed to read template {}: {}", template_path, e))?;

    // Find and replace ARGC
    let argc_pattern = b"@@RUNFILES_ARGC@@";
    let argc_pos = find_pattern(&data, argc_pattern)
        .ok_or("ARGC placeholder not found in template")?;

    let argc_str = argv.len().to_string();
    replace_at(&mut data, argc_pos, argc_str.as_bytes(), ARGC_SIZE)?;

    if verbose {
        eprintln!("Replaced ARGC with: {}", argc_str);
    }

    // Find and replace TRANSFORM_FLAGS
    let flags_pattern = b"@@RUNFILES_TRANSFORM_FLAGS@@";
    let flags_pos = find_pattern(&data, flags_pattern)
        .ok_or("TRANSFORM_FLAGS placeholder not found in template")?;

    let flags_str = transform_flags.to_string();
    replace_at(&mut data, flags_pos, flags_str.as_bytes(), 32)?;

    if verbose {
        eprintln!("Replaced TRANSFORM_FLAGS with: {} (0b{:b})", flags_str, transform_flags);
    }

    // Find and replace EXPORT_RUNFILES_ENV
    let export_pattern = b"@@RUNFILES_EXPORT_ENV@@";
    let export_pos = find_pattern(&data, export_pattern)
        .ok_or("EXPORT_RUNFILES_ENV placeholder not found in template")?;

    let export_str = if export_runfiles_env { "1" } else { "0" };
    replace_at(&mut data, export_pos, export_str.as_bytes(), 32)?;

    if verbose {
        eprintln!("Replaced EXPORT_RUNFILES_ENV with: {}", export_str);
    }

    // Preserve compatibility with older templates when no environment names are
    // requested. A nonempty list requires the corresponding runtime capability.
    if !unset_environment_data.is_empty() {
        const UNSET_MARKER: &[u8] = b"@@RUNFILES_UNSET_ENV=@@";
        let mut unset_pattern = [0; UNSET_ENV_SIZE];
        unset_pattern[..UNSET_MARKER.len()].copy_from_slice(UNSET_MARKER);
        let unset_pos = find_pattern(&data, &unset_pattern)
            .ok_or("UNSET_ENVIRONMENT placeholder not found; template does not support --unset-env")?;
        replace_at(&mut data, unset_pos, &unset_environment_data, UNSET_ENV_SIZE)?;

        if verbose {
            eprintln!("Embedded {} environment variable name(s) to unset", unset_environment.len());
        }
    }

    // Find and replace ARG placeholders
    let arg_pattern = &[b'@'; ARG_SIZE];

    // Find all placeholder positions FIRST (before any replacements modify the data)
    let mut arg_positions: Vec<usize> = Vec::new();
    for i in 0..argv.len() {
        let arg_pos = find_nth_pattern(&data, arg_pattern, i)
            .ok_or(format!("ARG{} placeholder not found in template", i))?;
        arg_positions.push(arg_pos);
    }

    // Now do the replacements
    for (i, arg) in argv.iter().enumerate() {
        let arg_pos = arg_positions[i];
        replace_at(&mut data, arg_pos, arg.as_bytes(), ARG_SIZE)?;
        if verbose {
            eprintln!("Replaced ARG{} with: {}", i, arg);
        }
    }

    // Post-process the finalized binary (e.g., re-signing)
    data = post_process_binary(data, verbose)?;

    // Write output
    if let Some(output) = output_path {
        fs::write(output, &data)
            .map_err(|e| format!("Failed to write output {}: {}", output, e))?;

        // Make executable (Unix only)
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(output)
                .map_err(|e| format!("Failed to get metadata: {}", e))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(output, perms)
                .map_err(|e| format!("Failed to set permissions: {}", e))?;
        }

        if verbose {
            eprintln!("\nFinalized stub written to: {}", output);
            eprintln!("Total arguments: {}", argv.len());
        }
    } else {
        // Write to stdout
        io::stdout().write_all(&data)
            .map_err(|e| format!("Failed to write to stdout: {}", e))?;
    }

    Ok(())
}

/// Post-processes a finalized binary based on its format
fn post_process_binary(data: Vec<u8>, verbose: bool) -> Result<Vec<u8>, String> {
    // Try Mach-O signing first
    if is_macho(&data) {
        if verbose {
            eprintln!("Post-processing: Detected Mach-O binary");
        }
        return resign_macho(data, verbose);
    }

    // Future: Add Windows PE signing here
    // if is_pe(&data) {
    //     if verbose {
    //         eprintln!("Post-processing: Detected PE binary");
    //     }
    //     return sign_pe(data, verbose);
    // }

    // No post-processing needed for this binary format
    Ok(data)
}

/// Checks if data is a Mach-O binary by examining magic bytes
fn is_macho(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }

    // Check for Mach-O magic numbers
    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    matches!(magic,
        0xfeedface |  // MH_MAGIC (32-bit)
        0xfeedfacf |  // MH_MAGIC_64 (64-bit)
        0xcefaedfe |  // MH_CIGAM (32-bit, swapped)
        0xcffaedfe    // MH_CIGAM_64 (64-bit, swapped)
    )
}

/// Re-signs a Mach-O binary with an ad-hoc signature
fn resign_macho(data: Vec<u8>, verbose: bool) -> Result<Vec<u8>, String> {
    use apple_codesign::{MachOSigner, SigningSettings};

    // Parse the Mach-O binary
    let signer = MachOSigner::new(&data)
        .map_err(|e| format!("Failed to parse Mach-O binary: {}", e))?;

    if verbose {
        eprintln!("Applying ad-hoc signature to Mach-O binary...");
    }

    // Create ad-hoc signing settings (no certificate = ad-hoc)
    let mut settings = SigningSettings::default();
    settings.set_binary_identifier(apple_codesign::SettingsScope::Main, "runfiles-stub");

    // Sign the binary
    let mut signed_data = Vec::new();
    signer.write_signed_binary(&settings, &mut signed_data)
        .map_err(|e| format!("Failed to sign Mach-O binary: {}", e))?;

    if verbose {
        eprintln!("Successfully signed Mach-O binary");
        eprintln!("  Original size: {} bytes", data.len());
        eprintln!("  Signed size: {} bytes", signed_data.len());
    }

    Ok(signed_data)
}

fn main() {
    let cli = Cli::parse();

    // Calculate transform flags bitmask
    let transform_flags = if cli.transform.is_empty() {
        // Default: transform none
        0
    } else {
        // Only transform specified indices
        let mut flags = 0u32;
        for idx in cli.transform {
            flags |= 1 << idx;
        }
        flags
    };

    match finalize_stub(&cli.template, cli.output.as_deref(), &cli.args, transform_flags, cli.export_runfiles_env, &cli.unset_environment, cli.verbose) {
        Ok(()) => {
            if cli.verbose {
                if let Some(output) = cli.output {
                    eprintln!("\nSuccess! Run with:");
                    eprintln!("  RUNFILES_DIR=<dir> {}", output);
                    eprintln!("  or");
                    eprintln!("  RUNFILES_MANIFEST_FILE=<file> {}", output);
                }
            }
            // If writing to stdout, don't print success message (binary data was written)
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}
