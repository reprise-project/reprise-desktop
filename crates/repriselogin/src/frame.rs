//! A frame is a 4-byte little-endian body length, then the message as JSON.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

/// Largest body a frame may declare, in bytes.
pub const MAX_FRAME: usize = 65536;

const PREFIX: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    TooLong(usize),
    Json(String),
    Io(io::ErrorKind),
    Closed,
    /// A second fd for one frame, a truncated control message, or an fd with no frame start.
    Fd,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::TooLong(n) => write!(f, "frame of {n} bytes is over {MAX_FRAME}"),
            FrameError::Json(e) => write!(f, "bad message: {e}"),
            FrameError::Io(k) => write!(f, "socket error: {k}"),
            FrameError::Closed => f.write_str("peer closed the socket"),
            FrameError::Fd => f.write_str("a file descriptor came out of place"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        FrameError::Io(e.kind())
    }
}

/// Serializes into one buffer sized for the largest frame, so a secret in
/// `msg` is never left behind in a buffer that grew.
pub fn encode<T: Serialize>(msg: &T) -> Result<Zeroizing<Vec<u8>>, FrameError> {
    let mut frame = Zeroizing::new(Vec::with_capacity(PREFIX + MAX_FRAME));
    frame.extend_from_slice(&[0; PREFIX]);
    let mut w = Bounded {
        frame: &mut frame,
        over: 0,
    };
    if let Err(e) = serde_json::to_writer(&mut w, msg) {
        return Err(match w.over {
            0 => json_error(&e),
            over => FrameError::TooLong(over),
        });
    }
    let len = frame.len() - PREFIX;
    frame[..PREFIX].copy_from_slice(&(len as u32).to_le_bytes());
    Ok(frame)
}

pub fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, FrameError> {
    serde_json::from_slice(body).map_err(|e| json_error(&e))
}

/// The error's category and place only: serde_json's text can quote the message, a secret too.
fn json_error(e: &serde_json::Error) -> FrameError {
    FrameError::Json(format!(
        "{:?} error at line {} column {}",
        e.classify(),
        e.line(),
        e.column()
    ))
}

/// Refuses a write past `MAX_FRAME` instead of reallocating; `over` is then
/// the body length the write asked for.
struct Bounded<'a> {
    frame: &'a mut Vec<u8>,
    over: usize,
}

impl Write for Bounded<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = self.frame.len() - PREFIX + buf.len();
        if len > MAX_FRAME {
            self.over = len;
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        self.frame.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Splits a byte stream into frame bodies. Each body is allocated once at
/// its declared length. After `TooLong` every push fails: drop the peer.
#[derive(Default)]
pub struct FrameReader {
    prefix: [u8; PREFIX],
    prefix_len: usize,
    body: Option<Zeroizing<Vec<u8>>>,
    need: usize,
    failed: Option<FrameError>,
    ready: VecDeque<Zeroizing<Vec<u8>>>,
    begun: u64,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, mut bytes: &[u8]) -> Result<(), FrameError> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        while !bytes.is_empty() {
            let Some(body) = &mut self.body else {
                if self.prefix_len == 0 {
                    self.begun += 1;
                }
                let take = (PREFIX - self.prefix_len).min(bytes.len());
                self.prefix[self.prefix_len..][..take].copy_from_slice(&bytes[..take]);
                self.prefix_len += take;
                bytes = &bytes[take..];
                if self.prefix_len == PREFIX {
                    self.prefix_len = 0;
                    self.start_body()?;
                }
                continue;
            };
            let take = (self.need - body.len()).min(bytes.len());
            body.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if body.len() == self.need
                && let Some(body) = self.body.take()
            {
                self.ready.push_back(body);
            }
        }
        Ok(())
    }

    fn start_body(&mut self) -> Result<(), FrameError> {
        let len = u32::from_le_bytes(self.prefix) as usize;
        if len > MAX_FRAME {
            let e = FrameError::TooLong(len);
            self.failed = Some(e.clone());
            return Err(e);
        }
        let body = Zeroizing::new(Vec::with_capacity(len));
        if len == 0 {
            self.ready.push_back(body);
        } else {
            self.need = len;
            self.body = Some(body);
        }
        Ok(())
    }

    pub fn next_frame(&mut self) -> Option<Zeroizing<Vec<u8>>> {
        self.ready.pop_front()
    }

    /// Frames whose first byte has been pushed.
    pub(crate) fn begun(&self) -> u64 {
        self.begun
    }
}

