//! The pipe to the thumbnail helper, which decodes pictures in a process of its own so its
//! host never runs a picture decoder. The host writes a picture's path and a NUL on the helper's
//! stdin; the helper answers each path in order on its stdout with a [`Thumb`], or an empty answer
//! for a file it cannot decode.

use std::ffi::OsString;
use std::io::{self, BufRead, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// The helper: [`PROGRAM`] run with [`FLAG`].
pub const PROGRAM: &str = "reprise-explorer";
pub const FLAG: &str = "--thumbnailer";

/// A thumbnail of a picture `dims` in size: `width` x `height` premultiplied ARGB words, row by
/// row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
    pub dims: (u32, u32),
}

pub fn write_request(w: &mut impl Write, path: &Path) -> io::Result<()> {
    w.write_all(path.as_os_str().as_bytes())?;
    w.write_all(&[0])?;
    w.flush()
}

/// The next path asked for; None once the host has closed the pipe.
pub fn read_request(r: &mut impl BufRead) -> io::Result<Option<PathBuf>> {
    let mut buf = Vec::new();
    if r.read_until(0, &mut buf)? == 0 {
        return Ok(None);
    }
    if buf.pop() != Some(0) {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(Some(PathBuf::from(OsString::from_vec(buf))))
}

/// Four little-endian words (width, height, the picture's width and height), then the pixels as
/// little-endian words. A zero width is no thumbnail.
pub fn write_reply(w: &mut impl Write, thumb: Option<&Thumb>) -> io::Result<()> {
    let Some(t) = thumb.filter(|t| t.pixels.len() == t.width as usize * t.height as usize) else {
        w.write_all(&[0; 16])?;
        return w.flush();
    };
    let head = [t.width, t.height, t.dims.0, t.dims.1];
    let mut bytes = Vec::with_capacity(16 + t.pixels.len() * 4);
    bytes.extend(head.iter().chain(&t.pixels).flat_map(|v| v.to_le_bytes()));
    w.write_all(&bytes)?;
    w.flush()
}

/// The helper's answer to one request. A thumbnail wider or taller than `max_edge` is a broken
/// helper, not a picture.
pub fn read_reply(r: &mut impl Read, max_edge: u32) -> io::Result<Option<Thumb>> {
    let mut head = [0u8; 16];
    r.read_exact(&mut head)?;
    let word = |i: usize| u32::from_le_bytes(head[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
    let (width, height, dims) = (word(0), word(1), (word(2), word(3)));
    if width == 0 {
        return Ok(None);
    }
    if width > max_edge || height == 0 || height > max_edge {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a {width}x{height} thumbnail"),
        ));
    }
    let mut bytes = vec![0u8; width as usize * height as usize * 4];
    r.read_exact(&mut bytes)?;
    let pixels = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&b| u32::from_le_bytes(b))
        .collect();
    Ok(Some(Thumb {
        width,
        height,
        pixels,
        dims,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn requests_come_back_in_order_until_the_pipe_closes() {
        let odd = Path::new(std::ffi::OsStr::from_bytes(b"/pics/caf\xe9.png"));
        let mut pipe = Vec::new();
        write_request(&mut pipe, Path::new("/pics/a b.jpg")).unwrap();
        write_request(&mut pipe, odd).unwrap();
        let mut r = io::Cursor::new(pipe);
        assert_eq!(
            read_request(&mut r).unwrap().as_deref(),
            Some(Path::new("/pics/a b.jpg"))
        );
        assert_eq!(read_request(&mut r).unwrap().as_deref(), Some(odd));
        assert_eq!(read_request(&mut r).unwrap(), None);
    }

    #[test]
    fn a_reply_carries_the_thumbnail_or_nothing() {
        let t = Thumb {
            width: 2,
            height: 1,
            pixels: vec![0xff00_00ff, 0x8000_4000],
            dims: (640, 320),
        };
        let mut pipe = Vec::new();
        write_reply(&mut pipe, Some(&t)).unwrap();
        write_reply(&mut pipe, None).unwrap();
        let mut r = io::Cursor::new(pipe);
        assert_eq!(read_reply(&mut r, 128).unwrap(), Some(t));
        assert_eq!(read_reply(&mut r, 128).unwrap(), None);
        assert_eq!(
            read_reply(&mut r, 128).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn a_thumbnail_past_the_edge_or_cut_short_is_an_error() {
        let big = Thumb {
            width: 129,
            height: 1,
            pixels: vec![0; 129],
            dims: (1000, 8),
        };
        let mut pipe = Vec::new();
        write_reply(&mut pipe, Some(&big)).unwrap();
        let err = read_reply(&mut io::Cursor::new(&pipe), 128).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        pipe.truncate(pipe.len() - 1);
        let err = read_reply(&mut io::Cursor::new(&pipe), 256).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
