//! Post-link binary optimizer for the launcher stubs.
//!
//! Runs a small, target-OS-specific pipeline of size optimizations on an
//! already-linked binary and writes the result to a new file. Each step is a
//! transformation that may shell out to a tool from the LLVM toolchain (passed
//! in explicitly so the build stays hermetic). Steps that don't apply to a
//! target are simply not part of that target's pipeline.
//!
//! Usage:
//!   optimize-binary --input <PATH> --output <PATH> --target-os <OS> \
//!                   --objcopy <PATH>
//!
//! `--target-os` is the OS the binary runs on (linux | macos | windows). For
//! targets with no optimization step, the input is copied through unchanged so
//! the output is always produced.

use std::env;
use std::fs;
use std::process::{Command, ExitCode};

struct Args {
    input: String,
    output: String,
    target_os: String,
    objcopy: String,
}

fn parse_args() -> Result<Args, String> {
    let mut input = None;
    let mut output = None;
    let mut target_os = None;
    let mut objcopy = None;

    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut take = |name: &str| -> Result<String, String> {
            it.next().ok_or_else(|| format!("{name} requires a value"))
        };
        match arg.as_str() {
            "--input" => input = Some(take("--input")?),
            "--output" => output = Some(take("--output")?),
            "--target-os" => target_os = Some(take("--target-os")?),
            "--objcopy" => objcopy = Some(take("--objcopy")?),
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(Args {
        input: input.ok_or("--input is required")?,
        output: output.ok_or("--output is required")?,
        target_os: target_os.ok_or("--target-os is required")?,
        objcopy: objcopy.ok_or("--objcopy is required")?,
    })
}

/// Mach-O sections that are dead weight under `panic = "abort"`: nothing unwinds
/// at runtime, so the unwind tables are never read. They come from the precompiled
/// std/alloc rlibs (the stub crate emits none), so they can't be dropped at
/// compile time — only removed from the linked image here.
const MACHO_DEAD_SECTIONS: &[&str] = &["__TEXT,__unwind_info", "__TEXT,__eh_frame"];

/// Run the optimization pipeline for `target_os`, transforming `input` into
/// `output`. Returns `Ok(())` on success.
fn optimize(args: &Args) -> Result<(), String> {
    match args.target_os.as_str() {
        "macos" => strip_sections(&args.objcopy, &args.input, &args.output, MACHO_DEAD_SECTIONS),
        // Linux stubs already carry no unwind sections, and Windows PE is handled
        // by its own toolchain; both pass through unchanged for now.
        "linux" | "windows" => copy_through(&args.input, &args.output),
        other => Err(format!("unsupported --target-os: {other}")),
    }
}

/// Copy `input` to `output` verbatim (the no-op pipeline).
fn copy_through(input: &str, output: &str) -> Result<(), String> {
    fs::copy(input, output)
        .map(|_| ())
        .map_err(|e| format!("failed to copy {input} -> {output}: {e}"))
}

/// Remove `sections` from the binary using `objcopy`, writing the result to
/// `output`. `objcopy` accepts repeated `--remove-section` and is a no-op for any
/// section that is absent, so the same section list is safe across stubs.
fn strip_sections(objcopy: &str, input: &str, output: &str, sections: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(objcopy);
    for section in sections {
        cmd.arg(format!("--remove-section={section}"));
    }
    // llvm-objcopy treats trailing positional args as <input> [output].
    cmd.arg(input).arg(output);

    let status = cmd
        .status()
        .map_err(|e| format!("failed to run objcopy ({objcopy}): {e}"))?;
    if !status.success() {
        return Err(format!("objcopy ({objcopy}) failed with status {status}"));
    }
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: optimize-binary --input <PATH> --output <PATH> \
                 --target-os <linux|macos|windows> --objcopy <PATH>"
            );
            return ExitCode::from(2);
        }
    };

    match optimize(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
