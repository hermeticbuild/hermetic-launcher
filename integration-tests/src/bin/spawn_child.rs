//! Demo program: Start a long-running child, record its pid, then wait.
//!
//! Usage: spawn-child <pid-file>
//! The child is this same executable run as `spawn-child --sleep`.

use std::env;
use std::fs;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.get(1).map(String::as_str) != Some("--sleep") {
        let child = Command::new(&args[0])
            .arg("--sleep")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start child");
        fs::write(&args[1], child.id().to_string()).expect("failed to write pid file");
    }
    sleep(Duration::from_secs(120));
}
