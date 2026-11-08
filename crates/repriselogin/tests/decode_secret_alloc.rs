//! Fails when a block freed while a secret-carrying message is encoded and decoded still holds
//! the secret. Own binary: it installs a global allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::Debug;

use repriselogin::account::AccountRequest;
use repriselogin::frame::{decode, encode};
use repriselogin::front::{FrontEvent, FrontRequest, NlaUser, Secret};
use repriselogin::greeter::GreeterRequest;
use repriselogin::handover::{
    Cipher, DirectionKeys, HandOver, IntakeMessage, Step, TlsState, TlsVersion,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const SECRET_BYTE: u8 = 0xA7;
/// `§` is C2 A7; the quote and backslash need escapes in JSON text.
const PASSWORD: &str = "§§§§\"§§§§\\§§§§";

/// The secrets as raw bytes, as UTF-8 text and as hex in either case.
const NEEDLES: [&[u8]; 6] = [
    &[SECRET_BYTE; 4],
    "§§".as_bytes(),
    b"a7a7a7a7",
    b"A7A7A7A7",
    b"c2a7c2a7",
    b"C2A7C2A7",
];

/// Aligned words holding `SECRET_BYTE` alone, as a buffered sequence of numbers stores them.
const WORDS: usize = 8;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static COPIES: Cell<usize> = const { Cell::new(0) };
}

struct Scanning;

fn holds_a_secret(block: &[u8]) -> bool {
    let words = (block.as_chunks().0.iter())
        .filter(|w| u64::from_le_bytes(**w) == u64::from(SECRET_BYTE))
        .count();
    words >= WORDS
        || NEEDLES
            .iter()
            .any(|n| block.windows(n.len()).any(|w| w == *n))
}

// SAFETY: forwards every call to `System`, zero-filling fresh blocks so `dealloc` can read them
// whole before freeing them.
unsafe impl GlobalAlloc for Scanning {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract is `System::alloc_zeroed`'s.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.try_with(Cell::get).unwrap_or(false) {
            // SAFETY: `ptr` is a live block of `layout.size()` bytes until the call below, and
            // `alloc` zero-filled it, so spare capacity past a `Vec`'s length is initialised.
            let block = unsafe { std::slice::from_raw_parts(ptr, layout.size()) };
            if holds_a_secret(block) {
                let _ = COPIES.try_with(|n| n.set(n.get() + 1));
            }
        }
        // SAFETY: the caller's contract is `System::dealloc`'s.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Scanning = Scanning;

/// The blocks holding a secret that `f` frees.
fn copies_freed_by(f: impl FnOnce()) -> usize {
    COPIES.set(0);
    ARMED.set(true);
    f();
    ARMED.set(false);
    COPIES.get()
}

/// The copies freed while `sent` is encoded, decoded and the decoded message dropped.
fn copies_left<T: Serialize + DeserializeOwned + PartialEq + Debug>(sent: &T) -> usize {
    let mut same = false;
    let copies = copies_freed_by(|| {
        let frame = encode(sent).expect("encodes");
        let got = decode::<T>(&frame[4..]).expect("decodes");
        same = got == *sent;
    });
    assert!(same, "the decoded message equals the sent one");
    copies
}

/// Internally tagged, with plain fields: decoding buffers the bytes as numbers and unescapes
/// the text into serde_json's scratch.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
enum Leaky {
    Plain { bytes: Vec<u8>, text: String },
}

fn secret(n: usize) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(vec![SECRET_BYTE; n])
}

fn password() -> Zeroizing<String> {
    Zeroizing::new(PASSWORD.to_owned())
}

fn hand_over() -> HandOver {
    let keys = || DirectionKeys {
        seq: 5,
        key: secret(32),
        iv: secret(12),
        secret: Some(secret(48)),
    };
    HandOver {
        conn: 9,
        user: "anna".into(),
        peer: "192.0.2.1:50000".parse().unwrap(),
        security: 2,
        steps: vec![
            Step::Pdu {
                input: secret(300),
                output: secret(40),
            },
            Step::SecurityUpgradeDone,
            Step::CredsspDone {
                user: "anna".into(),
                domain: None,
            },
        ],
        tls: TlsState {
            version: TlsVersion::Tls13,
            cipher: Cipher::Aes256Gcm,
            tx: keys(),
            rx: keys(),
            record_limit: 1 << 24,
        },
    }
}

#[test]
fn decoding_leaves_no_copy_of_a_secret_in_freed_memory() {
    let controls = [
        (
            "raw bytes",
            copies_freed_by(|| drop(std::hint::black_box(vec![SECRET_BYTE; 32]))),
        ),
        (
            "text",
            copies_freed_by(|| drop(std::hint::black_box(PASSWORD.to_owned()))),
        ),
        (
            "numbers",
            copies_freed_by(|| drop(std::hint::black_box(vec![u64::from(SECRET_BYTE); 16]))),
        ),
        (
            "leaky decode",
            copies_freed_by(|| {
                let sent = Leaky::Plain {
                    bytes: vec![SECRET_BYTE; 32],
                    text: PASSWORD.to_owned(),
                };
                let frame = std::hint::black_box(encode(&sent).expect("encodes"));
                std::mem::forget(sent);
                std::mem::forget(decode::<Leaky>(&frame[4..]).expect("decodes"));
            }),
        ),
    ];
    let missed: Vec<_> = controls.iter().filter(|(_, copies)| *copies == 0).collect();
    assert!(
        missed.is_empty(),
        "the scan sees every planted copy: {missed:?}"
    );
    let authenticate = |secret| FrontRequest::Authenticate {
        conn: 7,
        peer: "[2001:db8::1]:3389".parse().unwrap(),
        user: "anna".into(),
        secret,
    };
    let cases: [(&str, usize); 8] = [
        (
            "password",
            copies_left(&authenticate(Secret::Password(password()))),
        ),
        (
            "delegated",
            copies_left(&authenticate(Secret::Delegated(password()))),
        ),
        (
            "ready",
            copies_left(&FrontRequest::Ready {
                conn: 9,
                handover: Box::new(hand_over()),
            }),
        ),
        (
            "hand_over",
            copies_left(&IntakeMessage::HandOver(Box::new(hand_over()))),
        ),
        (
            "passwords",
            copies_left(&FrontEvent::Passwords {
                users: vec![NlaUser {
                    user: "anna".into(),
                    nt_hash: Zeroizing::new("A7".repeat(16)),
                }],
            }),
        ),
        (
            "post_auth_response",
            copies_left(&GreeterRequest::PostAuthResponse {
                response: Some(password()),
            }),
        ),
        (
            "set_remote_password",
            copies_left(&AccountRequest::SetRemotePassword {
                login_password: password(),
                remote_password: password(),
            }),
        ),
        (
            "clear_remote_password",
            copies_left(&AccountRequest::ClearRemotePassword {
                login_password: password(),
            }),
        ),
    ];
    let leaky: Vec<_> = cases.iter().filter(|(_, copies)| *copies > 0).collect();
    assert!(leaky.is_empty(), "no freed block holds a secret: {leaky:?}");
}
