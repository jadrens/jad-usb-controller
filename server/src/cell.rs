//! Serving / neighbour cell information (Quectel `AT+QENG`) and LTE band math.

use serde::Serialize;

use crate::status::params;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellInfo {
    pub serving: Option<ServingCell>,
    /// Measured neighbour cells, strongest first
    pub neighbours: Vec<NeighbourCell>,
    /// Neighbour frequencies the network configured but the modem has not measured
    pub configured: Vec<Carrier>,
    /// Neighbour cells of other RATs (GSM/WCDMA) that reported a measurement
    pub other_rat: Vec<String>,
    /// LTE bands the modem is allowed to use (`AT+QCFG="band"`)
    pub lte_bands: Vec<u16>,
    /// `AT+QCFG="nwscanmode"`: 0 auto, 1 GSM only, 3 LTE only, ...
    pub scan_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServingCell {
    /// SEARCH | LIMSRV | NOCONN | CONNECT
    pub state: String,
    pub rat: String,
    pub duplex: Option<String>,
    pub mcc: Option<String>,
    pub mnc: Option<String>,
    /// Hex E-UTRAN cell identity
    pub cell_id: Option<String>,
    pub enb_id: Option<u32>,
    pub sector: Option<u32>,
    pub pci: Option<u32>,
    pub tac: Option<String>,
    pub carrier: Option<Carrier>,
    pub ul_bandwidth_mhz: Option<f32>,
    pub dl_bandwidth_mhz: Option<f32>,
    pub rsrp: Option<i32>,
    pub rsrq: Option<i32>,
    pub rssi: Option<i32>,
    pub sinr: Option<i32>,
    pub srxlev: Option<i32>,
    /// Raw fields for RATs that are not decoded
    pub raw: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighbourCell {
    /// intra (same frequency) | inter
    pub kind: String,
    pub carrier: Carrier,
    pub pci: u32,
    pub rsrp: Option<i32>,
    pub rsrq: Option<i32>,
    pub sinr: Option<i32>,
    pub srxlev: Option<i32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Carrier {
    pub earfcn: u32,
    pub band: Option<u16>,
    pub duplex: Option<&'static str>,
    pub dl_mhz: Option<f64>,
    pub ul_mhz: Option<f64>,
}

/// (band, F_DL_low, N_Offs_DL, N_DL_high, F_UL_low or None for TDD/SDL) — 3GPP TS 36.101 table 5.7.3-1
#[rustfmt::skip]
const LTE_BANDS: &[(u16, f64, u32, u32, Option<f64>)] = &[
    (1, 2110.0, 0, 599, Some(1920.0)),
    (2, 1930.0, 600, 1199, Some(1850.0)),
    (3, 1805.0, 1200, 1949, Some(1710.0)),
    (4, 2110.0, 1950, 2399, Some(1710.0)),
    (5, 869.0, 2400, 2649, Some(824.0)),
    (7, 2620.0, 2750, 3449, Some(2500.0)),
    (8, 925.0, 3450, 3799, Some(880.0)),
    (12, 729.0, 5010, 5179, Some(699.0)),
    (13, 746.0, 5180, 5279, Some(777.0)),
    (17, 734.0, 5730, 5849, Some(704.0)),
    (18, 860.0, 5850, 5999, Some(815.0)),
    (19, 875.0, 6000, 6149, Some(830.0)),
    (20, 791.0, 6150, 6449, Some(832.0)),
    (25, 1930.0, 8040, 8689, Some(1850.0)),
    (26, 859.0, 8690, 9039, Some(814.0)),
    (28, 758.0, 9210, 9659, Some(703.0)),
    (32, 1452.0, 9920, 10359, None),
    (34, 2010.0, 36200, 36349, None),
    (38, 2570.0, 37750, 38249, None),
    (39, 1880.0, 38250, 38649, None),
    (40, 2300.0, 38650, 39649, None),
    (41, 2496.0, 39650, 41589, None),
    (42, 3400.0, 41590, 43589, None),
    (43, 3600.0, 43590, 45589, None),
    (46, 5150.0, 46790, 54539, None),
    (48, 3550.0, 55240, 56739, None),
    (66, 2110.0, 66436, 67335, Some(1710.0)),
    (71, 617.0, 68586, 68935, Some(663.0)),
];

pub fn lte_carrier(earfcn: u32) -> Carrier {
    let found = LTE_BANDS
        .iter()
        .find(|(_, _, lo, hi, _)| (*lo..=*hi).contains(&earfcn));
    match found {
        Some(&(band, f_dl, offs, _, f_ul)) => {
            let step = (earfcn - offs) as f64 / 10.0;
            let duplex = match (band, f_ul) {
                (32, _) => "SDL",
                (_, Some(_)) => "FDD",
                _ => "TDD",
            };
            let dl = round1(f_dl + step);
            Carrier {
                earfcn,
                band: Some(band),
                duplex: Some(duplex),
                dl_mhz: Some(dl),
                // TDD uses the same frequency both ways
                ul_mhz: match duplex {
                    "FDD" => f_ul.map(|f| round1(f + step)),
                    "TDD" => Some(dl),
                    _ => None,
                },
            }
        }
        None => Carrier {
            earfcn,
            band: None,
            duplex: None,
            dl_mhz: None,
            ul_mhz: None,
        },
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn bandwidth_mhz(code: &str) -> Option<f32> {
    Some(match code.parse::<u8>().ok()? {
        0 => 1.4,
        1 => 3.0,
        2 => 5.0,
        3 => 10.0,
        4 => 15.0,
        5 => 20.0,
        _ => return None,
    })
}

fn num(s: Option<&String>) -> Option<i32> {
    s.and_then(|v| v.parse().ok())
}

/// Parse a `+QENG: "servingcell",...` value (without the prefix).
pub fn parse_serving(v: &str) -> Option<ServingCell> {
    let p = params(v);
    if p.first().map(String::as_str) != Some("servingcell") {
        return None;
    }
    let state = p.get(1).cloned().unwrap_or_default();
    let rat = p.get(2).cloned().unwrap_or_default();
    let mut cell = ServingCell {
        state,
        rat: rat.clone(),
        duplex: None,
        mcc: None,
        mnc: None,
        cell_id: None,
        enb_id: None,
        sector: None,
        pci: None,
        tac: None,
        carrier: None,
        ul_bandwidth_mhz: None,
        dl_bandwidth_mhz: None,
        rsrp: None,
        rsrq: None,
        rssi: None,
        sinr: None,
        srxlev: None,
        raw: p.iter().skip(2).cloned().collect(),
    };
    if rat == "LTE" {
        // "servingcell",<state>,"LTE",<is_tdd>,<MCC>,<MNC>,<cellID>,<PCID>,<earfcn>,<band>,
        // <UL_bw>,<DL_bw>,<TAC>,<RSRP>,<RSRQ>,<RSSI>,<SINR>,<srxlev>
        cell.duplex = p.get(3).cloned();
        cell.mcc = p.get(4).cloned();
        cell.mnc = p.get(5).cloned();
        cell.cell_id = p.get(6).filter(|v| !v.is_empty()).cloned();
        if let Some(ci) = cell
            .cell_id
            .as_deref()
            .and_then(|v| u32::from_str_radix(v, 16).ok())
        {
            cell.enb_id = Some(ci >> 8);
            cell.sector = Some(ci & 0xFF);
        }
        cell.pci = p.get(7).and_then(|v| v.parse().ok());
        cell.carrier = p.get(8).and_then(|v| v.parse().ok()).map(lte_carrier);
        if let (Some(c), Some(b)) = (cell.carrier.as_mut(), p.get(9).and_then(|v| v.parse().ok()))
            && c.band.is_none()
        {
            c.band = Some(b);
        }
        cell.ul_bandwidth_mhz = p.get(10).and_then(|v| bandwidth_mhz(v));
        cell.dl_bandwidth_mhz = p.get(11).and_then(|v| bandwidth_mhz(v));
        cell.tac = p.get(12).cloned();
        cell.rsrp = num(p.get(13));
        cell.rsrq = num(p.get(14));
        cell.rssi = num(p.get(15));
        cell.sinr = num(p.get(16));
        cell.srxlev = num(p.get(17));
        cell.raw.clear();
    }
    Some(cell)
}

/// Parse all `+QENG: "neighbourcell..."` values. Returns (measured LTE cells,
/// configured-but-unmeasured LTE carriers, other RAT summaries).
pub fn parse_neighbours<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> (Vec<NeighbourCell>, Vec<Carrier>, Vec<String>) {
    let mut cells = vec![];
    let mut configured: Vec<Carrier> = vec![];
    let mut other = vec![];
    for v in lines {
        let p = params(v);
        let Some(kind) = p.first().and_then(|k| k.strip_prefix("neighbourcell")) else {
            continue;
        };
        let rat = p.get(1).map(String::as_str).unwrap_or("");
        if rat != "LTE" {
            // GSM/WCDMA entries whose measurement fields are all zero are just configured ARFCNs
            if p.iter()
                .skip(3)
                .any(|x| x != "0" && x != "-" && x != "255" && x != "65535")
            {
                other.push(format!("{rat} {}", p[2..].join(",")));
            }
            continue;
        }
        let Some(earfcn) = p.get(2).and_then(|v| v.parse().ok()) else {
            continue;
        };
        let pci: u32 = p.get(3).and_then(|v| v.parse().ok()).unwrap_or(65535);
        if pci == 65535 {
            let c = lte_carrier(earfcn);
            if !configured.contains(&c) {
                configured.push(c);
            }
            continue;
        }
        // Quectel documents <rsrq>,<rsrp> but EC200T reports RSRP first; tell them apart by range
        let (a, b) = (num(p.get(4)), num(p.get(5)));
        let (rsrp, rsrq) = match (a, b) {
            (Some(x), Some(y)) if x < y => (Some(x), Some(y)),
            (x, y) => (y, x),
        };
        cells.push(NeighbourCell {
            kind: if kind.contains("intra") { "intra" } else { "inter" }.to_string(),
            carrier: lte_carrier(earfcn),
            pci,
            rsrp,
            rsrq,
            sinr: num(p.get(7)),
            srxlev: num(p.get(8)),
        });
    }
    cells.sort_by_key(|c| std::cmp::Reverse(c.rsrp.unwrap_or(-200)));
    // a frequency that has a measured cell is not "unmeasured"
    configured.retain(|c| !cells.iter().any(|n| n.carrier.earfcn == c.earfcn));
    (cells, configured, other)
}

/// Decode an LTE band bitmask (bit n-1 = band n), e.g. from `AT+QCFG="band"`.
pub fn bands_from_mask(hex: &str) -> Vec<u16> {
    let Ok(mask) = u128::from_str_radix(hex.trim_start_matches("0x").trim_start_matches("0X"), 16) else {
        return vec![];
    };
    (0..128u16)
        .filter(|b| mask & (1u128 << b) != 0)
        .map(|b| b + 1)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earfcn_to_frequency() {
        let c = lte_carrier(1600);
        assert_eq!(
            (c.band, c.duplex, c.dl_mhz, c.ul_mhz),
            (Some(3), Some("FDD"), Some(1845.0), Some(1750.0))
        );
        let t = lte_carrier(38950);
        assert_eq!(
            (t.band, t.duplex, t.dl_mhz),
            (Some(40), Some("TDD"), Some(2330.0))
        );
        assert_eq!(lte_carrier(3740).band, Some(8));
    }

    #[test]
    fn serving_ec200t() {
        let s = parse_serving(
            r#""servingcell","NOCONN","LTE","FDD",460,01,B9AADB7,237,1600,3,5,5,EF13,-97,-16,-,-1,18"#,
        )
        .unwrap();
        assert_eq!(s.pci, Some(237));
        assert_eq!(s.enb_id, Some(0xB9AAD));
        assert_eq!(s.sector, Some(0xB7));
        assert_eq!(s.carrier.unwrap().band, Some(3));
        assert_eq!(s.dl_bandwidth_mhz, Some(20.0));
        assert_eq!(
            (s.rsrp, s.rsrq, s.rssi, s.sinr),
            (Some(-97), Some(-16), None, Some(-1))
        );
    }

    #[test]
    fn neighbours_ec200t() {
        let lines = [
            r#""neighbourcell inter","LTE",2452,65535,-140,-20,-,-,0,12,12,5"#,
            r#""neighbourcell inter","LTE",3740,23,-102,-17,-,-,21,12,12,5"#,
            r#""neighbourcell intra","LTE",1600,389,-105,-17,-,-,24,6,5,4,30"#,
            r#""neighbourcell","GSM",96,0,0,0,0,0,255,0,0"#,
        ];
        let (cells, configured, other) = parse_neighbours(lines.into_iter());
        assert_eq!(cells.len(), 2);
        assert_eq!(
            (cells[0].pci, cells[0].rsrp, cells[0].rsrq),
            (23, Some(-102), Some(-17))
        );
        assert_eq!(cells[1].kind, "intra");
        assert_eq!(configured.len(), 1);
        assert_eq!(configured[0].band, Some(5));
        assert!(other.is_empty());
    }

    #[test]
    fn band_mask() {
        assert_eq!(
            bands_from_mask("0x1e200000095"),
            vec![1, 3, 5, 8, 34, 38, 39, 40, 41]
        );
    }
}
