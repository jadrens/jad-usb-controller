//! A simulated Quectel-like modem speaking AT commands over an in-memory pipe.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::Local;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, WriteHalf};

use super::Transport;
use crate::pdu;

pub fn open() -> Transport {
    let (client, modem) = tokio::io::duplex(16 * 1024);
    tokio::spawn(async move {
        if let Err(e) = Sim::default().run(modem).await {
            tracing::warn!("mock modem stopped: {e}");
        }
    });
    let (r, w) = tokio::io::split(client);
    Transport {
        reader: Box::new(r),
        writer: Box::new(w),
        description: "mock modem".into(),
    }
}

struct Sim {
    echo: bool,
    cfun: u8,
    attached: bool,
    data_active: bool,
    apn: String,
    pdp_type: String,
    /// SMS memories ("SM" SIM card, "ME" module): index -> (stat, pdu)
    storage: BTreeMap<String, BTreeMap<u32, (u8, String)>>,
    /// mem1 (read/delete), mem2 (write), mem3 (receive)
    mems: [String; 3],
    msisdn: Option<String>,
    phonebook: String,
    channel_open: bool,
    /// eUICCs on the card (like eSTK.me SE0/SE1) and the one selected on the logical channel
    ses: Vec<MockSe>,
    selected: Option<usize>,
    tx_bytes: u64,
    rx_bytes: u64,
    concat_ref: u8,
    mr: u8,
}

impl Default for Sim {
    fn default() -> Self {
        let mut sim = Self {
            echo: true,
            cfun: 1,
            attached: true,
            data_active: true,
            apn: "cmiot".into(),
            pdp_type: "IP".into(),
            storage: [
                ("SM".to_string(), BTreeMap::new()),
                ("ME".to_string(), BTreeMap::new()),
            ]
            .into(),
            mems: ["SM".into(), "SM".into(), "SM".into()],
            msisdn: Some("+861440123456789".into()),
            phonebook: "SM".into(),
            channel_open: false,
            ses: vec![
                MockSe {
                    eid: "89049032000001000000305878985177".into(),
                    profiles: vec![MockProfile {
                        iccid: "89440000000000000017".into(),
                        nickname: String::new(),
                        provider: "Example UK".into(),
                        enabled: false,
                    }],
                },
                MockSe {
                    eid: "89049032123451234512345678901235".into(),
                    profiles: vec![
                        MockProfile {
                            iccid: "89860412345678901234".into(),
                            nickname: "物联卡".into(),
                            provider: "China Mobile".into(),
                            enabled: true,
                        },
                        MockProfile {
                            iccid: "89852000263000012345".into(),
                            nickname: String::new(),
                            provider: "Example Roaming".into(),
                            enabled: false,
                        },
                    ],
                },
            ],
            selected: None,
            tx_bytes: 1_234_567,
            rx_bytes: 45_678_901,
            concat_ref: 1,
            mr: 0,
        };
        sim.deliver_to(
            "SM",
            "10086",
            "【流量提醒】尊敬的客户，截至本月28日，您的物联网卡套餐内通用流量已使用 812.4MB，剩余 211.6MB。",
        );
        sim.deliver_to(
            "ME",
            "+8613800138000",
            &"这是一条模拟的长短信，用来演示多段短信在服务端被正确合并。".repeat(4),
        );
        sim
    }
}

const CTRL_Z: u8 = 0x1A;
const ESC: u8 = 0x1B;

impl Sim {
    /// Store an incoming message in the receive memory; returns (memory, indexes).
    fn deliver(&mut self, from: &str, text: &str) -> (String, Vec<u32>) {
        let mem = self.mems[2].clone();
        let idx = self.deliver_to(&mem, from, text);
        (mem, idx)
    }

    fn deliver_to(&mut self, mem: &str, from: &str, text: &str) -> Vec<u32> {
        self.concat_ref = self.concat_ref.wrapping_add(1);
        let pdus = pdu::encode_deliver(from, text, Local::now().fixed_offset(), self.concat_ref);
        let store = self.storage.get_mut(mem).unwrap();
        pdus.into_iter()
            .map(|p| {
                let idx = (0..50).find(|i| !store.contains_key(i)).unwrap_or(49);
                store.insert(idx, (0, p));
                idx
            })
            .collect()
    }

