//! Tests that run reprise-decode or its confinement for real. Their names start with `spawned_`
//! so a host run can skip them (`cargo test -- --skip spawned_`); they run on test machines.
//! `REPRISE_DECODE_BIN` points the tests at a reprise-decode away from the build tree.

#[path = "../src/avif_pictures.rs"]
mod avif_pictures;
#[path = "../src/pictures.rs"]
mod pictures;

use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

use pictures::{Colour, Picture, encode, noise, tabled_srgb_icc};
use reprise_decode::confine;
use reprise_decode::heapcap::Capped;
use reprise_decode::jxl::first_frame;
use reprise_decode_protocol::{Format, Limits, Reply, Request};
use repriseconfine::helper;

#[global_allocator]
static HEAP: Capped = Capped;

const LIMITS: Limits = Limits {
    file: 64 << 20,
    pixels: 96 << 20,
    deadline: Duration::from_secs(60),
};

fn helper() -> Command {
    let bin = std::env::var_os("REPRISE_DECODE_BIN")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_reprise-decode").into());
    let mut c = helper::command(bin.as_ref());
    c.args(LIMITS.to_args());
    c
}

fn decode(file: &[u8]) -> (Reply, helper::Ran) {
    decode_as(Format::Jxl, file)
}

fn decode_as(format: Format, file: &[u8]) -> (Reply, helper::Ran) {
    let request = Request {
        format,
        len: file.len() as u64,
    };
    let mut ran = helper::run(
        helper(),
        std::io::Cursor::new(request.to_bytes()).chain(file),
        LIMITS.reply_cap(),
        LIMITS.deadline,
    )
    .unwrap();
    let reply = Reply::parse(std::mem::take(&mut ran.reply), LIMITS).unwrap();
    (reply, ran)
}

#[test]
fn spawned_each_picture_decodes_through_the_confined_helper_as_in_process() {
    let px = noise(64, 48);
    let grey: Vec<u8> = (0..64 * 48).map(|i| (i % 251) as u8).collect();
    let icc = tabled_srgb_icc();
    let files = [
        ("lossless", encode(64, 48, &[&px], false)),
        ("premultiplied", encode(64, 48, &[&px], true)),
        ("animation", encode(64, 48, &[&px, &grey.repeat(4)], false)),
        (
            "lossy P3",
            Picture {
                colour: Colour::P3,
                ..Picture::new(64, 48, &[(&px, 1)])
            }
            .encode(),
        ),
        (
            "lossy ICC",
            Picture {
                colour: Colour::Icc(&icc),
                ..Picture::new(64, 48, &[(&px, 1)])
            }
            .encode(),
        ),
        (
            "grey",
            Picture {
                channels: 1,
                ..Picture::new(64, 48, &[(&grey, 1)])
            }
            .encode(),
        ),
        (
            "extra channels",
            Picture {
                extra: 2,
                ..Picture::new(64, 48, &[(&px, 1)])
            }
            .encode(),
        ),
    ];
    for (name, file) in files {
        let want = first_frame(&file, LIMITS.pixels, LIMITS.deadline).unwrap();
        let (reply, ran) = decode(&file);
        let Reply::Picture(p) = reply else {
            panic!("{name}: {reply:?}, said {}", ran.said)
        };
        assert_eq!((p.width, p.height), (want.width, want.height), "{name}");
        assert!(p.rgba() == want.px, "{name}: the pixels differ");
        assert!(ran.status.unwrap().success(), "{name}");
    }
}

#[test]
fn spawned_each_avif_decodes_through_the_confined_helper_as_in_process() {
    use avif_pictures::*;
    use rav1e::prelude::ChromaSampling;
    let (w, h) = (64, 48);
    let deep = Coding {
        bits: 10,
        sampling: ChromaSampling::Cs420,
        full_range: false,
        cicp: (9, 16, 9),
        quantizer: 60,
        grain: false,
    };
    let alpha: Vec<u8> = (0..w * h).map(|i| (i * 5 % 256) as u8).collect();
    let rgba: Vec<u8> = (0..w * h)
        .flat_map(|i| [(i % w * 4) as u8, 90, 200, alpha[i]])
        .collect();
    let picture = |c: Coding| av1_rgb(w, h, &noise_rgb(w, h, c.bits), c);
    let size = ispe(w as u32, h as u32);
    let files = [
        ("8-bit", single(picture(Coding::EXACT), vec![size.clone()])),
        (
            "10-bit PQ 4:2:0, turned",
            single(
                picture(deep),
                vec![size.clone(), nclx(9, 16, 9, false), irot(1)],
            ),
        ),
        ("ravif with alpha", ravif(w, h, &rgba, false)),
        (
            "sequence",
            sequence(
                w as u16,
                h as u16,
                &[picture(Coding::EXACT), picture(Coding::EXACT)],
            ),
        ),
    ];
    for (name, file) in files {
        let want =
            reprise_decode::avif::first_frame(&file, LIMITS.pixels, LIMITS.deadline).unwrap();
        let (reply, ran) = decode_as(Format::Avif, &file);
        let Reply::Picture(p) = reply else {
            panic!("{name}: {reply:?}, said {}", ran.said)
        };
        assert_eq!((p.width, p.height), (want.width, want.height), "{name}");
        assert!(p.rgba() == want.px, "{name}: the pixels differ");
        assert!(ran.status.unwrap().success(), "{name}");
    }
    let file = single(picture(Coding::EXACT), vec![size]);
    let (reply, _) = decode_as(Format::Avif, &file[..file.len() - 20]);
    assert!(matches!(reply, Reply::Unreadable(_)), "{reply:?}");
    let huge = single(vec![0x12, 0, 0x0a, 0x0a], vec![ispe(30_000, 30_000)]);
    assert_eq!(decode_as(Format::Avif, &huge).0, Reply::TooLarge);
}

