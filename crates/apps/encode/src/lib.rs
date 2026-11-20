//! reprise-encode turns a screenshot's pixels into a file in a process of its own, confined before
//! it reads them, so a codec bug or a broken codec library costs one screenshot and never the
//! session. The compositor starts it as `reprise-encode --budget BYTES --deadline SECONDS`,
//! writes a [`Request`] and the RGB pixels to its stdin and reads a [`Reply`] from its stdout.
//! Its stderr is for the compositor's log only.

pub mod codec;
pub mod confine;

use std::io::{self, Read, Write};
use std::time::Duration;

use codec::JxlCodec;
use repriseconfine::helper::{TEXT_MAX, printable};

/// A request's first byte: the protocol's version, which the compositor and its reprise-encode
/// agree on when both come from one build.
pub const PROTOCOL: u8 = 1;

/// A request's format byte: JPEG XL with [`Jxl`] settings.
pub const FORMAT_JXL: u8 = 1;

/// The bytes ahead of the RGB pixels: [`PROTOCOL`], the format, the width and height
/// (little-endian) and four bytes of the format's settings.
pub const REQUEST_LEN: usize = 14;

/// What to encode; 8-bit RGB pixels, row by row, follow it on stdin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub w: u32,
    pub h: u32,
    pub format: Format,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jxl(Jxl),
}

/// `[screenshot] jxl_lossless`, `jxl_quality` and `jxl_effort`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Jxl {
    pub lossless: bool,
    pub quality: u8,
    pub effort: u8,
}

impl Request {
    pub fn to_bytes(self) -> [u8; REQUEST_LEN] {
        let mut b = [0; REQUEST_LEN];
        b[0] = PROTOCOL;
        b[2..6].copy_from_slice(&self.w.to_le_bytes());
        b[6..10].copy_from_slice(&self.h.to_le_bytes());
        match self.format {
            Format::Jxl(j) => {
                b[1] = FORMAT_JXL;
                b[10] = j.lossless.into();
                b[11] = j.quality;
                b[12] = j.effort;
            }
        }
        b
    }

    pub fn from_bytes(b: &[u8; REQUEST_LEN]) -> Result<Request, Unknown> {
        if b[0] != PROTOCOL {
            return Err(Unknown::Protocol(b[0]));
        }
        let word = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let format = match b[1] {
            FORMAT_JXL => Format::Jxl(Jxl {
                lossless: b[10] != 0,
                quality: b[11],
                effort: b[12],
            }),
            other => return Err(Unknown::Format(other)),
        };
        Ok(Request {
            w: word(2),
            h: word(6),
            format,
        })
    }

    /// The length of the RGB pixels that follow.
    pub fn rgb_len(&self) -> Option<usize> {
        (self.w as usize)
            .checked_mul(self.h as usize)?
            .checked_mul(3)
    }
}

/// A request this build cannot read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unknown {
    /// The protocol version of another build.
    Protocol(u8),
    Format(u8),
}

/// The first byte of a reply: the file follows.
pub const REPLY_FILE: u8 = 0;
/// The encode needed more than the budget.
pub const REPLY_MEMORY: u8 = 1;
/// The codec's error message follows.
pub const REPLY_CODEC: u8 = 2;
/// The format's codec library did not load; why follows.
pub const REPLY_UNAVAILABLE: u8 = 3;

#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    File(Vec<u8>),
    Memory,
    Codec(String),
    Unavailable(String),
}

impl Reply {
    pub fn write_to(&self, out: &mut impl Write) -> io::Result<()> {
        let (tag, rest) = match self {
            Reply::File(file) => (REPLY_FILE, file.as_slice()),
            Reply::Memory => (REPLY_MEMORY, &[][..]),
            Reply::Codec(why) => (REPLY_CODEC, why.as_bytes()),
            Reply::Unavailable(why) => (REPLY_UNAVAILABLE, why.as_bytes()),
        };
        out.write_all(&[tag])?;
        out.write_all(rest)?;
        out.flush()
    }

