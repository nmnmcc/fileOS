// SPDX-License-Identifier: AGPL-3.0-only

//! Shamir secret sharing over GF(256), for social recovery of the personal
//! key: split it among trusted friends so that any `threshold` of them can
//! restore it, and fewer learn nothing.

fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        let carry = a & 0x80;
        a <<= 1;
        if carry != 0 {
            a ^= 0x1b; // x^8 + x^4 + x^3 + x + 1
        }
        b >>= 1;
    }
    p
}

fn inv(a: u8) -> u8 {
    // a^254 = a^-1 in GF(256).
    let mut result = 1u8;
    let mut base = a;
    let mut e = 254u8;
    while e > 0 {
        if e & 1 != 0 {
            result = mul(result, base);
        }
        base = mul(base, base);
        e >>= 1;
    }
    result
}

/// A share: its x coordinate (1..=255) and one y byte per secret byte.
#[derive(Clone, PartialEq, Eq)]
pub struct Share {
    /// The x coordinate.
    pub x: u8,
    /// The y values.
    pub y: Vec<u8>,
}

impl std::fmt::Debug for Share {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Share")
            .field("x", &self.x)
            .finish_non_exhaustive()
    }
}

/// Split `secret` into `count` shares, any `threshold` of which restore it.
/// `random` must hold `(threshold - 1) * secret.len()` secure random bytes.
pub fn split(secret: &[u8], threshold: u8, count: u8, random: &[u8]) -> Option<Vec<Share>> {
    if threshold == 0 || threshold > count || random.len() < (threshold as usize - 1) * secret.len()
    {
        return None;
    }
    let mut shares: Vec<Share> = (1..=count)
        .map(|x| Share {
            x,
            y: Vec::with_capacity(secret.len()),
        })
        .collect();
    for (i, &s) in secret.iter().enumerate() {
        let coeffs: Vec<u8> = std::iter::once(s)
            .chain((0..threshold as usize - 1).map(|j| random[i * (threshold as usize - 1) + j]))
            .collect();
        for share in &mut shares {
            // Horner's rule.
            let y = coeffs
                .iter()
                .rev()
                .fold(0u8, |acc, &c| mul(acc, share.x) ^ c);
            share.y.push(y);
        }
    }
    Some(shares)
}

/// Restore the secret from at least `threshold` distinct shares.
pub fn combine(shares: &[Share]) -> Option<Vec<u8>> {
    let len = shares.first()?.y.len();
    if shares.iter().any(|s| s.y.len() != len || s.x == 0) {
        return None;
    }
    for (i, a) in shares.iter().enumerate() {
        if shares[..i].iter().any(|b| b.x == a.x) {
            return None;
        }
    }
    let mut secret = vec![0u8; len];
    for (i, si) in shares.iter().enumerate() {
        // Lagrange basis at x = 0.
        let mut basis = 1u8;
        for (j, sj) in shares.iter().enumerate() {
            if i != j {
                basis = mul(basis, mul(sj.x, inv(sj.x ^ si.x)));
            }
        }
        for (k, out) in secret.iter_mut().enumerate() {
            *out ^= mul(si.y[k], basis);
        }
    }
    Some(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_share_bytes() {
        let share = Share {
            x: 7,
            y: vec![179, 178, 177],
        };
        let rendered = format!("{share:?}");
        assert!(rendered.contains("x: 7"));
        assert!(!rendered.contains("179"));
    }

    #[test]
    fn any_three_of_five() {
        let secret = b"a personal key seed of 32 bytes!";
        let random: Vec<u8> = (0..64u32).map(|i| (i * 37 + 11) as u8).collect();
        let shares = split(secret, 3, 5, &random).unwrap();
        assert_eq!(combine(&shares[0..3]).unwrap(), secret);
        assert_eq!(
            combine(&[shares[4].clone(), shares[1].clone(), shares[2].clone()]).unwrap(),
            secret
        );
        assert_ne!(combine(&shares[0..2]).unwrap(), secret);
    }
}
