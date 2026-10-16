//! Lighting transport for macOS.
//!
//! macOS attaches no HID driver to the QuadCast S lighting interface
//! (03F0:028C, interface 0), so the reports go as USB HID class
//! SET_REPORT(Feature) control requests straight to that interface:
//! bmRequestType 0x21, bRequest 0x09, wValue 0x0300, wIndex 0, 64 bytes.
//! The payload is identical to the HidD_SetFeature reports verified on
//! the real microphone from Windows. Status reports from the mic arrive
//! on the same interface's interrupt IN endpoint.

use crate::protocol::{Report, LIGHT_PRODUCT_ID, REPORT_GAP_MS, VENDOR_ID};
use rusb::{DeviceHandle, Direction, GlobalContext, TransferType};
use std::thread::sleep;
use std::time::Duration;

/// Other product IDs used by QuadCast S lighting controllers.
const ALT_PRODUCT_IDS: [u16; 1] = [0x098C];
const INTERFACE: u8 = 0;

pub struct Light {
    handle: DeviceHandle<GlobalContext>,
    claimed: bool,
    in_endpoint: Option<u8>,
    pub product: String,
}

impl Light {
    pub fn open() -> Result<Light, String> {
        let devices = rusb::devices().map_err(|e| format!("cannot list USB devices: {}", e))?;
        for dev in devices.iter() {
            let Ok(desc) = dev.device_descriptor() else { continue };
            let pid = desc.product_id();
            if desc.vendor_id() != VENDOR_ID || !(pid == LIGHT_PRODUCT_ID || ALT_PRODUCT_IDS.contains(&pid)) {
                continue;
            }
            let mut in_endpoint = None;
            if let Ok(cfg) = dev.active_config_descriptor() {
                for iface in cfg.interfaces().filter(|i| i.number() == INTERFACE) {
                    for alt in iface.descriptors() {
                        for ep in alt.endpoint_descriptors() {
                            if ep.direction() == Direction::In && ep.transfer_type() == TransferType::Interrupt {
                                in_endpoint = Some(ep.address());
                            }
                        }
                    }
                }
            }
            let handle = dev.open().map_err(|e| format!("found the mic's lighting controller but could not open it: {}", e))?;
            let claimed = handle.claim_interface(INTERFACE).is_ok();
            let product = handle
                .read_product_string_ascii(&desc)
                .unwrap_or_else(|_| "HyperX QuadCast S".into());
            return Ok(Light { handle, claimed, in_endpoint, product });
        }
        Err("QuadCast S lighting controller (03F0:028C) not found. Is the mic plugged in?".into())
    }

    /// Sends one 65-byte report (byte 0 = report ID 0, not transmitted).
    pub fn send(&self, r: &Report) -> Result<(), String> {
        let value = 0x0300 | r[0] as u16; // Feature report, ID 0
        self.handle
            .write_control(0x21, 0x09, value, INTERFACE as u16, &r[1..], Duration::from_millis(1000))
            .map(|_| ())
            .map_err(|e| format!("the mic rejected a lighting command: {}", e))
    }

    pub fn send_all(&self, reports: &[Report]) -> Result<(), String> {
        for r in reports {
            self.send(r)?;
            sleep(Duration::from_millis(REPORT_GAP_MS));
        }
        Ok(())
    }

    /// Description of the status channel, for diagnostics.
    pub fn status_channel(&self) -> String {
        match self.in_endpoint {
            Some(ep) => format!("interrupt IN endpoint 0x{:02X} (interface claimed: {})", ep, self.claimed),
            None => "no interrupt IN endpoint found".into(),
        }
    }

    /// Waits up to `timeout` for one status report from the mic.
    pub fn read_status(&self, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
        let Some(ep) = self.in_endpoint else { return Err("no status endpoint".into()) };
        let mut buf = [0u8; 64];
        match self.handle.read_interrupt(ep, &mut buf, timeout) {
            Ok(n) => Ok(Some(buf[..n].to_vec())),
            Err(rusb::Error::Timeout) => Ok(None),
            Err(e) => Err(format!("status read failed: {}", e)),
        }
    }

    /// Asks the mic for a feature report (GET_REPORT), for diagnostics.
    pub fn get_feature(&self) -> Result<Vec<u8>, String> {
        self.get_report(0x0300)
    }

    /// GET_REPORT with the given wValue (0x0100 input, 0x0300 feature).
    pub fn get_report(&self, value: u16) -> Result<Vec<u8>, String> {
        let mut buf = [0u8; 64];
        self.handle
            .read_control(0xA1, 0x01, value, INTERFACE as u16, &mut buf, Duration::from_millis(500))
            .map(|n| buf[..n].to_vec())
            .map_err(|e| format!("{}", e))
    }
}

impl Drop for Light {
    fn drop(&mut self) {
        if self.claimed {
            let _ = self.handle.release_interface(INTERFACE);
        }
    }
}
