// Global allocator shared by all platforms: a bump allocator over a static arena.
//
// The stub is a short-lived process that parses the manifest, resolves paths, and
// then execve's / ExitProcess's — it never needs to reclaim memory, so `dealloc` is
// a no-op and allocation is a single bump of an offset. This is smaller than a
// general-purpose allocator and needs no external crate. 8 MiB is plenty for
// manifest parsing, path resolution, and environment handling; the arena lives in
// .bss (zero-fill), so it costs no file bytes. The stub is single-threaded, so the
// offset is a plain Cell with no synchronization.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;

const ARENA_SIZE: usize = 8 * 1024 * 1024;
static mut ARENA: [u8; ARENA_SIZE] = [0; ARENA_SIZE];

struct BumpAllocator {
    next: Cell<usize>,
}

// Safe: the stub never spawns threads, so the Cell is only ever touched from one thread.
unsafe impl Sync for BumpAllocator {}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let base = core::ptr::addr_of!(ARENA) as usize;
        let align = layout.align();
        // Align the current offset up, then reserve `size` bytes.
        let aligned = (base + self.next.get()).wrapping_add(align - 1) & !(align - 1);
        let offset = aligned - base;
        match offset.checked_add(layout.size()) {
            Some(end) if end <= ARENA_SIZE => {
                self.next.set(end);
                aligned as *mut u8
            }
            _ => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator: memory is reclaimed only when the process is replaced/exits.
    }
}

#[global_allocator]
static ALLOCATOR: BumpAllocator = BumpAllocator { next: Cell::new(0) };