    fn mem1(&mut self) -> &mut BTreeMap<u32, (u8, String)> {
        let m = self.mems[0].clone();
        self.storage.get_mut(&m).unwrap()
    }

    fn cpms(&self) -> String {
        let usage = |m: &String| format!("\"{m}\",{},50", self.storage[m].len());
        format!(
            "+CPMS: {}",
            self.mems.iter().map(usage).collect::<Vec<_>>().join(",")
        )
    }

    async fn run(mut self, modem: DuplexStream) -> anyhow::Result<()> {
        let (mut r, mut w) = tokio::io::split(modem);
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let mut sms_prompt: Option<usize> = None;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut seconds: u64 = 0;
        loop {
            tokio::select! {
                n = r.read(&mut chunk) => {
                    let n = n?;
                    if n == 0 { return Ok(()); }
                    buf.extend_from_slice(&chunk[..n]);
                }
                _ = tick.tick() => {
                    seconds += 1;
                    if self.data_active && self.cfun == 1 {
                        self.tx_bytes += 200 + seconds % 7 * 150;
                        self.rx_bytes += 1500 + seconds % 11 * 900;
                    }
                    if seconds.is_multiple_of(180) && self.cfun == 1 {
                        let (mem, idx) = self.deliver("106575000123", &format!("【模拟】您的验证码是 {:06}，5分钟内有效。", seconds * 7919 % 1_000_000));
                        for i in idx {
                            reply(&mut w, &format!("+CMTI: \"{mem}\",{i}")).await?;
                        }
                    }
                    continue;
                }
            }

            loop {
                if let Some(len) = sms_prompt {
                    let Some(end) = buf.iter().position(|&b| b == CTRL_Z || b == ESC) else {
                        break;
                    };
                    let data: Vec<u8> = buf.drain(..=end).collect();
                    sms_prompt = None;
                    if data[end] == ESC {
                        reply(&mut w, "OK").await?;
                        continue;
                    }
                    let hex = String::from_utf8_lossy(&data[..end]).trim().to_string();
                    self.on_sms_sent(&mut w, &hex, len).await?;
                    continue;
                }
                let Some(end) = buf.iter().position(|&b| b == b'\r') else {
                    break;
                };
                let line: Vec<u8> = buf.drain(..=end).collect();
                let line = String::from_utf8_lossy(&line).trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if self.echo {
                    w.write_all(format!("{line}\r").as_bytes()).await?;
                }
                if let Some(len) = line.strip_prefix("AT+CMGS=").and_then(|v| v.parse().ok()) {
                    sms_prompt = Some(len);
                    w.write_all(b"\r\n> ").await?;
                    continue;
                }
                let resp = self.handle(&line);
                for l in resp {
                    reply(&mut w, &l).await?;
                }
                if let Some(code) = line
                    .strip_prefix("AT+CUSD=1,\"")
                    .and_then(|s| s.split('"').next())
                {
                    let text = format!("您查询的 {code} 业务：当前余额 18.50 元，本月已用流量 812.4MB。");
                    reply(
                        &mut w,
                        &format!("+CUSD: 0,\"{}\",72", pdu::ucs2_hex_encode(&text)),
                    )
                    .await?;
                }
            }
        }
    }

    async fn on_sms_sent(
        &mut self,
        w: &mut WriteHalf<DuplexStream>,
        hex: &str,
        len: usize,
    ) -> anyhow::Result<()> {
        let submit = match pdu::decode(hex) {
            Ok(pdu::Pdu::Submit(s)) if hex.len() == len * 2 + 2 => s,
            _ => return reply(w, "+CMS ERROR: 304").await,
        };
        self.mr = self.mr.wrapping_add(1);
        reply(w, &format!("+CMGS: {}", self.mr)).await?;
        reply(w, "OK").await?;
        // the operator answers queries sent to 10086 once the last part arrives
        let last = submit.concat.is_none_or(|c| c.seq == c.total);
        if submit.recipient.trim_start_matches('+') == "10086" && last {
            let reply_text = format!(
                "您好，您发送的“{}”已收到。本月套餐通用流量 1024MB，已用 812.4MB，剩余 211.6MB。",
                submit.text
            );
            let (mem, idx) = self.deliver("10086", &reply_text);
            for i in idx {
                reply(w, &format!("+CMTI: \"{mem}\",{i}")).await?;
            }
        }
        Ok(())
    }

