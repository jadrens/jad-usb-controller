//! Direct USB access to the modem's vendor-specific AT interfaces.
//!
//! This bypasses the kernel `option` serial driver, so it works even when no
//! /dev/ttyUSB* node exists. Requires write access to /dev/bus/usb/* (udev rule).

use anyhow::{Result, bail};
use nusb::DeviceInfo;
use nusb::descriptors::TransferType;
use nusb::transfer::{Bulk, Direction, In, Out};
use serde::Serialize;

use super::{KNOWN_VENDORS, Transport, probe, vendor_name};
use crate::config::Config;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbCandidate {
    pub vid: u16,
    pub pid: u16,
    pub bus: String,
    pub address: u8,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub known_vendor: Option<String>,
    pub interfaces: Vec<UsbInterface>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbInterface {
    pub number: u8,
    pub class: u8,
    pub name: Option<String>,
}

struct AtCandidate {
    interface: u8,
    ep_in: u8,
    ep_out: u8,
}

fn matches(cfg: &Config, d: &DeviceInfo) -> bool {
    match (cfg.usb_vid, cfg.usb_pid) {
        (Some(v), Some(p)) => d.vendor_id() == v && d.product_id() == p,
        (Some(v), None) => d.vendor_id() == v,
        (None, Some(p)) => d.product_id() == p,
        (None, None) => KNOWN_VENDORS.iter().any(|(v, _)| *v == d.vendor_id()),
    }
}

pub async fn scan() -> Vec<UsbCandidate> {
    let Ok(list) = nusb::list_devices().await else {
        return vec![];
    };
    list.filter(|d| vendor_name(d.vendor_id()).is_some())
        .map(|d| UsbCandidate {
            vid: d.vendor_id(),
            pid: d.product_id(),
            bus: d.bus_id().to_string(),
            address: d.device_address(),
            manufacturer: d.manufacturer_string().map(str::to_string),
            product: d.product_string().map(str::to_string),
            known_vendor: vendor_name(d.vendor_id()).map(str::to_string),
            interfaces: d
                .interfaces()
                .map(|i| UsbInterface {
                    number: i.interface_number(),
                    class: i.class(),
                    name: i.interface_string().map(str::to_string),
                })
                .collect(),
        })
        .collect()
}

/// Vendor-specific interfaces with a bulk IN/OUT pair, best guess first:
/// interfaces named like an AT port, then those with an interrupt endpoint
/// (modem status notifications, typical for AT ports). Diagnostic ports are skipped.
fn at_candidates(info: &DeviceInfo, dev: &nusb::Device, only: Option<u8>) -> Vec<AtCandidate> {
    let Ok(config) = dev.active_configuration() else {
        return vec![];
    };
    let name_of = |n: u8| {
        info.interfaces()
            .find(|i| i.interface_number() == n)
            .and_then(|i| i.interface_string())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let mut out: Vec<(u8, AtCandidate)> = vec![];
    for group in config.interfaces() {
        let alt = group.first_alt_setting();
        let number = alt.interface_number();
        let name = name_of(number);
        if let Some(n) = only {
            if n != number {
                continue;
            }
        } else if alt.class() != 0xFF || name.contains("diag") || name.contains("nmea") {
            continue;
        }
        let mut ep_in = None;
        let mut ep_out = None;
        let mut has_interrupt = false;
        for ep in alt.endpoints() {
            match (ep.transfer_type(), ep.direction()) {
                (TransferType::Bulk, Direction::In) => ep_in = ep_in.or(Some(ep.address())),
                (TransferType::Bulk, Direction::Out) => ep_out = ep_out.or(Some(ep.address())),
                (TransferType::Interrupt, _) => has_interrupt = true,
                _ => {}
            }
        }
        if let (Some(ep_in), Some(ep_out)) = (ep_in, ep_out) {
            let rank = if name.split_whitespace().any(|w| w == "at") {
                0
            } else if has_interrupt {
                1
            } else {
                2
            };
            out.push((
                rank,
                AtCandidate {
                    interface: number,
                    ep_in,
                    ep_out,
                },
            ));
        }
    }
    out.sort_by_key(|(rank, c)| (*rank, c.interface));
    out.into_iter().map(|(_, c)| c).collect()
}

pub async fn open(cfg: &Config) -> Result<Transport> {
    let devices: Vec<DeviceInfo> = nusb::list_devices().await?.filter(|d| matches(cfg, d)).collect();
    if devices.is_empty() {
        bail!("no USB modem found");
    }
    let mut errors = vec![];
    for info in devices {
        let id = format!("{:04x}:{:04x}", info.vendor_id(), info.product_id());
        let dev = match info.open().await {
            Ok(d) => d,
            Err(e) => {
                errors.push(format!("open {id}: {e} (check udev permissions)"));
                continue;
            }
        };
        for c in at_candidates(&info, &dev, cfg.usb_interface) {
            let iface = match dev.detach_and_claim_interface(c.interface).await {
                Ok(i) => i,
                Err(e) => {
                    errors.push(format!("{id} claim interface {}: {e}", c.interface));
                    continue;
                }
            };
            let (mut reader, mut writer) = match (
                iface.endpoint::<Bulk, In>(c.ep_in),
                iface.endpoint::<Bulk, Out>(c.ep_out),
            ) {
                (Ok(i), Ok(o)) => (i.reader(4096).with_num_transfers(4), o.writer(4096)),
                (Err(e), _) | (_, Err(e)) => {
                    errors.push(format!("{id} interface {}: {e}", c.interface));
                    continue;
                }
            };
            if probe(&mut reader, &mut writer).await {
                let description = format!(
                    "usb {id} {} interface {}",
                    vendor_name(info.vendor_id()).unwrap_or(""),
                    c.interface
                );
                tracing::info!(%description, "AT port found");
                return Ok(Transport {
                    reader: Box::new(reader),
                    writer: Box::new(writer),
                    description,
                });
            }
            errors.push(format!("{id} interface {}: no AT response", c.interface));
        }
    }
    bail!("{}", errors.join("; "))
}