    /// The reply in `bytes`, all of the helper's stdout. Its text is cut to [`TEXT_MAX`] bytes
    /// and [`printable`].
    pub fn parse(mut bytes: Vec<u8>) -> Result<Reply, BadReply> {
        let Some(&tag) = bytes.first() else {
            return Err(BadReply::Empty);
        };
        let text = |b: &[u8]| printable(&b[..b.len().min(TEXT_MAX)]);
        match tag {
            REPLY_FILE => {
                bytes.remove(0);
                Ok(Reply::File(bytes))
            }
            REPLY_MEMORY => Ok(Reply::Memory),
            REPLY_CODEC => Ok(Reply::Codec(text(&bytes[1..]))),
            REPLY_UNAVAILABLE => Ok(Reply::Unavailable(text(&bytes[1..]))),
            other => Err(BadReply::Tag(other)),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum BadReply {
    /// Nothing came back.
    Empty,
    /// A first byte no reply has.
    Tag(u8),
}

/// What the compositor allows one encode, fixed before the helper reads its request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The bytes the codec may allocate.
    pub budget: u64,
    /// When the compositor kills the helper.
    pub deadline: Duration,
}

impl Limits {
    pub fn to_args(self) -> [String; 4] {
        [
            "--budget".into(),
            self.budget.to_string(),
            "--deadline".into(),
            self.deadline.as_secs().to_string(),
        ]
    }

    pub fn from_args(args: &[String]) -> Option<Limits> {
        let [budget_flag, budget, deadline_flag, deadline] = args else {
            return None;
        };
        if budget_flag != "--budget" || deadline_flag != "--deadline" {
            return None;
        }
        Some(Limits {
            budget: budget.parse().ok()?,
            deadline: Duration::from_secs(deadline.parse().ok()?),
        })
    }
}

/// Answers the request on `input` on `output`. Pixels past `budget` are refused unread: the codec
/// copies them into memory it counts against the budget anyway.
pub fn serve(
    input: &mut impl Read,
    output: &mut impl Write,
    jxl: &Result<JxlCodec, String>,
    budget: u64,
    threads: usize,
) -> io::Result<()> {
    let mut head = [0; REQUEST_LEN];
    input.read_exact(&mut head)?;
    let reply = match Request::from_bytes(&head) {
        Err(Unknown::Protocol(p)) => Reply::Codec(format!(
            "the encoder is from another version (protocol {p} asked, reprise-encode speaks \
             {PROTOCOL})"
        )),
        Err(Unknown::Format(format)) => {
            Reply::Codec(format!("no format {format} in this reprise-encode"))
        }
        Ok(r) => match r.rgb_len() {
            Some(len) if len as u64 <= budget => {
                let mut rgb = vec![0; len];
                input.read_exact(&mut rgb)?;
                match (r.format, jxl) {
                    (Format::Jxl(s), Ok(codec)) => {
                        codec.encode(&rgb, r.w, r.h, s, budget, threads as u32)
                    }
                    (Format::Jxl(_), Err(why)) => Reply::Unavailable(why.clone()),
                }
            }
            _ => Reply::Memory,
        },
    };
    reply.write_to(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_reads_back_as_written() {
        let r = Request {
            w: 1920,
            h: 1080,
            format: Format::Jxl(Jxl {
                lossless: false,
                quality: 90,
                effort: 7,
            }),
        };
        let b = r.to_bytes();
        assert_eq!(
            b[..2],
            [PROTOCOL, FORMAT_JXL],
            "the version, then the format"
        );
        assert_eq!(Request::from_bytes(&b), Ok(r));
        assert_eq!(r.rgb_len(), Some(1920 * 1080 * 3));
        let mut avif = b;
        avif[1] = 2;
        assert_eq!(Request::from_bytes(&avif), Err(Unknown::Format(2)));
        let mut newer = b;
        newer[0] = PROTOCOL + 1;
        assert_eq!(
            Request::from_bytes(&newer),
            Err(Unknown::Protocol(PROTOCOL + 1))
        );
    }

    #[test]
    fn each_reply_reads_back_as_written() {
        for reply in [
            Reply::File(vec![0xff, 0x0a, 0]),
            Reply::File(Vec::new()),
            Reply::Memory,
            Reply::Codec("bad".into()),
            Reply::Unavailable("libjxl.so.0.13: cannot open shared object file".into()),
        ] {
            let mut bytes = Vec::new();
            reply.write_to(&mut bytes).unwrap();
            assert_eq!(Reply::parse(bytes).as_ref(), Ok(&reply));
        }
        assert_eq!(Reply::parse(Vec::new()), Err(BadReply::Empty));
        assert_eq!(Reply::parse(vec![9, 1]), Err(BadReply::Tag(9)));
    }

    #[test]
    fn a_replys_text_is_one_line_of_at_most_1_kib() {
        let mut said = vec![REPLY_CODEC];
        said.extend_from_slice(b"bad\nline\x1b[31m\0\xff");
        assert_eq!(
            Reply::parse(said),
            Ok(Reply::Codec("bad line [31m \u{fffd}".into()))
        );
        let mut long = vec![REPLY_UNAVAILABLE];
        long.resize(5000, b'x');
        let Ok(Reply::Unavailable(why)) = Reply::parse(long) else {
            panic!("an unavailable reply")
        };
        assert_eq!(why.len(), TEXT_MAX);
    }

    #[test]
    fn the_limits_read_back_from_the_arguments() {
        let l = Limits {
            budget: 512 << 20,
            deadline: Duration::from_secs(600),
        };
        assert_eq!(Limits::from_args(&l.to_args()), Some(l));
        assert_eq!(Limits::from_args(&["--budget".into(), "1".into()]), None);
    }
}

#[cfg(test)]
mod serve_tests {
    use super::*;

    fn rgb(w: u32, h: u32) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [(x * 4) as u8, (y * 5) as u8, (((x ^ y) * 3) & 0xff) as u8]
            })
            .collect()
    }

