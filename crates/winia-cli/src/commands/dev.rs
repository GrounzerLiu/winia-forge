//! 'winia dev' — watch sources, rebuild, and restart the application.
//!
//! Rust has no code hot-swapping, so a "hot restart" is a process restart. Desktop
//! builds copy the executable on Windows (the running binary cannot be replaced),
//! kill and respawn it; Android builds reinstall the APK and restart the activity.
//! Build failures keep the watcher alive so the next edit can retry.

use crate::cli::{BuildArgs, DevArgs};
use crate::commands::{build, run};
use crate::config::WiniaConfig;
use crate::platform::android::Abi;
use anyhow::{Context, Result, bail};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

pub fn run(args: DevArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let config = WiniaConfig::load(&config_path)?;
    config.validate()?;

    let target = args
        .target
        .clone()
        .unwrap_or_else(|| build::host_target().to_owned());
    if target != "android" && target != build::host_target() {
        bail!(
            "dev for the {target} target is not supported on {}",
            build::host_target()
        );
    }

    if args.no_debug_server && args.debug_port.is_some() {
        eprintln!("warning: --debug-port is ignored with --no-debug-server");
    }
    if args.release && !args.no_debug_server {
        eprintln!("note: debug server auto-start is disabled for release builds");
    }
    let debug_enabled = !args.no_debug_server && !args.release;
    let explicit_debug = args
        .features
        .iter()
        .any(|feature| feature == "winia/debug-server" || feature == "debug-server");
    if explicit_debug && !debug_enabled {
        eprintln!(
            "warning: debug-server is enabled in --features but automatic port/forward setup is disabled"
        );
    }
    let debug_port = if debug_enabled {
        if args.debug_port == Some(0) {
            bail!("--debug-port must be between 1 and 65535");
        }
        if target == "android" {
            choose_debug_port(args.debug_port)?
        } else {
            args.debug_port.unwrap_or(9998)
        }
    } else {
        0
    };

    if target == "android" && debug_enabled && !android_has_internet_permission(&root) {
        eprintln!(
            "warning: AndroidManifest.xml lacks android.permission.INTERNET; the debug server cannot bind"
        );
    }

    let session = if target == "android" {
        let device = run::select_device(args.device.as_deref())?;
        let abi = run::device_abi(&device)?;
        println!("Device {device} uses {}", abi.as_str());
        Session::Android { device, abi }
    } else {
        if args.device.is_some() {
            eprintln!("warning: --device is ignored for desktop targets");
        }
        Session::Desktop {
            child: None,
            executable: None,
        }
    };

    let mut dev = Dev {
        root,
        config_path,
        config,
        args,
        target,
        debug_enabled,
        debug_port,
        debug_actual_port: debug_port,
        session,
    };

    if dev.args.once {
        println!("Initial build...");
        if let Err(error) = dev.rebuild() {
            dev.stop_debug_forward();
            return Err(error);
        }
        return dev.wait_once();
    }
    dev.watch()
}

enum Session {
    Desktop {
        child: Option<Child>,
        executable: Option<PathBuf>,
    },
    Android {
        device: String,
        abi: Abi,
    },
}

struct Dev {
    root: PathBuf,
    config_path: PathBuf,
    config: WiniaConfig,
    args: DevArgs,
    target: String,
    debug_enabled: bool,
    debug_port: u16,
    debug_actual_port: u16,
    session: Session,
}

