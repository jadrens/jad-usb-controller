use anyhow::{Context, Result, bail};
use serde::Serialize;
use tokio_serial::{SerialPortBuilderExt, SerialPortType};

use super::{Transport, probe, vendor_name};
use crate::config::Config;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialCandidate {
    pub path: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub product: Option<String>,
    pub known_vendor: Option<String>,
}

pub fn scan() -> Vec<SerialCandidate> {
    let mut out: Vec<SerialCandidate> = tokio_serial::available_ports()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            p.port_name.contains("ttyUSB") || p.port_name.contains("ttyACM") || !cfg!(target_os = "linux")
        })
        .map(|p| {
            let (vid, pid, product) = match &p.port_type {
                SerialPortType::UsbPort(u) => (Some(u.vid), Some(u.pid), u.product.clone()),
                _ => (None, None, None),
            };
            SerialCandidate {
                path: p.port_name,
                vid,
                pid,
                product,
                known_vendor: vid.and_then(vendor_name).map(str::to_string),
            }
        })
        .collect();
    // known modem vendors first
    out.sort_by_key(|c| (c.known_vendor.is_none(), c.path.clone()));
    out
}

pub async fn open(cfg: &Config) -> Result<Transport> {
    let paths: Vec<String> = match &cfg.serial_port {
        Some(p) => vec![p.clone()],
        None => scan().into_iter().map(|c| c.path).collect(),
    };
    if paths.is_empty() {
        bail!("no serial ports found");
    }
    let mut errors = vec![];
    for path in paths {
        let stream = match tokio_serial::new(&path, cfg.baud)
            .open_native_async()
            .with_context(|| format!("open {path}"))
        {
            Ok(s) => s,
            Err(e) => {
                errors.push(format!("{e:#}"));
                continue;
            }
        };
        let (mut r, mut w) = tokio::io::split(stream);
        if probe(&mut r, &mut w).await {
            tracing::info!(%path, "AT port found");
            return Ok(Transport {
                reader: Box::new(r),
                writer: Box::new(w),
                description: format!("serial {path} @ {}", cfg.baud),
            });
        }
        errors.push(format!("{path}: no AT response"));
    }
    bail!("{}", errors.join("; "))
}