    fn ask(r: Request, rgb: &[u8], jxl: &Result<JxlCodec, String>, budget: u64) -> Reply {
        let mut input = r.to_bytes().to_vec();
        input.extend_from_slice(rgb);
        let mut out = Vec::new();
        serve(&mut input.as_slice(), &mut out, jxl, budget, 2).unwrap();
        Reply::parse(out).unwrap()
    }

    fn jxl() -> Result<JxlCodec, String> {
        // SAFETY: the codec crate's own entry.
        Ok(unsafe { JxlCodec::from_fn(reprise_encode_jxl::reprise_encode_jxl_v1) })
    }

    const LOSSLESS: Request = Request {
        w: 64,
        h: 48,
        format: Format::Jxl(Jxl {
            lossless: true,
            quality: 90,
            effort: 3,
        }),
    };

    #[test]
    fn a_request_gets_the_file_or_why_not() {
        let px = rgb(64, 48);
        let Reply::File(file) = ask(LOSSLESS, &px, &jxl(), 256 << 20) else {
            panic!("a file")
        };
        assert!(file.starts_with(&[0xff, 0x0a]), "a JPEG XL codestream");
        assert_eq!(ask(LOSSLESS, &px, &jxl(), 64 << 10), Reply::Memory);
        assert_eq!(
            ask(LOSSLESS, &px, &jxl(), 1000),
            Reply::Memory,
            "pixels past the budget are not read"
        );
    }

    #[test]
    fn a_jxl_request_without_the_codec_says_why_it_did_not_load() {
        let why = "/usr/lib/reprise/libreprise_encode_jxl.so: libjxl.so.0.13: cannot open shared object file";
        assert_eq!(
            ask(LOSSLESS, &rgb(64, 48), &Err(why.into()), 256 << 20),
            Reply::Unavailable(why.into())
        );
    }

    #[test]
    fn a_request_of_another_protocol_version_is_a_codec_error() {
        let mut input = LOSSLESS.to_bytes().to_vec();
        input[0] = PROTOCOL + 1;
        let mut out = Vec::new();
        serve(&mut input.as_slice(), &mut out, &jxl(), 1 << 20, 1).unwrap();
        let Ok(Reply::Codec(why)) = Reply::parse(out) else {
            panic!("a codec error")
        };
        assert!(
            why.starts_with("the encoder is from another version"),
            "{why}"
        );
    }

    #[test]
    fn a_format_this_build_lacks_is_a_codec_error() {
        let mut input = LOSSLESS.to_bytes().to_vec();
        input[1] = 9;
        let mut out = Vec::new();
        serve(&mut input.as_slice(), &mut out, &jxl(), 1 << 20, 1).unwrap();
        assert_eq!(
            Reply::parse(out),
            Ok(Reply::Codec("no format 9 in this reprise-encode".into()))
        );
    }
}
