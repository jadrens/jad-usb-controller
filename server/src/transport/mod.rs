//! Ways to reach the modem's AT port.

pub mod mock;
pub mod serial;
pub mod usb;

use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::config::{Config, TransportKind};

pub type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
pub type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

pub struct Transport {
    pub reader: BoxRead,
    pub writer: BoxWrite,
    pub description: String,
}

/// USB vendor ids of common cellular module makers.
pub const KNOWN_VENDORS: &[(u16, &str)] = &[
    (0x2c7c, "Quectel"),
    (0x05c6, "Qualcomm"),
    (0x1e0e, "SIMCom"),
    (0x12d1, "Huawei"),
    (0x19d2, "ZTE"),
    (0x2cb7, "Fibocom"),
    (0x1508, "Fibocom"),
    (0x2dee, "MeiG"),
    (0x1c9e, "Longsung"),
    (0x305a, "GosunCN"),
    (0x1bc7, "Telit"),
    (0x1199, "Sierra Wireless"),
    (0x1782, "Unisoc"),
    (0x3c93, "Neoway"),
];

pub fn vendor_name(vid: u16) -> Option<&'static str> {
    KNOWN_VENDORS.iter().find(|(v, _)| *v == vid).map(|(_, n)| *n)
}

pub async fn open(cfg: &Config) -> Result<Transport> {
    match cfg.transport {
        TransportKind::Mock => Ok(mock::open()),
        TransportKind::Serial => serial::open(cfg).await,
        TransportKind::Usb => usb::open(cfg).await,
        TransportKind::Auto => {
            if cfg.serial_port.is_some() {
                return serial::open(cfg).await;
            }
            let usb_err = match usb::open(cfg).await {
                Ok(t) => return Ok(t),
                Err(e) => e,
            };
            match serial::open(cfg).await {
                Ok(t) => Ok(t),
                Err(serial_err) => bail!("usb: {usb_err:#}; serial: {serial_err:#}"),
            }
        }
    }
}

/// Check that an AT interpreter answers on this port.
pub async fn probe<R, W>(reader: &mut R, writer: &mut W) -> bool
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 512];
    for _ in 0..3 {
        if writer.write_all(b"AT\r").await.is_err() || writer.flush().await.is_err() {
            return false;
        }
        let mut got = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(700);
        while let Ok(Ok(n)) = tokio::time::timeout_at(deadline, reader.read(&mut buf)).await {
            if n == 0 {
                return false;
            }
            got.extend_from_slice(&buf[..n]);
            if got.windows(2).any(|w| w == b"OK") {
                // swallow answers to earlier probes so they can't be mistaken
                // for the response to the first real command
                drain(reader).await;
                return true;
            }
        }
    }
    false
}

/// Read and discard input until the line has been quiet for a moment.
async fn drain<R: AsyncRead + Unpin>(reader: &mut R) {
    let mut buf = vec![0u8; 512];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(300), reader.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => continue,
            _ => break,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceListing {
    pub usb: Vec<usb::UsbCandidate>,
    pub serial: Vec<serial::SerialCandidate>,
}

pub async fn scan() -> DeviceListing {
    DeviceListing {
        usb: usb::scan().await,
        serial: serial::scan(),
    }
}
