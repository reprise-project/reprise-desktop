//! What a program and reprise-decode say to each other. The program starts
//! `reprise-decode --file BYTES --pixels BYTES --deadline SECONDS`, writes a [`Request`] and the file to its
//! stdin, and reads a [`Reply`] from its stdout. Its stderr is for the program's log only.

use std::io::{self, Write};
use std::time::Duration;

use repriseconfine::helper::{ReplyCap, TEXT_MAX, printable};

pub const PROGRAM: &str = "reprise-decode";

/// A request's first byte: the protocol's version, which a program and its reprise-decode agree on
/// when both come from one build.
pub const PROTOCOL: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jxl = 1,
}

impl Format {
    fn of(byte: u8) -> Option<Format> {
        match byte {
            1 => Some(Format::Jxl),
            _ => None,
        }
    }
}

/// The bytes ahead of the file: [`PROTOCOL`], the format and the file's length (little-endian).
pub const REQUEST_LEN: usize = 10;

/// What to decode; the file's `len` bytes follow it on stdin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub format: Format,
    pub len: u64,
}

impl Request {
    pub fn to_bytes(self) -> [u8; REQUEST_LEN] {
        let mut b = [0; REQUEST_LEN];
        b[0] = PROTOCOL;
        b[1] = self.format as u8;
        b[2..].copy_from_slice(&self.len.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; REQUEST_LEN]) -> Result<Request, Unknown> {
        if b[0] != PROTOCOL {
            return Err(Unknown::Protocol(b[0]));
        }
        let format = Format::of(b[1]).ok_or(Unknown::Format(b[1]))?;
        let mut len = [0; 8];
        len.copy_from_slice(&b[2..]);
        Ok(Request {
            format,
            len: u64::from_le_bytes(len),
        })
    }
}

/// A request this build cannot read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unknown {
    /// The protocol version of another build.
    Protocol(u8),
    Format(u8),
}

/// What the program allows one decode, fixed before reprise-decode reads its request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The longest file.
    pub file: u64,
    /// The most bytes of RGBA the picture, or any frame of it, may state.
    pub pixels: u64,
    /// How long the decode may run; the program kills reprise-decode a little after.
    pub deadline: Duration,
}

impl Limits {
    pub fn to_args(self) -> [String; 6] {
        [
            "--file".into(),
            self.file.to_string(),
            "--pixels".into(),
            self.pixels.to_string(),
            "--deadline".into(),
            self.deadline.as_secs().to_string(),
        ]
    }

    pub fn from_args(args: &[String]) -> Option<Limits> {
        let [
            file_flag,
            file,
            pixels_flag,
            pixels,
            deadline_flag,
            deadline,
        ] = args
        else {
            return None;
        };
        if file_flag != "--file" || pixels_flag != "--pixels" || deadline_flag != "--deadline" {
            return None;
        }
        Some(Limits {
            file: file.parse().ok()?,
            pixels: pixels.parse().ok()?,
            deadline: Duration::from_secs(deadline.parse().ok()?),
        })
    }

    /// The most of reprise-decode's stdout a program reads, the largest picture reply, with a
    /// picture's buffer sized from its head.
    pub fn reply_cap(self) -> ReplyCap {
        let max = usize::try_from(self.pixels)
            .unwrap_or(usize::MAX)
            .saturating_add(PICTURE_HEAD)
            .max(1 + TEXT_MAX);
        ReplyCap {
            max,
            head: PICTURE_HEAD,
            len_of: picture_len,
        }
    }
}

/// The first byte of a reply: the width and height (little-endian words) and the RGBA follow.
pub const REPLY_PICTURE: u8 = 0;
/// The picture, or the memory its decode asked for, passes the limits.
pub const REPLY_TOO_LARGE: u8 = 1;
/// The file does not decode; why follows.
pub const REPLY_UNREADABLE: u8 = 2;
/// reprise-decode could not take the request (another version, a format it lacks); why follows.
pub const REPLY_FAILED: u8 = 3;