impl Dev {
    fn rebuild(&mut self) -> Result<()> {
        self.config = WiniaConfig::load(&self.config_path)?;
        self.config.validate()?;
        if self.debug_enabled {
            let _ = std::fs::remove_file(inspect_registry_file(&self.root));
        }

        let abis = match &self.session {
            Session::Android { abi, .. } => vec![abi.as_str().to_owned()],
            Session::Desktop { .. } => Vec::new(),
        };
        let debug_feature = if project_has_debug_feature(&self.root) {
            "debug-server"
        } else {
            "winia/debug-server"
        };
        let mut features = self.args.features.clone();
        if self.debug_enabled && !features.iter().any(|feature| feature == debug_feature) {
            features.push(debug_feature.to_owned());
        }
        let build_args = BuildArgs {
            target: vec![self.target.clone()],
            release: self.args.release,
            abi: abis,
            features,
            json: false,
            out_dir: None,
        };
        let built = build::build_all(&self.root, &self.config, &build_args)?;

        match &mut self.session {
            Session::Desktop { child, executable } => {
                let artifact = run::artifacts(&built)
                    .into_iter()
                    .find(|artifact| run::extension(&artifact.path) != Some("apk"))
                    .ok_or_else(|| anyhow::anyhow!("the build produced no executable"))?;
                let prepared = prepare_executable(&self.root, &artifact.path)?;

                if let Some(mut current) = child.take() {
                    stop_child(&mut current);
                }
                if let Some(previous) = executable.take() {
                    let _ = std::fs::remove_file(previous);
                }
                let port_file = debug_port_file(&self.root);
                if self.debug_enabled {
                    if let Some(parent) = port_file.parent() {
                        std::fs::create_dir_all(parent)
                            .with_context(|| format!("creating {}", parent.display()))?;
                    }
                    let _ = std::fs::remove_file(&port_file);
                }
                println!("Running {}", prepared.display());
                let mut command = Command::new(&prepared);
                command.args(&self.args.args).current_dir(&self.root);
                if self.debug_enabled {
                    command
                        .env("WINIA_DEBUG_PORT", self.debug_port.to_string())
                        .env("WINIA_DEBUG_PORT_FILE", &port_file)
                        .env("WINIA_DEBUG_APP_NAME", &self.config.app.name)
                        .env("WINIA_DEBUG_APP_VERSION", &self.config.app.version)
                        .env("WINIA_DEBUG_APP_ID", &self.config.app.id)
                        .env("WINIA_DEBUG_TARGET", &self.target);
                }
                let spawned = command
                    .spawn()
                    .with_context(|| format!("running {}", prepared.display()))?;
                println!("Application started (pid {})", spawned.id());
                *child = Some(spawned);
                if self.debug_enabled {
                    self.debug_actual_port =
                        wait_for_debug_port_file(&port_file, Duration::from_secs(10))
                            .unwrap_or(self.debug_port);
                }
                if cfg!(windows) {
                    *executable = Some(prepared);
                }
            }
            Session::Android { device, .. } => {
                let apk = run::artifacts(&built)
                    .into_iter()
                    .find(|artifact| run::extension(&artifact.path) == Some("apk"))
                    .ok_or_else(|| anyhow::anyhow!("the build produced no APK"))?;
                run::install(device, &apk.path)?;
                let package = if self.config.android.package.is_empty() {
                    self.config.app.id.clone()
                } else {
                    self.config.android.package.clone()
                };
                if self.debug_enabled {
                    let info = serde_json::json!({
                        "app": self.config.app.name,
                        "version": self.config.app.version,
                        "id": self.config.app.id,
                        "target": "android",
                    })
                    .to_string();
                    if let Err(error) = run::push_android_debug_info(device, &package, &info) {
                        eprintln!("warning: cannot write Android debug metadata: {error:#}");
                    }
                    run::clear_logcat(device);
                }
                let component = run::activity_component(&self.root, &package);
                run::launch(device, &package, &component, &self.args.args)?;
                if self.debug_enabled {
                    let device_port = match discover_android_debug_port(device) {
                        Some(port) => port,
                        None => {
                            eprintln!(
                                "warning: could not discover the Android debug port; assuming 9998"
                            );
                            9998
                        }
                    };
                    run::adb_forward(device, self.debug_port, device_port)?;
                    self.debug_actual_port = self.debug_port;
                }
            }
        }
        if self.debug_enabled {
            self.write_inspect_registry();
            self.wait_for_debug_server();
        }
        Ok(())
    }

