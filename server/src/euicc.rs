//! eUICC (eSIM) local profile management, GSMA SGP.22 ES10c over `AT+CSIM`.
//!
//! The ISD-R is reached on a logical channel opened with a MANAGE CHANNEL APDU
//! sent through `AT+CSIM`. `AT+CCHO` is deliberately not used: it reboots
//! EC200T modules (firmware EC200TCNDAR02A15M16).
//!
//! A card may carry several eUICCs behind different ISD-R AIDs (eSTK.me has
//! SE0 and SE1, the GSMA AID aliasing one of them), so every known AID is tried.
//!
//! Profile download (ES9+/ES10b) is not implemented.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::at::AtClient;
use crate::pdu::{hex_decode, hex_encode};
use crate::status::params;
use crate::tlv::{self, Tlv, encode, find};

/// Known ISD-R applet IDs, vendor-specific first so that an eUICC also
/// reachable through the GSMA AID is reported under its vendor name.
/// Source: https://euicc-manual.osmocom.org/docs/lpa/applet-id-oem
pub const ISD_R_AIDS: &[(&str, &str)] = &[
    ("eSTK.me SE0", "A06573746B6D65FFFF4953442D522030"),
    ("eSTK.me SE1", "A06573746B6D65FFFF4953442D522031"),
    ("eSIM.me", "A0000005591010000000008900000300"),
    ("5ber.eSIM", "A0000005591010FFFFFFFF8900050500"),
    ("Xesim", "A0000005591010FFFFFFFF8900000177"),
    ("LinksField", "A000000559104C696E6B736669656C64"),
    ("GSMA", "A0000005591010FFFFFFFF8900000100"),
];
const CSIM_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EuiccInfo {
    pub eid: Option<String>,
    pub default_smdp: Option<String>,
    pub root_smds: Option<String>,
    pub firmware: Option<String>,
    pub svn: Option<String>,
    /// Free non-volatile memory in bytes
    pub free_nvm: Option<u64>,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub iccid: String,
    pub isdp_aid: Option<String>,
    pub enabled: bool,
    pub nickname: Option<String>,
    pub provider: Option<String>,
    pub name: Option<String>,
    /// test | provisioning | operational
    pub class: Option<String>,
    /// Operator icon as a data: URL
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum ProfileAction {
    Enable,
    Disable,
    Delete,
}

pub struct Channel<'a> {
    c: &'a AtClient,
    ch: u8,
}

/// Class byte for logical channel `ch`; `base` is 0x00 (interindustry) or 0x80 (proprietary).
fn cla(ch: u8, base: u8) -> u8 {
    if ch < 4 { base | ch } else { base | 0x40 | (ch - 4) }
}

async fn csim(c: &AtClient, apdu: &[u8]) -> Result<(Vec<u8>, u16)> {
    let hex = hex_encode(apdu);
    let r = c
        .cmd_timeout(format!("AT+CSIM={},\"{hex}\"", hex.len()), CSIM_TIMEOUT)
        .await?;
    let v = r.get("+CSIM:").context("no +CSIM response")?;
    let mut data = hex_decode(params(v).get(1).context("malformed +CSIM response")?)?;
    if data.is_empty() {
        // The modem answers +CSIM: 0,"" while the SIM is not (re)initialised
        bail!("SIM did not answer the APDU (empty +CSIM response); it may still be initialising");
    }
    if data.len() < 2 {
        bail!("+CSIM response without status word");
    }
    let sw = u16::from_be_bytes([data[data.len() - 2], data[data.len() - 1]]);
    data.truncate(data.len() - 2);
    Ok((data, sw))
}

impl<'a> Channel<'a> {
    /// Open a logical channel. Select an ISD-R with [`Channel::select`] before ES10 calls.
    pub async fn open(c: &'a AtClient) -> Result<Self> {
        let (d, sw) = csim(c, &[0x00, 0x70, 0x00, 0x00, 0x01]).await?;
        if sw != 0x9000 || d.is_empty() {
            bail!("cannot open a logical channel (SW {sw:04X})");
        }
        Ok(Channel { c, ch: d[0] })
    }

    /// Select an application on this channel; `false` if the card does not have it.
    pub async fn select(&self, aid: &str) -> Result<bool> {
        let aid = hex_decode(aid)?;
        let mut apdu = vec![cla(self.ch, 0x00), 0xA4, 0x04, 0x00, aid.len() as u8];
        apdu.extend(aid);
        apdu.push(0x00);
        let (_, sw) = self.transmit(apdu).await?;
        if sw != 0x9000 {
            tracing::debug!("select: SW {sw:04X}");
        }
        Ok(sw == 0x9000)
    }

    pub async fn close(self) {
        if let Err(e) = csim(self.c, &[0x00, 0x70, 0x80, self.ch, 0x00]).await {
            tracing::warn!(ch = self.ch, "closing logical channel failed: {e:#}");
        }
    }

