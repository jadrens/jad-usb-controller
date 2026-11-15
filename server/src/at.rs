//! AT command engine.
//!
//! A single actor task owns the modem port. Commands are queued and executed
//! one at a time; any line that is not part of the running command's response
//! is published as an unsolicited result code (URC).
//!
//! Once echo is known to be on (`ATE1`), a response only starts after the
//! command's own echo. Anything before it is a leftover from an earlier,
//! timed-out command and is dropped, so a stray `OK` cannot shift every
//! following response by one.

use std::time::Duration;

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::Instant;

use crate::events::{Event, EventBus};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Prefixes of URCs that may arrive while another command is running.
const URC_PREFIXES: &[&str] = &[
    "+CMTI:",
    "+CMT:",
    "+CDS:",
    "+CDSI:",
    "+CBM:",
    "+CUSD:",
    "+CREG:",
    "+CGREG:",
    "+CEREG:",
    "+C5GREG:",
    "+CPIN:",
    "+CGEV:",
    "+QIND:",
    "+QUSIM:",
    "+QNETDEVCTL:",
    "+QSTAT:",
    "+CRING:",
    "RING",
    "RDY",
    "POWERED DOWN",
    "+CFUN:",
    "^SIMST:",
    "^MODE:",
    "^RSSI:",
    "^HCSQ:",
];

/// URCs followed by one extra line of payload (PDU mode).
const TWO_LINE_URCS: &[&str] = &["+CMT:", "+CDS:", "+CBM:"];

#[derive(Debug, Error, Clone)]
pub enum AtError {
    #[error("command timed out")]
    Timeout,
    #[error("{0}")]
    Error(String),
    #[error("modem disconnected")]
    Closed,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Response {
    pub lines: Vec<String>,
}

impl Response {
    /// Value of the first line starting with `prefix` (e.g. `+CSQ:`), trimmed.
    pub fn get(&self, prefix: &str) -> Option<&str> {
        self.lines
            .iter()
            .find_map(|l| l.strip_prefix(prefix).map(str::trim))
    }

    pub fn all<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.lines
            .iter()
            .filter_map(move |l| l.strip_prefix(prefix).map(str::trim))
    }

    /// The value after `prefix`, or the first line if the modem omitted the prefix.
    pub fn get_or_bare(&self, prefix: &str) -> Option<&str> {
        self.get(prefix)
            .or_else(|| self.lines.first().map(|s| s.trim()))
            .filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone)]
pub struct Urc {
    pub line: String,
    /// Second line for two-line URCs such as `+CMT:` (the PDU).
    pub payload: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogDir {
    Tx,
    Rx,
    Urc,
}

struct Request {
    cmd: String,
    payload: Option<String>,
    timeout: Duration,
    reply: oneshot::Sender<Result<Response, AtError>>,
}

#[derive(Clone)]
pub struct AtClient {
    tx: mpsc::Sender<Request>,
    urc: broadcast::Sender<Urc>,
}

impl AtClient {
    pub fn spawn<R, W>(
        reader: R,
        writer: W,
        events: EventBus,
    ) -> (Self, tokio::task::JoinHandle<anyhow::Result<()>>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, rx) = mpsc::channel(32);
        let (urc, _) = broadcast::channel(64);
        let actor = Actor {
            rx,
            writer,
            urc: urc.clone(),
            events,
            buf: Vec::new(),
            current: None,
            pending_urc: None,
            echo: false,
            unechoed_timeouts: 0,
        };
        let handle = tokio::spawn(actor.run(reader));
        (Self { tx, urc }, handle)
    }

    pub fn subscribe_urc(&self) -> broadcast::Receiver<Urc> {
        self.urc.subscribe()
    }

    pub async fn send_full(
        &self,
        cmd: impl Into<String>,
        payload: Option<String>,
        timeout: Duration,
    ) -> Result<Response, AtError> {
        let (reply, rx) = oneshot::channel();
        let req = Request {
            cmd: cmd.into(),
            payload,
            timeout,
            reply,
        };
        self.tx.send(req).await.map_err(|_| AtError::Closed)?;
        rx.await.map_err(|_| AtError::Closed)?
    }

    pub async fn cmd(&self, cmd: impl Into<String>) -> Result<Response, AtError> {
        self.send_full(cmd, None, DEFAULT_TIMEOUT).await
    }

