use clap::{ArgAction, Parser};
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process;

mod template;
use template::{prefer, Template};

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
    /// Template binaries to consider (repeat for automatic smallest-compatible selection)
    #[arg(short, long, required = true)]
    template: Vec<String>,

    /// Write output to file (default: stdout)
    #[arg(short, long)]
    output: Option<String>,

    /// Argument indices to transform. Can be specified multiple times or comma-separated.
    /// If not specified, no arguments are transformed by default.
    #[arg(long, action = ArgAction::Append, value_delimiter = ',', value_parser = clap::value_parser!(u32).range(0..64))]
    transform: Vec<u32>,

    /// Export runfiles environment variables (RUNFILES_DIR, RUNFILES_MANIFEST_FILE, JAVA_RUNFILES) to the executed process
    #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
    export_runfiles_env: bool,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Arguments to embed in the stub (argv[0], argv[1], ...)
    #[arg(required = true)]
    args: Vec<String>,
}

fn finalize_stub(
    template_paths: &[String],
    output_path: Option<&str>,
    argv: &[String],
    transform_flags: u64,
    export_runfiles_env: bool,
    verbose: bool,
) -> Result<(), String> {
    if argv.is_empty() {
        return Err("At least one argument (argv[0]) is required".into());
    }
    if argv.len() < 64 && transform_flags >> argv.len() != 0 {
        return Err("Transform index must refer to an embedded argument".into());
    }
    let output_canon = output_path.and_then(|p| fs::canonicalize(p).ok());
    let mut selected: Option<(String, Template)> = None;
    let mut available = Vec::new();
    for path in template_paths {
        let canon = fs::canonicalize(path)
            .map_err(|e| format!("Failed to resolve template {path}: {e}"))?;
        if output_canon.as_ref() == Some(&canon) {
            return Err(
                "Output path cannot be the same as template path (would overwrite input)".into(),
            );
        }
        let data = fs::read(path).map_err(|e| format!("Failed to read template {path}: {e}"))?;
        let candidate =
            Template::parse(data).map_err(|e| format!("Invalid template {path}: {e}"))?;
        let caps = candidate.capabilities;
        available.push(format!(
            "{path}: {} args, {} bytes/arg",
            caps.max_args, caps.arg_size
        ));
        if prefer(
            &candidate,
            selected.as_ref().map(|(_, t)| t),
            argv,
            transform_flags,
        ) {
            selected = Some((path.clone(), candidate));
        }
    }
    let (path, template) = selected.ok_or_else(|| {
        format!(
            "No compatible template for {} arguments (longest: {} bytes). Available: {}",
            argv.len(),
            argv.iter().map(|a| a.len()).max().unwrap_or(0),
            available.join("; ")
        )
    })?;
    if verbose {
        eprintln!(
            "Selected template {path} ({} bytes; {} args, {} bytes/arg)",
            template.data.len(),
            template.capabilities.max_args,
            template.capabilities.arg_size
        );
    }
    let mut data = template.patch(argv, transform_flags, export_runfiles_env)?;

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
        io::stdout()
            .write_all(&data)
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
    matches!(
        magic,
        0xfeedface |  // MH_MAGIC (32-bit)
        0xfeedfacf |  // MH_MAGIC_64 (64-bit)
        0xcefaedfe |  // MH_CIGAM (32-bit, swapped)
        0xcffaedfe // MH_CIGAM_64 (64-bit, swapped)
    )
}

/// Re-signs a Mach-O binary with an ad-hoc signature
fn resign_macho(data: Vec<u8>, verbose: bool) -> Result<Vec<u8>, String> {
    use apple_codesign::{MachOSigner, SigningSettings};

    // Parse the Mach-O binary
    let signer =
        MachOSigner::new(&data).map_err(|e| format!("Failed to parse Mach-O binary: {}", e))?;

    if verbose {
        eprintln!("Applying ad-hoc signature to Mach-O binary...");
    }

    // Create ad-hoc signing settings (no certificate = ad-hoc)
    let mut settings = SigningSettings::default();
    settings.set_binary_identifier(apple_codesign::SettingsScope::Main, "runfiles-stub");

    // Sign the binary
    let mut signed_data = Vec::new();
    signer
        .write_signed_binary(&settings, &mut signed_data)
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
        let mut flags = 0u64;
        for idx in cli.transform {
            flags |= 1 << idx;
        }
        flags
    };

    match finalize_stub(
        &cli.template,
        cli.output.as_deref(),
        &cli.args,
        transform_flags,
        cli.export_runfiles_env,
        cli.verbose,
    ) {
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