    async fn transmit(&self, mut apdu: Vec<u8>) -> Result<(Vec<u8>, u16)> {
        let (mut data, mut sw) = csim(self.c, &apdu).await?;
        if sw >> 8 == 0x6C {
            // wrong Le: resend with the length the card asked for
            *apdu.last_mut().unwrap() = sw as u8;
            (data, sw) = csim(self.c, &apdu).await?;
        }
        while sw >> 8 == 0x61 {
            // Ask with Le=00 rather than the advertised length: some modem/card
            // combinations (EC200T) answer GET RESPONSE with an exact Le with 6D00.
            let get = |le: u8| [cla(self.ch, 0x00), 0xC0, 0x00, 0x00, le];
            let (mut more, mut next) = csim(self.c, &get(0x00)).await?;
            if next >> 8 == 0x6C {
                (more, next) = csim(self.c, &get(next as u8)).await?;
            }
            data.extend(more);
            sw = next;
        }
        Ok((data, sw))
    }

    /// Send an ES10 request with STORE DATA and return the response TLV.
    async fn es10(&self, req: &[u8]) -> Result<Vec<u8>> {
        let blocks: Vec<&[u8]> = req.chunks(255).collect();
        let mut resp = vec![];
        for (i, block) in blocks.iter().enumerate() {
            let last = i + 1 == blocks.len();
            let mut apdu = vec![
                cla(self.ch, 0x80),
                0xE2,
                if last { 0x91 } else { 0x11 },
                i as u8,
                block.len() as u8,
            ];
            apdu.extend_from_slice(block);
            if last {
                apdu.push(0x00);
            }
            let (d, sw) = self.transmit(apdu).await?;
            if sw != 0x9000 {
                bail!("STORE DATA failed (SW {sw:04X})");
            }
            resp = d;
        }
        Ok(resp)
    }

    pub async fn eid(&self) -> Result<String> {
        let r = self.es10(&encode(0xBF3E, &encode(0x5C, &[0x5A]))).await?;
        let c = tlv::children(&r, 0xBF3E)?;
        Ok(hex_encode(find(&c, 0x5A).context("EID missing")?))
    }

    pub async fn info(&self) -> Result<EuiccInfo> {
        let eid = self.eid().await.ok();
        let (mut default_smdp, mut root_smds) = (None, None);
        if let Ok(r) = self.es10(&encode(0xBF3C, &[])).await
            && let Ok(c) = tlv::children(&r, 0xBF3C)
        {
            default_smdp = find(&c, 0x80).map(utf8).filter(|s| !s.is_empty());
            root_smds = find(&c, 0x81).map(utf8).filter(|s| !s.is_empty());
        }
        let (mut firmware, mut svn, mut free_nvm) = (None, None, None);
        if let Ok(r) = self.es10(&encode(0xBF22, &[])).await
            && let Ok(c) = tlv::children(&r, 0xBF22)
        {
            svn = find(&c, 0x82).map(version);
            firmware = find(&c, 0x83).map(version);
            free_nvm = find(&c, 0x84)
                .and_then(|v| tlv::parse(v).ok())
                .and_then(|r| find(&r, 0x82).map(|b| b.iter().fold(0u64, |a, x| (a << 8) | *x as u64)));
        }
        Ok(EuiccInfo {
            eid,
            default_smdp,
            root_smds,
            firmware,
            svn,
            free_nvm,
            profiles: self.profiles().await?,
        })
    }

    pub async fn profiles(&self) -> Result<Vec<Profile>> {
        let r = self.es10(&encode(0xBF2D, &[])).await?;
        let c = tlv::children(&r, 0xBF2D)?;
        if let Some(err) = find(&c, 0x80) {
            bail!("ProfileInfoList error {}", err.first().copied().unwrap_or(0xFF));
        }
        let Some(list) = find(&c, 0xA0) else {
            return Ok(vec![]);
        };
        tlv::parse(list)?
            .into_iter()
            .filter(|t| t.tag == 0xE3)
            .map(|t| parse_profile(t.value))
            .collect()
    }

    pub async fn profile_action(&self, iccid: &str, action: ProfileAction) -> Result<()> {
        let id = encode(0x5A, &iccid_encode(iccid)?);
        let (tag, req) = match action {
            // refreshFlag FALSE (SGP.22 3.2.1 steps 9-11 / 3.2.2): the eUICC switches
            // immediately and the Device performs the UICC reset itself (Modem::reload_sim).
            // With TRUE the switch is deferred until the eUICC's REFRESH proactive command is
            // answered, which needs CAT on the modem and races our own CFUN reset: the EC200T
            // then stays half-initialised (QINISTAT 1) and every APDU gets an empty +CSIM.
            ProfileAction::Enable => (0xBF31, [encode(0xA0, &id), encode(0x81, &[0x00])].concat()),
            ProfileAction::Disable => (0xBF32, [encode(0xA0, &id), encode(0x81, &[0x00])].concat()),
            ProfileAction::Delete => (0xBF33, id),
        };
        let r = self.es10(&encode(tag, &req)).await?;
        check_result(&r, tag, action)
    }

