//! Separate game WebView process. Launch configuration is read from stdin, not argv.
mod telemetry;

use serde::Deserialize;
use std::{
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use url::Url;
use wry::{WebContext, WebViewBuilder};

#[derive(Deserialize)]
struct Config {
    url: String,
    title: String,
    profile: PathBuf,
    exports: PathBuf,
    init_script: String,
    width: u32,
    height: u32,
}
#[derive(Clone)]
struct NavigationPolicy {
    url: Url,
}
impl NavigationPolicy {
    fn new(value: &str) -> Result<Self, String> {
        let url = Url::parse(value).map_err(|_| "Invalid game URL")?;
        if !["https", "http", "file"].contains(&url.scheme())
            || (url.scheme() != "file" && url.host_str().is_none())
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Invalid game URL".into());
        }
        Ok(Self { url })
    }
    fn allows(&self, value: &str) -> bool {
        let Ok(url) = Url::parse(value) else {
            return false;
        };
        if self.url.scheme() == "file" {
            url.scheme() == "file"
                && url.host_str() == self.url.host_str()
                && url.path() == self.url.path()
        } else {
            url.scheme() == self.url.scheme()
                && url.host_str() == self.url.host_str()
                && url.port_or_known_default() == self.url.port_or_known_default()
        }
    }
}
fn unique_download(dir: &Path, input: &str) -> PathBuf {
    let name = Path::new(input)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .unwrap_or("download");
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let stem = Path::new(name)
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("download");
    let ext = Path::new(name).extension().and_then(|v| v.to_str());
    for i in 1..10000 {
        let name = if let Some(ext) = ext {
            format!("{stem} ({i}).{ext}")
        } else {
            format!("{stem} ({i})")
        };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("download-{}", std::process::id()))
}
fn run() -> Result<(), String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut line = Vec::new();
    input
        .by_ref()
        .take(100 * 1024 * 1024)
        .read_until(b'\n', &mut line)
        .map_err(|_| "Couldn't read launch settings")?;
    if !line.ends_with(b"\n") {
        return Err("Launch settings are incomplete or too large.".into());
    }
    let config: Config = serde_json::from_slice(&line).map_err(|_| "Invalid launch settings")?;
    drop(line);
    let policy = NavigationPolicy::new(&config.url)?;
    if !config.profile.is_absolute() || !config.exports.is_absolute() {
        return Err("Invalid profile path.".into());
    }
    std::fs::create_dir_all(&config.profile).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&config.exports).map_err(|e| e.to_string())?;
    let event_loop = EventLoopBuilder::<Cmd>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    drop(input);
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        let mut command = String::new();
        loop {
            command.clear();
            match input.read_line(&mut command) {
                Ok(0) | Err(_) => break,
                Ok(_) if command.trim() == "stop" => break,
                // The launcher's "Show" button.
                Ok(_) if command.trim() == "focus" => {
                    let _ = proxy.send_event(Cmd::Focus);
                }
                _ => {}
            }
        }
        let _ = proxy.send_event(Cmd::Stop);
    });
    let window = WindowBuilder::new()
        .with_title(config.title)
        .with_inner_size(LogicalSize::new(
            config.width.max(640) as f64,
            config.height.max(400) as f64,
        ))
        .with_min_inner_size(LogicalSize::new(640.0, 400.0))
        .build(&event_loop)
        .map_err(|e| e.to_string())?;
    let mut context = WebContext::new(Some(config.profile));
    let exports = config.exports;
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(&config.url)
        .with_initialization_script(config.init_script)
        .with_initialization_script(include_str!("telemetry.js"))
        .with_ipc_handler(|request| {
            if let Some(status) = telemetry::Snapshot::parse(request.body()) {
                if let Ok(json) = serde_json::to_string(&status) {
                    println!("STATUS {json}");
                    let _ = io::stdout().flush();
                }
            }
        })
        .with_navigation_handler(move |url| policy.allows(&url))
        .with_download_started_handler(move |_, dest| {
            let name = dest
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("download");
            *dest = unique_download(&exports, name);
            true
        });
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let _webview = builder.build(&window).map_err(|e| e.to_string())?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().ok_or("Couldn't open game window")?;
        builder.build_gtk(vbox).map_err(|e| e.to_string())?
    };
    println!("READY");
    let _ = io::stdout().flush();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Cmd::Focus) => {
                window.set_minimized(false);
                window.set_visible(true);
                window.set_focus();
            }
            Event::UserEvent(Cmd::Stop)
            | Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });
}
/// Commands from the launcher on stdin.
enum Cmd {
    Stop,
    Focus,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_is_restricted_to_game_origin() {
        let policy = NavigationPolicy::new("https://game.example/play?launcher=1").unwrap();
        assert!(policy.allows("https://game.example/next"));
        assert!(!policy.allows("https://other.example/"));
        assert!(!policy.allows("http://game.example/"));
        assert!(!policy.allows("https://game.example:444/"));
        let local = NavigationPolicy::new("file:///tmp/game.html?launcher=1").unwrap();
        assert!(local.allows("file:///tmp/game.html"));
        assert!(!local.allows("file:///tmp/other.html"));
        assert!(!local.allows("file://remote/tmp/game.html"));
    }
    #[test]
    fn download_name_cannot_escape_target() {
        let dir = Path::new("/tmp/downloads");
        assert_eq!(
            unique_download(dir, "../../world.zip"),
            dir.join("world.zip")
        );
    }
}
