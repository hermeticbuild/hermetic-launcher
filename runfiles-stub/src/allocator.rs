// Global allocator shared by all platforms: talc over a static memory arena.
// 8 MiB is plenty for manifest parsing, path resolution, and environment handling.
// Single-threaded use, so no locking is needed.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use talc::{ClaimOnOom, Span, Talc};

static mut ARENA: [u8; 8 * 1024 * 1024] = [0; 8 * 1024 * 1024];

struct TalcAllocator(UnsafeCell<Talc<ClaimOnOom>>);
unsafe impl Sync for TalcAllocator {}

unsafe impl GlobalAlloc for TalcAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        (*self.0.get()).malloc(layout).map_or(core::ptr::null_mut(), |p| p.as_ptr())
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        (*self.0.get()).free(core::ptr::NonNull::new_unchecked(ptr), layout);
    }
}

#[global_allocator]
static ALLOCATOR: TalcAllocator = TalcAllocator(UnsafeCell::new(Talc::new(unsafe {
    ClaimOnOom::new(Span::from_array(core::ptr::addr_of!(ARENA).cast_mut()))
})));
