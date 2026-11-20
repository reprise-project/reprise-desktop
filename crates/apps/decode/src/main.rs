use std::process::ExitCode;

use reprise_decode::heapcap::Capped;
use reprise_decode_protocol::Limits;

#[global_allocator]
static HEAP: Capped = Capped;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(limits) = Limits::from_args(&args) else {
        eprintln!("usage: reprise-decode --file BYTES --pixels BYTES --deadline SECONDS");
        return ExitCode::from(2);
    };
    // The OOM killer's first pick, so it takes the decoder rather than the program that asked.
    // Raising one's own score needs no privilege.
    let _ = std::fs::write("/proc/self/oom_score_adj", "1000");
    match reprise_decode::confine::confine(limits) {
        Ok(c) => {
            if !c.seccomp {
                eprintln!("reprise-decode: no seccomp filter for this architecture");
            }
            if c.landlock == 0 {
                eprintln!("reprise-decode: the kernel has no Landlock");
            }
        }
        Err(e) => {
            eprintln!("reprise-decode: confinement: {e}");
            return ExitCode::FAILURE;
        }
    }
    let served = reprise_decode::serve(
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        limits,
    );
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("reprise-decode: {e}");
            ExitCode::FAILURE
        }
    }
}
