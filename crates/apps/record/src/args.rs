//! The command line the compositor starts a recording with.

use std::ffi::OsString;
use std::os::fd::RawFd;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    /// The inherited descriptor of the empty file the movie goes into.
    pub fd: RawFd,
    /// The output's connector name.
    pub output: String,
    /// The part of the output recorded, in its logical pixels; None for all of it.
    pub area: Option<Area>,
    /// Record what plays too.
    pub sound: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub const USAGE: &str =
    "usage: reprise-record --fd N --output NAME [--area X,Y,WIDTH,HEIGHT] [--sound]";

impl Args {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
        let mut args = args.into_iter();
        let (mut fd, mut output, mut area, mut sound) = (None, None, None, false);
        while let Some(arg) = args.next() {
            let mut value = |name: &str| {
                args.next()
                    .ok_or_else(|| format!("{name} needs a value; {USAGE}"))
            };
            match arg.to_str() {
                Some("--fd") => {
                    let n = value("--fd")?;
                    let n = (n.to_str().and_then(|n| n.parse().ok()))
                        .filter(|n: &RawFd| *n > 2)
                        .ok_or_else(|| format!("--fd takes a descriptor above 2, not {n:?}"))?;
                    fd = Some(n);
                }
                Some("--output") => {
                    let name = value("--output")?;
                    output = Some(name.into_string().map_err(|_| "--output is not UTF-8")?);
                }
                Some("--area") => {
                    let text = value("--area")?;
                    area = Some(parse_area(&text.to_string_lossy())?);
                }
                Some("--sound") => sound = true,
                _ => return Err(format!("unknown argument {arg:?}; {USAGE}")),
            }
        }
        Ok(Args {
            fd: fd.ok_or_else(|| format!("--fd is missing; {USAGE}"))?,
            output: output.ok_or_else(|| format!("--output is missing; {USAGE}"))?,
            area,
            sound,
        })
    }
}

fn parse_area(text: &str) -> Result<Area, String> {
    let bad = || format!("--area takes X,Y,WIDTH,HEIGHT, not {text:?}");
    let n: Vec<i32> = (text.split(','))
        .map(|p| p.trim().parse().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    match n[..] {
        [x, y, width, height] if width > 0 && height > 0 => Ok(Area {
            x,
            y,
            width,
            height,
        }),
        _ => Err(bad()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn an_area_and_sound_are_optional() {
        assert_eq!(
            parse(&["--fd", "3", "--output", "DP-1"]),
            Ok(Args {
                fd: 3,
                output: "DP-1".into(),
                area: None,
                sound: false,
            })
        );
        let args = parse(&[
            "--output",
            "DP-1",
            "--sound",
            "--area",
            "10,-4,320,200",
            "--fd",
            "7",
        ])
        .unwrap();
        assert!(args.sound);
        assert_eq!(
            args.area,
            Some(Area {
                x: 10,
                y: -4,
                width: 320,
                height: 200
            })
        );
    }

    #[test]
    fn a_missing_or_malformed_argument_is_refused() {
        assert!(
            parse(&["--output", "DP-1"])
                .unwrap_err()
                .starts_with("--fd is missing")
        );
        for fd in ["1", "-3", "a.mp4"] {
            let e = parse(&["--fd", fd, "--output", "o"]).unwrap_err();
            assert!(e.starts_with("--fd takes"), "{fd}: {e}");
        }
        assert!(
            parse(&["--fd", "3"])
                .unwrap_err()
                .starts_with("--output is missing")
        );
        for area in ["1,2,3", "1,2,0,4", "a,b,c,d"] {
            let e = parse(&["--fd", "3", "--output", "o", "--area", area]).unwrap_err();
            assert!(e.starts_with("--area takes"), "{area}: {e}");
        }
        assert!(
            parse(&["--frobnicate"])
                .unwrap_err()
                .starts_with("unknown argument")
        );
    }
}