#[test]
fn spawned_a_broken_file_or_one_past_the_limits_says_so() {
    let file = encode(64, 48, &[&noise(64, 48)], false);
    assert!(matches!(
        decode(&file[..file.len() / 2]).0,
        Reply::Unreadable(_)
    ));
    let big = encode(4096, 8, &[&noise(4096, 8)], false);
    let tight = Limits {
        pixels: 4096 * 8 * 4 - 1,
        ..LIMITS
    };
    let mut c = helper::command(helper().get_program().as_ref());
    c.args(tight.to_args());
    let request = Request {
        format: Format::Jxl,
        len: big.len() as u64,
    };
    let ran = helper::run(
        c,
        std::io::Cursor::new(request.to_bytes()).chain(&big[..]),
        tight.reply_cap(),
        LIMITS.deadline,
    )
    .unwrap();
    assert_eq!(Reply::parse(ran.reply, tight), Ok(Reply::TooLarge));
}

#[test]
fn spawned_the_helper_confines_itself_before_reading_and_is_the_oom_killers_first_pick() {
    let mut c = helper();
    c.stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = c.spawn().unwrap();
    let pid = child.id();
    let read = |f: &str| std::fs::read_to_string(format!("/proc/{pid}/{f}")).unwrap();
    let reading = |status: &str| status.contains("NoNewPrivs:\t1") && status.contains("State:\tS");
    let start = Instant::now();
    while !reading(&read("status")) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{}",
            read("status")
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = read("status");
    assert!(status.contains("Seccomp:\t2"), "{status}");
    assert!(status.contains("Threads:\t1"), "{status}");
    assert_eq!(read("oom_score_adj"), "1000\n");
    let set = confine::limits(LIMITS, repriseconfine::child::stack_size(8 << 20));
    let limits = read("limits");
    let row = |name: &str| {
        let line = limits.lines().find(|l| l.starts_with(name)).unwrap();
        line.split_whitespace().nth(3).unwrap().to_owned()
    };
    assert_eq!(row("Max address space"), set.address_space.to_string());
    assert_eq!(row("Max cpu time"), set.cpu_seconds.to_string());
    drop(child.stdin.take());
    assert!(!child.wait().unwrap().success(), "no request is an error");
}

/// Forks a child that confines itself as reprise-decode does, being the one thread of its
/// process, then runs `attempt`; the child exits 0 when `attempt` returns true.
fn confined(attempt: impl FnOnce() -> bool) -> ExitStatus {
    // SAFETY: the child confines itself, runs `attempt` and leaves through _exit without
    // returning into the test harness; glibc's malloc is usable after fork.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // SAFETY: plain system calls on this process's own descriptors.
        unsafe { libc::dup2(libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY), 2) };
        let code = match confine::confine(LIMITS) {
            Ok(c) if c.seccomp => {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(attempt)) {
                    Ok(true) => 0,
                    Ok(false) => 1,
                    Err(_) => 2,
                }
            }
            Ok(_) => 3,
            Err(_) => 4,
        };
        // SAFETY: ends the child without running the parent's exit handlers.
        unsafe { libc::_exit(code) };
    }
    assert!(pid > 0, "fork: {}", std::io::Error::last_os_error());
    let mut status = 0;
    // SAFETY: status is a live int; pid is the child forked above.
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    ExitStatus::from_raw(status)
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn spawned_the_confined_decoder_cannot_open_a_file() {
    let status = confined(|| {
        let refused = |r: std::io::Result<std::fs::File>| {
            r.err().and_then(|e| e.raw_os_error()) == Some(libc::EACCES)
        };
        refused(std::fs::File::open("/etc/passwd"))
            && refused(std::fs::File::create(
                std::env::temp_dir().join("reprise-decode-probe"),
            ))
    });
    assert!(status.success(), "{status}");
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn spawned_the_confined_decoder_cannot_open_a_socket_or_start_a_thread() {
    let status = confined(|| std::os::unix::net::UnixDatagram::unbound().is_ok());
    assert_eq!(status.signal(), Some(libc::SIGSYS), "{status}");
    let status = confined(|| std::thread::spawn(|| ()).join().is_ok());
    assert_eq!(status.signal(), Some(libc::SIGSYS), "{status}");
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn spawned_the_confined_decoder_dies_of_its_own_abort_and_past_its_address_space() {
    let status = confined(|| std::process::abort());
    assert_eq!(status.signal(), Some(libc::SIGABRT), "{status}");
    let all = confine::limits(LIMITS, 0).address_space as usize;
    let status = confined(|| std::hint::black_box(Vec::<u8>::with_capacity(all)).capacity() > 0);
    assert_eq!(status.signal(), Some(libc::SIGABRT), "{status}");
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn spawned_the_confined_decoder_dies_of_its_own_segfault() {
    let status = confined(|| {
        let nowhere: *mut u8 = std::hint::black_box(std::ptr::null_mut());
        // SAFETY: none; the fault is what the test is after.
        unsafe { nowhere.write_volatile(1) };
        false
    });
    assert_eq!(status.signal(), Some(libc::SIGSEGV), "{status}");
}
