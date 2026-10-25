use std::sync::{Mutex, MutexGuard};

/// `m` locked, even when a holder panicked: its value is still usable.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
