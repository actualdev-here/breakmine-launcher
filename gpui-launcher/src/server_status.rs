use base64::Engine;
use std::{
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};
use tungstenite::{client_tls_with_config, protocol::WebSocketConfig, Message};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub latency_ms: u64,
    pub players: Option<u64>,
    pub max_players: Option<u64>,
    pub player_names: Vec<String>,
    pub icon: Option<(String, Vec<u8>)>,
}
const TIMEOUT: Duration = Duration::from_millis(2500);

fn websocket_urls(address: &str) -> Vec<String> {
    let address = address.trim();
    if address.is_empty() {
        return Vec::new();
    }
    if let Some(rest) = address.strip_prefix("https://") {
        vec![format!("wss://{rest}")]
    } else if let Some(rest) = address.strip_prefix("http://") {
        vec![format!("ws://{rest}")]
    } else if address.starts_with("ws://") || address.starts_with("wss://") {
        vec![address.into()]
    } else if address.contains("://") {
        Vec::new()
    } else {
        vec![format!("wss://{address}"), format!("ws://{address}")]
    }
}

fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    let addrs = (host, port).to_socket_addrs().map_err(|e| e.to_string())?;
    for addr in addrs.take(4) {
        if let Ok(stream) = TcpStream::connect_timeout(&addr, TIMEOUT) {
            stream
                .set_read_timeout(Some(TIMEOUT))
                .map_err(|e| e.to_string())?;
            stream
                .set_write_timeout(Some(TIMEOUT))
                .map_err(|e| e.to_string())?;
            return Ok(stream);
        }
    }
    Err("No response".into())
}
fn query_websocket(address: &str) -> Result<Status, String> {
    let url = Url::parse(address).map_err(|e| e.to_string())?;
    let host = url.host_str().ok_or("Missing host")?;
    let port = url
        .port()
        .unwrap_or(if url.scheme() == "wss" { 443 } else { 80 });
    let stream = connect_tcp(host, port)?;
    let config = WebSocketConfig::default()
        .read_buffer_size(4096)
        .max_message_size(Some(256 * 1024))
        .max_frame_size(Some(256 * 1024));
    let (mut socket, _) = client_tls_with_config(url.as_str(), stream, Some(config), None)
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    socket
        .send(Message::Text("ping".into()))
        .map_err(|e| e.to_string())?;
    loop {
        if start.elapsed() > TIMEOUT {
            return Err("Server status timed out".into());
        }
        let message = socket.read().map_err(|e| e.to_string())?;
        let text = match message {
            Message::Text(text) => Some(text.to_string()),
            Message::Binary(_) => None,
            Message::Close(_) => return Err("Server closed the connection".into()),
            _ => continue,
        };
        let payload: serde_json::Value = text
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(serde_json::Value::Null);
        let names = payload["playerNames"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
            .filter(|name| !name.is_empty() && name.chars().count() <= 32)
            .take(100)
            .map(str::to_string)
            .collect();
        let icon = payload["icon"].as_str().and_then(decode_icon);
        return Ok(Status {
            latency_ms: start.elapsed().as_millis().max(1) as u64,
            players: payload["players"].as_u64(),
            max_players: payload["maxPlayers"].as_u64(),
            player_names: names,
            icon,
        });
    }
}
fn decode_icon(icon: &str) -> Option<(String, Vec<u8>)> {
    let (header, data) = icon.split_once(',')?;
    let format = match header {
        "data:image/png;base64" => "png",
        "data:image/jpeg;base64" => "jpeg",
        "data:image/webp;base64" => "webp",
        "data:image/gif;base64" => "gif",
        _ => return None,
    };
    if data.len() > 128 * 1024 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    let image_format = match format {
        "png" => image::ImageFormat::Png,
        "jpeg" => image::ImageFormat::Jpeg,
        "webp" => image::ImageFormat::WebP,
        "gif" => image::ImageFormat::Gif,
        _ => return None,
    };
    let (width, height) =
        image::ImageReader::with_format(std::io::Cursor::new(&bytes), image_format)
            .into_dimensions()
            .ok()?;
    if width == 0 || height == 0 || width > 512 || height > 512 {
        return None;
    }
    Some((format.to_string(), bytes))
}

pub fn query(address: &str, protocol: &str) -> Result<Status, String> {
    if protocol == "json" {
        let raw = address.trim();
        let url = if raw.contains("://") {
            Url::parse(raw)
        } else {
            Url::parse(&format!("tcp://{raw}"))
        }
        .map_err(|e| e.to_string())?;
        let host = url.host_str().ok_or("Missing host")?;
        let port = url.port().unwrap_or(match url.scheme() {
            "https" => 443,
            "http" => 80,
            _ => 25565,
        });
        let start = Instant::now();
        connect_tcp(host, port)?;
        return Ok(Status {
            latency_ms: start.elapsed().as_millis().max(1) as u64,
            players: None,
            max_players: None,
            player_names: Vec::new(),
            icon: None,
        });
    }
    for url in websocket_urls(address) {
        if let Ok(status) = query_websocket(&url) {
            return Ok(status);
        }
    }
    Err("No response".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};
    #[test]
    fn websocket_ping_reads_server_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            assert_eq!(socket.read().unwrap().into_text().unwrap(), "ping");
            socket
                .send(Message::Text(
                    r#"{"players":2,"maxPlayers":20,"playerNames":["Alex","Steve"],"icon":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/WZ8AAAAASUVORK5CYII="}"#.into(),
                ))
                .unwrap();
        });
        let status = query(&format!("ws://{addr}"), "binary").unwrap();
        assert_eq!((status.players, status.max_players), (Some(2), Some(20)));
        assert_eq!(status.player_names, ["Alex", "Steve"]);
        assert_eq!(
            status.icon.as_ref().map(|(format, _)| format.as_str()),
            Some("png")
        );
        server.join().unwrap();
    }
    #[test]
    fn normalizes_addresses_and_rejects_unknown_schemes() {
        assert_eq!(
            websocket_urls("example.com:123"),
            ["wss://example.com:123", "ws://example.com:123"]
        );
        assert_eq!(
            websocket_urls("https://example.com/x"),
            ["wss://example.com/x"]
        );
        assert!(websocket_urls("file:///tmp/example").is_empty());
        assert!(decode_icon("data:text/html;base64,AQID").is_none());
        assert!(decode_icon("data:image/png;base64,invalid!").is_none());
    }
}
