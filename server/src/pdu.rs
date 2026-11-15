//! SMS PDU codec (3GPP TS 23.040 / 23.038).
//!
//! Supports decoding SMS-DELIVER / SMS-SUBMIT / SMS-STATUS-REPORT and encoding
//! SMS-SUBMIT (and SMS-DELIVER for the mock modem), with GSM 7-bit / UCS2
//! alphabets and concatenated (long) messages.

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, FixedOffset, TimeZone};

const ESC: u8 = 0x1B;

#[rustfmt::skip]
const GSM7_BASIC: [char; 128] = [
    '@', '£', '$', '¥', 'è', 'é', 'ù', 'ì', 'ò', 'Ç', '\n', 'Ø', 'ø', '\r', 'Å', 'å',
    'Δ', '_', 'Φ', 'Γ', 'Λ', 'Ω', 'Π', 'Ψ', 'Σ', 'Θ', 'Ξ', '\u{1b}', 'Æ', 'æ', 'ß', 'É',
    ' ', '!', '"', '#', '¤', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '/',
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ':', ';', '<', '=', '>', '?',
    '¡', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O',
    'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'Ä', 'Ö', 'Ñ', 'Ü', '§',
    '¿', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o',
    'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'ä', 'ö', 'ñ', 'ü', 'à',
];

const GSM7_EXT: [(u8, char); 10] = [
    (0x0A, '\u{0c}'),
    (0x14, '^'),
    (0x28, '{'),
    (0x29, '}'),
    (0x2F, '\\'),
    (0x3C, '['),
    (0x3D, '~'),
    (0x3E, ']'),
    (0x40, '|'),
    (0x65, '€'),
];

const GSM7_SINGLE: usize = 160;
const GSM7_MULTI: usize = 153;
const UCS2_SINGLE: usize = 70;
const UCS2_MULTI: usize = 67;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coding {
    Gsm7,
    Bit8,
    Ucs2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Concat {
    pub reference: u16,
    pub total: u8,
    pub seq: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pdu {
    Deliver(Deliver),
    Submit(Submit),
    StatusReport(StatusReport),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Deliver {
    pub smsc: Option<String>,
    pub sender: String,
    pub timestamp: DateTime<FixedOffset>,
    pub coding: Coding,
    pub text: String,
    pub concat: Option<Concat>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Submit {
    pub recipient: String,
    pub coding: Coding,
    pub text: String,
    pub concat: Option<Concat>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusReport {
    pub message_ref: u8,
    pub recipient: String,
    pub status: u8,
}

/// One encoded PDU ready for `AT+CMGS=<tpdu_len>`.
#[derive(Debug, Clone)]
pub struct OutPdu {
    pub hex: String,
    /// Length in octets of the TPDU (excluding the SMSC part).
    pub tpdu_len: usize,
}

// ---------------------------------------------------------------- helpers

pub fn hex_decode(s: &str) -> Result<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        bail!("odd hex length");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| anyhow!("bad hex: {e}")))
        .collect()
}

pub fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn gsm7_char(septet: u8, escaped: bool) -> char {
    if escaped {
        GSM7_EXT
            .iter()
            .find(|(c, _)| *c == septet)
            .map(|(_, ch)| *ch)
            .unwrap_or(' ')
    } else {
        GSM7_BASIC[(septet & 0x7F) as usize]
    }
}

/// Encode text to GSM 7-bit septets. Returns `None` if a char is not representable.
pub fn gsm7_encode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        out.extend(gsm7_char_septets(ch)?);
    }
    Some(out)
}

fn gsm7_char_septets(ch: char) -> Option<Vec<u8>> {
    if ch != '\u{1b}'
        && let Some(i) = GSM7_BASIC.iter().position(|c| *c == ch)
    {
        return Some(vec![i as u8]);
    }
    GSM7_EXT
        .iter()
        .find(|(_, c)| *c == ch)
        .map(|(code, _)| vec![ESC, *code])
}

pub fn gsm7_decode(septets: &[u8]) -> String {
    let mut out = String::with_capacity(septets.len());
    let mut escaped = false;
    for &s in septets {
        if s == ESC && !escaped {
            escaped = true;
            continue;
        }
        out.push(gsm7_char(s, escaped));
        escaped = false;
    }
    out
}

