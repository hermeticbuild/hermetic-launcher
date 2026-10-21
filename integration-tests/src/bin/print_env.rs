//! Demo program: Print environment variables and command line arguments
//!
//! Usage: print-env [args...]
//! Outputs:
//!   ARGS:<arg0>|<arg1>|...
//!   EXE:<path of the running executable>
//!   ENV:<key>=<value> (for each env var)
//!
//! This is useful for debugging and validating what the stub passes to child processes.

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    // Print all command line arguments
    let args: Vec<String> = env::args().collect();
    let args_str = args.join("|");
    println!("ARGS:{}", args_str);
    println!("ARGC:{}", args.len());

    // The file this process was started as, which can differ from argv[0]
    match env::current_exe() {
        Ok(exe) => println!("EXE:{}", exe.display()),
        Err(_) => println!("EXE:<unknown>"),
    }

    // Print selected environment variables (runfiles-related and common ones)
    let interesting_vars = [
        "RUNFILES_DIR",
        "RUNFILES_MANIFEST_FILE",
        "JAVA_RUNFILES",
        "PATH",
        "PWD",
        "HOME",
        "USER",
        "USERPROFILE",
        "TEMP",
        "TMP",
    ];

    println!("---ENV_START---");
    for var in &interesting_vars {
        match env::var(var) {
            Ok(value) => println!("ENV:{}={}", var, value),
            Err(_) => println!("ENV:{}=<unset>", var),
        }
    }
    println!("---ENV_END---");

    // Print all environment variables (sorted for consistent output)
    println!("---ALL_ENV_START---");
    let mut all_vars: Vec<(String, String)> = env::vars().collect();
    all_vars.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, value) in all_vars {
        println!("ALL_ENV:{}={}", key, value);
    }
    println!("---ALL_ENV_END---");

    ExitCode::SUCCESS
}
