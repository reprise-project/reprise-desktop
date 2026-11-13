//! Annex B NAL units, emulation prevention and the RBSP bit syntax (H.264 7.2, 7.4.1, 9.1).

use super::Error;

/// The NAL units of an Annex B access unit, without start codes or trailing zero bytes.
pub fn nal_units(au: &[u8]) -> Vec<&[u8]> {
    let starts: Vec<usize> = (0..au.len().saturating_sub(2))
        .filter(|&i| au[i..i + 3] == [0, 0, 1])
        .map(|i| i + 3)
        .collect();
    let ends = starts.iter().skip(1).map(|&s| s - 3).chain([au.len()]);
    starts
        .iter()
        .zip(ends)
        .filter_map(|(&start, end)| {
            let nal = &au[start..end.max(start)];
            let len = nal.iter().rposition(|&b| b != 0)? + 1;
            Some(&nal[..len])
        })
        .collect()
}

/// The RBSP of an escaped NAL unit payload: each 0x03 after two zero bytes dropped.
pub fn unescape(escaped: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(escaped.len());
    let mut zeros = 0;
    for &b in escaped {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// `rbsp` with an 0x03 before each byte up to 0x03 that follows two zero bytes, and after two
/// final zero bytes.
pub fn escape(rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rbsp.len() + rbsp.len() / 64);
    let mut zeros = 0;
    for &b in rbsp {
        if zeros >= 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    if zeros >= 2 {
        out.push(3);
    }
    out
}

/// Reads fixed-width and Exp-Golomb fields from an RBSP, most significant bit first.
pub struct Reader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, bit: 0 }
    }

    /// An `n`-bit unsigned field, `n` up to 32.
    pub fn u(&mut self, n: u32) -> Result<u32, Error> {
        debug_assert!(n <= 32);
        if self.bit + n as usize > self.data.len() * 8 {
            return Err(Error::Malformed("a field past the end of its NAL unit"));
        }
        let mut v = 0u64;
        for _ in 0..n {
            let byte = self.data[self.bit / 8];
            v = (v << 1) | u64::from((byte >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(v as u32)
    }

    pub fn flag(&mut self) -> Result<bool, Error> {
        Ok(self.u(1)? == 1)
    }

    /// ue(v).
    pub fn ue(&mut self) -> Result<u32, Error> {
        let mut zeros = 0;
        while !self.flag()? {
            zeros += 1;
            if zeros > 31 {
                return Err(Error::Malformed("an Exp-Golomb code over 32 bits"));
            }
        }
        let v = (1u64 << zeros) - 1 + u64::from(self.u(zeros)?);
        u32::try_from(v).map_err(|_| Error::Malformed("an Exp-Golomb code over 32 bits"))
    }

    /// se(v).
    pub fn se(&mut self) -> Result<i32, Error> {
        let k = i64::from(self.ue()?);
        let v = if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) };
        Ok(v as i32)
    }

    pub fn aligned(&self) -> bool {
        self.bit.is_multiple_of(8)
    }

    /// The bytes from the current position on, which must be byte-aligned.
    pub fn rest(&self) -> &'a [u8] {
        debug_assert!(self.aligned());
        &self.data[self.bit / 8..]
    }
}

/// Writes fields the way [`Reader`] reads them.
#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
    bits: usize,
}

impl Writer {
    /// `v` in `n` bits, `n` up to 32.
    pub fn u(&mut self, n: u32, v: u32) {
        debug_assert!(n <= 32);
        for i in (0..n).rev() {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = ((v >> i) & 1) as u8;
            *self.bytes.last_mut().expect("pushed above") |= bit << (7 - self.bits % 8);
            self.bits += 1;
        }
    }

    pub fn flag(&mut self, v: bool) {
        self.u(1, v.into());
    }

    /// ue(v), `v` up to 2^32 - 2.
    pub fn ue(&mut self, v: u32) {
        let code = v.checked_add(1).expect("ue(v) codes up to 2^32 - 2");
        let n = 32 - code.leading_zeros();
        self.u(n - 1, 0);
        self.u(n, code);
    }

