//! X25519 (RFC 7748) for one job: the public key of a WireGuard private
//! key, so Generate can show both halves. Field elements are five 51-bit
//! limbs; the ladder swaps by mask, never by branch.

type Fe = [u64; 5];

const MASK: u64 = (1 << 51) - 1;

fn load(b: &[u8; 32], i: usize) -> u64 {
    let mut w = [0u8; 8];
    w.copy_from_slice(&b[i..i + 8]);
    u64::from_le_bytes(w)
}

fn from_bytes(b: &[u8; 32]) -> Fe {
    [
        load(b, 0) & MASK,
        (load(b, 6) >> 3) & MASK,
        (load(b, 12) >> 6) & MASK,
        (load(b, 19) >> 1) & MASK,
        (load(b, 24) >> 12) & MASK,
    ]
}

/// Carry every limb down to 51 bits, folding the top carry back in times
/// 19 (2^255 = 19 mod p).
fn carry(mut h: Fe) -> Fe {
    for _ in 0..2 {
        for i in 0..4 {
            h[i + 1] += h[i] >> 51;
            h[i] &= MASK;
        }
        h[0] += (h[4] >> 51) * 19;
        h[4] &= MASK;
    }
    h
}

fn add(a: &Fe, b: &Fe) -> Fe {
    carry([
        a[0] + b[0],
        a[1] + b[1],
        a[2] + b[2],
        a[3] + b[3],
        a[4] + b[4],
    ])
}

/// `a - b` as `a + 4p - b`, so no limb goes below zero.
fn sub(a: &Fe, b: &Fe) -> Fe {
    const P4_0: u64 = 4 * ((1 << 51) - 19);
    const P4_N: u64 = 4 * ((1 << 51) - 1);
    carry([
        a[0] + P4_0 - b[0],
        a[1] + P4_N - b[1],
        a[2] + P4_N - b[2],
        a[3] + P4_N - b[3],
        a[4] + P4_N - b[4],
    ])
}

fn mul(a: &Fe, b: &Fe) -> Fe {
    let m = |x: u64, y: u64| u128::from(x) * u128::from(y);
    let b19 = [b[1] * 19, b[2] * 19, b[3] * 19, b[4] * 19];
    let r = [
        m(a[0], b[0]) + m(a[1], b19[3]) + m(a[2], b19[2]) + m(a[3], b19[1]) + m(a[4], b19[0]),
        m(a[0], b[1]) + m(a[1], b[0]) + m(a[2], b19[3]) + m(a[3], b19[2]) + m(a[4], b19[1]),
        m(a[0], b[2]) + m(a[1], b[1]) + m(a[2], b[0]) + m(a[3], b19[3]) + m(a[4], b19[2]),
        m(a[0], b[3]) + m(a[1], b[2]) + m(a[2], b[1]) + m(a[3], b[0]) + m(a[4], b19[3]),
        m(a[0], b[4]) + m(a[1], b[3]) + m(a[2], b[2]) + m(a[3], b[1]) + m(a[4], b[0]),
    ];
    let mut out = [0u64; 5];
    let mut c: u128 = 0;
    for i in 0..5 {
        let v = r[i] + c;
        out[i] = (v as u64) & MASK;
        c = v >> 51;
    }
    out[0] += (c as u64) * 19;
    carry(out)
}

fn square(a: &Fe) -> Fe {
    mul(a, a)
}

/// `z^(p-2)`, which is `1/z`: p - 2 = 2^255 - 21 has every bit of 0..255
/// set but bits 2 and 4.
fn invert(z: &Fe) -> Fe {
    let mut r: Fe = [1, 0, 0, 0, 0];
    for bit in (0..255).rev() {
        r = square(&r);
        if bit != 2 && bit != 4 {
            r = mul(&r, z);
        }
    }
    r
}

/// The canonical little-endian bytes of `h`, fully reduced mod p.
fn to_bytes(h: &Fe) -> [u8; 32] {
    let mut h = carry(*h);
    // h < 2^255 + a little; subtract p once if h >= p.
    let mut q = (h[0] + 19) >> 51;
    for limb in &h[1..] {
        q = (limb + q) >> 51;
    }
    h[0] += 19 * q;
    for i in 0..4 {
        h[i + 1] += h[i] >> 51;
        h[i] &= MASK;
    }
    h[4] &= MASK;
    let mut out = [0u8; 32];
    let (mut acc, mut bits, mut i) = (0u128, 0, 0);
    for limb in h {
        acc |= u128::from(limb) << bits;
        bits += 51;
        while bits >= 8 {
            out[i] = acc as u8;
            acc >>= 8;
            bits -= 8;
            i += 1;
        }
    }
    out[i] = acc as u8;
    out
}

fn cswap(swap: u64, a: &mut Fe, b: &mut Fe) {
    let mask = 0u64.wrapping_sub(swap);
    for i in 0..5 {
        let t = mask & (a[i] ^ b[i]);
        a[i] ^= t;
        b[i] ^= t;
    }
}

/// X25519(`scalar`, `u`): the scalar is clamped first.
pub fn x25519(scalar: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;
    let x1 = from_bytes(u);
    let (mut x2, mut z2): (Fe, Fe) = ([1, 0, 0, 0, 0], [0; 5]);
    let (mut x3, mut z3): (Fe, Fe) = (x1, [1, 0, 0, 0, 0]);
    let a24: Fe = [121_665, 0, 0, 0, 0];
    let mut swap = 0u64;
    for t in (0..255).rev() {
        let bit = u64::from((k[t / 8] >> (t % 8)) & 1);
        swap ^= bit;
        cswap(swap, &mut x2, &mut x3);
        cswap(swap, &mut z2, &mut z3);
        swap = bit;
        let a = add(&x2, &z2);
        let aa = square(&a);
        let b = sub(&x2, &z2);
        let bb = square(&b);
        let e = sub(&aa, &bb);
        let c = add(&x3, &z3);
        let d = sub(&x3, &z3);
        let da = mul(&d, &a);
        let cb = mul(&c, &b);
        x3 = square(&add(&da, &cb));
        z3 = mul(&x1, &square(&sub(&da, &cb)));
        x2 = mul(&aa, &bb);
        z2 = mul(&e, &add(&aa, &mul(&a24, &e)));
    }
    cswap(swap, &mut x2, &mut x3);
    cswap(swap, &mut z2, &mut z3);
    to_bytes(&mul(&x2, &invert(&z2)))
}

pub fn public_key(private: &[u8; 32]) -> [u8; 32] {
    let mut nine = [0u8; 32];
    nine[0] = 9;
    x25519(private, &nine)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    /// RFC 7748 section 5.2's first vector.
    #[test]
    fn the_rfc_scalar_multiplication_vector() {
        let k = hex("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4");
        let u = hex("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c");
        assert_eq!(
            x25519(&k, &u),
            hex("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")
        );
    }

    /// RFC 7748 section 6.1: Alice's and Bob's key pairs and their shared
    /// secret.
    #[test]
    fn the_rfc_key_agreement_vectors() {
        let alice = hex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let bob = hex("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
        let alice_pub = public_key(&alice);
        let bob_pub = public_key(&bob);
        assert_eq!(
            alice_pub,
            hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        );
        assert_eq!(
            bob_pub,
            hex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
        );
        let shared = hex("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
        assert_eq!(x25519(&alice, &bob_pub), shared);
        assert_eq!(x25519(&bob, &alice_pub), shared);
    }
}