    fn wait_for_debug_server(&self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if websocket_handshake_ok(self.debug_actual_port) {
                println!(
                    "Debug server ready at ws://127.0.0.1:{}",
                    self.debug_actual_port
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        eprintln!(
            "warning: debug server did not become ready on port {}",
            self.debug_actual_port
        );
    }

    fn wait_once(mut self) -> Result<()> {
        let status = match &mut self.session {
            Session::Desktop { child, .. } => match child.take() {
                Some(mut child) => Some(child.wait().context("waiting for the application")?),
                None => None,
            },
            _ => None,
        };
        let keep_forward = self.debug_enabled && matches!(&self.session, Session::Android { .. });
        if keep_forward {
            if let Session::Android { device, .. } = &self.session {
                println!(
                    "Debug server forward kept on {device}: ws://127.0.0.1:{} (remove: adb -s {device} forward --remove tcp:{})",
                    self.debug_port, self.debug_port
                );
            }
        } else {
            self.stop_debug_forward();
        }
        self.stop_desktop();
        if let Some(status) = status {
            if !status.success() {
                bail!("application exited with {status}");
            }
        }
        Ok(())
    }

    fn watch(mut self) -> Result<()> {
        let paths = watch_paths(&self.root);
        if paths.is_empty() {
            bail!(
                "no source paths found to watch under {}",
                self.root.display()
            );
        }

        let (sender, receiver) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |result| {
            let _ = sender.send(result);
        })
        .context("creating the file watcher")?;
        for (path, mode) in &paths {
            if let Err(error) = watcher.watch(path, *mode) {
                eprintln!("warning: cannot watch {}: {error}", path.display());
            }
        }

        let running = Arc::new(AtomicBool::new(true));
        let signal = running.clone();
        ctrlc::set_handler(move || signal.store(false, Ordering::SeqCst))
            .context("installing the Ctrl+C handler")?;

        println!("Initial build...");
        if let Err(error) = self.rebuild() {
            self.stop_debug_forward();
            eprintln!("Build failed: {error:#}");
            eprintln!("Watching for changes to retry...");
        }
        println!("Watching for changes (Ctrl+C to stop)...");

        'watch: while running.load(Ordering::SeqCst) {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(Ok(event)) => {
                    if !event_is_relevant(&event, &self.root) {
                        continue;
                    }
                    let mut latest = event;
                    loop {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        match receiver
                            .recv_timeout(Duration::from_millis(self.args.debounce.max(50)))
                        {
                            Ok(Ok(next)) => {
                                if event_is_relevant(&next, &self.root) {
                                    latest = next;
                                }
                            }
                            Ok(Err(error)) => {
                                eprintln!("watch error: {error}");
                                break;
                            }
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => break 'watch,
                        }
                    }
                    if !running.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Some(path) = latest.paths.first() {
                        println!(
                            "Change detected: {}",
                            path.strip_prefix(&self.root).unwrap_or(path).display()
                        );
                    }
                    if let Err(error) = self.rebuild() {
                        self.stop_debug_forward();
                        eprintln!("Build failed: {error:#}");
                    }
                }
                Ok(Err(error)) => eprintln!("watch error: {error}"),
                Err(RecvTimeoutError::Timeout) => self.check_application_exit(),
                Err(RecvTimeoutError::Disconnected) => break 'watch,
            }
        }

        self.stop_debug_forward();
        self.stop_desktop();
        println!("Stopped.");
        Ok(())
    }

    fn check_application_exit(&mut self) {
        let status = match &mut self.session {
            Session::Desktop {
                child: Some(child), ..
            } => match child.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    eprintln!("watch error: {error}");
                    return;
                }
            },
            _ => None,
        };
        if let Some(status) = status {
            if let Session::Desktop { child, executable } = &mut self.session {
                let _ = child.take();
                if let Some(path) = executable.take() {
                    let _ = std::fs::remove_file(path);
                }
            }
            println!("Application exited with {status}; waiting for changes...");
        }
    }

    fn write_inspect_registry(&self) {
        if !self.debug_enabled {
            return;
        }
        let path = inspect_registry_file(&self.root);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let value = serde_json::json!({
            "port": self.debug_actual_port,
            "app": self.config.app.name,
            "version": self.config.app.version,
            "id": self.config.app.id,
            "target": self.target,
        });
        if let Ok(text) = serde_json::to_string_pretty(&value) {
            let _ = std::fs::write(path, text);
        }
    }

    fn stop_debug_forward(&self) {
        if self.debug_enabled {
            if let Session::Android { device, .. } = &self.session {
                let _ = run::adb_remove_forward(device, self.debug_port);
            }
        }
    }

    fn stop_desktop(&mut self) {
        let _ = std::fs::remove_file(debug_port_file(&self.root));
        if let Session::Desktop { child, executable } = &mut self.session {
            if let Some(mut current) = child.take() {
                stop_child(&mut current);
            }
            if let Some(path) = executable.take() {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn project_has_debug_feature(root: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return false;
    };
    let mut in_features = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[features]" {
            in_features = true;
            continue;
        }
        if in_features && trimmed.starts_with('[') {
            break;
        }
        if in_features && trimmed.starts_with("debug-server") && trimmed.contains('=') {
            return true;
        }
    }
    false
}

