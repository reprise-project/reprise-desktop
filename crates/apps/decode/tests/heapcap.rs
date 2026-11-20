//! `heapcap` installed as this binary's global allocator.

use reprise_decode::heapcap::{self, Capped, LARGE, Spent};

#[global_allocator]
static HEAP: Capped = Capped;

fn take(n: usize) -> Result<Vec<u8>, std::collections::TryReserveError> {
    let mut v = Vec::new();
    v.try_reserve_exact(n)?;
    Ok(v)
}

#[test]
fn a_large_request_past_the_limit_is_refused_and_one_within_it_is_not() {
    let (got, spent) = heapcap::within(1 << 20, || {
        let a = take(512 << 10);
        let b = take(600 << 10);
        (a.is_ok(), b.is_ok())
    })
    .unwrap();
    assert_eq!(
        got,
        (true, false),
        "the first fits, the second would pass 1 MiB"
    );
    assert!(spent.refused);
    assert_eq!(spent.large, 2);
}

#[test]
fn a_small_request_past_the_limit_still_succeeds() {
    let (held, spent) = heapcap::within(LARGE, || {
        (0..8).map(|_| vec![1u8; LARGE - 1]).collect::<Vec<_>>()
    })
    .unwrap();
    assert_eq!(held.len(), 8);
    assert!(!spent.refused, "nothing was large");
    assert!(spent.peak >= 8 * (LARGE - 1));
}

#[test]
fn freed_memory_makes_room_again() {
    let (ok, spent) =
        heapcap::within(1 << 20, || (0..10).all(|_| take(900 << 10).is_ok())).unwrap();
    assert!(ok, "each block is freed before the next");
    assert!(!spent.refused);
    assert!(spent.peak < 1 << 20);
}

#[test]
fn freeing_memory_from_before_the_call_does_not_lend_it_room() {
    let old = vec![0u8; 4 << 20];
    let (got, spent) = heapcap::within(1 << 20, || {
        drop(old);
        take(2 << 20).is_ok()
    })
    .unwrap();
    assert!(!got, "the 4 MiB freed was never counted");
    assert!(spent.refused);
}

#[test]
fn a_growing_vector_is_refused_once_it_would_pass_the_limit() {
    let (len, _) = heapcap::within(1 << 20, || {
        let mut v: Vec<u8> = Vec::new();
        while v.try_reserve(v.len().max(LARGE)).is_ok() {
            v.resize(v.capacity(), 0);
        }
        v.len()
    })
    .unwrap();
    assert!((512 << 10..=1 << 20).contains(&len), "{len}");
}

#[test]
fn another_thread_is_not_held() {
    let (ok, spent) = heapcap::within(LARGE, || {
        std::thread::scope(|s| s.spawn(|| take(8 << 20).is_ok()).join().unwrap())
    })
    .unwrap();
    assert!(ok);
    assert!(!spent.refused);
}

#[test]
fn the_cap_ends_with_the_call_and_after_a_panic() {
    let _ = heapcap::within(LARGE, || take(1 << 20).is_ok()).unwrap();
    assert!(take(8 << 20).is_ok());
    let panicked = std::panic::catch_unwind(|| {
        heapcap::within(LARGE, || panic!("inside")).unwrap();
    });
    assert!(panicked.is_err());
    assert!(take(8 << 20).is_ok());
}

#[test]
fn a_nested_call_is_held_to_the_room_its_outer_call_has_left() {
    let ((inner, spent), outer) = heapcap::within(1 << 20, || {
        let held = take(768 << 10).unwrap();
        let got = heapcap::within(usize::MAX, || take(512 << 10).is_ok()).unwrap();
        drop(held);
        got
    })
    .unwrap();
    assert!(!inner, "only 256 KiB was left");
    assert!(spent.refused && outer.refused);
    let (_, outer) = heapcap::within(usize::MAX, || {
        heapcap::within(usize::MAX, || take(2 << 20).unwrap()).unwrap()
    })
    .unwrap();
    assert!(
        outer.peak >= 2 << 20,
        "the inner peak counts toward the outer"
    );
}

#[test]
fn the_test_hook_refuses_only_the_large_request_it_names() {
    let (got, spent) =
        heapcap::within_refusing(usize::MAX, 1, || [0, 1, 2].map(|_| take(LARGE).is_ok())).unwrap();
    assert_eq!(got, [true, false, true]);
    assert_eq!(
        spent,
        Spent {
            peak: spent.peak,
            refused: true,
            large: 3
        }
    );
}

#[test]
fn the_test_hook_reaches_into_a_nested_call() {
    let (got, spent) = heapcap::within_refusing(usize::MAX, 2, || {
        let first = take(LARGE).is_ok();
        let (inner, _) =
            heapcap::within(usize::MAX, || [0, 1, 2].map(|_| take(LARGE).is_ok())).unwrap();
        (first, inner)
    })
    .unwrap();
    assert_eq!(got, (true, [true, false, true]));
    assert_eq!(spent.large, 4);
}
