//! 'winia inspect' — query a running application through its debug server.
//!
//! The debug server exposes a small text protocol over WebSocket. This command is a
//! thin client for identity, layout tree, input events, screenshots, and port
//! discovery; it does not start or stop applications.

use crate::cli::{InspectArgs, InspectCommand};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::BufWriter;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;
use tungstenite::{Message, WebSocket, client};

type Socket = WebSocket<TcpStream>;

pub fn run(args: InspectArgs) -> Result<()> {
    match args.command.unwrap_or(InspectCommand::Info) {
        InspectCommand::Info => show(args.port, args.json, "info"),
        InspectCommand::Tree => show(args.port, args.json, "t"),
        InspectCommand::Click { x, y } => command(args.port, args.json, format!("c {x} {y}")),
        InspectCommand::Key { keys } => {
            if keys.is_empty() {
                bail!("key requires at least one key name, e.g. 'winia inspect key Return'");
            }
            command(args.port, args.json, format!("k {}", keys.join(" ")))
        }
        InspectCommand::Scroll { values } => {
            let text = match values.as_slice() {
                [dy] => format!("s {dy}"),
                [dx, dy] => format!("s {dx} {dy}"),
                _ => bail!("scroll expects one value (dy) or two values (dx dy)"),
            };
            command(args.port, args.json, text)
        }
        InspectCommand::Drag {
            x1,
            y1,
            x2,
            y2,
            steps,
            delay_ms,
        } => drag(
            args.port,
            args.json,
            DragPath {
                x1,
                y1,
                x2,
                y2,
                steps,
                delay_ms,
            },
        ),
        InspectCommand::Screenshot { out } => screenshot(args.port, args.json, out),
        InspectCommand::Discover { end, scan } => {
            let start = args.port;
            discover(
                start,
                end.unwrap_or(start.saturating_add(100)),
                args.json,
                args.timeout,
                scan,
            )
        }
    }
}

fn connect(port: u16) -> Result<Socket> {
    connect_with_timeout(port, Duration::from_secs(10))
}

fn connect_with_timeout(port: u16, timeout: Duration) -> Result<Socket> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect_timeout(&address, timeout.min(Duration::from_secs(2)))
        .with_context(|| {
            format!("connecting to debug server at 127.0.0.1:{port} (is 'winia dev' running?)")
        })?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let (socket, _) = client(format!("ws://127.0.0.1:{port}"), stream)
        .map_err(|error| anyhow::anyhow!("WebSocket handshake failed: {error}"))?;
    Ok(socket)
}

fn send(socket: &mut Socket, command: &str) -> Result<Message> {
    socket
        .send(Message::text(command))
        .with_context(|| format!("sending '{command}'"))?;
    loop {
        match socket.read().context("reading debug server response")? {
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => bail!("debug server closed the connection"),
            other => return Ok(other),
        }
    }
}

fn text_response(message: Message) -> Result<String> {
    match message {
        Message::Text(text) => Ok(text.to_string()),
        other => bail!("expected a text response, got {other:?}"),
    }
}

fn show(port: u16, json: bool, command: &str) -> Result<()> {
    let mut socket = connect(port)?;
    let text = text_response(send(&mut socket, command)?)?;
    if json {
        println!("{text}");
        return Ok(());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        Err(_) => println!("{text}"),
    }
    Ok(())
}

fn command(port: u16, json: bool, command: String) -> Result<()> {
    let mut socket = connect(port)?;
    let response = text_response(send(&mut socket, &command)?)?;
    let ok = response.starts_with("ok");
    if json {
        println!("{}", serde_json::json!({"ok": ok, "response": response}));
    } else {
        println!("{response}");
    }
    if !ok {
        bail!("debug server rejected '{command}': {response}");
    }
    Ok(())
}

fn ensure_ok(response: &str, action: &str) -> Result<()> {
    if !response.starts_with("ok") {
        bail!("debug server rejected {action}: {response}");
    }
    Ok(())
}

struct DragPath {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    steps: u32,
    delay_ms: u64,
}