fn android_has_internet_permission(root: &Path) -> bool {
    [
        "android/app/src/main/AndroidManifest.xml",
        "android/app/src/debug/AndroidManifest.xml",
    ]
    .iter()
    .any(|path| {
        std::fs::read_to_string(root.join(path))
            .map(|manifest| manifest.contains("android.permission.INTERNET"))
            .unwrap_or(false)
    })
}

fn websocket_handshake_ok(port: u16) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
    let request = concat!(
        "GET / HTTP/1.1\r\n",
        "Host: 127.0.0.1\r\n",
        "Upgrade: websocket\r\n",
        "Connection: Upgrade\r\n",
        "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n",
        "Sec-WebSocket-Version: 13\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut buffer = [0u8; 256];
    match stream.read(&mut buffer) {
        Ok(bytes) if bytes > 0 => {
            let response = String::from_utf8_lossy(&buffer[..bytes]);
            response.starts_with("HTTP/1.1 101") || response.starts_with("HTTP/1.0 101")
        }
        _ => false,
    }
}

fn choose_debug_port(requested: Option<u16>) -> Result<u16> {
    let start = requested.unwrap_or(9998);
    if start == 0 {
        bail!("--debug-port must be between 1 and 65535");
    }
    for offset in 0..=100u16 {
        let port = start.saturating_add(offset);
        if port == 0 {
            continue;
        }
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    bail!(
        "no free debug port in {}..={}",
        start,
        start.saturating_add(100)
    )
}

fn inspect_registry_file(root: &Path) -> PathBuf {
    root.join("target")
        .join("winia-dev")
        .join(format!("inspect-port-{}.json", std::process::id()))
}

fn debug_port_file(root: &Path) -> PathBuf {
    root.join("target")
        .join("winia-dev")
        .join(format!("debug-port-{}.txt", std::process::id()))
}

fn wait_for_debug_port_file(path: &Path, timeout: Duration) -> Option<u16> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(port) = text.trim().parse::<u16>() {
                if port > 0 {
                    return Some(port);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn discover_android_debug_port(device: &str) -> Option<u16> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match run::debug_server_port(device) {
            Ok(Some(port)) => return Some(port),
            Ok(None) => {}
            Err(error) => {
                eprintln!("watch error: {error}");
                return None;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    None
}

fn prepare_executable(root: &Path, artifact: &Path) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let directory = root.join("target").join("winia-dev");
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("creating {}", directory.display()))?;
        let stem = artifact
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "app".to_owned());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let destination = directory.join(format!("{stem}-{stamp}.exe"));
        std::fs::copy(artifact, &destination)
            .with_context(|| format!("copying {}", artifact.display()))?;
        Ok(destination)
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Ok(artifact.to_owned())
    }
}

