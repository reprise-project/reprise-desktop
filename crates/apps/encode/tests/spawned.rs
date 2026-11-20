//! Tests that run reprise-encode or its confinement for real. Their names start with `spawned_`
//! so a host run can skip them (`cargo test -- --skip spawned_`); they run on test machines.
//! The helper finds its codec beside itself once `-p reprise-encode-jxl` is built too;
//! `REPRISE_ENCODE_BIN` points the tests at a helper away from the build tree.

use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use reprise_encode::confine::{self, Plan};
use reprise_encode::{Format, Jxl, Limits, Reply, Request};

const LIMITS: Limits = Limits {
    budget: 512 << 20,
    deadline: Duration::from_secs(120),
};

fn helper() -> Command {
    let bin = std::env::var_os("REPRISE_ENCODE_BIN")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_reprise-encode").into());
    let mut c = Command::new(bin);
    c.args(LIMITS.to_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    c
}

fn picture(w: u32, h: u32) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            [(x * 2) as u8, (y * 3) as u8, ((x + y) & 0xff) as u8]
        })
        .collect()
}

fn encode(w: u32, h: u32, jxl: Jxl) -> Vec<u8> {
    let mut child = helper().spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let request = Request {
        w,
        h,
        format: Format::Jxl(jxl),
    };
    stdin.write_all(&request.to_bytes()).unwrap();
    stdin.write_all(&picture(w, h)).unwrap();
    drop(stdin);
    let mut reply = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut reply)
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "{status}");
    match Reply::parse(reply) {
        Ok(Reply::File(file)) => file,
        other => panic!("{other:?}"),
    }
}

fn decode(file: &[u8]) -> (u32, u32, Vec<u8>) {
    let dec = jpegxl_rs::decoder_builder()
        .pixel_format(jpegxl_rs::decode::PixelFormat {
            num_channels: 3,
            ..Default::default()
        })
        .build()
        .unwrap();
    let (meta, px) = dec.decode_with::<u8>(file).unwrap();
    (meta.width, meta.height, px)
}

#[test]
fn spawned_a_lossless_encode_through_the_confined_helper_keeps_every_pixel() {
    let (w, h) = (300, 200);
    for effort in [1, 7, 9] {
        let file = encode(
            w,
            h,
            Jxl {
                lossless: true,
                quality: 90,
                effort,
            },
        );
        assert_eq!(decode(&file), (w, h, picture(w, h)), "effort {effort}");
    }
}

#[test]
fn spawned_a_lossy_encode_through_the_confined_helper_stays_close() {
    let (w, h) = (300, 200);
    let want = picture(w, h);
    for effort in [1, 7, 9] {
        let file = encode(
            w,
            h,
            Jxl {
                lossless: false,
                quality: 90,
                effort,
            },
        );
        let (dw, dh, px) = decode(&file);
        assert_eq!((dw, dh), (w, h));
        let off: u64 = (px.iter().zip(&want))
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        let mean = off as f64 / want.len() as f64;
        assert!(mean < 3.0, "effort {effort}: off by {mean} on average");
    }
}

#[test]
fn spawned_the_helper_confines_itself_before_reading_and_is_the_oom_killers_first_pick() {
    let mut child = helper().spawn().unwrap();
    let pid = child.id();
    let read = |f: &str| std::fs::read_to_string(format!("/proc/{pid}/{f}")).unwrap();
    // No new privileges comes first and the blocking read of the request last, with nothing that
    // sleeps in between.
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
    assert_eq!(read("oom_score_adj"), "1000\n");
    assert_eq!(read("comm"), "reprise-encode\n");
    let plan = Plan::new(LIMITS);
    let limits = read("limits");
    let row = |name: &str| {
        let line = limits.lines().find(|l| l.starts_with(name)).unwrap();
        line.split_whitespace().nth(3).unwrap().to_owned()
    };
    assert_eq!(row("Max address space"), plan.address_space.to_string());
    assert_eq!(row("Max cpu time"), plan.cpu_seconds.to_string());
    drop(child.stdin.take());
    assert!(!child.wait().unwrap().success(), "no request is an error");
}