    fn handle(&mut self, line: &str) -> Vec<String> {
        let up = line.to_ascii_uppercase();
        let ok = || vec!["OK".to_string()];
        let with_ok = |v: Vec<String>| v.into_iter().chain(["OK".to_string()]).collect::<Vec<_>>();
        let registered = self.cfun == 1;
        match up.as_str() {
            "AT" => ok(),
            "ATE0" => {
                self.echo = false;
                ok()
            }
            "ATE1" => {
                self.echo = true;
                ok()
            }
            "ATI" => with_ok(vec![
                "Quectel".into(),
                "EC200T".into(),
                "Revision: EC200TCNDAR02A07M16".into(),
            ]),
            "AT+CGMI" => with_ok(vec!["Quectel".into()]),
            "AT+CGMM" => with_ok(vec!["EC200T".into()]),
            "AT+CGMR" => with_ok(vec!["EC200TCNDAR02A07M16 (mock)".into()]),
            "AT+CGSN" | "AT+GSN" => with_ok(vec!["862012345678901".into()]),
            "AT+CPIN?" => with_ok(vec!["+CPIN: READY".into()]),
            "AT+QCCID" => with_ok(vec!["+QCCID: 89860412345678901234".into()]),
            "AT+CIMI" => with_ok(vec!["460040123456789".into()]),
            "AT+CSQ" if registered => {
                with_ok(vec![format!("+CSQ: {},99", 20 + Local::now().timestamp() % 6)])
            }
            "AT+CSQ" => with_ok(vec!["+CSQ: 99,99".into()]),
            "AT+QCSQ" if registered => with_ok(vec![format!(
                "+QCSQ: \"LTE\",-67,{},{},-9",
                -92 - Local::now().timestamp() % 5,
                110 + Local::now().timestamp() % 20
            )]),
            "AT+QCSQ" => with_ok(vec!["+QCSQ: \"NOSERVICE\"".into()]),
            "AT+COPS?" if registered => with_ok(vec!["+COPS: 0,0,\"CHINA MOBILE\",7".into()]),
            "AT+COPS?" => with_ok(vec!["+COPS: 0".into()]),
            "AT+CREG?" | "AT+CEREG?" | "AT+CGREG?" => {
                let p = &up[2..up.len() - 1];
                let stat = if registered { 1 } else { 0 };
                with_ok(vec![format!("{p}: 2,{stat},\"5A2B\",\"0A1B2C3D\",7")])
            }
            "AT+QNWINFO" if registered => {
                with_ok(vec!["+QNWINFO: \"FDD LTE\",\"46000\",\"LTE BAND 3\",1300".into()])
            }
            "AT+QNWINFO" => with_ok(vec!["+QNWINFO: No Service".into()]),
            "AT+QENG=\"SERVINGCELL\"" if registered => {
                let t = Local::now().timestamp();
                // hand over between two cells every 30 s to exercise the history view
                let (ci, pci, earfcn, band) = if t / 30 % 2 == 0 {
                    ("B9AADB7", 237, 1600, 3)
                } else {
                    ("B9AAD01", 94, 300, 1)
                };
                with_ok(vec![format!(
                    "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,00,{ci},{pci},{earfcn},{band},5,5,5A2B,{},{},-,{},18",
                    -92 - t % 6,
                    -9 - t % 4,
                    8 + t % 9
                )])
            }
            "AT+QENG=\"SERVINGCELL\"" => with_ok(vec!["+QENG: \"servingcell\",\"SEARCH\"".into()]),
            "AT+QENG=\"NEIGHBOURCELL\"" if registered => with_ok(
                [
                    "\"neighbourcell intra\",\"LTE\",1600,389,-105,-17,-,-,24,6,5,4,30",
                    "\"neighbourcell inter\",\"LTE\",3740,23,-102,-17,-,-,21,12,12,5",
                    "\"neighbourcell inter\",\"LTE\",300,94,-99,-10,-,-,24,12,12,6",
                    "\"neighbourcell inter\",\"LTE\",38950,301,-110,-14,-,-,9,12,12,5",
                    "\"neighbourcell inter\",\"LTE\",2452,65535,-140,-20,-,-,0,12,12,5",
                    "\"neighbourcell inter\",\"LTE\",100,65535,-140,-20,-,-,0,12,12,6",
                ]
                .iter()
                .map(|l| format!("+QENG: {l}"))
                .collect(),
            ),
            "AT+QCFG=\"BAND\"" => with_ok(vec!["+QCFG: \"band\",0xd3,0x1e200000095".into()]),
            "AT+QCFG=\"NWSCANMODE\"" => with_ok(vec!["+QCFG: \"nwscanmode\",0".into()]),
            "AT+QCFG=\"USBNET\"" => with_ok(vec!["+QCFG: \"usbnet\",1".into()]),
            "AT+CFUN?" => with_ok(vec![format!("+CFUN: {}", self.cfun)]),
            "AT+CGATT?" => with_ok(vec![format!("+CGATT: {}", (self.attached && registered) as u8)]),
            "AT+CGACT?" => with_ok(vec![format!(
                "+CGACT: 1,{}",
                (self.data_active && registered) as u8
            )]),
            "AT+QNETDEVCTL?" => with_ok(vec![format!(
                "+QNETDEVCTL: 1,1,1,{}",
                (self.data_active && registered) as u8
            )]),
            "AT+CGPADDR=1" | "AT+CGPADDR" => {
                let ip = if self.data_active && registered {
                    "10.143.27.88"
                } else {
                    "0.0.0.0"
                };
                with_ok(vec![format!("+CGPADDR: 1,\"{ip}\"")])
            }
            "AT+CGDCONT?" => with_ok(vec![format!(
                "+CGDCONT: 1,\"{}\",\"{}\",\"0.0.0.0\",0,0",
                self.pdp_type, self.apn
            )]),
            "AT+QGDCNT?" => with_ok(vec![format!("+QGDCNT: {},{}", self.tx_bytes, self.rx_bytes)]),
            "AT+QGDCNT=0" => {
                self.tx_bytes = 0;
                self.rx_bytes = 0;
                ok()
            }
            "AT+CPMS?" => with_ok(vec![self.cpms()]),
            "AT+CPMS=?" => with_ok(vec![
                "+CPMS: (\"SM\",\"ME\"),(\"SM\",\"ME\"),(\"SM\",\"ME\")".into(),
            ]),
            "AT+CNUM" => with_ok(
                self.msisdn
                    .iter()
                    .map(|n| {
                        format!(
                            "+CNUM: \"\",\"{n}\",{}",
                            if n.starts_with('+') { 145 } else { 129 }
                        )
                    })
                    .collect(),
            ),
            "AT+CMGL=4" => {
                let mut out = vec![];
                for (i, (stat, p)) in self.mem1().iter_mut() {
                    out.push(format!("+CMGL: {i},{stat},,{}", p.len() / 2 - 1));
                    out.push(p.clone());
                    *stat = 1;
                }
                with_ok(out)
            }
            "AT+CFUN=1,1" => {
                self.cfun = 1;
                ok()
            }
            _ => self.handle_set(&up, line),
        }
    }