fn stop_child(child: &mut Child) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .output();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn watch_paths(root: &Path) -> Vec<(PathBuf, RecursiveMode)> {
    let mut paths = Vec::new();
    for directory in ["src", "assets", "android/app/src"] {
        let path = root.join(directory);
        if path.is_dir() {
            paths.push((path, RecursiveMode::Recursive));
        }
    }
    for file in [
        "Cargo.toml",
        "winia.toml",
        "build.rs",
        "android/app/build.gradle.kts",
        "android/build.gradle.kts",
        "android/settings.gradle.kts",
        "android/gradle.properties",
    ] {
        let path = root.join(file);
        if path.is_file() {
            paths.push((path, RecursiveMode::NonRecursive));
        }
    }
    paths
}

fn event_is_relevant(event: &Event, root: &Path) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.iter().any(|path| is_relevant(path, root))
}

fn is_relevant(path: &Path, root: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let text = relative.to_string_lossy().replace('\\', "/");
    if [
        "target/",
        "dist/",
        ".git/",
        "android/.gradle/",
        "android/app/build/",
        "android/build/",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
    {
        return false;
    }
    if text == "assets" || text.starts_with("assets/") {
        return true;
    }
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("rs") | Some("toml") | Some("java") | Some("kt") | Some("xml")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watches_sources_config_and_assets() {
        let root = Path::new("C:/app");
        assert!(is_relevant(&root.join("src/main.rs"), root));
        assert!(is_relevant(&root.join("winia.toml"), root));
        assert!(is_relevant(&root.join("assets/icon.png"), root));
        assert!(is_relevant(
            &root.join("android/app/src/main/AndroidManifest.xml"),
            root
        ));
    }

    #[test]
    fn ignores_build_outputs_and_vcs() {
        let root = Path::new("C:/app");
        assert!(!is_relevant(&root.join("target/debug/app.exe"), root));
        assert!(!is_relevant(&root.join("dist/app.apk"), root));
        assert!(!is_relevant(&root.join(".git/index"), root));
        assert!(!is_relevant(
            &root.join("android/app/build/outputs/apk/app.apk"),
            root
        ));
    }

    #[test]
    fn accepts_a_websocket_upgrade_response() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\n");
        });
        assert!(websocket_handshake_ok(port));
        server.join().unwrap();
    }

    #[test]
    fn detects_a_project_debug_server_feature() {
        let root = std::env::temp_dir().join(format!("winia-dev-feature-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\n\n[features]\ndebug-server = []\n",
        )
        .unwrap();
        assert!(project_has_debug_feature(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_zero_debug_port() {
        assert!(choose_debug_port(Some(0)).is_err());
    }

    #[test]
    fn finds_internet_permission_in_the_debug_manifest() {
        let root =
            std::env::temp_dir().join(format!("winia-dev-permission-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("android/app/src/main")).unwrap();
        std::fs::create_dir_all(root.join("android/app/src/debug")).unwrap();
        std::fs::write(
            root.join("android/app/src/main/AndroidManifest.xml"),
            "<manifest />",
        )
        .unwrap();
        std::fs::write(
            root.join("android/app/src/debug/AndroidManifest.xml"),
            "<manifest><uses-permission android:name=\"android.permission.INTERNET\" /></manifest>",
        )
        .unwrap();
        assert!(android_has_internet_permission(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn watch_paths_skip_build_directories() {
        let root = std::env::temp_dir().join(format!("winia-dev-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "").unwrap();
        let paths = watch_paths(&root);
        assert!(paths.iter().any(|(path, _)| path.ends_with("src")));
        assert!(paths.iter().any(|(path, _)| path.ends_with("Cargo.toml")));
        assert!(!paths.iter().any(|(path, _)| path.ends_with("target")));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
