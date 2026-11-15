//! Modem status model and AT response parsers.

use chrono::{DateTime, Local};
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionState {
    pub connected: bool,
    pub transport: Option<String>,
    pub error: Option<String>,
    pub since: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub updated_at: Option<DateTime<Local>>,
    pub device: DeviceInfo,
    pub sim: SimInfo,
    pub network: NetworkInfo,
    pub signal: SignalInfo,
    pub data: DataInfo,
    pub radio: RadioInfo,
    /// Memory receiving new messages
    pub sms_storage: Option<SmsStorage>,
    /// Usage of every SMS memory, from the last sync
    pub sms_storages: Vec<SmsStorage>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub revision: Option<String>,
    pub imei: Option<String>,
    pub usb_net_mode: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimInfo {
    /// Raw `+CPIN` state: READY, SIM PIN, SIM PUK, NOT INSERTED, ...
    pub state: Option<String>,
    pub ready: bool,
    pub iccid: Option<String>,
    pub imsi: Option<String>,
    pub msisdn: Option<String>,
    /// Carrier guessed from IMSI / ICCID
    pub carrier: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInfo {
    pub operator: Option<String>,
    pub plmn: Option<String>,
    pub access_tech: Option<String>,
    pub registration: Option<String>,
    pub ps_registration: Option<String>,
    pub area_code: Option<String>,
    pub cell_id: Option<String>,
    pub band: Option<String>,
    pub channel: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalInfo {
    pub csq: Option<u8>,
    pub rssi: Option<i32>,
    pub ber: Option<u8>,
    pub rsrp: Option<i32>,
    pub rsrq: Option<i32>,
    pub sinr: Option<i32>,
    /// 0-5 bars
    pub level: u8,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataInfo {
    pub attached: Option<bool>,
    /// PDP context active on the module
    pub active: Option<bool>,
    /// Quectel ECM/RNDIS: whether the module routes the PDP context to the host NIC (`AT+QNETDEVCTL`)
    pub netdev: Option<bool>,
    pub cid: u8,
    /// Switched on by the user; kept up automatically when it drops
    pub wanted: bool,
    pub ip: Vec<String>,
    pub apn: Option<String>,
    pub pdp_type: Option<String>,
    pub tx_bytes: Option<u64>,
    pub rx_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadioInfo {
    pub cfun: Option<u8>,
    /// on | airplane | off | unknown
    pub mode: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmsStorage {
    pub memory: String,
    pub used: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdpContext {
    pub cid: u8,
    pub pdp_type: String,
    pub apn: String,
    pub address: Option<String>,
}

/// Split a comma separated AT parameter list, honouring quotes. Quotes are removed.
pub fn params(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

pub fn non_empty(s: Option<&String>) -> Option<String> {
    s.filter(|v| !v.is_empty()).cloned()
}

pub fn reg_state(stat: &str) -> String {
    match stat {
        "0" => "not_registered",
        "1" => "home",
        "2" => "searching",
        "3" => "denied",
        "5" => "roaming",
        "6" | "9" => "home_sms_only",
        "7" | "10" => "roaming_sms_only",
        "8" => "emergency_only",
        _ => "unknown",
    }
    .to_string()
}

pub fn is_registered(state: Option<&str>) -> bool {
    matches!(state, Some(s) if s.starts_with("home") || s.starts_with("roaming"))
}

pub fn access_tech(act: &str) -> Option<String> {
    Some(
        match act {
            "0" => "GSM",
            "1" => "GSM Compact",
            "2" => "UMTS",
            "3" => "EDGE",
            "4" => "HSDPA",
            "5" => "HSUPA",
            "6" => "HSPA",
            "7" => "LTE",
            "8" => "EC-GSM-IoT",
            "9" => "NB-IoT",
            "10" => "LTE (5GC)",
            "11" => "NR (5GC)",
            "12" => "NG-RAN",
            "13" => "EN-DC",
            _ => return None,
        }
        .to_string(),
    )
}

/// Parsed `+CREG`/`+CEREG` query: (stat, area code, cell id, act).
pub fn parse_creg(v: &str) -> (String, Option<String>, Option<String>, Option<String>) {
    let p = params(v);
    // query response is "<n>,<stat>[,<lac>,<ci>[,<act>]]"
    let stat = p.get(1).or(p.first()).cloned().unwrap_or_default();
    (
        reg_state(&stat),
        non_empty(p.get(2)),
        non_empty(p.get(3)),
        p.get(4).and_then(|a| access_tech(a)),
    )
}

pub fn csq_to_dbm(csq: u8) -> Option<i32> {
    (csq <= 31).then(|| -113 + 2 * csq as i32)
}

pub fn signal_level(sig: &SignalInfo) -> u8 {
    if let Some(rsrp) = sig.rsrp {
        return match rsrp {
            r if r >= -85 => 5,
            r if r >= -95 => 4,
            r if r >= -105 => 3,
            r if r >= -115 => 2,
            r if r >= -125 => 1,
            _ => 0,
        };
    }
    match sig.csq {
        Some(c) if c <= 31 => match c {
            c if c >= 25 => 5,
            c if c >= 19 => 4,
            c if c >= 13 => 3,
            c if c >= 7 => 2,
            c if c >= 1 => 1,
            _ => 0,
        },
        _ => 0,
    }
}

/// Carrier name from an IMSI or PLMN prefix (MCC+MNC).
pub fn carrier_from_plmn(plmn: &str) -> Option<String> {
    let name = match plmn.get(..5)? {
        "46000" | "46002" | "46004" | "46007" | "46008" | "46013" => "中国移动",
        "46001" | "46006" | "46009" | "46010" => "中国联通",
        "46003" | "46005" | "46011" | "46012" => "中国电信",
        "46015" => "中国广电",
        "46020" => "中国铁通",
        "45400" | "45402" | "45410" | "45416" | "45418" | "45419" => "csl",
        "45403" | "45404" => "3 香港",
        "45406" | "45415" | "45417" => "SmarTone",
        "45407" => "中国联通(香港)",
        "45412" | "45413" => "中国移动香港",
        "45501" => "澳门电讯 CTM",
        "45507" => "中国电信(澳门)",
        _ => return None,
    };
    Some(name.to_string())
}

/// Carrier name from a Chinese ICCID prefix (8986 + issuer code).
pub fn carrier_from_iccid(iccid: &str) -> Option<String> {
    let name = match iccid.get(..6)? {
        "898600" | "898602" | "898604" | "898607" | "898608" | "898613" => "中国移动",
        "898601" | "898606" | "898609" | "898610" => "中国联通",
        "898603" | "898611" | "898612" => "中国电信",
        "898615" => "中国广电",
        _ => return None,
    };
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_split() {
        assert_eq!(
            params(r#"0,0,"CHINA MOBILE",7"#),
            vec!["0", "0", "CHINA MOBILE", "7"]
        );
        assert_eq!(params(r#""a,b",1"#), vec!["a,b", "1"]);
    }

    #[test]
    fn creg() {
        let (s, lac, ci, act) = parse_creg(r#"2,1,"5A2B","0A1B2C3D",7"#);
        assert_eq!(s, "home");
        assert_eq!(lac.as_deref(), Some("5A2B"));
        assert_eq!(ci.as_deref(), Some("0A1B2C3D"));
        assert_eq!(act.as_deref(), Some("LTE"));
    }
}
