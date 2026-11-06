//! 'winia devices' lists the Android devices adb can see.
//!
//! The output mirrors 'adb devices -l' but with stable columns, so scripts can parse the table and
//! CI can read the JSON form.

use crate::cli::DevicesArgs;
use crate::commands::run;
use anyhow::Result;

pub fn run(args: DevicesArgs) -> Result<()> {
    let output = run::adb(&["devices", "-l"])?;
    let devices = parse(&output);
    if args.json {
        let values: Vec<_> = devices
            .iter()
            .map(|device| {
                serde_json::json!({
                    "serial": device.serial,
                    "state": device.state,
                    "model": device.model,
                    "product": device.product,
                    "transport_id": device.transport_id,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "devices": values }))?
        );
        return Ok(());
    }
    if devices.is_empty() {
        println!("No devices found. Start an emulator or connect a device with adb.");
        return Ok(());
    }
    println!("{:<24} {:<10} {:<24} PRODUCT", "SERIAL", "STATE", "MODEL");
    for device in &devices {
        println!(
            "{:<24} {:<10} {:<24} {}",
            device.serial,
            device.state,
            device.model.as_deref().unwrap_or("-"),
            device.product.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub struct Device {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
    pub product: Option<String>,
    pub transport_id: Option<String>,
}

/// Parse the 'adb devices -l' listing: header, one line per device, blank line at the end.
pub fn parse(output: &str) -> Vec<Device> {
    let mut devices = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("List of devices") || line.starts_with('*') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(serial) = fields.next() else {
            continue;
        };
        // "no permissions (user ...)" is a two-word state; taking one token reports "no".
        let rest = &line[serial.len()..];
        let state = if rest.trim_start().starts_with("no permissions") {
            "no permissions".to_owned()
        } else {
            match fields.next() {
                Some(state) => state.to_owned(),
                None => continue,
            }
        };
        let mut model = None;
        let mut product = None;
        let mut transport_id = None;
        for field in fields {
            let Some((key, value)) = field.split_once(':') else {
                continue;
            };
            match key {
                "model" => model = Some(value.to_owned()),
                "product" => product = Some(value.to_owned()),
                "transport_id" => transport_id = Some(value.to_owned()),
                _ => {}
            }
        }
        devices.push(Device {
            serial: serial.to_owned(),
            state: state.to_owned(),
            model,
            product,
            transport_id,
        });
    }
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_long_listing() {
        let output = "List of devices attached\n\
            emulator-5554          device product:sdk_gphone64_x86_64 model:sdk_gphone64_x86_64 device:emu64xa transport_id:3\n\
            192.0.0.172:5555       offline transport_id:7\n\n";
        let devices = parse(output);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].serial, "emulator-5554");
        assert_eq!(devices[0].state, "device");
        assert_eq!(devices[0].model.as_deref(), Some("sdk_gphone64_x86_64"));
        assert_eq!(devices[0].transport_id.as_deref(), Some("3"));
        assert_eq!(devices[1].state, "offline");
        assert!(devices[1].model.is_none());
    }

    #[test]
    fn ignores_the_header_and_blank_lines() {
        assert!(parse("List of devices attached\n\n").is_empty());
    }
}
