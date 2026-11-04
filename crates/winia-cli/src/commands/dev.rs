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
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

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
        session,
    };

    if dev.args.once {
        println!("Initial build...");
        dev.rebuild()?;
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
    session: Session,
}

impl Dev {
    fn rebuild(&mut self) -> Result<()> {
        self.config = WiniaConfig::load(&self.config_path)?;
        self.config.validate()?;

        let abis = match &self.session {
            Session::Android { abi, .. } => vec![abi.as_str().to_owned()],
            Session::Desktop { .. } => Vec::new(),
        };
        let build_args = BuildArgs {
            target: vec![self.target.clone()],
            release: self.args.release,
            abi: abis,
            features: self.args.features.clone(),
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
                println!("Running {}", prepared.display());
                let spawned = Command::new(&prepared)
                    .args(&self.args.args)
                    .current_dir(&self.root)
                    .spawn()
                    .with_context(|| format!("running {}", prepared.display()))?;
                println!("Application started (pid {})", spawned.id());
                *child = Some(spawned);
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
                let component = run::activity_component(&self.root, &package);
                run::launch(device, &package, &component, &self.args.args)?;
            }
        }
        Ok(())
    }

    fn wait_once(mut self) -> Result<()> {
        let status = match &mut self.session {
            Session::Desktop { child, .. } => match child.take() {
                Some(mut child) => Some(child.wait().context("waiting for the application")?),
                None => None,
            },
            _ => None,
        };
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
                        eprintln!("Build failed: {error:#}");
                    }
                }
                Ok(Err(error)) => eprintln!("watch error: {error}"),
                Err(RecvTimeoutError::Timeout) => self.check_application_exit(),
                Err(RecvTimeoutError::Disconnected) => break 'watch,
            }
        }

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

    fn stop_desktop(&mut self) {
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