    fn handle_set(&mut self, up: &str, line: &str) -> Vec<String> {
        let ok = vec!["OK".to_string()];
        let err = vec!["ERROR".to_string()];
        let arg = |prefix: &str| up.strip_prefix(prefix).map(str::to_string);
        if let Some(v) = arg("AT+CFUN=") {
            return match v.as_str() {
                "0" | "1" | "4" => {
                    self.cfun = v.parse().unwrap();
                    ok
                }
                _ => err,
            };
        }
        if let Some(v) = arg("AT+CGATT=") {
            self.attached = v == "1";
            if !self.attached {
                self.data_active = false;
            }
            return ok;
        }
        if let Some(v) = arg("AT+CGACT=") {
            self.data_active = v.starts_with('1');
            return ok;
        }
        if let Some(v) = arg("AT+QNETDEVCTL=") {
            self.data_active = v.starts_with('1') || v.starts_with('3');
            return ok;
        }
        if let Some(v) = arg("AT+CPMS=") {
            let mems = crate::status::params(&v);
            if mems.iter().any(|m| !self.storage.contains_key(m)) {
                return vec!["+CMS ERROR: 302".into()];
            }
            for (slot, m) in self.mems.iter_mut().zip(mems) {
                *slot = m;
            }
            let usage: Vec<String> = self
                .mems
                .iter()
                .map(|m| format!("{},50", self.storage[m].len()))
                .collect();
            return vec![format!("+CPMS: {}", usage.join(",")), "OK".into()];
        }
        if let Some(v) = arg("AT+CPBS=") {
            self.phonebook = v.trim_matches('"').to_string();
            return ok;
        }
        if let Some(v) = arg("AT+CPBW=") {
            if self.phonebook != "ON" {
                return err;
            }
            let p = crate::status::params(&v);
            self.msisdn = p.get(1).filter(|n| !n.is_empty()).cloned();
            return ok;
        }
        if arg("AT+CRSM=176,12258,").is_some() {
            // READ BINARY EF_ICCID
            return vec!["+CRSM: 144,0,\"98684031101152816020\"".into(), "OK".into()];
        }
        if let Some(v) = arg("AT+CSIM=") {
            return match self.csim(&v) {
                Some(resp) => vec![format!("+CSIM: {},\"{resp}\"", resp.len()), "OK".into()],
                None => err,
            };
        }
        if let Some(i) = arg("AT+CMGD=").and_then(|v| v.split(',').next()?.parse::<u32>().ok()) {
            self.mem1().remove(&i);
            return ok;
        }
        if let Some(i) = arg("AT+CMGR=").and_then(|v| v.parse::<u32>().ok()) {
            return match self.mem1().get(&i) {
                Some((stat, p)) => vec![
                    format!("+CMGR: {stat},,{}", p.len() / 2 - 1),
                    p.clone(),
                    "OK".into(),
                ],
                None => vec!["+CMS ERROR: 321".into()],
            };
        }
        if up.starts_with("AT+CGDCONT=") {
            let p = crate::status::params(&line["AT+CGDCONT=".len()..]);
            if let (Some(t), Some(apn)) = (p.get(1), p.get(2)) {
                self.pdp_type = t.clone();
                self.apn = apn.clone();
            }
            return ok;
        }
        if up.starts_with("AT+CPIN=") {
            return ok;
        }
        let accepted = [
            "AT+CMEE=",
            "AT+CMGF=",
            "AT+CNMI=",
            "AT+CREG=",
            "AT+CEREG=",
            "AT+CGREG=",
            "AT+COPS=3",
            "AT+CSCS=",
            "AT+QICSGP=",
            "AT+CUSD=",
        ];
        if accepted.iter().any(|p| up.starts_with(p)) {
            ok
        } else {
            err
        }
    }
}