/// The bytes of a picture reply ahead of its RGBA.
pub const PICTURE_HEAD: usize = 9;

/// The length of the picture reply starting with `head`, None for another reply.
fn picture_len(head: &[u8]) -> Option<usize> {
    let (&[REPLY_PICTURE], size) = head.split_at_checked(1)? else {
        return None;
    };
    let (w, h) = (word(size, 0)?, word(size, 4)?);
    let rgba = u64::from(w) * u64::from(h) * 4;
    usize::try_from(rgba).ok()?.checked_add(PICTURE_HEAD)
}

fn word(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Unpremultiplied 8-bit sRGB RGBA, rows top to bottom, upright.
#[derive(Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// The reply as read: [`PICTURE_HEAD`] bytes, then [`Picture::rgba`].
    reply: Vec<u8>,
}

/// Writes the picture reply of `rgba`, `width` x `height` pixels.
pub fn write_picture(out: &mut impl Write, width: u32, height: u32, rgba: &[u8]) -> io::Result<()> {
    out.write_all(&[REPLY_PICTURE])?;
    out.write_all(&width.to_le_bytes())?;
    out.write_all(&height.to_le_bytes())?;
    out.write_all(rgba)?;
    out.flush()
}

impl Picture {
    /// The picture reply `reply`, None when it is empty, or its size and RGBA do not match or
    /// pass `pixels`.
    fn of_reply(reply: Vec<u8>, pixels: u64) -> Option<Picture> {
        let (width, height) = (word(&reply, 1)?, word(&reply, 5)?);
        let len = (u64::from(width) * u64::from(height)).saturating_mul(4);
        let fits = len > 0 && len <= pixels && len == (reply.len() - PICTURE_HEAD) as u64;
        fits.then_some(Picture {
            width,
            height,
            reply,
        })
    }

    pub fn rgba(&self) -> &[u8] {
        &self.reply[PICTURE_HEAD..]
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Picture(Picture),
    TooLarge,
    Unreadable(String),
    Failed(String),
}

impl Reply {
    pub fn write_to(&self, out: &mut impl Write) -> io::Result<()> {
        match self {
            Reply::Picture(p) => out.write_all(&p.reply)?,
            Reply::TooLarge => out.write_all(&[REPLY_TOO_LARGE])?,
            Reply::Unreadable(why) => {
                out.write_all(&[REPLY_UNREADABLE])?;
                out.write_all(why.as_bytes())?;
            }
            Reply::Failed(why) => {
                out.write_all(&[REPLY_FAILED])?;
                out.write_all(why.as_bytes())?;
            }
        }
        out.flush()
    }