fn drag(port: u16, json: bool, path: DragPath) -> Result<()> {
    let DragPath {
        x1,
        y1,
        x2,
        y2,
        steps,
        delay_ms,
    } = path;
    if steps == 0 || steps > 1000 {
        bail!("drag steps must be between 1 and 1000");
    }
    let mut socket = connect(port)?;
    let down = text_response(send(&mut socket, &format!("d {x1} {y1}"))?)?;
    ensure_ok(&down, "pointer down")?;

    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let x = x1 + (x2 - x1) * t;
        let y = y1 + (y2 - y1) * t;
        let response = match send(&mut socket, &format!("m {x} {y}")) {
            Ok(message) => text_response(message),
            Err(error) => Err(error),
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                let _ = send(&mut socket, &format!("u {x} {y}"));
                return Err(error);
            }
        };
        if let Err(error) = ensure_ok(&response, "pointer move") {
            let _ = send(&mut socket, &format!("u {x} {y}"));
            return Err(error);
        }
        if delay_ms > 0 && step < steps {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }

    let up = text_response(send(&mut socket, &format!("u {x2} {y2}"))?)?;
    ensure_ok(&up, "pointer up")?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "steps": steps,
                "from": [x1, y1],
                "to": [x2, y2],
            })
        );
    } else {
        println!("ok drag ({x1}, {y1}) -> ({x2}, {y2}) in {steps} steps");
    }
    Ok(())
}

fn screenshot(port: u16, json: bool, out: Option<PathBuf>) -> Result<()> {
    let mut socket = connect(port)?;
    let response = text_response(send(&mut socket, "r")?)?;
    if !response.starts_with("screenshot ") {
        bail!("unexpected screenshot response: {response}");
    }
    let frame = send(&mut socket, "p")?;
    let bytes = match frame {
        Message::Binary(bytes) => bytes,
        other => bail!("expected a binary frame, got {other:?}"),
    };
    let (width, height, pixels, expected) = frame_parts(&bytes)?;
    if let Some((reported_width, reported_height)) = parse_screenshot_size(&response)
        && (reported_width != width || reported_height != height)
    {
        bail!(
            "screenshot response says {reported_width}x{reported_height}, frame is {width}x{height}"
        );
    }
    let rgba = unpremultiply(&pixels[..expected]);

    let path = out.unwrap_or_else(|| PathBuf::from(format!("inspect-{port}.png")));
    let file =
        std::fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().context("writing PNG header")?;
    writer
        .write_image_data(&rgba)
        .context("writing PNG pixels")?;
    drop(writer);

    if json {
        println!(
            "{}",
            serde_json::json!({
                "path": path,
                "width": width,
                "height": height,
            })
        );
    } else {
        println!("Saved {} ({width}x{height})", path.display());
    }
    Ok(())
}

fn parse_screenshot_size(response: &str) -> Option<(u32, u32)> {
    let rest = response.strip_prefix("screenshot ")?;
    let (width, height) = rest.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

fn unpremultiply(pixels: &[u8]) -> Vec<u8> {
    let mut output = pixels.to_vec();
    for pixel in output.as_chunks_mut::<4>().0 {
        let alpha = pixel[3] as u32;
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * 255) / alpha).min(255) as u8;
            }
        }
    }
    output
}

fn frame_parts(bytes: &[u8]) -> Result<(u32, u32, &[u8], usize)> {
    if bytes.len() < 8 {
        bail!("frame header is only {} bytes", bytes.len());
    }
    let width = u32::from_le_bytes(bytes[0..4].try_into().expect("length checked"));
    let height = u32::from_le_bytes(bytes[4..8].try_into().expect("length checked"));
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("frame dimensions are too large: {width}x{height}"))?;
    let pixels = &bytes[8..];
    if pixels.len() < expected {
        bail!(
            "frame has {} pixel bytes, expected {expected} for {width}x{height}",
            pixels.len()
        );
    }
    Ok((width, height, pixels, expected))
}