struct MockSe {
    eid: String,
    profiles: Vec<MockProfile>,
}

struct MockProfile {
    iccid: String,
    nickname: String,
    provider: String,
    enabled: bool,
}

impl Sim {
    /// Minimal eUICC: MANAGE CHANNEL, SELECT ISD-R and a few ES10c commands on channel 1.
    fn csim(&mut self, arg: &str) -> Option<String> {
        use crate::euicc::{iccid_decode, iccid_encode};
        use crate::tlv::{self, encode, find};
        let p = crate::status::params(arg);
        let apdu = pdu::hex_decode(p.get(1)?).ok()?;
        let ok = |data: Vec<u8>| format!("{}9000", pdu::hex_encode(&data));
        match apdu.get(..4)? {
            [0x00, 0x70, 0x00, 0x00] => {
                self.channel_open = true;
                return Some("019000".into());
            }
            [0x00, 0x70, 0x80, _] => {
                self.channel_open = false;
                self.selected = None;
                return Some("9000".into());
            }
            [0x01, 0xA4, 0x04, _] if self.channel_open => {
                let aid = pdu::hex_encode(apdu.get(5..5 + *apdu.get(4)? as usize)?);
                // eSTK.me layout: SE0 on its own AID, SE1 also reachable through the GSMA AID
                self.selected = match aid.as_str() {
                    "A06573746B6D65FFFF4953442D522030" => Some(0),
                    "A06573746B6D65FFFF4953442D522031" | "A0000005591010FFFFFFFF8900000100" => Some(1),
                    _ => None,
                };
                return Some(if self.selected.is_some() { "9000" } else { "6A82" }.into());
            }
            [0x81, 0xE2, 0x91, _] if self.channel_open && self.selected.is_some() => {}
            _ => return Some("6D00".into()),
        }
        let se = &mut self.ses[self.selected?];
        let req = apdu.get(5..5 + apdu[4] as usize)?;
        let top = tlv::parse(req).ok()?;
        let t = top.first()?;
        let body = tlv::parse(t.value).unwrap_or_default();
        let result = |tag: u32, code: u8| ok(encode(tag, &encode(0x80, &[code])));
        let target = |b: &[tlv::Tlv]| -> Option<String> {
            let id =
                find(b, 0x5A).or_else(|| find(b, 0xA0).and_then(|a| find(&tlv::parse(a).ok()?, 0x5A)))?;
            Some(iccid_decode(id))
        };
        Some(match t.tag {
            0xBF3E => ok(encode(0xBF3E, &encode(0x5A, &pdu::hex_decode(&se.eid).ok()?))),
            0xBF3C => ok(encode(
                0xBF3C,
                &[encode(0x80, b"rsp.example.com"), encode(0x81, b"lpa.ds.gsma.com")].concat(),
            )),
            0xBF22 => ok(encode(
                0xBF22,
                &[
                    encode(0x82, &[2, 2, 2]),
                    encode(0x83, &[4, 6, 0]),
                    encode(
                        0x84,
                        &[encode(0x81, &[2]), encode(0x82, &[0x05, 0x1C, 0x00])].concat(),
                    ),
                ]
                .concat(),
            )),
            0xBF2D => {
                let items: Vec<u8> = se
                    .profiles
                    .iter()
                    .flat_map(|p| {
                        encode(
                            0xE3,
                            &[
                                encode(0x5A, &iccid_encode(&p.iccid).unwrap()),
                                encode(0x9F70, &[p.enabled as u8]),
                                encode(0x90, p.nickname.as_bytes()),
                                encode(0x91, p.provider.as_bytes()),
                                encode(0x92, p.provider.as_bytes()),
                                encode(0x95, &[2]),
                            ]
                            .concat(),
                        )
                    })
                    .collect();
                ok(encode(0xBF2D, &encode(0xA0, &items)))
            }
            0xBF31 | 0xBF32 => {
                let iccid = target(&body)?;
                let enable = t.tag == 0xBF31;
                let Some(i) = se.profiles.iter().position(|p| p.iccid == iccid) else {
                    return Some(result(t.tag, 1));
                };
                if se.profiles[i].enabled == enable {
                    return Some(result(t.tag, 2));
                }
                for (j, p) in se.profiles.iter_mut().enumerate() {
                    p.enabled = if j == i { enable } else { false };
                }
                result(t.tag, 0)
            }
            0xBF33 => {
                let iccid = target(&body)?;
                match se.profiles.iter().position(|p| p.iccid == iccid) {
                    None => result(t.tag, 1),
                    Some(i) if se.profiles[i].enabled => result(t.tag, 2),
                    Some(i) => {
                        se.profiles.remove(i);
                        result(t.tag, 0)
                    }
                }
            }
            0xBF29 => {
                let iccid = target(&body)?;
                let nick = String::from_utf8_lossy(find(&body, 0x90).unwrap_or_default()).into_owned();
                match se.profiles.iter_mut().find(|p| p.iccid == iccid) {
                    Some(p) => {
                        p.nickname = nick;
                        result(t.tag, 0)
                    }
                    None => result(t.tag, 1),
                }
            }
            _ => "6A88".into(),
        })
    }
}

async fn reply(w: &mut WriteHalf<DuplexStream>, line: &str) -> anyhow::Result<()> {
    w.write_all(format!("\r\n{line}\r\n").as_bytes()).await?;
    Ok(())
}