    pub async fn cmd_timeout(&self, cmd: impl Into<String>, timeout: Duration) -> Result<Response, AtError> {
        self.send_full(cmd, None, timeout).await
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

struct Running {
    cmd: String,
    echoed: bool,
    payload: Option<String>,
    deadline: Instant,
    lines: Vec<String>,
    reply: oneshot::Sender<Result<Response, AtError>>,
}

struct Actor<W> {
    rx: mpsc::Receiver<Request>,
    writer: W,
    urc: broadcast::Sender<Urc>,
    events: EventBus,
    buf: Vec<u8>,
    current: Option<Running>,
    pending_urc: Option<String>,
    /// Modem echo is on, so responses can be framed by the echo.
    echo: bool,
    /// Consecutive timeouts without seeing the command's echo.
    unechoed_timeouts: u8,
}

enum Item {
    Line(String),
    Prompt,
}

impl<W: AsyncWrite + Unpin> Actor<W> {
    async fn run<R: AsyncRead + Unpin + Send + 'static>(mut self, reader: R) -> anyhow::Result<()> {
        let (chunk_tx, mut chunk_rx) = mpsc::channel::<std::io::Result<Vec<u8>>>(64);
        let read_task = tokio::spawn(async move {
            let mut reader = reader;
            let mut buf = vec![0u8; 4096];
            loop {
                let r = reader.read(&mut buf).await.map(|n| buf[..n].to_vec());
                let stop = !matches!(&r, Ok(v) if !v.is_empty());
                if chunk_tx.send(r).await.is_err() || stop {
                    break;
                }
            }
        });

        let result = loop {
            let deadline = self.current.as_ref().map(|c| c.deadline);
            tokio::select! {
                chunk = chunk_rx.recv() => match chunk {
                    Some(Ok(data)) if !data.is_empty() => {
                        self.buf.extend_from_slice(&data);
                        let mut err = None;
                        for item in self.drain_items() {
                            if let Err(e) = self.handle_item(item).await {
                                err = Some(e);
                                break;
                            }
                        }
                        if let Some(e) = err {
                            break Err(e);
                        }
                    }
                    Some(Err(e)) => break Err(anyhow::anyhow!("read error: {e}")),
                    _ => break Err(anyhow::anyhow!("port closed")),
                },
                req = self.rx.recv(), if self.current.is_none() => match req {
                    Some(req) => {
                        if let Err(e) = self.start(req).await {
                            break Err(e);
                        }
                    }
                    None => break Ok(()),
                },
                _ = sleep_until_opt(deadline) => {
                    if let Some(cur) = self.current.take() {
                        tracing::warn!(cmd = %cur.cmd, "AT command timed out");
                        if self.echo && !cur.echoed {
                            // A busy modem (e.g. AT+COPS=? scanning) can delay a single echo past
                            // the timeout, so only give up on echo framing after several misses:
                            // unframed, the late output would be read as the next command's reply.
                            self.unechoed_timeouts += 1;
                            if self.unechoed_timeouts >= UNECHOED_TIMEOUTS_MAX {
                                tracing::warn!("no echo seen, falling back to unframed responses");
                                self.echo = false;
                            }
                        }
                        let _ = cur.reply.send(Err(AtError::Timeout));
                    }
                }
            }
        };
        read_task.abort();
        if let Some(cur) = self.current.take() {
            let _ = cur.reply.send(Err(AtError::Closed));
        }
        result
    }

    async fn start(&mut self, req: Request) -> anyhow::Result<()> {
        self.log(LogDir::Tx, &req.cmd);
        let mut data = req.cmd.clone().into_bytes();
        data.push(b'\r');
        self.writer.write_all(&data).await?;
        self.writer.flush().await?;
        self.current = Some(Running {
            cmd: req.cmd,
            echoed: false,
            payload: req.payload,
            deadline: Instant::now() + req.timeout,
            lines: Vec::new(),
            reply: req.reply,
        });
        Ok(())
    }

    fn drain_items(&mut self) -> Vec<Item> {
        let mut items = vec![];
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&raw).trim().to_string();
            if !line.is_empty() {
                items.push(Item::Line(line));
            }
        }
        // `> ` prompt arrives without a trailing newline
        let rest = String::from_utf8_lossy(&self.buf);
        if rest.trim() == ">" {
            self.buf.clear();
            items.push(Item::Prompt);
        }
        items
    }

    async fn handle_item(&mut self, item: Item) -> anyhow::Result<()> {
        let line = match item {
            Item::Prompt => {
                self.log(LogDir::Rx, ">");
                let payload = self.current.as_mut().and_then(|c| c.payload.take());
                match payload {
                    Some(p) => {
                        self.log(LogDir::Tx, &p);
                        let mut data = p.into_bytes();
                        data.push(0x1A);
                        self.writer.write_all(&data).await?;
                    }
                    // nothing to send: cancel the prompt
                    None => self.writer.write_all(&[0x1B]).await?,
                }
                self.writer.flush().await?;
                return Ok(());
            }
            Item::Line(l) => l,
        };

        if let Some(head) = self.pending_urc.take() {
            self.emit_urc(head, Some(line));
            return Ok(());
        }

        let Some(cur) = self.current.as_mut() else {
            if is_final(&line).is_some() {
                // late result of a timed-out command
                self.log(LogDir::Rx, &line);
            } else {
                self.on_urc(line);
            }
            return Ok(());
        };

        if !cur.echoed && line.eq_ignore_ascii_case(&cur.cmd) {
            cur.echoed = true;
            self.unechoed_timeouts = 0;
            return Ok(());
        }
        if !cur.echoed && line.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("AT")) {
            // echo of an earlier, timed-out command: the modem does echo, and everything up
            // to our own echo belongs to that command
            tracing::debug!(%line, cmd = %cur.cmd, "dropping stale echo");
            self.echo = true;
            return Ok(());
        }
        if is_urc_for(&line, &cur.cmd) {
            self.on_urc(line);
            return Ok(());
        }
        self.log(LogDir::Rx, &line);
        let echo = self.echo;
        let cur = self.current.as_mut().unwrap();
        if echo && !cur.echoed {
            tracing::debug!(%line, cmd = %cur.cmd, "dropping stale line received before echo");
            return Ok(());
        }
        match is_final(&line) {
            Some(Ok(())) => {
                let cur = self.current.take().unwrap();
                match cur.cmd.to_ascii_uppercase().as_str() {
                    "ATE1" => self.echo = true,
                    // ATZ / AT&F restore a profile whose echo setting we don't know
                    "ATE0" | "ATE" | "ATZ" | "AT&F" => self.echo = false,
                    _ => {}
                }
                let _ = cur.reply.send(Ok(Response { lines: cur.lines }));
            }
            Some(Err(e)) => {
                let cur = self.current.take().unwrap();
                let _ = cur.reply.send(Err(AtError::Error(e)));
            }
            None => cur.lines.push(line),
        }
        Ok(())
    }

    fn on_urc(&mut self, line: String) {
        if TWO_LINE_URCS.iter().any(|p| line.starts_with(p)) {
            self.log(LogDir::Urc, &line);
            self.pending_urc = Some(line);
            return;
        }
        self.emit_urc(line, None);
    }

    fn emit_urc(&mut self, line: String, payload: Option<String>) {
        match &payload {
            Some(p) => self.log(LogDir::Urc, p),
            None => self.log(LogDir::Urc, &line),
        }
        tracing::debug!(%line, "URC");
        let _ = self.urc.send(Urc { line, payload });
    }

    fn log(&self, dir: LogDir, line: &str) {
        tracing::trace!(?dir, line);
        self.events.send(Event::AtLog {
            dir,
            line: line.to_string(),
            ts: chrono::Local::now(),
        });
    }
}

