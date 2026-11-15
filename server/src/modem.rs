//! High level modem service: connection supervision, status polling, SMS and
//! data control on top of the AT engine.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use serde::Serialize;
use thiserror::Error;
use tokio::sync::{Mutex, broadcast::error::RecvError};
use tokio::task::JoinSet;

use crate::at::{AtClient, AtError, Response};
use crate::cell::{self, CellInfo};
use crate::config::Config;
use crate::euicc::{self, EuiccInfo, ProfileAction};
use crate::events::{Event, EventBus};
use crate::pdu::{self, Pdu};
use crate::status::*;
use crate::store::{Sms, Store};
use crate::transport::{self, Transport};

const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const SMS_TIMEOUT: Duration = Duration::from_secs(60);
const STALE_PARTS: chrono::Duration = chrono::Duration::minutes(30);
/// Retry delays for re-enabling mobile data after it dropped
const DATA_RETRY_MIN: Duration = Duration::from_secs(20);
const DATA_RETRY_MAX: Duration = Duration::from_secs(300);

#[derive(Debug, Error)]
#[error("modem not connected")]
pub struct NotConnected;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RadioMode {
    On,
    Airplane,
    Off,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UssdReply {
    /// 0 done, 1 further user action required, 2 terminated by network, 4 unsupported, 5 timeout
    pub status: Option<u8>,
    pub text: String,
    pub dcs: Option<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawReply {
    pub ok: bool,
    pub lines: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmsSettings {
    pub keep_on_modem: bool,
    /// Memory new messages are stored in (`mem3`)
    pub receive_storage: Option<String>,
    pub memories: Vec<String>,
    pub usage: Vec<SmsStorage>,
}

fn check_isd_r(aid: &str) -> Result<()> {
    if euicc::ISD_R_AIDS.iter().any(|(_, a)| a.eq_ignore_ascii_case(aid)) {
        Ok(())
    } else {
        bail!("unknown ISD-R AID {aid}")
    }
}

/// The currently selected read memory (`mem1`).
async fn read_memory(c: &AtClient) -> Option<String> {
    c.cmd("AT+CPMS?")
        .await
        .ok()?
        .get("+CPMS:")
        .map(params)?
        .into_iter()
        .next()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EuiccState {
    pub supported: bool,
    /// One entry per distinct eUICC (EID) on the card
    pub cards: Vec<EuiccCard>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EuiccCard {
    pub label: String,
    pub aid: String,
    /// Other ISD-R AIDs that reach the same eUICC
    pub aliases: Vec<String>,
    /// Holds the profile the modem is currently using
    pub active: bool,
    pub info: EuiccInfo,
}

pub struct Modem {
    pub cfg: Config,
    pub events: EventBus,
    pub store: Store,
    client: RwLock<Option<AtClient>>,
    status: RwLock<Arc<Status>>,
    connection: RwLock<ConnectionState>,
    refresh_lock: Mutex<()>,
    sms_lock: Mutex<()>,
    concat_ref: AtomicU8,
    quectel: AtomicBool,
    keep_on_modem: AtomicBool,
    /// Mobile data was switched on by the user and should be kept up
    data_wanted: AtomicBool,
    memories: RwLock<Option<Vec<String>>>,
    euicc_lock: Mutex<()>,
}

impl Modem {
    pub fn new(cfg: Config, store: Store, events: EventBus) -> Arc<Self> {
        let keep_on_modem = match store.setting("keep_on_modem") {
            Ok(Some(v)) => v == "1",
            _ => cfg.keep_sms_on_sim,
        };
        let data_wanted = matches!(store.setting("data_wanted"), Ok(Some(v)) if v == "1");
        Arc::new(Self {
            cfg,
            events,
            store,
            client: RwLock::new(None),
            status: RwLock::new(Arc::new(Status::default())),
            connection: RwLock::new(ConnectionState::default()),
            refresh_lock: Mutex::new(()),
            sms_lock: Mutex::new(()),
            concat_ref: AtomicU8::new(Local::now().timestamp() as u8),
            quectel: AtomicBool::new(false),
            keep_on_modem: AtomicBool::new(keep_on_modem),
            data_wanted: AtomicBool::new(data_wanted),
            memories: RwLock::new(None),
            euicc_lock: Mutex::new(()),
        })
    }

    pub fn client(&self) -> Result<AtClient> {
        self.client
            .read()
            .unwrap()
            .clone()
            .filter(|c| !c.is_closed())
            .ok_or_else(|| NotConnected.into())
    }

    pub fn status(&self) -> Arc<Status> {
        self.status.read().unwrap().clone()
    }

    pub fn connection(&self) -> ConnectionState {
        self.connection.read().unwrap().clone()
    }

    fn set_connection(&self, connected: bool, transport: Option<String>, error: Option<String>) {
        let state = ConnectionState {
            connected,
            transport,
            error,
            since: Some(Local::now()),
        };
        *self.connection.write().unwrap() = state.clone();
        self.events.send(Event::Connection { connection: state });
    }

    // ------------------------------------------------------------ supervisor

    pub async fn run(self: Arc<Self>) {
        let mut last_error = String::new();
        loop {
            match transport::open(&self.cfg).await {
                Ok(t) => {
                    last_error.clear();
                    self.session(t).await;
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    if msg != last_error {
                        tracing::warn!("modem unavailable: {msg}");
                        self.set_connection(false, None, Some(msg.clone()));
                        last_error = msg;
                    }
                }
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    }

    async fn session(self: &Arc<Self>, t: Transport) {
        tracing::info!(transport = %t.description, "modem connected");
        let (client, actor) = AtClient::spawn(t.reader, t.writer, self.events.clone());
        *self.client.write().unwrap() = Some(client.clone());
        self.set_connection(true, Some(t.description.clone()), None);

        let mut tasks = JoinSet::new();
        tasks.spawn(self.clone().urc_loop(client.clone()));
        tasks.spawn(self.clone().poll_loop(client.clone()));

        let err = match actor.await {
            Ok(Ok(())) => "AT engine stopped".to_string(),
            Ok(Err(e)) => format!("{e:#}"),
            Err(e) => format!("AT engine crashed: {e}"),
        };
        tasks.shutdown().await;
        *self.client.write().unwrap() = None;
        tracing::warn!(transport = %t.description, "modem disconnected: {err}");
        self.set_connection(false, Some(t.description), Some(err));
    }

    async fn init(&self, c: &AtClient) {
        for cmd in [
            // echo on: lets the AT engine frame responses and resync after timeouts
            "ATE1",
            "AT+CMEE=2",
            "AT+CMGF=0",
            "AT+CNMI=2,1,0,0,0",
            "AT+CREG=2",
            "AT+CEREG=2",
            "AT+COPS=3,0",
            "AT+CSCS=\"GSM\"",
        ] {
            if let Err(e) = c.cmd(cmd).await {
                tracing::debug!(cmd, "init command failed: {e}");
            }
        }
        if let Ok(Some(mem)) = self.store.setting("sms_storage")
            && let Err(e) = c.cmd(format!("AT+CPMS=\"{mem}\",\"{mem}\",\"{mem}\"")).await
        {
            tracing::warn!(mem, "cannot select preferred SMS storage: {e}");
        }
    }

    async fn poll_loop(self: Arc<Self>, c: AtClient) {
        self.init(&c).await;
        let _ = self.refresh_with(&c, true).await;
        if let Err(e) = self.sync_sms_with(&c).await {
            tracing::warn!("SMS sync failed: {e:#}");
        }
        let mut keeper = DataKeeper::default();
        self.keep_data(&c, &mut keeper).await;
        let mut tick = tokio::time::interval(Duration::from_secs(self.cfg.poll_secs.max(2)));
        tick.tick().await;
        for n in 1u64.. {
            tick.tick().await;
            let _ = self.refresh_with(&c, false).await;
            self.keep_data(&c, &mut keeper).await;
            // safety net in case a +CMTI notification was missed
            if n % 6 == 0
                && let Err(e) = self.sync_sms_with(&c).await
            {
                tracing::warn!("SMS sync failed: {e:#}");
            }
        }
    }

    async fn urc_loop(self: Arc<Self>, c: AtClient) {
        let mut rx = c.subscribe_urc();
        loop {
            let urc = match rx.recv().await {
                Ok(u) => u,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            };
            let line = urc.line.as_str();
            if line.starts_with("+CMTI:") || line.starts_with("+CDSI:") {
                if let Err(e) = self.sync_sms_with(&c).await {
                    tracing::warn!("SMS sync failed: {e:#}");
                }
            } else if line.starts_with("+CMT:") {
                if let Some(p) = urc.payload
                    && let Err(e) = self.ingest_pdu(&p, "DIRECT")
                {
                    tracing::warn!("bad +CMT PDU: {e:#}");
                }
            } else if line.starts_with("+CPIN:") || line.starts_with("+QUSIM") || line == "RDY" {
                let _ = self.refresh_with(&c, true).await;
            } else if ["+CREG:", "+CEREG:", "+CGREG:", "+CGEV:", "+QNETDEVCTL:", "+CFUN:"]
                .iter()
                .any(|p| line.starts_with(p))
            {
                let _ = self.refresh_with(&c, false).await;
            }
        }
    }

    /// Re-enable mobile data when the user wants it on but it dropped (network
    /// detach, modem reset, ...), backing off while attempts keep failing.
    async fn keep_data(&self, c: &AtClient, k: &mut DataKeeper) {
        if !self.data_wanted.load(Ordering::Relaxed) {
            k.reset();
            return;
        }
        let s = self.status();
        // Radio off, SIM locked, no registration: nothing to retry until that changes
        if !s.sim.ready
            || s.radio.cfun != Some(1)
            || !(is_registered(s.network.registration.as_deref()) || is_registered(s.network.ps_registration.as_deref()))
        {
            return;
        }
        let up = s.data.active == Some(true) && s.data.netdev != Some(false);
        if up || s.data.active.is_none() {
            k.reset();
            return;
        }
        if k.next.is_some_and(|t| tokio::time::Instant::now() < t) {
            return;
        }
        tracing::warn!(active = ?s.data.active, netdev = ?s.data.netdev, "mobile data dropped, reconnecting");
        match self.enable_data(c).await {
            Ok(()) => {
                let s = self.refresh_with(c, false).await.ok();
                if s.is_some_and(|s| s.data.active == Some(true) && s.data.netdev != Some(false)) {
                    tracing::info!("mobile data reconnected");
                    k.reset();
                    return;
                }
                tracing::warn!("mobile data still down after reconnect attempt");
            }
            Err(e) => tracing::warn!("mobile data reconnect failed: {e:#}"),
        }
        let delay = k.delay.map_or(DATA_RETRY_MIN, |d| (d * 2).min(DATA_RETRY_MAX));
        k.delay = Some(delay);
        k.next = Some(tokio::time::Instant::now() + delay);
    }

    // ------------------------------------------------------------ status

    pub async fn refresh(&self) -> Result<Arc<Status>> {
        let c = self.client()?;
        self.refresh_with(&c, true).await
    }

    async fn refresh_with(&self, c: &AtClient, full: bool) -> Result<Arc<Status>> {
        let _g = self.refresh_lock.lock().await;
        let mut s = (*self.status()).clone();
        let ok = |r: Result<Response, AtError>| r.ok();

        if full || s.device.model.is_none() {
            s.device.manufacturer =
                ok(c.cmd("AT+CGMI").await).and_then(|r| r.get_or_bare("+CGMI:").map(str::to_string));
            s.device.model =
                ok(c.cmd("AT+CGMM").await).and_then(|r| r.get_or_bare("+CGMM:").map(str::to_string));
            s.device.revision =
                ok(c.cmd("AT+CGMR").await).and_then(|r| r.get_or_bare("+CGMR:").map(str::to_string));
            s.device.imei = ok(c.cmd("AT+CGSN").await)
                .and_then(|r| r.get_or_bare("+CGSN:").map(|v| v.trim_matches('"').to_string()));
            let quectel = s
                .device
                .manufacturer
                .as_deref()
                .is_some_and(|m| m.to_lowercase().contains("quectel"));
            self.quectel.store(quectel, Ordering::Relaxed);
            if quectel {
                s.device.usb_net_mode = ok(c.cmd("AT+QCFG=\"usbnet\"").await)
                    .and_then(|r| r.get("+QCFG:").map(params))
                    .and_then(|p| p.get(1).cloned())
                    .map(|m| match m.as_str() {
                        "0" => "RMNET/QMI".to_string(),
                        "1" => "ECM".to_string(),
                        "2" => "MBIM".to_string(),
                        "3" => "RNDIS".to_string(),
                        other => other.to_string(),
                    });
            }
        }

        // SIM
        let prev_ready = s.sim.ready;
        match c.cmd("AT+CPIN?").await {
            Ok(r) => s.sim.state = r.get("+CPIN:").map(str::to_string),
            Err(AtError::Error(e)) => {
                let l = e.to_lowercase();
                s.sim.state = Some(
                    if l.contains("not inserted") || l.ends_with(": 10") {
                        "NOT INSERTED"
                    } else if l.contains("busy") || l.ends_with(": 14") {
                        "BUSY"
                    } else if l.contains("failure") || l.ends_with(": 13") {
                        "FAILURE"
                    } else {
                        "ERROR"
                    }
                    .to_string(),
                );
            }
            Err(_) => {}
        }
        s.sim.ready = s.sim.state.as_deref() == Some("READY");
        if s.sim.ready && (full || !prev_ready || s.sim.iccid.is_none()) {
            let mut iccid = None;
            for (cmd, prefix) in [
                ("AT+QCCID", "+QCCID:"),
                ("AT+ICCID", "+ICCID:"),
                ("AT+CCID", "+CCID:"),
            ] {
                if let Ok(r) = c.cmd(cmd).await
                    && let Some(v) = r.get_or_bare(prefix)
                {
                    // some modules pad odd-length ICCIDs with the BCD filler 'F'
                    iccid = Some(v.trim_matches('"').trim_end_matches(['F', 'f']).to_string());
                    break;
                }
            }
            s.sim.iccid = iccid;
            s.sim.imsi = ok(c.cmd("AT+CIMI").await).and_then(|r| r.get_or_bare("+CIMI:").map(str::to_string));
            s.sim.msisdn = ok(c.cmd("AT+CNUM").await)
                .and_then(|r| r.get("+CNUM:").map(params))
                .and_then(|p| non_empty(p.get(1)));
            s.sim.carrier = s
                .sim
                .imsi
                .as_deref()
                .and_then(carrier_from_plmn)
                .or_else(|| s.sim.iccid.as_deref().and_then(carrier_from_iccid));
        } else if !s.sim.ready {
            s.sim.iccid = None;
            s.sim.imsi = None;
            s.sim.msisdn = None;
            s.sim.carrier = None;
        }

        // radio
        s.radio.cfun =
            ok(c.cmd("AT+CFUN?").await).and_then(|r| r.get("+CFUN:").and_then(|v| params(v)[0].parse().ok()));
        s.radio.mode = match s.radio.cfun {
            Some(1) => "on",
            Some(4) => "airplane",
            Some(0) => "off",
            _ => "unknown",
        }
        .to_string();

        // network
        let mut net = NetworkInfo::default();
        if let Some(v) = ok(c.cmd("AT+COPS?").await).and_then(|r| r.get("+COPS:").map(params)) {
            let name = non_empty(v.get(2));
            net.operator = match name {
                Some(n) if n.chars().all(|ch| ch.is_ascii_digit()) => {
                    net.plmn = Some(n.clone());
                    carrier_from_plmn(&n).or(Some(n))
                }
                n => n,
            };
            net.access_tech = v.get(3).and_then(|a| access_tech(a));
        }
        if let Some(v) = ok(c.cmd("AT+CREG?").await).and_then(|r| r.get("+CREG:").map(parse_creg)) {
            net.registration = Some(v.0);
            net.area_code = v.1;
            net.cell_id = v.2;
            net.access_tech = net.access_tech.or(v.3);
        }
        if let Some(v) = ok(c.cmd("AT+CEREG?").await).and_then(|r| r.get("+CEREG:").map(parse_creg)) {
            if is_registered(Some(&v.0)) || net.registration.is_none() {
                net.area_code = v.1.or(net.area_code);
                net.cell_id = v.2.or(net.cell_id);
            }
            net.ps_registration = Some(v.0);
        }
        // LTE-only modules often report CS as not registered while EPS is fine
        if !is_registered(net.registration.as_deref()) && is_registered(net.ps_registration.as_deref()) {
            net.registration = net.ps_registration.clone();
        }
        if let Some(p) = ok(c.cmd("AT+QNWINFO").await).and_then(|r| r.get("+QNWINFO:").map(params))
            && p.len() >= 4
        {
            net.access_tech = non_empty(p.first()).or(net.access_tech);
            net.plmn = non_empty(p.get(1)).or(net.plmn);
            net.band = non_empty(p.get(2));
            net.channel = p.get(3).and_then(|v| v.parse().ok());
        }
        if net.operator.is_none() {
            net.operator = net.plmn.as_deref().and_then(carrier_from_plmn);
        }
        s.network = net;

        // signal
        let mut sig = SignalInfo::default();
        if let Some(p) = ok(c.cmd("AT+CSQ").await).and_then(|r| r.get("+CSQ:").map(params)) {
            sig.csq = p[0].parse().ok().filter(|v| *v != 99);
            sig.rssi = sig.csq.and_then(csq_to_dbm);
            sig.ber = p.get(1).and_then(|v| v.parse().ok()).filter(|v| *v != 99);
        }
        if let Some(p) = ok(c.cmd("AT+QCSQ").await).and_then(|r| r.get("+QCSQ:").map(params)) {
            let n = |i: usize| p.get(i).and_then(|v| v.parse::<i32>().ok());
            match p[0].as_str() {
                "LTE" | "CAT-M" | "CAT-NB" => {
                    // EC200T reports a non-dBm value here; keep the CSQ-derived RSSI then
                    sig.rssi = n(1).filter(|v| *v < 0).or(sig.rssi);
                    sig.rsrp = n(2);
                    sig.sinr = n(3).map(|v| if v > 50 { v / 5 - 20 } else { v });
                    sig.rsrq = n(4);
                }
                "NR5G" => {
                    sig.rsrp = n(1);
                    sig.sinr = n(2);
                    sig.rsrq = n(3);
                }
                _ => {}
            }
        }
        sig.level = signal_level(&sig);
        s.signal = sig;

        // data
        let cid = self.cfg.data_cid;
        let mut data = DataInfo {
            cid,
            wanted: self.data_wanted.load(Ordering::Relaxed),
            ..Default::default()
        };
        data.attached = ok(c.cmd("AT+CGATT?").await).and_then(|r| r.get("+CGATT:").map(|v| v == "1"));
        data.active = ok(c.cmd("AT+CGACT?").await).and_then(|r| {
            r.all("+CGACT:")
                .map(params)
                .find(|p| p[0] == cid.to_string())
                .map(|p| p.get(1).is_some_and(|v| v == "1"))
        });
        if self.quectel.load(Ordering::Relaxed)
            && s.device.usb_net_mode.as_deref() == Some("ECM")
            && let Some(p) = ok(c.cmd("AT+QNETDEVCTL?").await).and_then(|r| r.get("+QNETDEVCTL:").map(params))
            && let Some(state) = p.get(3)
        {
            data.netdev = Some(state == "1");
        }
        if let Some(r) = ok(c.cmd(format!("AT+CGPADDR={cid}")).await) {
            data.ip = r
                .all("+CGPADDR:")
                .flat_map(|v| params(v).into_iter().skip(1))
                .filter(|ip| {
                    !ip.is_empty() && ip != "0.0.0.0" && !ip.chars().all(|c| c == '0' || c == '.' || c == ':')
                })
                .collect();
        }
        if let Some(r) = ok(c.cmd("AT+CGDCONT?").await)
            && let Some(p) = r.all("+CGDCONT:").map(params).find(|p| p[0] == cid.to_string())
        {
            data.pdp_type = non_empty(p.get(1));
            data.apn = non_empty(p.get(2));
        }
        if let Some(p) = ok(c.cmd("AT+QGDCNT?").await).and_then(|r| r.get("+QGDCNT:").map(params)) {
            data.tx_bytes = p.first().and_then(|v| v.parse().ok());
            data.rx_bytes = p.get(1).and_then(|v| v.parse().ok());
        }
        s.data = data;

        // +CPMS: <mem1>,<used>,<total>,<mem2>,...,<mem3>,<used>,<total>; mem3 receives new messages
        s.sms_storage = ok(c.cmd("AT+CPMS?").await)
            .and_then(|r| r.get("+CPMS:").map(params))
            .and_then(|p| {
                let base = if p.len() >= 9 { 6 } else { 0 };
                Some(SmsStorage {
                    memory: p.get(base)?.clone(),
                    used: p.get(base + 1)?.parse().ok()?,
                    total: p.get(base + 2)?.parse().ok()?,
                })
            });

        s.updated_at = Some(Local::now());
        let s = Arc::new(s);
        *self.status.write().unwrap() = s.clone();
        self.events.send(Event::Status { status: s.clone() });
        Ok(s)
    }

    // ------------------------------------------------------------ SMS

    pub async fn sync_sms(&self) -> Result<usize> {
        let c = self.client()?;
        self.sync_sms_with(&c).await
    }

    /// Pull every message from each modem memory (SIM card and module) into the database.
    async fn sync_sms_with(&self, c: &AtClient) -> Result<usize> {
        let _g = self.sms_lock.lock().await;
        let keep = self.keep_on_modem.load(Ordering::Relaxed);
        let original = read_memory(c).await;
        let mut memories = self.sms_memories(c).await;
        if memories.is_empty() {
            memories.extend(original.clone());
        }
        let mut new = 0;
        let mut usage = vec![];
        let mut last_err = None;
        for mem in &memories {
            if let Err(e) = c.cmd(format!("AT+CPMS=\"{mem}\"")).await {
                tracing::debug!(mem, "SMS memory not available: {e}");
                continue;
            }
            match self.sync_memory(c, mem, keep).await {
                Ok(n) => new += n,
                Err(e) => last_err = Some(e),
            }
            // usage after deletions
            if let Some(u) = c
                .cmd(format!("AT+CPMS=\"{mem}\""))
                .await
                .ok()
                .and_then(|r| r.get("+CPMS:").map(params))
                .and_then(|p| {
                    Some(SmsStorage {
                        memory: mem.clone(),
                        used: p.first()?.parse().ok()?,
                        total: p.get(1)?.parse().ok()?,
                    })
                })
            {
                usage.push(u);
            }
        }
        if let Some(m) = original {
            let _ = c.cmd(format!("AT+CPMS=\"{m}\"")).await;
        }
        if !usage.is_empty() {
            let mut st = (*self.status()).clone();
            st.sms_storages = usage;
            *self.status.write().unwrap() = Arc::new(st);
        }
        for sms in self.store.flush_stale(STALE_PARTS)? {
            new += 1;
            self.events.send(Event::Sms { sms });
        }
        self.events.send(Event::SmsSync { new });
        match last_err {
            Some(e) if new == 0 => Err(e),
            _ => Ok(new),
        }
    }

    /// Read every message of the currently selected memory (`mem1`).
    async fn sync_memory(&self, c: &AtClient, mem: &str, keep: bool) -> Result<usize> {
        let r = c.cmd_timeout("AT+CMGL=4", Duration::from_secs(30)).await?;
        let mut new = 0;
        let mut delete = vec![];
        let mut present = vec![];
        let mut lines = r.lines.iter();
        while let Some(line) = lines.next() {
            let Some(head) = line.strip_prefix("+CMGL:") else {
                continue;
            };
            let Some(idx) = params(head).first().and_then(|v| v.parse::<u32>().ok()) else {
                continue;
            };
            let Some(hex) = lines.next() else { break };
            match pdu::decode(hex) {
                Ok(Pdu::Deliver(_)) => {
                    new += self.ingest_pdu(hex, mem)? as usize;
                    if keep {
                        present.push((hex.clone(), idx));
                    } else {
                        delete.push(idx);
                    }
                }
                Ok(Pdu::StatusReport(_)) => delete.push(idx),
                Ok(Pdu::Submit(_)) => {}
                Err(e) => tracing::warn!(idx, mem, "undecodable PDU kept on modem: {e:#}"),
            }
        }
        for idx in delete {
            if let Err(e) = c.cmd(format!("AT+CMGD={idx}")).await {
                tracing::warn!(idx, mem, "delete from modem failed: {e}");
            }
        }
        self.store.mark_present(mem, &present)?;
        Ok(new)
    }

    /// SMS memories the modem can read from (`SM`, `ME`), from `AT+CPMS=?`.
    async fn sms_memories(&self, c: &AtClient) -> Vec<String> {
        if let Some(m) = self.memories.read().unwrap().clone() {
            return m;
        }
        let Ok(r) = c.cmd("AT+CPMS=?").await else {
            return vec![];
        };
        // +CPMS: ("SM","ME"),("SM","ME"),("SM","ME")
        let first = r
            .get("+CPMS:")
            .and_then(|v| v.split(')').next())
            .unwrap_or_default()
            .to_string();
        let mems: Vec<String> = ["SM", "ME"]
            .into_iter()
            .filter(|m| first.contains(&format!("\"{m}\"")))
            .map(str::to_string)
            .collect();
        *self.memories.write().unwrap() = Some(mems.clone());
        mems
    }

    /// Delete messages from modem memory, e.g. before removing them from the database
    /// (otherwise the next sync would import them again).
    async fn delete_on_modem(&self, locations: Vec<(String, u32)>) -> Result<()> {
        if locations.is_empty() {
            return Ok(());
        }
        let c = self.client().context("消息仍存储在模块上，模块离线时无法删除")?;
        let _g = self.sms_lock.lock().await;
        let original = read_memory(&c).await;
        let mut result = Ok(());
        let mut current = String::new();
        for (mem, idx) in &locations {
            if *mem != current {
                c.cmd(format!("AT+CPMS=\"{mem}\"")).await?;
                current = mem.clone();
            }
            if let Err(e) = c.cmd(format!("AT+CMGD={idx}")).await {
                result = Err(anyhow!("delete {mem} #{idx}: {e}"));
            }
        }
        if let Some(m) = original {
            let _ = c.cmd(format!("AT+CPMS=\"{m}\"")).await;
        }
        result
    }

    pub async fn delete_sms(&self, id: i64) -> Result<bool> {
        self.delete_on_modem(self.store.modem_locations(Some(id), None)?)
            .await?;
        self.store.delete(id)
    }

    pub async fn delete_conversation(&self, peer: &str) -> Result<usize> {
        self.delete_on_modem(self.store.modem_locations(None, Some(peer))?)
            .await?;
        self.store.delete_peer(peer)
    }

    pub fn sms_settings(&self) -> SmsSettings {
        let st = self.status();
        SmsSettings {
            keep_on_modem: self.keep_on_modem.load(Ordering::Relaxed),
            receive_storage: st.sms_storage.as_ref().map(|s| s.memory.clone()),
            memories: self.memories.read().unwrap().clone().unwrap_or_default(),
            usage: st.sms_storages.clone(),
        }
    }

    pub async fn set_sms_settings(
        &self,
        keep_on_modem: Option<bool>,
        receive: Option<&str>,
    ) -> Result<SmsSettings> {
        if let Some(mem) = receive {
            if !matches!(mem, "SM" | "ME") {
                bail!("storage must be SM or ME");
            }
            let c = self.client()?;
            c.cmd(format!("AT+CPMS=\"{mem}\",\"{mem}\",\"{mem}\"")).await?;
            self.store.set_setting("sms_storage", mem)?;
        }
        if let Some(keep) = keep_on_modem {
            self.keep_on_modem.store(keep, Ordering::Relaxed);
            self.store
                .set_setting("keep_on_modem", if keep { "1" } else { "0" })?;
        }
        if let Ok(c) = self.client() {
            let _ = self.refresh_with(&c, false).await;
            let _ = self.sync_sms_with(&c).await;
        }
        Ok(self.sms_settings())
    }

    /// Returns true when a complete new message was stored.
    fn ingest_pdu(&self, hex: &str, storage: &str) -> Result<bool> {
        let Pdu::Deliver(d) = pdu::decode(hex)? else {
            return Ok(false);
        };
        let (reference, total, seq) = d
            .concat
            .map(|c| (c.reference, c.total, c.seq))
            .unwrap_or((0, 1, 1));
        let stored = self.store.ingest(
            hex,
            storage,
            &d.sender,
            reference,
            total,
            seq,
            &d.text,
            d.timestamp,
        )?;
        if let Some(sms) = stored {
            tracing::info!(from = %sms.peer, "new SMS");
            self.events.send(Event::Sms { sms });
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn send_sms(&self, to: &str, text: &str) -> Result<Sms> {
        let to = to.trim();
        if to.is_empty() || !to.chars().all(|c| c.is_ascii_digit() || c == '+') {
            bail!("invalid phone number");
        }
        if text.is_empty() {
            bail!("empty message");
        }
        let c = self.client()?;
        let reference = self.concat_ref.fetch_add(1, Ordering::Relaxed);
        let pdus = pdu::encode_submit(to, text, reference, false);
        if pdus.len() > 10 {
            bail!("message too long ({} parts)", pdus.len());
        }
        let sms = self.store.insert_outgoing(to, text, pdus.len())?;
        self.events.send(Event::Sms { sms: sms.clone() });
        let mut error = None;
        for p in &pdus {
            if let Err(e) = c
                .send_full(
                    format!("AT+CMGS={}", p.tpdu_len),
                    Some(p.hex.clone()),
                    SMS_TIMEOUT,
                )
                .await
            {
                error = Some(e.to_string());
                break;
            }
        }
        let status = if error.is_some() { "failed" } else { "sent" };
        let sms = self
            .store
            .set_status(sms.id, status, error.as_deref())?
            .context("message vanished")?;
        self.events.send(Event::Sms { sms: sms.clone() });
        Ok(sms)
    }

    // ------------------------------------------------------------ control

    fn quectel_ecm(&self) -> bool {
        self.quectel.load(Ordering::Relaxed) && self.status().device.usb_net_mode.as_deref() == Some("ECM")
    }

    async fn enable_data(&self, c: &AtClient) -> Result<()> {
        let cid = self.cfg.data_cid;
        let _ = c.cmd_timeout("AT+CGATT=1", Duration::from_secs(75)).await;
        let r = if self.quectel_ecm() {
            c.cmd_timeout(format!("AT+QNETDEVCTL=1,{cid},1"), Duration::from_secs(30))
                .await
        } else {
            c.cmd_timeout(format!("AT+CGACT=1,{cid}"), Duration::from_secs(150))
                .await
        };
        r.map_err(|e| {
            // A bare ERROR is all the modem says when there is no PS registration
            let ps = self.status().network.ps_registration.clone();
            if is_registered(ps.as_deref()) {
                anyhow!("enable data: {e}")
            } else {
                anyhow!("enable data: {e} (not registered for packet data: {})", ps.as_deref().unwrap_or("unknown"))
            }
        })?;
        Ok(())
    }

    pub async fn set_data(&self, enabled: bool) -> Result<Arc<Status>> {
        let c = self.client()?;
        let cid = self.cfg.data_cid;
        // Remember the intent first so a failed attempt is still retried in the background
        self.data_wanted.store(enabled, Ordering::Relaxed);
        if let Err(e) = self.store.set_setting("data_wanted", if enabled { "1" } else { "0" }) {
            tracing::warn!("cannot save data setting: {e:#}");
        }
        let quectel_ecm = self.quectel_ecm();
        if enabled {
            self.enable_data(&c).await?;
        } else {
            let mut result = Ok(Response::default());
            if quectel_ecm {
                result = c
                    .cmd_timeout(format!("AT+QNETDEVCTL=0,{cid},0"), Duration::from_secs(30))
                    .await;
            }
            if !quectel_ecm || result.is_err() {
                result = c
                    .cmd_timeout(format!("AT+CGACT=0,{cid}"), Duration::from_secs(40))
                    .await;
            }
            result.map_err(|e| anyhow!("disable data: {e}"))?;
        }
        self.refresh_with(&c, false).await
    }

    pub async fn set_radio(&self, mode: RadioMode) -> Result<Arc<Status>> {
        let c = self.client()?;
        let fun = match mode {
            RadioMode::On => 1,
            RadioMode::Airplane => 4,
            RadioMode::Off => 0,
        };
        c.cmd_timeout(format!("AT+CFUN={fun}"), Duration::from_secs(20))
            .await?;
        self.refresh_with(&c, true).await
    }

    pub async fn reboot(&self) -> Result<()> {
        let c = self.client()?;
        c.cmd_timeout("AT+CFUN=1,1", Duration::from_secs(10)).await?;
        Ok(())
    }

    pub async fn enter_pin(&self, pin: &str) -> Result<Arc<Status>> {
        if pin.is_empty() || pin.len() > 8 || !pin.chars().all(|c| c.is_ascii_digit()) {
            bail!("PIN must be 4-8 digits");
        }
        let c = self.client()?;
        c.cmd_timeout(format!("AT+CPIN=\"{pin}\""), Duration::from_secs(10))
            .await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        self.refresh_with(&c, true).await
    }

    /// Write the subscriber number to the SIM's own-number phonebook so `AT+CNUM` reports it.
    /// An empty number clears the entry.
    pub async fn set_msisdn(&self, number: &str) -> Result<Arc<Status>> {
        let number = number.trim();
        if number.len() > 20
            || !number
                .chars()
                .enumerate()
                .all(|(i, c)| c.is_ascii_digit() || (i == 0 && c == '+'))
        {
            bail!("invalid phone number");
        }
        let c = self.client()?;
        c.cmd("AT+CPBS=\"ON\"").await?;
        let r = if number.is_empty() {
            c.cmd("AT+CPBW=1").await
        } else {
            let ty = if number.starts_with('+') { 145 } else { 129 };
            c.cmd(format!("AT+CPBW=1,\"{number}\",{ty},\"MSISDN\"")).await
        };
        let _ = c.cmd("AT+CPBS=\"SM\"").await;
        r.map_err(|e| anyhow!("写入 SIM 失败（部分卡不允许写入本机号码）：{e}"))?;
        self.refresh_with(&c, true).await
    }

    pub async fn reset_counters(&self) -> Result<Arc<Status>> {
        let c = self.client()?;
        c.cmd("AT+QGDCNT=0").await?;
        self.refresh_with(&c, false).await
    }

    pub async fn pdp_contexts(&self) -> Result<Vec<PdpContext>> {
        let c = self.client()?;
        let r = c.cmd("AT+CGDCONT?").await?;
        Ok(r.all("+CGDCONT:")
            .map(params)
            .filter_map(|p| {
                Some(PdpContext {
                    cid: p.first()?.parse().ok()?,
                    pdp_type: p.get(1).cloned().unwrap_or_default(),
                    apn: p.get(2).cloned().unwrap_or_default(),
                    address: non_empty(p.get(3)),
                })
            })
            .collect())
    }

    pub async fn set_apn(
        &self,
        cid: u8,
        pdp_type: &str,
        apn: &str,
        auth: Option<(&str, &str, u8)>,
    ) -> Result<Vec<PdpContext>> {
        let bad = |s: &str| s.contains('"') || s.contains('\r') || s.contains('\n');
        if bad(apn) || !matches!(pdp_type, "IP" | "IPV6" | "IPV4V6") {
            bail!("invalid APN settings");
        }
        let c = self.client()?;
        c.cmd(format!("AT+CGDCONT={cid},\"{pdp_type}\",\"{apn}\""))
            .await?;
        if let Some((user, pass, method)) = auth {
            if bad(user) || bad(pass) {
                bail!("invalid credentials");
            }
            if !self.quectel.load(Ordering::Relaxed) {
                bail!("APN authentication is only supported on Quectel modules");
            }
            let ctx = match pdp_type {
                "IPV6" => 2,
                "IPV4V6" => 3,
                _ => 1,
            };
            c.cmd(format!(
                "AT+QICSGP={cid},{ctx},\"{apn}\",\"{user}\",\"{pass}\",{method}"
            ))
            .await?;
        }
        self.pdp_contexts().await
    }

    pub async fn ussd(&self, code: &str) -> Result<UssdReply> {
        if code.is_empty() || code.contains('"') || code.contains('\r') {
            bail!("invalid USSD code");
        }
        let c = self.client()?;
        let mut rx = c.subscribe_urc();
        c.cmd_timeout(format!("AT+CUSD=1,\"{code}\",15"), Duration::from_secs(15))
            .await?;
        let wait = async {
            loop {
                match rx.recv().await {
                    Ok(u) if u.line.starts_with("+CUSD:") => return Some(u.line),
                    Ok(_) | Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return None,
                }
            }
        };
        let line = tokio::time::timeout(Duration::from_secs(30), wait)
            .await
            .map_err(|_| anyhow!("no USSD response from network"))?
            .ok_or(NotConnected)?;
        let p = params(line.trim_start_matches("+CUSD:"));
        let status = p.first().and_then(|v| v.parse().ok());
        let dcs: Option<u8> = p.get(2).and_then(|v| v.parse().ok());
        let raw = p.get(1).cloned().unwrap_or_default();
        let ucs2 = dcs.is_some_and(|d| d == 72 || (d & 0xF0 == 0x40 && d & 0x0C == 0x08));
        let text = if ucs2 {
            pdu::ucs2_hex_decode(&raw).unwrap_or(raw)
        } else {
            raw
        };
        Ok(UssdReply { status, text, dcs })
    }

    pub async fn ussd_cancel(&self) -> Result<()> {
        self.client()?.cmd("AT+CUSD=2").await?;
        Ok(())
    }

    // ------------------------------------------------------------ cells

    /// Serving and neighbour cells plus allowed bands (Quectel `AT+QENG`).
    pub async fn cell_info(&self) -> Result<CellInfo> {
        let c = self.client()?;
        let serving = c
            .cmd("AT+QENG=\"servingcell\"")
            .await
            .ok()
            .and_then(|r| r.all("+QENG:").find_map(cell::parse_serving));
        let (neighbours, configured, other_rat) = match c
            .cmd_timeout("AT+QENG=\"neighbourcell\"", Duration::from_secs(10))
            .await
        {
            Ok(r) => cell::parse_neighbours(r.all("+QENG:")),
            Err(_) => Default::default(),
        };
        // +QCFG: "band",<gsm mask>,<lte mask>
        let lte_bands = c
            .cmd("AT+QCFG=\"band\"")
            .await
            .ok()
            .and_then(|r| r.get("+QCFG:").map(params))
            .and_then(|p| p.get(2).map(|m| cell::bands_from_mask(m)))
            .unwrap_or_default();
        let scan_mode = c
            .cmd("AT+QCFG=\"nwscanmode\"")
            .await
            .ok()
            .and_then(|r| r.get("+QCFG:").map(params))
            .and_then(|p| p.get(1).cloned());
        Ok(CellInfo {
            serving,
            neighbours,
            configured,
            other_rat,
            lte_bands,
            scan_mode,
        })
    }

    // ------------------------------------------------------------ eUICC

    /// Detect every eUICC on the card (one per ISD-R) and read their EIDs and profiles.
    pub async fn euicc(&self) -> Result<EuiccState> {
        let c = self.client()?;
        let _g = self.euicc_lock.lock().await;
        let ch = euicc::Channel::open(&c).await?;
        let cards = self.read_euiccs(&ch).await;
        ch.close().await;
        let cards = cards?;
        Ok(EuiccState {
            supported: !cards.is_empty(),
            cards,
        })
    }

    async fn read_euiccs(&self, ch: &euicc::Channel<'_>) -> Result<Vec<EuiccCard>> {
        let sim_iccid = self.status().sim.iccid.clone();
        let mut cards: Vec<EuiccCard> = vec![];
        for (label, aid) in euicc::ISD_R_AIDS {
            if !ch.select(aid).await? {
                continue;
            }
            let eid = ch.eid().await.ok();
            if let Some(known) = cards.iter_mut().find(|c| eid.is_some() && c.info.eid == eid) {
                known.aliases.push(label.to_string());
                continue;
            }
            let info = ch.info().await?;
            let active = info
                .profiles
                .iter()
                .any(|p| p.enabled && sim_iccid.as_deref() == Some(p.iccid.as_str()));
            cards.push(EuiccCard {
                label: label.to_string(),
                aid: aid.to_string(),
                aliases: vec![],
                active,
                info,
            });
        }
        Ok(cards)
    }

    pub async fn euicc_profile(&self, aid: &str, iccid: &str, action: ProfileAction) -> Result<EuiccState> {
        check_isd_r(aid)?;
        let c = self.client()?;
        {
            let _g = self.euicc_lock.lock().await;
            let ch = euicc::Channel::open(&c).await?;
            let r = async {
                if !ch.select(aid).await? {
                    bail!("eUICC {aid} not found on this card");
                }
                ch.profile_action(iccid, action).await
            }
            .await;
            ch.close().await;
            r?;
        }
        if !matches!(action, ProfileAction::Delete) {
            self.reload_sim(&c).await;
        }
        self.euicc().await
    }

    pub async fn euicc_nickname(&self, aid: &str, iccid: &str, nickname: &str) -> Result<EuiccState> {
        check_isd_r(aid)?;
        let c = self.client()?;
        {
            let _g = self.euicc_lock.lock().await;
            let ch = euicc::Channel::open(&c).await?;
            let r = async {
                if !ch.select(aid).await? {
                    bail!("eUICC {aid} not found on this card");
                }
                ch.set_nickname(iccid, nickname).await
            }
            .await;
            ch.close().await;
            r?;
        }
        self.euicc().await
    }

    /// Power-cycle the SIM so the modem picks up a newly enabled profile.
    async fn reload_sim(&self, c: &AtClient) {
        let prev = self.status().radio.cfun.filter(|f| *f != 0).unwrap_or(1);
        if let Err(e) = c.cmd_timeout("AT+CFUN=0", Duration::from_secs(30)).await {
            tracing::warn!("SIM reload (CFUN=0) failed: {e}");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Err(e) = c
            .cmd_timeout(format!("AT+CFUN={prev}"), Duration::from_secs(30))
            .await
        {
            tracing::warn!("SIM reload (CFUN={prev}) failed: {e}");
        }
        // The card needs the full UICC activation before APDUs work again
        // (until then the modem answers +CSIM: 0,""): wait until EF_ICCID is readable.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let ready = c
                .cmd("AT+CRSM=176,12258,0,0,10")
                .await
                .ok()
                .and_then(|r| r.get("+CRSM:").map(params))
                .is_some_and(|p| p.first().map(String::as_str) == Some("144"));
            if ready {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!("SIM not ready 30s after reload");
                break;
            }
        }
        let _ = self.refresh_with(c, true).await;
    }

    pub async fn raw(&self, command: &str, timeout: Duration) -> Result<RawReply> {
        let command = command.trim();
        if !command.to_ascii_uppercase().starts_with("AT") {
            bail!("command must start with AT");
        }
        let c = self.client()?;
        Ok(match c.cmd_timeout(command, timeout).await {
            Ok(r) => RawReply {
                ok: true,
                lines: r.lines,
                error: None,
            },
            Err(AtError::Closed) => return Err(NotConnected.into()),
            Err(e) => RawReply {
                ok: false,
                lines: vec![],
                error: Some(e.to_string()),
            },
        })
    }
}

#[derive(Default)]
struct DataKeeper {
    delay: Option<Duration>,
    next: Option<tokio::time::Instant>,
}

impl DataKeeper {
    fn reset(&mut self) {
        self.delay = None;
        self.next = None;
    }
}