fn discover(start: u16, end: u16, json: bool, timeout_ms: u64, scan: bool) -> Result<()> {
    if start == 0 || end < start {
        bail!("invalid discovery range {start}..={end}");
    }
    let timeout = Duration::from_millis(timeout_ms.max(50));
    let mut entries: BTreeMap<u16, Value> = BTreeMap::new();

    if scan {
        for port in start..=end {
            if let Some(value) = probe(port, timeout) {
                entries.insert(port, value);
            }
        }
    } else {
        for (port, value) in registry_entries(timeout)? {
            entries.insert(port, value);
        }
    }

    let found: Vec<_> = entries.into_iter().collect();
    if json {
        let values: Vec<_> = found
            .iter()
            .map(|(port, value)| {
                serde_json::json!({
                    "port": port,
                    "app": value.get("app"),
                    "version": value.get("version"),
                    "id": value.get("id"),
                    "target": value.get("target"),
                    "platform": value.get("platform"),
                    "arch": value.get("arch"),
                    "pid": value.get("pid"),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
        return Ok(());
    }

    if found.is_empty() {
        if scan {
            println!("No debug servers found in {start}..={end}.");
        } else {
            println!(
                "No registered debug servers in target/winia-dev; run 'winia dev' in this project, or pass --scan to search {start}..={end}."
            );
        }
        return Ok(());
    }
    println!(
        "{:<6} {:<24} {:<10} {:<28} {:<10} PID",
        "PORT", "APP", "VERSION", "ID", "TARGET"
    );
    for (port, value) in found {
        let field = |name: &str| {
            value
                .get(name)
                .and_then(|value| value.as_str())
                .unwrap_or("-")
        };
        let pid = value.get("pid").and_then(|value| value.as_u64());
        println!(
            "{:<6} {:<24} {:<10} {:<28} {:<10} {}",
            port,
            field("app"),
            field("version"),
            field("id"),
            field("target"),
            pid.map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned())
        );
    }
    Ok(())
}

fn registry_entries(timeout: Duration) -> Result<Vec<(u16, Value)>> {
    let directory = PathBuf::from("target/winia-dev");
    let mut ports = Vec::new();
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Ok(Vec::new());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("inspect-port-") || !name.ends_with(".json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(port) = value.get("port").and_then(|value| value.as_u64()) else {
            continue;
        };
        if let Ok(port) = u16::try_from(port) {
            ports.push((port, value));
        }
    }
    let mut found = Vec::new();
    for (port, _) in ports {
        if let Some(value) = probe(port, timeout) {
            found.push((port, value));
        }
    }
    Ok(found)
}

fn probe(port: u16, timeout: Duration) -> Option<Value> {
    let mut socket = connect_with_timeout(port, timeout).ok()?;
    let message = send(&mut socket, "info").ok()?;
    let text = text_response(message).ok()?;
    serde_json::from_str::<Value>(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frame_header_and_pixels() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[255, 0, 0, 255, 0, 255, 0, 255]);
        let (width, height, pixels, expected) = frame_parts(&bytes).unwrap();
        assert_eq!((width, height), (2, 1));
        assert_eq!(expected, 8);
        assert_eq!(pixels.len(), 8);
    }

    #[test]
    fn unpremultiplies_alpha_for_png() {
        assert_eq!(unpremultiply(&[128, 0, 0, 128]), vec![255, 0, 0, 128]);
        assert_eq!(unpremultiply(&[0, 0, 0, 0]), vec![0, 0, 0, 0]);
        assert_eq!(unpremultiply(&[20, 30, 40, 255]), vec![20, 30, 40, 255]);
    }

    #[test]
    fn parses_screenshot_dimensions() {
        assert_eq!(
            parse_screenshot_size("screenshot 1500x1050"),
            Some((1500, 1050))
        );
        assert_eq!(parse_screenshot_size("no frame"), None);
    }

    #[test]
    fn rejects_truncated_frames() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 0]);
        assert!(frame_parts(&bytes).is_err());
    }
}