/// Pack septets into octets, starting after `fill_bits` zero bits.
fn pack7(septets: &[u8], fill_bits: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut nbits = fill_bits;
    for &s in septets {
        acc |= ((s & 0x7F) as u32) << nbits;
        nbits += 7;
        while nbits >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            nbits -= 8;
        }
    }
    if nbits > 0 {
        out.push(acc as u8);
    }
    out
}

/// Unpack `count` septets starting at bit offset `start_bit`.
fn unpack7(bytes: &[u8], start_bit: usize, count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let bit = start_bit + i * 7;
        let idx = bit / 8;
        let shift = bit % 8;
        let Some(&lo) = bytes.get(idx) else { break };
        let mut v = (lo as u16) >> shift;
        if shift > 1 {
            v |= (*bytes.get(idx + 1).unwrap_or(&0) as u16) << (8 - shift);
        }
        out.push((v & 0x7F) as u8);
    }
    out
}

fn decode_bcd_digit(n: u8) -> Option<char> {
    match n {
        0..=9 => Some((b'0' + n) as char),
        0xA => Some('*'),
        0xB => Some('#'),
        0xC => Some('a'),
        0xD => Some('b'),
        0xE => Some('c'),
        _ => None,
    }
}

/// Decode an address field whose first byte is the digit count.
/// Returns (address, bytes consumed).
fn decode_address(b: &[u8]) -> Result<(String, usize)> {
    let digits = *b.first().context("address truncated")? as usize;
    let toa = *b.get(1).context("address truncated")?;
    let nbytes = digits.div_ceil(2);
    let body = b.get(2..2 + nbytes).context("address truncated")?;
    let addr = if toa & 0x70 == 0x50 {
        // alphanumeric, GSM 7-bit packed
        gsm7_decode(&unpack7(body, 0, digits * 4 / 7))
    } else {
        let mut s = String::new();
        if toa & 0x70 == 0x10 {
            s.push('+');
        }
        for &x in body {
            for n in [x & 0x0F, x >> 4] {
                if let Some(c) = decode_bcd_digit(n) {
                    s.push(c);
                }
            }
        }
        s
    };
    Ok((addr, 2 + nbytes))
}

/// Decode the SMSC field (first byte = octet length incl. TOA).
fn decode_smsc(b: &[u8]) -> Result<(Option<String>, usize)> {
    let len = *b.first().context("empty pdu")? as usize;
    if len == 0 {
        return Ok((None, 1));
    }
    let toa = *b.get(1).context("smsc truncated")?;
    let body = b.get(2..1 + len).context("smsc truncated")?;
    let mut s = String::new();
    if toa & 0x70 == 0x10 {
        s.push('+');
    }
    for &x in body {
        for n in [x & 0x0F, x >> 4] {
            if let Some(c) = decode_bcd_digit(n) {
                s.push(c);
            }
        }
    }
    Ok((Some(s), 1 + len))
}

fn encode_address(addr: &str) -> Vec<u8> {
    let international = addr.starts_with('+');
    let digits: Vec<u8> = addr
        .chars()
        .filter_map(|c| match c {
            '0'..='9' => Some(c as u8 - b'0'),
            '*' => Some(0xA),
            '#' => Some(0xB),
            _ => None,
        })
        .collect();
    let mut out = vec![digits.len() as u8, if international { 0x91 } else { 0x81 }];
    for pair in digits.chunks(2) {
        let lo = pair[0];
        let hi = pair.get(1).copied().unwrap_or(0xF);
        out.push(lo | (hi << 4));
    }
    out
}

fn swap_bcd(b: u8) -> u32 {
    ((b & 0x0F) * 10 + (b >> 4)) as u32
}

fn decode_scts(b: &[u8]) -> Result<DateTime<FixedOffset>> {
    if b.len() < 7 {
        bail!("scts truncated");
    }
    let tz_raw = b[6];
    let quarters = ((tz_raw & 0x07) * 10 + (tz_raw >> 4)) as i32;
    let offset = if tz_raw & 0x08 != 0 { -quarters } else { quarters } * 15 * 60;
    let tz = FixedOffset::east_opt(offset).unwrap_or(FixedOffset::east_opt(0).unwrap());
    tz.with_ymd_and_hms(
        2000 + swap_bcd(b[0]) as i32,
        swap_bcd(b[1]),
        swap_bcd(b[2]),
        swap_bcd(b[3]),
        swap_bcd(b[4]),
        swap_bcd(b[5]),
    )
    .single()
    .context("invalid scts")
}

