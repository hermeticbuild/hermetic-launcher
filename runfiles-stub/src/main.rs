// Tiny, cross-platform runfiles launcher template.
//
// The platform-agnostic logic lives in `common`, `runfiles`, `placeholders`, and `run`.
// Each platform backend (`linux`/`macos`/`windows`, selected below) provides the OS
// primitives, the entry point, and the process-launch step behind a small seam.

#![no_std]
#![no_main]

extern crate alloc;

use core::panic::PanicInfo;

// Provide a personality symbol so linking succeeds with prebuilt libcore when
// using panic=abort in a no_std binary.
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    platform::exit(1)
}

mod allocator;
mod common;
mod native_path;
mod placeholders;
mod run;
mod runfiles;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;