    /// The reply in `bytes`, all of reprise-decode's stdout, for a request under `limits`. Its
    /// text is [`printable`].
    pub fn parse(bytes: Vec<u8>, limits: Limits) -> Result<Reply, BadReply> {
        let Some(&tag) = bytes.first() else {
            return Err(BadReply::Empty);
        };
        let text = |b: &[u8]| printable(&b[..b.len().min(TEXT_MAX)]);
        match tag {
            REPLY_PICTURE => Picture::of_reply(bytes, limits.pixels)
                .map(Reply::Picture)
                .ok_or(BadReply::Size),
            REPLY_TOO_LARGE => Ok(Reply::TooLarge),
            REPLY_UNREADABLE => Ok(Reply::Unreadable(text(&bytes[1..]))),
            REPLY_FAILED => Ok(Reply::Failed(text(&bytes[1..]))),
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
    /// A picture whose RGBA is not its stated size, or passes the limits.
    Size,
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Picture {
        fn new(width: u32, height: u32, rgba: &[u8]) -> Option<Picture> {
            let mut reply = Vec::new();
            write_picture(&mut reply, width, height, rgba).unwrap();
            Picture::of_reply(reply, u64::MAX)
        }
    }

    const LIMITS: Limits = Limits {
        file: 1 << 20,
        pixels: 1 << 20,
        deadline: Duration::from_secs(60),
    };

    #[test]
    fn a_request_reads_back_as_written() {
        let r = Request {
            format: Format::Jxl,
            len: 0x0102_0304_0506,
        };
        let b = r.to_bytes();
        assert_eq!(b[..2], [PROTOCOL, 1], "the version, then the format");
        assert_eq!(Request::from_bytes(&b), Ok(r));
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
            Reply::Picture(Picture::new(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap()),
            Reply::TooLarge,
            Reply::Unreadable("cut short".into()),
            Reply::Failed("another version".into()),
        ] {
            let mut bytes = Vec::new();
            reply.write_to(&mut bytes).unwrap();
            assert_eq!(Reply::parse(bytes, LIMITS).as_ref(), Ok(&reply));
        }
        assert_eq!(Reply::parse(Vec::new(), LIMITS), Err(BadReply::Empty));
        assert_eq!(Reply::parse(vec![9, 1], LIMITS), Err(BadReply::Tag(9)));
        assert!(Picture::new(2, 2, &[0; 8]).is_none());
        for (w, h) in [(0, 0), (0, 5), (5, 0)] {
            assert!(Picture::new(w, h, &[]).is_none(), "{w}x{h}");
        }
    }

    #[test]
    fn a_picture_reply_is_its_stated_size_within_the_limits() {
        let mut short = Vec::new();
        Reply::Picture(Picture::new(2, 1, &[0; 8]).unwrap())
            .write_to(&mut short)
            .unwrap();
        short.pop();
        assert_eq!(Reply::parse(short, LIMITS), Err(BadReply::Size));
        assert_eq!(Reply::parse(vec![0, 1, 0], LIMITS), Err(BadReply::Size));
        let tight = Limits {
            pixels: 7,
            ..LIMITS
        };
        let mut bytes = Vec::new();
        Reply::Picture(Picture::new(2, 1, &[0; 8]).unwrap())
            .write_to(&mut bytes)
            .unwrap();
        assert_eq!(Reply::parse(bytes, tight), Err(BadReply::Size));
        assert_eq!(LIMITS.reply_cap().max, PICTURE_HEAD + (1 << 20));
        assert_eq!(
            tight.reply_cap().max,
            1 + TEXT_MAX,
            "room for a reply's text"
        );
        let len_of = LIMITS.reply_cap().len_of;
        let mut head = vec![REPLY_PICTURE];
        head.extend(3u32.to_le_bytes());
        head.extend(2u32.to_le_bytes());
        assert_eq!(len_of(&head), Some(PICTURE_HEAD + 24));
        assert_eq!(len_of(&head[..5]), None, "a cut head");
        head[0] = REPLY_UNREADABLE;
        assert_eq!(len_of(&head), None, "text");
    }

    #[test]
    fn a_replys_text_is_one_line_of_at_most_1_kib() {
        let mut said = vec![REPLY_UNREADABLE];
        said.extend_from_slice(b"bad\nline\x1b[31m\0\xff");
        assert_eq!(
            Reply::parse(said, LIMITS),
            Ok(Reply::Unreadable("bad line [31m \u{fffd}".into()))
        );
        let mut long = vec![REPLY_FAILED];
        long.resize(5000, b'x');
        let Ok(Reply::Failed(why)) = Reply::parse(long, LIMITS) else {
            panic!("a failed reply")
        };
        assert_eq!(why.len(), TEXT_MAX);
    }

    #[test]
    fn the_limits_read_back_from_the_arguments() {
        assert_eq!(Limits::from_args(&LIMITS.to_args()), Some(LIMITS));
        assert_eq!(Limits::from_args(&["--pixels".into(), "1".into()]), None);
    }
}
