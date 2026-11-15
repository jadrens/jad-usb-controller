//! Minimal BER-TLV codec for eUICC (SGP.22) messages.

use anyhow::{Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u32,
    pub value: &'a [u8],
}

/// Parse a sequence of TLVs.
pub fn parse(mut b: &[u8]) -> Result<Vec<Tlv<'_>>> {
    let mut out = vec![];
    while !b.is_empty() {
        let (tlv, rest) = parse_one(b)?;
        out.push(tlv);
        b = rest;
    }
    Ok(out)
}

fn parse_one(b: &[u8]) -> Result<(Tlv<'_>, &[u8])> {
    let mut i = 0;
    let first = *b.first().ok_or_else(|| anyhow::anyhow!("empty TLV"))?;
    let mut tag = first as u32;
    i += 1;
    if first & 0x1F == 0x1F {
        loop {
            let Some(&x) = b.get(i) else {
                bail!("truncated tag")
            };
            tag = (tag << 8) | x as u32;
            i += 1;
            if x & 0x80 == 0 {
                break;
            }
        }
    }
    let Some(&l) = b.get(i) else {
        bail!("truncated length")
    };
    i += 1;
    let len = if l & 0x80 == 0 {
        l as usize
    } else {
        let n = (l & 0x7F) as usize;
        if n == 0 || n > 3 {
            bail!("unsupported length form");
        }
        let Some(bytes) = b.get(i..i + n) else {
            bail!("truncated length")
        };
        i += n;
        bytes.iter().fold(0usize, |acc, x| (acc << 8) | *x as usize)
    };
    let Some(value) = b.get(i..i + len) else {
        bail!("TLV value truncated (tag {tag:X})")
    };
    Ok((Tlv { tag, value }, &b[i + len..]))
}

pub fn find<'a>(tlvs: &[Tlv<'a>], tag: u32) -> Option<&'a [u8]> {
    tlvs.iter().find(|t| t.tag == tag).map(|t| t.value)
}

/// Parse `b` and return the children of its first TLV with `tag`.
pub fn children(b: &[u8], tag: u32) -> Result<Vec<Tlv<'_>>> {
    let top = parse(b)?;
    match find(&top, tag) {
        Some(v) => parse(v),
        None => bail!("expected tag {tag:X}"),
    }
}

pub fn encode(tag: u32, value: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = tag.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
    if out.is_empty() {
        out.push(0);
    }
    let len = value.len();
    match len {
        0..=0x7F => out.push(len as u8),
        0x80..=0xFF => out.extend([0x81, len as u8]),
        _ => out.extend([0x82, (len >> 8) as u8, len as u8]),
    }
    out.extend_from_slice(value);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let inner = encode(0x5A, &[0x98, 0x10]);
        let outer = encode(0xBF2D, &encode(0xA0, &encode(0xE3, &inner)));
        assert_eq!(&outer[..2], &[0xBF, 0x2D]);
        let a0 = children(&outer, 0xBF2D).unwrap();
        let e3 = parse(find(&a0, 0xA0).unwrap()).unwrap();
        let p = parse(e3[0].value).unwrap();
        assert_eq!(find(&p, 0x5A), Some(&[0x98, 0x10][..]));
        let long = encode(0x9F70, &[1u8; 300]);
        assert_eq!(&long[..5], &[0x9F, 0x70, 0x82, 0x01, 0x2C]);
        assert_eq!(parse(&long).unwrap()[0].value.len(), 300);
    }
}