fn encode_scts(ts: &DateTime<FixedOffset>) -> Vec<u8> {
    use chrono::{Datelike, Timelike};
    let bcd = |v: u32| -> u8 { ((v % 10) << 4) as u8 | (v / 10) as u8 };
    let q = ts.offset().local_minus_utc() / 60 / 15;
    let mut tz = bcd(q.unsigned_abs());
    if q < 0 {
        tz |= 0x08;
    }
    vec![
        bcd(ts.year() as u32 % 100),
        bcd(ts.month()),
        bcd(ts.day()),
        bcd(ts.hour()),
        bcd(ts.minute()),
        bcd(ts.second()),
        tz,
    ]
}

fn coding_from_dcs(dcs: u8) -> Coding {
    match dcs & 0xF0 {
        0x00..=0x30 | 0x40..=0x70 => match (dcs >> 2) & 0x03 {
            1 => Coding::Bit8,
            2 => Coding::Ucs2,
            _ => Coding::Gsm7,
        },
        0xE0 => Coding::Ucs2,
        0xF0 => {
            if dcs & 0x04 != 0 {
                Coding::Bit8
            } else {
                Coding::Gsm7
            }
        }
        _ => Coding::Gsm7,
    }
}

fn parse_udh(udh: &[u8]) -> Option<Concat> {
    let mut i = 0;
    while i + 1 < udh.len() {
        let iei = udh[i];
        let len = udh[i + 1] as usize;
        let data = udh.get(i + 2..i + 2 + len)?;
        match (iei, len) {
            (0x00, 3) => {
                return Some(Concat {
                    reference: data[0] as u16,
                    total: data[1],
                    seq: data[2],
                });
            }
            (0x08, 4) => {
                return Some(Concat {
                    reference: u16::from_be_bytes([data[0], data[1]]),
                    total: data[2],
                    seq: data[3],
                });
            }
            _ => {}
        }
        i += 2 + len;
    }
    None
}