/// Runs `child` in a forked child, the one thread of its process, which exits with its code.
fn forked(child: impl FnOnce() -> i32) -> ExitStatus {
    // SAFETY: the child runs `child` and leaves through _exit without returning into the test
    // harness; glibc's malloc is usable after fork.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // SAFETY: plain system calls on this process's own descriptors.
        unsafe { libc::dup2(libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY), 2) };
        let code = std::panic::catch_unwind(std::panic::AssertUnwindSafe(child)).unwrap_or(2);
        // SAFETY: ends the child without running the parent's exit handlers.
        unsafe { libc::_exit(code) };
    }
    assert!(pid > 0, "fork: {}", std::io::Error::last_os_error());
    let mut status = 0;
    // SAFETY: status is a live int; pid is the child forked above.
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    ExitStatus::from_raw(status)
}

/// Forks a child that confines itself as the helper does, then runs `attempt`; the child exits 0
/// when `attempt` returns true.
fn confined(attempt: impl FnOnce() -> bool) -> ExitStatus {
    let plan = Plan::new(LIMITS);
    forked(|| match confine::confine(&plan) {
        Ok(c) if c.seccomp => i32::from(!attempt()),
        Ok(_) => 3,
        Err(_) => 4,
    })
}

/// This process's no-new-privileges and seccomp lines of /proc/self/status.
fn confinement() -> Vec<String> {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    (status.lines())
        .filter(|l| l.starts_with("NoNewPrivs:") || l.starts_with("Seccomp:"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn spawned_a_process_running_a_second_thread_is_not_confined() {
    let plan = Plan::new(LIMITS);
    let before = confinement();
    assert_eq!(before.len(), 2, "{before:?}");
    let status = forked(|| {
        let (stop, wait) = std::sync::mpsc::channel::<()>();
        let second = std::thread::spawn(move || wait.recv());
        let refused =
            confine::confine(&plan).is_err_and(|e| e.to_string().contains("threads are running"));
        drop(stop);
        let _ = second.join();
        i32::from(!(refused && confinement() == before))
    });
    assert!(status.success(), "{status}");
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn spawned_the_confined_helper_cannot_open_a_file() {
    let status = confined(|| {
        let refused = |r: std::io::Result<std::fs::File>| {
            r.err().and_then(|e| e.raw_os_error()) == Some(libc::EACCES)
        };
        refused(std::fs::File::open("/etc/passwd"))
            && refused(std::fs::File::create(
                std::env::temp_dir().join("reprise-encode-probe"),
            ))
    });
    assert!(status.success(), "{status}");
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn spawned_the_confined_helper_cannot_open_a_socket() {
    let status = confined(|| std::os::unix::net::UnixDatagram::unbound().is_ok());
    assert_eq!(status.signal(), Some(libc::SIGSYS), "{status}");
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn spawned_the_confined_helper_dies_of_its_own_abort() {
    let status = confined(|| std::process::abort());
    assert_eq!(status.signal(), Some(libc::SIGABRT), "{status}");
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn spawned_the_confined_helper_aborts_past_its_address_space() {
    let all = Plan::new(LIMITS).address_space as usize;
    let status = confined(|| std::hint::black_box(Vec::<u8>::with_capacity(all)).capacity() > 0);
    assert_eq!(status.signal(), Some(libc::SIGABRT), "{status}");
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[test]
fn spawned_the_confined_helper_dies_of_its_own_segfault() {
    let status = confined(|| {
        let nowhere: *mut u8 = std::hint::black_box(std::ptr::null_mut());
        // SAFETY: none; the fault is what the test is after.
        unsafe { nowhere.write_volatile(1) };
        false
    });
    assert_eq!(status.signal(), Some(libc::SIGSEGV), "{status}");
}