const UNECHOED_TIMEOUTS_MAX: u8 = 3;

async fn sleep_until_opt(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending().await,
    }
}

fn is_final(line: &str) -> Option<Result<(), String>> {
    match line {
        "OK" => Some(Ok(())),
        "ERROR" | "NO CARRIER" | "NO ANSWER" | "BUSY" | "NO DIALTONE" => Some(Err(line.to_string())),
        _ if line.starts_with("+CME ERROR:") || line.starts_with("+CMS ERROR:") => {
            Some(Err(line.to_string()))
        }
        _ => None,
    }
}

/// A line is a URC while `cmd` runs if it carries a known URC prefix that
/// the command itself would not produce.
fn is_urc_for(line: &str, cmd: &str) -> bool {
    let Some(prefix) = URC_PREFIXES.iter().find(|p| line.starts_with(**p)) else {
        return false;
    };
    let name = prefix.trim_end_matches(':');
    let cmd = cmd.to_ascii_uppercase();
    if name.starts_with('+') || name.starts_with('^') {
        !cmd.contains(name)
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urc_detection() {
        assert!(!is_urc_for("+CPIN: READY", "AT+CPIN?"));
        assert!(is_urc_for("+CPIN: NOT READY", "AT+CSQ"));
        assert!(is_urc_for("+CMTI: \"SM\",3", "AT+CMGL=4"));
        assert!(!is_urc_for("+CSQ: 20,99", "AT+CSQ"));
        assert!(!is_urc_for("+CEREG: 2,1", "AT+CEREG?"));
    }

    #[tokio::test]
    async fn stale_ok_is_dropped_once_echo_is_on() {
        let (client_side, modem_side) = tokio::io::duplex(1024);
        let (r, w) = tokio::io::split(client_side);
        let (client, _h) = AtClient::spawn(r, w, EventBus::new());
        let (mut mr, mut mw) = tokio::io::split(modem_side);
        tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let _ = mr.read(&mut buf).await.unwrap();
            mw.write_all(b"ATE1\r\r\nOK\r\n").await.unwrap();
            let _ = mr.read(&mut buf).await.unwrap();
            // a leftover OK from some earlier command, then the real answer
            mw.write_all(b"\r\nOK\r\nAT+CSQ\r\r\n+CSQ: 18,99\r\n\r\nOK\r\n")
                .await
                .unwrap();
            let _ = mr.read(&mut buf).await;
        });
        client.cmd("ATE1").await.unwrap();
        let resp = client.cmd("AT+CSQ").await.unwrap();
        assert_eq!(resp.get("+CSQ:"), Some("18,99"));
    }

    #[tokio::test]
    async fn late_reply_after_timeout_does_not_shift_responses() {
        let (client_side, modem_side) = tokio::io::duplex(1024);
        let (r, w) = tokio::io::split(client_side);
        let (client, _h) = AtClient::spawn(r, w, EventBus::new());
        let (mut mr, mut mw) = tokio::io::split(modem_side);
        tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let _ = mr.read(&mut buf).await.unwrap();
            mw.write_all(b"ATE1\r\r\nOK\r\n").await.unwrap();
            // AT+COPS=? is echoed, then the modem stays busy past both timeouts
            let _ = mr.read(&mut buf).await.unwrap();
            mw.write_all(b"AT+COPS=?\r").await.unwrap();
            let _ = mr.read(&mut buf).await.unwrap(); // AT+CSQ, not echoed in time
            let _ = mr.read(&mut buf).await.unwrap(); // AT+CGATT?
            // late output: scan result, then the queued AT+CSQ, then AT+CGATT?
            mw.write_all(
                b"\r\n+COPS: (1,\"X\",\"X\",\"46001\",7)\r\n\r\nOK\r\n\
                  AT+CSQ\r\r\n+CSQ: 18,99\r\n\r\nOK\r\n\
                  AT+CGATT?\r\r\n+CGATT: 1\r\n\r\nOK\r\n",
            )
            .await
            .unwrap();
            let _ = mr.read(&mut buf).await;
        });
        client.cmd("ATE1").await.unwrap();
        let ms = Duration::from_millis(50);
        assert!(matches!(client.cmd_timeout("AT+COPS=?", ms).await, Err(AtError::Timeout)));
        assert!(matches!(client.cmd_timeout("AT+CSQ", ms).await, Err(AtError::Timeout)));
        let resp = client.cmd("AT+CGATT?").await.unwrap();
        assert_eq!(resp.lines, vec!["+CGATT: 1".to_string()]);
    }

    #[tokio::test]
    async fn command_roundtrip() {
        let (client_side, modem_side) = tokio::io::duplex(1024);
        let (r, w) = tokio::io::split(client_side);
        let bus = EventBus::new();
        let (client, _h) = AtClient::spawn(r, w, bus);
        let mut urcs = client.subscribe_urc();
        let (mut mr, mut mw) = tokio::io::split(modem_side);
        tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let n = mr.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"AT+CSQ\r");
            mw.write_all(b"AT+CSQ\r\r\n+CMTI: \"SM\",1\r\n+CSQ: 21,99\r\n\r\nOK\r\n")
                .await
                .unwrap();
            let n = mr.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"AT+CMGS=5\r");
            mw.write_all(b"\r\n> ").await.unwrap();
            let n = mr.read(&mut buf).await.unwrap();
            assert_eq!(&buf[..n], b"0011\x1a");
            mw.write_all(b"\r\n+CMGS: 9\r\n\r\nOK\r\n").await.unwrap();
            let _ = mr.read(&mut buf).await;
        });
        let resp = client.cmd("AT+CSQ").await.unwrap();
        assert_eq!(resp.get("+CSQ:"), Some("21,99"));
        assert_eq!(urcs.recv().await.unwrap().line, "+CMTI: \"SM\",1");
        let resp = client
            .send_full("AT+CMGS=5", Some("0011".into()), DEFAULT_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(resp.get("+CMGS:"), Some("9"));
    }
}
