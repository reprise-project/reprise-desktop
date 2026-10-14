//! glibc malloc tuning for a long-running process that decodes pictures on short-lived threads.

/// Blocks of this size and up get their own mapping, unmapped on free.
#[cfg(target_env = "gnu")]
const MMAP_THRESHOLD: usize = 1 << 20;

/// Pin glibc's mmap threshold. Left adaptive, it rises to the largest block freed, so later
/// picture buffers land in per-thread arenas whose free tops glibc never returns.
pub fn pin_mmap_threshold() {
    #[cfg(target_env = "gnu")]
    // SAFETY: mallopt only sets an allocator parameter, under glibc's own lock.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, MMAP_THRESHOLD as libc::c_int);
    }
}

#[cfg(all(test, target_env = "gnu"))]
mod tests {
    use super::*;

    /// glibc's chunk header, the word before the pointer, has bit 1 set for a mapped chunk.
    fn is_mapped(block: &[u8]) -> bool {
        // SAFETY: `block` came from malloc, which puts its chunk's size word just before it.
        unsafe { *(block.as_ptr() as *const usize).sub(1) & 2 != 0 }
    }

    #[test]
    fn a_block_after_a_larger_freed_one_is_still_mapped() {
        // Arenas the other tests left with large free tops would serve the block: run alone.
        const ALONE: &str = "REPRISE_MALLOC_TEST_ALONE";
        if std::env::var_os(ALONE).is_none() {
            let name = "malloc::tests::a_block_after_a_larger_freed_one_is_still_mapped";
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--quiet"])
                .env(ALONE, "1")
                .status()
                .unwrap();
            assert!(status.success(), "the test passes in a process of its own");
            return;
        }
        pin_mmap_threshold();
        let big = std::thread::spawn(|| vec![1u8; 16 * MMAP_THRESHOLD])
            .join()
            .unwrap();
        drop(big);
        let block = std::thread::spawn(|| vec![1u8; 2 * MMAP_THRESHOLD])
            .join()
            .unwrap();
        assert!(is_mapped(&block), "a 2 MiB block is mapped");
    }
}