pub fn write_msg<T: Serialize>(w: &mut impl Write, msg: &T) -> Result<(), FrameError> {
    w.write_all(&encode(msg)?)?;
    Ok(w.flush()?)
}

/// `Closed` when the stream ends before a frame starts; an end inside a
/// frame is `Io(UnexpectedEof)`.
pub fn read_msg<T: DeserializeOwned>(r: &mut impl Read) -> Result<T, FrameError> {
    let mut prefix = [0; PREFIX];
    let mut have = 0;
    while have < PREFIX {
        match r.read(&mut prefix[have..]) {
            Ok(0) if have == 0 => return Err(FrameError::Closed),
            Ok(0) => return Err(FrameError::Io(io::ErrorKind::UnexpectedEof)),
            Ok(n) => have += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let len = u32::from_le_bytes(prefix) as usize;
    if len > MAX_FRAME {
        return Err(FrameError::TooLong(len));
    }
    let mut body = Zeroizing::new(vec![0; len]);
    r.read_exact(&mut body)?;
    decode(&body)
}

/// Secrets travel as hex text: serde_json borrows it from the frame body, and it decodes into
/// one buffer of its final size, so no unwiped copy is left on the heap.
pub mod hex {
    use std::fmt::{self, Write};

    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use zeroize::Zeroizing;

    struct Hex<'a>(&'a [u8]);

    impl fmt::Display for Hex<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            for b in self.0 {
                for nibble in [b >> 4, b & 15] {
                    f.write_char(char::from(b"0123456789abcdef"[usize::from(nibble)]))?;
                }
            }
            Ok(())
        }
    }

    /// serde_json's `collect_str` writes straight into the frame, with no `String` between.
    impl Serialize for Hex<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.collect_str(self)
        }
    }

    fn bytes<E: Error>(hex: &str) -> Result<Zeroizing<Vec<u8>>, E> {
        if !hex.len().is_multiple_of(2) {
            return Err(E::custom("hex of odd length"));
        }
        let mut out = Zeroizing::new(vec![0; hex.len() / 2]);
        for (o, [hi, lo]) in out.iter_mut().zip(hex.as_bytes().as_chunks().0) {
            let digit = |c: u8| {
                char::from(c)
                    .to_digit(16)
                    .ok_or_else(|| E::custom("not hex"))
            };
            *o = (digit(*hi)? << 4 | digit(*lo)?) as u8;
        }
        Ok(out)
    }

    /// Escaped hex would come from serde_json's scratch buffer, so only borrowed text is taken.
    fn borrowed<'de, D: Deserializer<'de>>(d: D) -> Result<&'de str, D::Error> {
        <&str>::deserialize(d)
    }

    pub fn serialize<S: Serializer>(v: &Zeroizing<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        Hex(v).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Zeroizing<Vec<u8>>, D::Error> {
        bytes(borrowed(d)?)
    }

    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(
            v: &Option<Zeroizing<Vec<u8>>>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            v.as_ref().map(|v| Hex(v)).serialize(s)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<Zeroizing<Vec<u8>>>, D::Error> {
            Option::<&str>::deserialize(d)?.map(bytes).transpose()
        }
    }

    fn text<E: Error>(hex: &str) -> Result<Zeroizing<String>, E> {
        let mut utf8 = bytes(hex)?;
        match String::from_utf8(std::mem::take(&mut *utf8)) {
            Ok(s) => Ok(Zeroizing::new(s)),
            Err(e) => {
                drop(Zeroizing::new(e.into_bytes()));
                Err(E::custom("not UTF-8"))
            }
        }
    }

    /// A password as the hex of its UTF-8 bytes.
    pub mod text {
        use super::*;

        pub fn serialize<S: Serializer>(v: &Zeroizing<String>, s: S) -> Result<S::Ok, S::Error> {
            Hex(v.as_bytes()).serialize(s)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Zeroizing<String>, D::Error> {
            super::text(borrowed(d)?)
        }

        pub mod option {
            use super::super::*;

            pub fn serialize<S: Serializer>(
                v: &Option<Zeroizing<String>>,
                s: S,
            ) -> Result<S::Ok, S::Error> {
                v.as_ref().map(|v| Hex(v.as_bytes())).serialize(s)
            }

            pub fn deserialize<'de, D: Deserializer<'de>>(
                d: D,
            ) -> Result<Option<Zeroizing<String>>, D::Error> {
                Option::<&str>::deserialize(d)?.map(text).transpose()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interface;
    use crate::greeter::{GreeterReply, GreeterRequest, Greeting, Keyboard, Note, UserEntry};
    use crate::session::SessionRequest;
    use std::path::PathBuf;

    fn greeting() -> Greeting {
        Greeting {
            interface: Interface::Blue,
            show_users: true,
            users: vec![
                UserEntry {
                    name: "anna".into(),
                    real_name: "Anna Berg".into(),
                    picture: Some(PathBuf::from("/var/lib/AccountsService/icons/anna")),
                    note: Some(Note::LoggedOn),
                },
                UserEntry {
                    name: "bo".into(),
                    real_name: String::new(),
                    picture: None,
                    note: None,
                },
            ],
            last_user: Some("anna".into()),
            keyboard: Keyboard {
                layout: "us,hu".into(),
                variant: String::new(),
                options: "grp:alt_shift_toggle".into(),
            },
            notice: None,
            outputs: Vec::new(),
            background: None,
            animation: true,
        }
    }

    #[test]
    fn a_frame_round_trips_through_the_reader_split_at_every_byte() {
        let sent = GreeterReply::Greeting(greeting());
        let bytes = encode(&sent).unwrap();
        let mut reader = FrameReader::new();
        let mut frames = Vec::new();
        for b in bytes.iter() {
            reader.push(std::slice::from_ref(b)).unwrap();
            frames.extend(std::iter::from_fn(|| reader.next_frame()));
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(decode::<GreeterReply>(&frames[0]).unwrap(), sent);
    }

    #[test]
    fn one_push_can_carry_several_frames() {
        let mut bytes = encode(&SessionRequest::SwitchUser).unwrap().to_vec();
        bytes.extend_from_slice(&encode(&GreeterRequest::StartSession).unwrap());
        let mut reader = FrameReader::new();
        reader.push(&bytes).unwrap();
        let first = reader.next_frame().unwrap();
        let second = reader.next_frame().unwrap();
        assert_eq!(reader.next_frame(), None);
        assert_eq!(
            decode::<SessionRequest>(&first).unwrap(),
            SessionRequest::SwitchUser
        );
        assert_eq!(
            decode::<GreeterRequest>(&second).unwrap(),
            GreeterRequest::StartSession
        );
    }

    #[test]
    fn a_declared_length_past_max_frame_is_refused() {
        let mut reader = FrameReader::new();
        let prefix = ((MAX_FRAME + 1) as u32).to_le_bytes();
        assert_eq!(reader.push(&prefix), Err(FrameError::TooLong(65537)));
        assert_eq!(reader.push(b"{}"), Err(FrameError::TooLong(65537)));
    }

    #[test]
    fn a_message_past_max_frame_is_not_encoded() {
        let user = "a".repeat(MAX_FRAME);
        assert!(matches!(
            encode(&GreeterRequest::CreateSession { user }),
            Err(FrameError::TooLong(_))
        ));
    }

    #[test]
    fn a_message_of_exactly_max_frame_is_encoded() {
        let overhead = encode(&GreeterRequest::CreateSession {
            user: String::new(),
        })
        .unwrap()
        .len()
            - PREFIX;
        let user = "a".repeat(MAX_FRAME - overhead);
        let frame = encode(&GreeterRequest::CreateSession { user: user.clone() }).unwrap();
        assert_eq!(frame.len(), PREFIX + MAX_FRAME);
        let over = encode(&GreeterRequest::CreateSession { user: user + "a" });
        assert_eq!(over, Err(FrameError::TooLong(MAX_FRAME + 1)));
    }

    #[test]
    fn read_msg_refuses_a_prefix_past_max_frame() {
        let prefix = ((MAX_FRAME + 1) as u32).to_le_bytes();
        assert_eq!(
            read_msg::<SessionRequest>(&mut &prefix[..]),
            Err(FrameError::TooLong(MAX_FRAME + 1))
        );
        let mut at_limit = (MAX_FRAME as u32).to_le_bytes().to_vec();
        at_limit.resize(PREFIX + MAX_FRAME, b' ');
        assert!(matches!(
            read_msg::<SessionRequest>(&mut &at_limit[..]),
            Err(FrameError::Json(_))
        ));
    }

    #[test]
    fn a_json_error_carries_no_text_of_the_message() {
        let e = decode::<SessionRequest>(br#"{"type":"hello","interface":"hunter2"}"#).unwrap_err();
        assert!(matches!(e, FrameError::Json(_)), "{e:?}");
        assert!(!format!("{e} {e:?}").contains("hunter2"), "{e:?}");
    }

    #[test]
    fn a_secret_that_is_not_plain_hex_is_a_json_error() {
        use crate::account::AccountRequest;
        use crate::handover::Step;
        for pdu in [
            br#"{"pdu":{"input":"0","output":""}}"#.as_slice(),
            br#"{"pdu":{"input":"zz","output":""}}"#,
            br#"{"pdu":{"input":"\u0030\u0031","output":""}}"#,
            br#"{"pdu":{"input":[1,2],"output":""}}"#,
        ] {
            assert!(
                matches!(decode::<Step>(pdu), Err(FrameError::Json(_))),
                "{}",
                String::from_utf8_lossy(pdu)
            );
        }
        assert!(matches!(
            decode::<AccountRequest>(br#"{"clear_remote_password":{"login_password":"ff"}}"#),
            Err(FrameError::Json(_))
        ));
        assert_eq!(
            decode::<Step>(br#"{"pdu":{"input":"0A0b","output":""}}"#).unwrap(),
            Step::Pdu {
                input: Zeroizing::new(vec![10, 11]),
                output: Zeroizing::new(vec![]),
            }
        );
    }

    #[test]
    fn an_unknown_type_is_a_json_error_not_a_panic() {
        assert!(matches!(
            decode::<GreeterRequest>(br#"{"teleport":{}}"#),
            Err(FrameError::Json(_))
        ));
    }

    #[test]
    fn read_msg_reports_closed_on_eof() {
        assert_eq!(
            read_msg::<SessionRequest>(&mut &b""[..]),
            Err(FrameError::Closed)
        );
    }

    #[test]
    fn write_msg_and_read_msg_round_trip() {
        let mut wire = Vec::new();
        let sent = SessionRequest::Hello {
            interface: Interface::Glass,
        };
        write_msg(&mut wire, &sent).unwrap();
        assert_eq!(read_msg::<SessionRequest>(&mut &wire[..]).unwrap(), sent);
        assert_eq!(
            read_msg::<SessionRequest>(&mut &wire[..wire.len() - 1]),
            Err(FrameError::Io(io::ErrorKind::UnexpectedEof))
        );
    }
}
