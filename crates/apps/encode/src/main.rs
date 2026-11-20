use std::process::ExitCode;

use reprise_encode::codec::JxlCodec;
use reprise_encode::confine::{self, Plan};
use reprise_encode::{Limits, serve};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(limits) = Limits::from_args(&args) else {
        eprintln!("usage: reprise-encode --budget BYTES --deadline SECONDS");
        return ExitCode::from(2);
    };
    // The OOM killer's first pick, so it takes the encoder rather than the compositor. Raising
    // one's own score needs no privilege.
    let _ = std::fs::write("/proc/self/oom_score_adj", "1000");
    let plan = Plan::new(limits);
    let jxl = JxlCodec::load();
    match confine::confine(&plan) {
        Ok(c) => {
            if !c.seccomp {
                eprintln!("reprise-encode: no seccomp filter for this architecture");
            }
            if c.landlock == 0 {
                eprintln!("reprise-encode: the kernel has no Landlock");
            }
        }
        Err(e) => {
            eprintln!("reprise-encode: confinement: {e}");
            return ExitCode::FAILURE;
        }
    }
    let served = serve(
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &jxl,
        limits.budget,
        plan.threads,
    );
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("reprise-encode: {e}");
            ExitCode::FAILURE
        }
    }
}