    pub async fn set_nickname(&self, iccid: &str, nickname: &str) -> Result<()> {
        if nickname.len() > 64 {
            bail!("nickname too long (max 64 bytes)");
        }
        let req = [
            encode(0x5A, &iccid_encode(iccid)?),
            encode(0x90, nickname.as_bytes()),
        ]
        .concat();
        let r = self.es10(&encode(0xBF29, &req)).await?;
        let c = tlv::children(&r, 0xBF29)?;
        match find(&c, 0x80).and_then(|v| v.first()) {
            Some(0) => Ok(()),
            Some(1) => bail!("ICCID not found"),
            other => bail!("set nickname failed ({other:?})"),
        }
    }
}

fn check_result(r: &[u8], tag: u32, action: ProfileAction) -> Result<()> {
    let c = tlv::children(r, tag)?;
    let code = find(&c, 0x80).and_then(|v| v.first().copied()).unwrap_or(0xFF);
    let msg = match (code, action) {
        (0, _) => return Ok(()),
        (1, _) => "ICCID not found",
        (2, ProfileAction::Disable) => "profile is not enabled",
        (2, _) => "profile is not disabled (disable it first)",
        (3, _) => "disallowed by profile policy rules",
        (4, _) => "wrong profile re-enabling",
        (5, _) => "CAT busy, try again",
        _ => "undefined error",
    };
    bail!("{msg} (result {code})")
}

fn parse_profile(b: &[u8]) -> Result<Profile> {
    let t: Vec<Tlv> = tlv::parse(b)?;
    Ok(Profile {
        iccid: find(&t, 0x5A).map(iccid_decode).unwrap_or_default(),
        isdp_aid: find(&t, 0x4F).map(hex_encode),
        enabled: find(&t, 0x9F70).and_then(|v| v.first()) == Some(&1),
        nickname: find(&t, 0x90).map(utf8).filter(|s| !s.is_empty()),
        provider: find(&t, 0x91).map(utf8).filter(|s| !s.is_empty()),
        name: find(&t, 0x92).map(utf8).filter(|s| !s.is_empty()),
        icon: find(&t, 0x94).filter(|v| !v.is_empty()).map(|data| {
            use base64::Engine;
            let mime = match find(&t, 0x93).and_then(|v| v.first()) {
                Some(0) => "image/jpeg",
                _ => "image/png",
            };
            format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(data)
            )
        }),
        class: find(&t, 0x95).and_then(|v| v.first()).map(|c| {
            match c {
                0 => "test",
                1 => "provisioning",
                _ => "operational",
            }
            .to_string()
        }),
    })
}

fn utf8(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn version(b: &[u8]) -> String {
    b.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(".")
}

/// ICCID digits are stored as swapped BCD, padded with F.
pub fn iccid_decode(b: &[u8]) -> String {
    b.iter()
        .flat_map(|x| [x & 0x0F, x >> 4])
        .filter(|n| *n < 10)
        .map(|n| (b'0' + n) as char)
        .collect()
}

pub fn iccid_encode(iccid: &str) -> Result<Vec<u8>> {
    if iccid.is_empty() || iccid.len() > 20 || !iccid.chars().all(|c| c.is_ascii_digit()) {
        bail!("invalid ICCID");
    }
    let mut digits: Vec<u8> = iccid.bytes().map(|c| c - b'0').collect();
    digits.resize(20, 0xF);
    Ok(digits.chunks(2).map(|p| p[0] | (p[1] << 4)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iccid_roundtrip() {
        let enc = iccid_encode("8985203103015273783").unwrap();
        assert_eq!(hex_encode(&enc), "985802133010253787F3");
        assert_eq!(iccid_decode(&enc), "8985203103015273783");
    }

    #[test]
    fn profile_list_parsing() {
        let profile = [
            encode(0x5A, &iccid_encode("89852000000000000012").unwrap()),
            encode(0x9F70, &[1]),
            encode(0x90, "工作卡".as_bytes()),
            encode(0x91, b"Example Mobile"),
            encode(0x95, &[2]),
        ]
        .concat();
        let resp = encode(0xBF2D, &encode(0xA0, &encode(0xE3, &profile)));
        let c = tlv::children(&resp, 0xBF2D).unwrap();
        let list: Vec<Profile> = tlv::parse(find(&c, 0xA0).unwrap())
            .unwrap()
            .into_iter()
            .map(|t| parse_profile(t.value).unwrap())
            .collect();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].iccid, "89852000000000000012");
        assert!(list[0].enabled);
        assert_eq!(list[0].nickname.as_deref(), Some("工作卡"));
        assert_eq!(list[0].class.as_deref(), Some("operational"));
    }
}
