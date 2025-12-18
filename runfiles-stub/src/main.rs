// Platform-specific implementations
// Linux uses raw syscalls, macOS uses libc, Windows uses Win32 API

#![no_std]
#![no_main]

// Provide a personality symbol so linking succeeds with prebuilt libcore when
// using panic=abort in a no_std binary.
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;
