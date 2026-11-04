//! 'winia inspect' — query a running application through its debug server.
//!
//! The debug server exposes a small text protocol over WebSocket. This command is a
//! thin client for identity, layout tree, input events, screenshots, and port
//! discovery; it does not start or stop applications.

use crate::cli::{InspectArgs, InspectCommand};
use anyhow::{Context, Result, bail};
use serde_json::Value;
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
        InspectCommand::Screenshot { out } => screenshot(args.port, args.json, out),
        InspectCommand::Discover { end } => {
            let start = args.port;
            discover(start, end.unwrap_or(start.saturating_add(100)), args.json)
        }
    }
}

fn connect(port: u16) -> Result<Socket> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let stream =
        TcpStream::connect_timeout(&address, Duration::from_millis(500)).with_context(|| {
            format!("connecting to debug server at 127.0.0.1:{port} (is 'winia dev' running?)")
        })?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
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
    if json {
        println!("{}", serde_json::json!({"ok": true, "response": response}));
    } else {
        println!("{response}");
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

    let path = out.unwrap_or_else(|| PathBuf::from(format!("inspect-{port}.png")));
    let file =
        std::fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().context("writing PNG header")?;
    writer
        .write_image_data(&pixels[..expected])
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

fn discover(start: u16, end: u16, json: bool) -> Result<()> {
    if start == 0 || end < start {
        bail!("invalid discovery range {start}..={end}");
    }
    let mut found = Vec::new();
    for port in start..=end {
        let Ok(mut socket) = connect(port) else {
            continue;
        };
        let Ok(message) = send(&mut socket, "info") else {
            continue;
        };
        let Ok(text) = text_response(message) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            found.push((port, value));
        }
    }

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
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
        return Ok(());
    }

    if found.is_empty() {
        println!("No debug servers found in {start}..={end}.");
        return Ok(());
    }
    println!(
        "{:<6} {:<24} {:<10} {:<28} TARGET",
        "PORT", "APP", "VERSION", "ID"
    );
    for (port, value) in found {
        let field = |name: &str| {
            value
                .get(name)
                .and_then(|value| value.as_str())
                .unwrap_or("-")
        };
        println!(
            "{:<6} {:<24} {:<10} {:<28} {}",
            port,
            field("app"),
            field("version"),
            field("id"),
            field("target")
        );
    }
    Ok(())
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
    fn rejects_truncated_frames() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 0]);
        assert!(frame_parts(&bytes).is_err());
    }
}
