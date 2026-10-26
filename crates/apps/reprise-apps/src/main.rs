//! The installed binary behind every program: `/usr/bin/reprise-notepad` and the rest link here,
//! and the name it is started under (or its first argument) picks the program.

use reprisefs::multicall;
use std::ffi::OsString;

fn entry(program: &str) -> Option<fn(Vec<OsString>)> {
    Some(match program {
        "reprise-about" => reprise_about::main,
        "reprise-accounts" => reprise_accounts::main,
        "reprise-bluetooth" => reprise_bluetooth::main,
        "reprise-datetime" => reprise_datetime::main,
        "reprise-drives" => reprise_drives::main,
        "reprise-explorer" => reprise_explorer::main,
        "reprise-network" => reprise_network::main,
        "reprise-notepad" => reprise_notepad::main,
        "reprise-polkit" => reprise_polkit::main,
        "reprise-power" => reprise_power::main,
        "reprise-printers" => reprise_printers::main,
        "reprise-settings" => reprise_settings::main,
        "reprise-sound" => reprise_sound::main,
        "reprise-terminal" => reprise_terminal::main,
        "reprise-updates" => reprise_updates::main,
        "reprise-volume" => reprise_volume::main,
        _ => return None,
    })
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().collect();
    let Some((main, rest)) =
        multicall::split(&args).and_then(|(p, rest)| Some((entry(p)?, rest.to_vec())))
    else {
        eprintln!(
            "usage: {} PROGRAM [ARGS...], or run it as PROGRAM; the programs are:",
            multicall::BINARY
        );
        for p in multicall::PROGRAMS {
            eprintln!("  {p}");
        }
        std::process::exit(2);
    };
    main(rest);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_program_has_an_entry() {
        for p in multicall::PROGRAMS {
            assert!(entry(p).is_some(), "{p} has an entry");
        }
        assert!(entry(multicall::BINARY).is_none());
    }
}