/// Decode user data. `ud` starts right after the UDL byte.
fn decode_ud(ud: &[u8], udl: usize, coding: Coding, has_udh: bool) -> (String, Option<Concat>) {
    let (udh_len, concat) = if has_udh && !ud.is_empty() {
        let l = ud[0] as usize;
        (l + 1, ud.get(1..1 + l).and_then(parse_udh))
    } else {
        (0, None)
    };
    let text = match coding {
        Coding::Gsm7 => {
            let header_bits = udh_len * 8;
            let header_septets = header_bits.div_ceil(7);
            let count = udl.saturating_sub(header_septets);
            gsm7_decode(&unpack7(ud, header_septets * 7, count))
        }
        Coding::Ucs2 => {
            let body = ud.get(udh_len..udl.min(ud.len())).unwrap_or(&[]);
            let units: Vec<u16> = body
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        Coding::Bit8 => {
            let body = ud.get(udh_len..udl.min(ud.len())).unwrap_or(&[]);
            String::from_utf8_lossy(body).into_owned()
        }
    };
    (text, concat)
}

// ---------------------------------------------------------------- decode

pub fn decode(hex: &str) -> Result<Pdu> {
    let b = hex_decode(hex)?;
    let (smsc, mut i) = decode_smsc(&b)?;
    let fo = *b.get(i).context("missing first octet")?;
    i += 1;
    let has_udh = fo & 0x40 != 0;
    match fo & 0x03 {
        0 => {
            let (sender, n) = decode_address(&b[i..])?;
            i += n;
            let _pid = *b.get(i).context("truncated")?;
            let dcs = *b.get(i + 1).context("truncated")?;
            i += 2;
            let timestamp = decode_scts(b.get(i..i + 7).context("truncated")?)?;
            i += 7;
            let udl = *b.get(i).context("truncated")? as usize;
            i += 1;
            let coding = coding_from_dcs(dcs);
            let (text, concat) = decode_ud(&b[i..], udl, coding, has_udh);
            Ok(Pdu::Deliver(Deliver {
                smsc,
                sender,
                timestamp,
                coding,
                text,
                concat,
            }))
        }
        1 => {
            let _mr = *b.get(i).context("truncated")?;
            i += 1;
            let (recipient, n) = decode_address(&b[i..])?;
            i += n;
            let dcs = *b.get(i + 1).context("truncated")?;
            i += 2;
            i += match (fo >> 3) & 0x03 {
                0 => 0,
                2 => 1,
                _ => 7,
            };
            let udl = *b.get(i).context("truncated")? as usize;
            i += 1;
            let coding = coding_from_dcs(dcs);
            let (text, concat) = decode_ud(&b[i..], udl, coding, has_udh);
            Ok(Pdu::Submit(Submit {
                recipient,
                coding,
                text,
                concat,
            }))
        }
        2 => {
            let message_ref = *b.get(i).context("truncated")?;
            i += 1;
            let (recipient, n) = decode_address(&b[i..])?;
            i += n + 14; // SCTS + DT
            let status = *b.get(i).unwrap_or(&0xFF);
            Ok(Pdu::StatusReport(StatusReport {
                message_ref,
                recipient,
                status,
            }))
        }
        _ => bail!("unsupported TP-MTI"),
    }
}

// ---------------------------------------------------------------- encode

/// User data of one segment, before the UDH is attached.
enum Segment {
    Gsm7(Vec<u8>),
    Ucs2(Vec<u16>),
}

fn split_text(text: &str) -> Vec<Segment> {
    if let Some(septets) = gsm7_encode(text) {
        if septets.len() <= GSM7_SINGLE {
            return vec![Segment::Gsm7(septets)];
        }
        let mut parts = vec![];
        let mut cur: Vec<u8> = vec![];
        for ch in text.chars() {
            let s = gsm7_char_septets(ch).unwrap_or_default();
            if cur.len() + s.len() > GSM7_MULTI {
                parts.push(Segment::Gsm7(std::mem::take(&mut cur)));
            }
            cur.extend(s);
        }
        if !cur.is_empty() {
            parts.push(Segment::Gsm7(cur));
        }
        return parts;
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() <= UCS2_SINGLE {
        return vec![Segment::Ucs2(units)];
    }
    let mut parts = vec![];
    let mut cur: Vec<u16> = vec![];
    let mut buf = [0u16; 2];
    for ch in text.chars() {
        let enc = ch.encode_utf16(&mut buf);
        if cur.len() + enc.len() > UCS2_MULTI {
            parts.push(Segment::Ucs2(std::mem::take(&mut cur)));
        }
        cur.extend_from_slice(enc);
    }
    if !cur.is_empty() {
        parts.push(Segment::Ucs2(cur));
    }
    parts
}

/// Returns (dcs, udl, ud).
fn build_ud(seg: &Segment, concat: Option<Concat>) -> (u8, u8, Vec<u8>) {
    let udh: Vec<u8> = match concat {
        Some(c) => vec![0x05, 0x00, 0x03, c.reference as u8, c.total, c.seq],
        None => vec![],
    };
    match seg {
        Segment::Gsm7(septets) => {
            let header_septets = (udh.len() * 8).div_ceil(7);
            let fill = header_septets * 7 - udh.len() * 8;
            let mut ud = udh.clone();
            ud.extend(pack7(septets, fill));
            (0x00, (header_septets + septets.len()) as u8, ud)
        }
        Segment::Ucs2(units) => {
            let mut ud = udh.clone();
            for u in units {
                ud.extend(u.to_be_bytes());
            }
            (0x08, ud.len() as u8, ud)
        }
    }
}

/// Number of PDUs `text` would be split into.
#[cfg(test)]
pub fn segment_count(text: &str) -> usize {
    split_text(text).len()
}

/// Encode an SMS-SUBMIT (possibly multiple parts). SMSC is left to the modem default.
pub fn encode_submit(to: &str, text: &str, reference: u8, status_report: bool) -> Vec<OutPdu> {
    let segs = split_text(text);
    let total = segs.len() as u8;
    segs.iter()
        .enumerate()
        .map(|(i, seg)| {
            let concat = (total > 1).then_some(Concat {
                reference: reference as u16,
                total,
                seq: i as u8 + 1,
            });
            // SUBMIT, relative validity period
            let mut fo = 0x11u8;
            if concat.is_some() {
                fo |= 0x40;
            }
            if status_report {
                fo |= 0x20;
            }
            let (dcs, udl, ud) = build_ud(seg, concat);
            let mut tpdu = vec![fo, 0x00];
            tpdu.extend(encode_address(to));
            tpdu.extend([0x00, dcs, 0xAA, udl]);
            tpdu.extend(ud);
            let mut full = vec![0x00];
            full.extend(&tpdu);
            OutPdu {
                hex: hex_encode(&full),
                tpdu_len: tpdu.len(),
            }
        })
        .collect()
}

/// Encode an SMS-DELIVER as a network would. Used by the mock modem and tests.
pub fn encode_deliver(from: &str, text: &str, ts: DateTime<FixedOffset>, reference: u8) -> Vec<String> {
    let segs = split_text(text);
    let total = segs.len() as u8;
    segs.iter()
        .enumerate()
        .map(|(i, seg)| {
            let concat = (total > 1).then_some(Concat {
                reference: reference as u16,
                total,
                seq: i as u8 + 1,
            });
            let fo = if concat.is_some() { 0x44 } else { 0x04 };
            let (dcs, udl, ud) = build_ud(seg, concat);
            let mut b = vec![0x00, fo];
            b.extend(encode_address(from));
            b.extend([0x00, dcs]);
            b.extend(encode_scts(&ts));
            b.push(udl);
            b.extend(ud);
            hex_encode(&b)
        })
        .collect()
}

/// Decode a UCS2 hex string as returned by e.g. `+CUSD` with dcs=72.
pub fn ucs2_hex_decode(s: &str) -> Option<String> {
    let b = hex_decode(s).ok()?;
    if b.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    Some(String::from_utf16_lossy(&units))
}

pub fn ucs2_hex_encode(s: &str) -> String {
    s.encode_utf16().map(|u| format!("{u:04X}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_classic_deliver() {
        let pdu = "07911326040000F0040B911346610089F60000208062917314080CC8F71D14969741F977FD07";
        let Pdu::Deliver(d) = decode(pdu).unwrap() else {
            panic!()
        };
        assert_eq!(d.smsc.as_deref(), Some("+31624000000"));
        assert_eq!(d.sender, "+31641600986");
        assert_eq!(d.text, "How are you?");
        assert_eq!(
            d.timestamp.format("%y-%m-%d %H:%M:%S").to_string(),
            "02-08-26 19:37:41"
        );
    }

    #[test]
    fn roundtrip_ucs2_long() {
        let text = "您好，这是一条很长的中文测试短信，用来验证长短信的拆分与合并是否正确。".repeat(3);
        let ts = FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 29, 12, 30, 0)
            .unwrap();
        let pdus = encode_deliver("+8613800138000", &text, ts, 42);
        assert!(pdus.len() > 1);
        let mut joined = String::new();
        for (i, p) in pdus.iter().enumerate() {
            let Pdu::Deliver(d) = decode(p).unwrap() else {
                panic!()
            };
            assert_eq!(d.sender, "+8613800138000");
            assert_eq!(d.timestamp, ts);
            let c = d.concat.unwrap();
            assert_eq!(
                (c.reference, c.total as usize, c.seq as usize),
                (42, pdus.len(), i + 1)
            );
            joined.push_str(&d.text);
        }
        assert_eq!(joined, text);
    }

    #[test]
    fn roundtrip_gsm7_long_with_escapes() {
        let text = "Hello [world] {test} ~€ ".repeat(10);
        let ts = FixedOffset::east_opt(-5 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
            .unwrap();
        let pdus = encode_deliver("10086", &text, ts, 7);
        assert_eq!(pdus.len(), 2);
        let joined: String = pdus
            .iter()
            .map(|p| match decode(p).unwrap() {
                Pdu::Deliver(d) => d.text,
                _ => panic!(),
            })
            .collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn submit_roundtrip() {
        let out = encode_submit("+8613800138000", "测试 test", 1, false);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tpdu_len * 2 + 2, out[0].hex.len());
        let Pdu::Submit(s) = decode(&out[0].hex).unwrap() else {
            panic!()
        };
        assert_eq!(s.recipient, "+8613800138000");
        assert_eq!(s.text, "测试 test");
    }

    #[test]
    fn gsm7_single_limit() {
        assert_eq!(segment_count(&"a".repeat(160)), 1);
        assert_eq!(segment_count(&"a".repeat(161)), 2);
        assert_eq!(segment_count(&"中".repeat(70)), 1);
        assert_eq!(segment_count(&"中".repeat(71)), 2);
    }
}
