//! Bitcoin CompactSize and unsigned LEB128.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackError(pub &'static str);

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for PackError {}

#[inline]
pub fn compact_size_len(n: u64) -> usize {
    if n < 253 {
        1
    } else if n <= u16::MAX as u64 {
        3
    } else if n <= u32::MAX as u64 {
        5
    } else {
        9
    }
}

pub fn write_compact_size(out: &mut Vec<u8>, n: u64) {
    if n < 253 {
        out.push(n as u8);
    } else if n <= u16::MAX as u64 {
        out.push(253);
        out.extend_from_slice(&(n as u16).to_le_bytes());
    } else if n <= u32::MAX as u64 {
        out.push(254);
        out.extend_from_slice(&(n as u32).to_le_bytes());
    } else {
        out.push(255);
        out.extend_from_slice(&n.to_le_bytes());
    }
}

pub fn read_compact_size(buf: &[u8]) -> Result<(u64, usize), PackError> {
    if buf.is_empty() {
        return Err(PackError("compact size empty"));
    }
    match buf[0] {
        n @ 0..=252 => Ok((u64::from(n), 1)),
        253 => {
            if buf.len() < 3 {
                return Err(PackError("compact size u16 truncated"));
            }
            let v = u16::from_le_bytes([buf[1], buf[2]]);
            Ok((u64::from(v), 3))
        }
        254 => {
            if buf.len() < 5 {
                return Err(PackError("compact size u32 truncated"));
            }
            let v = u32::from_le_bytes(buf[1..5].try_into().unwrap());
            Ok((u64::from(v), 5))
        }
        255 => {
            if buf.len() < 9 {
                return Err(PackError("compact size u64 truncated"));
            }
            let v = u64::from_le_bytes(buf[1..9].try_into().unwrap());
            Ok((v, 9))
        }
    }
}

#[inline]
pub fn uleb128_len(mut n: u64) -> usize {
    let mut len = 1usize;
    while n >= 0x80 {
        n >>= 7;
        len += 1;
    }
    len
}

pub fn write_uleb128_into(dst: &mut [u8], mut n: u64) -> Result<usize, PackError> {
    let mut i = 0usize;
    loop {
        if i >= dst.len() {
            return Err(PackError("uleb128 dest short"));
        }
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 {
            b |= 0x80;
        }
        dst[i] = b;
        i += 1;
        if n == 0 {
            return Ok(i);
        }
    }
}

pub fn write_uleb128(out: &mut Vec<u8>, n: u64) {
    let mut tmp = [0u8; 10];
    let used = write_uleb128_into(&mut tmp, n).expect("10-byte stack holds any u64 uleb128");
    out.extend_from_slice(&tmp[..used]);
}

pub fn read_uleb128(buf: &[u8]) -> Result<(u64, usize), PackError> {
    let mut result = 0u64;
    let mut shift = 0u32;
    for (i, &b) in buf.iter().enumerate() {
        if shift >= 64 {
            return Err(PackError("uleb128 overflow"));
        }
        result |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok((result, i + 1));
        }
        shift += 7;
    }
    Err(PackError("uleb128 truncated"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_size_roundtrip() {
        for n in [0u64, 1, 252, 253, 1000, u32::MAX as u64, u64::MAX] {
            let mut buf = Vec::new();
            write_compact_size(&mut buf, n);
            let (v, used) = read_compact_size(&buf).unwrap();
            assert_eq!(v, n);
            assert_eq!(used, buf.len());
            assert_eq!(compact_size_len(n), buf.len());
        }
        assert_eq!(compact_size_len(252), 1);
        assert_eq!(compact_size_len(253), 3);
        assert_eq!(compact_size_len(u16::MAX as u64), 3);
        assert_eq!(compact_size_len(u16::MAX as u64 + 1), 5);
        assert_eq!(compact_size_len(u32::MAX as u64), 5);
        assert_eq!(compact_size_len(u32::MAX as u64 + 1), 9);
        assert_eq!(uleb128_len(0), 1);
        assert_eq!(uleb128_len(127), 1);
        assert_eq!(uleb128_len(128), 2);
        assert!(uleb128_len(u64::MAX) >= 9);
    }

    #[test]
    fn uleb_roundtrip() {
        for n in [0u64, 1, 127, 128, 255, 300, u32::MAX as u64, u64::MAX >> 1] {
            let mut buf = Vec::new();
            write_uleb128(&mut buf, n);
            let (v, used) = read_uleb128(&buf).unwrap();
            assert_eq!(v, n);
            assert_eq!(used, buf.len());
        }
    }

    #[test]
    fn write_uleb128_into_matches_vec() {
        for n in [0u64, 127, 128, 255, 300, u32::MAX as u64, u64::MAX] {
            let mut vec = Vec::new();
            write_uleb128(&mut vec, n);
            let mut dst = [0u8; 16];
            let used = write_uleb128_into(&mut dst, n).unwrap();
            assert_eq!(used, vec.len());
            assert_eq!(used, uleb128_len(n));
            assert_eq!(&dst[..used], vec.as_slice());
        }
        assert_eq!(
            write_uleb128_into(&mut [0u8; 1], 128).unwrap_err(),
            PackError("uleb128 dest short")
        );
    }

    #[test]
    fn compact_and_uleb_error_paths() {
        assert_eq!(
            read_compact_size(&[]).unwrap_err(),
            PackError("compact size empty")
        );
        assert_eq!(
            read_compact_size(&[253, 1]).unwrap_err(),
            PackError("compact size u16 truncated")
        );
        assert_eq!(
            read_compact_size(&[254, 1, 2, 3]).unwrap_err(),
            PackError("compact size u32 truncated")
        );
        assert_eq!(
            read_compact_size(&[255, 1, 2, 3, 4, 5, 6, 7]).unwrap_err(),
            PackError("compact size u64 truncated")
        );
        assert_eq!(
            read_uleb128(&[0x80]).unwrap_err(),
            PackError("uleb128 truncated")
        );
        let mut over = vec![0x80u8; 10];
        over.push(0x01);
        assert_eq!(
            read_uleb128(&over).unwrap_err(),
            PackError("uleb128 overflow")
        );
        let (v, n) = read_compact_size(&[253, 0, 1]).unwrap();
        assert_eq!((v, n), (256, 3));
        let (v, n) = read_compact_size(&[254, 0, 0, 1, 0]).unwrap();
        assert_eq!((v, n), (1 << 16, 5));
        let mut u64b = vec![255u8];
        u64b.extend_from_slice(&u64::MAX.to_le_bytes());
        let (v, n) = read_compact_size(&u64b).unwrap();
        assert_eq!((v, n), (u64::MAX, 9));
    }

    #[cfg(miri)]
    #[test]
    fn miri_compact_wide_and_uleb_errors() {
        let mut five = Vec::new();
        write_compact_size(&mut five, u16::MAX as u64 + 1);
        assert_eq!(five.len(), 5);
        assert_eq!(read_compact_size(&five).unwrap().0, u16::MAX as u64 + 1);

        let mut nine = Vec::new();
        write_compact_size(&mut nine, u32::MAX as u64 + 1);
        assert_eq!(nine.len(), 9);
        assert_eq!(read_compact_size(&nine).unwrap().0, u32::MAX as u64 + 1);

        assert!(read_compact_size(&[254, 1, 2, 3]).is_err());
        assert!(read_compact_size(&[255, 1, 2, 3, 4, 5, 6, 7]).is_err());
        assert!(read_uleb128(&[0x80]).is_err());
        let mut over = vec![0x80u8; 10];
        over.push(0x01);
        assert!(read_uleb128(&over).is_err());
    }
}
