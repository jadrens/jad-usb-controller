use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TransportKind {
    /// Try direct USB access first, then serial ports
    Auto,
    /// Talk to the modem's USB bulk endpoints directly (no kernel driver needed)
    Usb,
    /// Use a serial device such as /dev/ttyUSB2
    Serial,
    /// Simulated modem for development
    Mock,
}

#[derive(Debug, Clone, Parser)]
#[command(version, about = "USB 4G modem / IoT SIM card manager")]
pub struct Config {
    /// HTTP listen address
    #[arg(long, env = "USB_CARD_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: SocketAddr,

    /// How to reach the modem
    #[arg(long, env = "USB_CARD_TRANSPORT", value_enum, default_value_t = TransportKind::Auto)]
    pub transport: TransportKind,

    /// Serial AT port; probed automatically when omitted
    #[arg(long, env = "USB_CARD_SERIAL_PORT")]
    pub serial_port: Option<String>,

    #[arg(long, env = "USB_CARD_BAUD", default_value_t = 115200)]
    pub baud: u32,

    /// USB vendor id (hex), e.g. 2c7c; known modem vendors are matched when omitted
    #[arg(long, env = "USB_CARD_USB_VID", value_parser = parse_hex)]
    pub usb_vid: Option<u16>,

    /// USB product id (hex)
    #[arg(long, env = "USB_CARD_USB_PID", value_parser = parse_hex)]
    pub usb_pid: Option<u16>,

    /// USB interface number of the AT port; probed automatically when omitted
    #[arg(long, env = "USB_CARD_USB_INTERFACE")]
    pub usb_interface: Option<u8>,

    /// SQLite database file for messages
    #[arg(long, env = "USB_CARD_DB", default_value = "usb-card.db")]
    pub db: PathBuf,

    /// Status polling interval in seconds
    #[arg(long, env = "USB_CARD_POLL_SECS", default_value_t = 10)]
    pub poll_secs: u64,

    /// Keep received messages on the SIM after storing them (default: delete to free space)
    #[arg(long, env = "USB_CARD_KEEP_SMS_ON_SIM")]
    pub keep_sms_on_sim: bool,

    /// PDP context id used for data
    #[arg(long, env = "USB_CARD_DATA_CID", default_value_t = 1)]
    pub data_cid: u8,
}

fn parse_hex(s: &str) -> Result<u16, String> {
    u16::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| e.to_string())
}