    /// se(v).
    pub fn se(&mut self, v: i32) {
        let v = i64::from(v);
        let k = if v > 0 { 2 * v - 1 } else { -2 * v };
        self.ue(u32::try_from(k).expect("se(v) codes above i32::MIN"));
    }

    pub fn aligned(&self) -> bool {
        self.bits.is_multiple_of(8)
    }

    /// The bytes written, the last one padded with zero bits.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(w: Writer) -> String {
        let n = w.bits;
        let s: String = w.into_bytes().iter().map(|b| format!("{b:08b}")).collect();
        s[..n].to_string()
    }

    #[test]
    fn exp_golomb_codes_follow_the_code_table() {
        let mut w = Writer::default();
        for v in [0, 1, 2, 3, 7] {
            w.ue(v);
        }
        assert_eq!(bits(w), "1 010 011 00100 0001000".replace(' ', ""));
        let mut w = Writer::default();
        for v in [0, 1, -1, 2, -2] {
            w.se(v);
        }
        assert_eq!(bits(w), "1 010 011 00100 00101".replace(' ', ""));
    }

    #[test]
    fn fields_read_back_as_written() {
        let ue = [0, 1, 2, 254, 255, 65_535, 1 << 20, u32::MAX - 1];
        let se = [0, 1, -1, 26, -26, i32::MAX, i32::MIN + 1];
        let mut w = Writer::default();
        w.u(3, 5);
        for v in ue {
            w.ue(v);
        }
        w.u(32, 0xdead_beef);
        for v in se {
            w.se(v);
        }
        w.flag(true);
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u(3).unwrap(), 5);
        for v in ue {
            assert_eq!(r.ue().unwrap(), v);
        }
        assert_eq!(r.u(32).unwrap(), 0xdead_beef);
        for v in se {
            assert_eq!(r.se().unwrap(), v);
        }
        assert!(r.flag().unwrap());
    }

    #[test]
    fn reading_past_the_end_is_an_error() {
        let mut r = Reader::new(&[0x80]);
        assert!(r.u(8).is_ok());
        assert!(r.u(1).is_err());
        assert!(Reader::new(&[0, 0, 0, 0, 0]).ue().is_err());
    }

    #[test]
    fn escape_breaks_start_code_patterns() {
        let cases: [(&[u8], &[u8]); 6] = [
            (&[0, 0, 0], &[0, 0, 3, 0]),
            (&[0, 0, 1], &[0, 0, 3, 1]),
            (&[0, 0, 3, 0, 0, 2], &[0, 0, 3, 3, 0, 0, 3, 2]),
            (&[0, 0, 4, 0, 0], &[0, 0, 4, 0, 0, 3]),
            (&[1, 0, 0, 0, 0], &[1, 0, 0, 3, 0, 0, 3]),
            (&[5, 0, 7], &[5, 0, 7]),
        ];
        for (rbsp, escaped) in cases {
            assert_eq!(escape(rbsp), escaped, "{rbsp:?}");
            assert_eq!(unescape(escaped), rbsp, "{escaped:?}");
        }
    }

    #[test]
    fn escape_round_trips_and_leaves_no_start_code() {
        let mut seed = 0x2545_f491_u32;
        let mut rbsp: Vec<u8> = (0..20_000)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                [0, 0, 0, 1, 2, 3, (seed >> 8) as u8][(seed % 7) as usize]
            })
            .collect();
        rbsp.extend([0, 0]);
        let escaped = escape(&rbsp);
        assert!(
            !escaped
                .windows(3)
                .any(|w| w[0] == 0 && w[1] == 0 && w[2] <= 2)
        );
        assert_ne!(escaped.last(), Some(&0));
        assert_eq!(unescape(&escaped), rbsp);
    }

    #[test]
    fn nal_units_split_on_both_start_code_lengths() {
        let au = [
            0, 0, 0, 1, 9, 0xf0, 0, 0, 1, 0x67, 1, 0, 0, 0, 0, 1, 0x68, 2, 0,
        ];
        assert_eq!(
            nal_units(&au),
            [&[9, 0xf0][..], &[0x67, 1][..], &[0x68, 2][..]]
        );
    }
}
