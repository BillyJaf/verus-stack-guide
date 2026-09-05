// rust_verify/tests/example.rs ignore --- ordinary rust, not verus

// ANCHOR: full
// Ordinary Rust code, not Verus

use std::sync::{
    Arc,
    atomic::*
};

pub struct StackCell {
    elem: u32,
    next: usize,
}

pub struct TreiberStack {
    top_address: AtomicUsize
}

impl TreiberStack {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { top_address: AtomicUsize::new(0) })
    }

    pub fn push(self: Arc<Self>, elem: u32) {
        loop {
            let loaded_top_address = self.top_address.load(Ordering::Relaxed);
            
            let new_top = Box::new(StackCell { elem, next: loaded_top_address });
            let new_top_address = Box::into_raw(new_top) as usize;

            if let Ok(_) = self.top_address.compare_exchange(loaded_top_address, new_top_address, Ordering::Release, Ordering::Relaxed) {
                return;
            };
        }
    }

    pub fn pop(self: Arc<Self>) -> Option<u32> {
        loop {
            let loaded_top_address = self.top_address.load(Ordering::Acquire);

            if loaded_top_address == 0 {
                return None
            }

            let loaded_top_elem;
            let next_top_address;
            unsafe {
                loaded_top_elem = (*(loaded_top_address as *const StackCell)).elem;
                next_top_address = (*(loaded_top_address as *const StackCell)).next;
            };

            if let Ok(_) = self.top_address.compare_exchange(loaded_top_address, next_top_address, Ordering::Acquire, Ordering::Relaxed) {
                return Some(loaded_top_elem)
            };
        }
    }
}

// ANCHOR_END: full