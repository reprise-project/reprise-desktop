//! reprise-decode turns a picture file into its pixels in a process of its own, so a decoder's
//! crash, abort or runaway allocation costs that process and never the program that asked. The
//! program starts it as `reprise-decode --file BYTES --pixels BYTES --deadline SECONDS` and
//! speaks [`reprise_decode_protocol`] to it.

pub mod confine;
pub mod heapcap;
pub mod jxl;
#[cfg(test)]
mod pictures;

use std::io::{self, Read, Write};

use reprise_decode_protocol::{Format, Limits, PROTOCOL, REQUEST_LEN, Reply, Request, Unknown};

#[cfg(test)]
#[global_allocator]
static HEAP: heapcap::Capped = heapcap::Capped;

/// Answers the request on `input` on `output`. A file longer than `limits.file` is refused unread.
pub fn serve(input: &mut impl Read, output: &mut impl Write, limits: Limits) -> io::Result<()> {
    let mut head = [0; REQUEST_LEN];
    input.read_exact(&mut head)?;
    let reply = match Request::from_bytes(&head) {
        Err(Unknown::Protocol(p)) => Reply::Failed(format!(
            "reprise-decode is from another version (protocol {p} asked, it speaks {PROTOCOL})"
        )),
        Err(Unknown::Format(format)) => {
            Reply::Failed(format!("no format {format} in this reprise-decode"))
        }
        Ok(r) if r.len > limits.file => Reply::TooLarge,
        Ok(r) => {
            let mut bytes = Vec::new();
            if bytes.try_reserve_exact(r.len as usize).is_err() {
                return Reply::TooLarge.write_to(output);
            }
            input.take(r.len).read_to_end(&mut bytes)?;
            match r.format {
                Format::Jxl => match jxl::first_frame(&bytes, limits.pixels, limits.deadline) {
                    Ok(p) => {
                        drop(bytes);
                        return reprise_decode_protocol::write_picture(
                            output, p.width, p.height, &p.px,
                        );
                    }
                    Err(jxl::Refused::Size | jxl::Refused::Memory) => Reply::TooLarge,
                    Err(jxl::Refused::Time) => Reply::Unreadable(format!(
                        "the decode ran past its {} s limit",
                        limits.deadline.as_secs()
                    )),
                    Err(jxl::Refused::Broken) => {
                        Reply::Unreadable("not a JPEG XL this decoder reads, or cut short".into())
                    }
                },
            }
        }
    };
    reply.write_to(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const LIMITS: Limits = Limits {
        file: 1 << 20,
        pixels: 1 << 20,
        deadline: Duration::from_secs(60),
    };

    fn ask(head: [u8; REQUEST_LEN], file: &[u8], limits: Limits) -> Reply {
        let mut input = head.to_vec();
        input.extend_from_slice(file);
        let mut out = Vec::new();
        serve(&mut input.as_slice(), &mut out, limits).unwrap();
        Reply::parse(out, limits).unwrap()
    }

    fn jxl(file: &[u8]) -> [u8; REQUEST_LEN] {
        Request {
            format: Format::Jxl,
            len: file.len() as u64,
        }
        .to_bytes()
    }

    #[test]
    fn a_request_gets_the_picture_or_why_not() {
        let px: Vec<u8> = (0..8 * 6 * 4).map(|i| (i * 5) as u8).collect();
        let file = pictures::encode(8, 6, &[&px], false);
        let Reply::Picture(p) = ask(jxl(&file), &file, LIMITS) else {
            panic!("a picture")
        };
        assert_eq!((p.width, p.height), (8, 6));
        assert_eq!(p.rgba(), px);
        let small = Limits {
            pixels: 8 * 6 * 4 - 1,
            ..LIMITS
        };
        assert_eq!(ask(jxl(&file), &file, small), Reply::TooLarge);
        let short = Limits {
            file: file.len() as u64 - 1,
            ..LIMITS
        };
        assert_eq!(
            ask(jxl(&file), &[], short),
            Reply::TooLarge,
            "a file past the limit is not read"
        );
        let cut = &file[..file.len() / 2];
        assert!(matches!(ask(jxl(cut), cut, LIMITS), Reply::Unreadable(_)));
    }

    #[test]
    fn a_request_of_another_version_or_format_fails_saying_so() {
        let mut newer = jxl(b"");
        newer[0] = PROTOCOL + 1;
        let Reply::Failed(why) = ask(newer, b"", LIMITS) else {
            panic!("failed")
        };
        assert!(
            why.starts_with("reprise-decode is from another version"),
            "{why}"
        );
        let mut other = jxl(b"");
        other[1] = 9;
        assert_eq!(
            ask(other, b"", LIMITS),
            Reply::Failed("no format 9 in this reprise-decode".into())
        );
    }
}
