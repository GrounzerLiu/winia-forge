//! 'winia run' — build and launch the application.
//!
//! Desktop targets build the host executable and wait for it to exit. Android
//! targets pick the connected device, read its primary ABI, build only that ABI,
//! install the APK, and start the activity. Arguments after `--` are forwarded
//! to the desktop application; on Android they become string intent extras
//! (`winia.args` and `winia.argN`).

use crate::cli::{BuildArgs, RunArgs};
use crate::commands::build;
use crate::config::WiniaConfig;
use crate::platform::android::Abi;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub fn run(args: RunArgs) -> Result<()> {
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
    if target == "android" {
        run_android(&root, &config, &args)
    } else if target == build::host_target() {
        run_desktop(&root, &config, &args, &target)
    } else {
        bail!(
            "running the {target} target on {} is not supported yet",
            build::host_target()
        )
    }
}

fn build_args(target: &str, args: &RunArgs, abis: Vec<String>) -> BuildArgs {
    BuildArgs {
        target: vec![target.to_owned()],
        release: args.release,
        abi: abis,
        features: args.features.clone(),
        json: false,
        out_dir: None,
    }
}

fn run_desktop(root: &Path, config: &WiniaConfig, args: &RunArgs, target: &str) -> Result<()> {
    let built = build::build_all(root, config, &build_args(target, args, Vec::new()))?;
    let executable = artifacts(&built)
        .into_iter()
        .find(|artifact| extension(&artifact.path) != Some("apk"))
        .ok_or_else(|| anyhow::anyhow!("the build produced no executable"))?;
    let assets_root = root.join("target/winia/assets/desktop");
    crate::assets::stage_desktop(root, config, &assets_root)?;
    println!("Running {}", executable.path.display());
    let status = Command::new(&executable.path)
        .args(&args.args)
        .env("WINIA_ASSET_ROOT", &assets_root)
        .current_dir(root)
        .status()
        .with_context(|| format!("running {}", executable.path.display()))?;
    if !status.success() {
        // Thin launcher: preserve the app's exit code so scripts can react to it.
        eprintln!("application exited with {status}");
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn run_android(root: &Path, config: &WiniaConfig, args: &RunArgs) -> Result<()> {
    let device = select_device(args.device.as_deref())?;
    let abi = device_abi(&device)?;
    println!("Device {device} uses {}", abi.as_str());

    let built = build::build_all(
        root,
        config,
        &build_args("android", args, vec![abi.as_str().to_owned()]),
    )?;
    let apk = artifacts(&built)
        .into_iter()
        .find(|artifact| extension(&artifact.path) == Some("apk"))
        .ok_or_else(|| anyhow::anyhow!("the build produced no APK"))?;
    install(&device, &apk.path)?;

    let package = if config.android.package.is_empty() {
        config.app.id.clone()
    } else {
        config.android.package.clone()
    };
    let component = activity_component(root, &package);
    launch(&device, &package, &component, &args.args)?;
    Ok(())
}

pub(crate) fn artifacts(built: &[(String, Vec<build::Artifact>)]) -> Vec<&build::Artifact> {
    built
        .iter()
        .flat_map(|(_, artifacts)| artifacts.iter())
        .collect()
}

pub(crate) fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|extension| extension.to_str())
}

pub(crate) fn adb(args: &[&str]) -> Result<String> {
    let mut command = Command::new(adb_program());
    command.args(args);
    let output = output_with_timeout(command, Duration::from_secs(15))?;
    if !output.status.success() {
        bail!(
            "adb {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn adb_output(args: &[&str], timeout: Duration) -> Result<std::process::Output> {
    let mut command = Command::new(adb_program());
    command.args(args);
    output_with_timeout(command, timeout)
}

/// Run adb and return raw stdout, for binary payloads such as screencap PNGs.
pub(crate) fn adb_binary(args: &[&str]) -> Result<Vec<u8>> {
    let mut command = Command::new(adb_program());
    command.args(args);
    let output = output_with_timeout(command, Duration::from_secs(60))?;
    if !output.status.success() {
        bail!(
            "adb {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// Run adb against one device and return its stdout.
pub(crate) fn adb_on(device: &str, args: &[&str]) -> Result<String> {
    let mut full: Vec<&str> = vec!["-s", device];
    full.extend_from_slice(args);
    adb(&full)
}

pub(crate) fn adb_program() -> PathBuf {
    if Command::new("adb")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        return PathBuf::from("adb");
    }
    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(value) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
            let candidate = PathBuf::from(value)
                .join("platform-tools")
                .join(if cfg!(windows) { "adb.exe" } else { "adb" });
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("adb")
}

fn output_with_timeout(mut command: Command, timeout: Duration) -> Result<Output> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting adb")?;
    // Drain both pipes from reader threads: a large payload (a screenshot is several hundred KB)
    // fills the pipe buffer and would otherwise block the child before it can exit.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || read_all(stdout));
    let stderr_reader = std::thread::spawn(move || read_all(stderr));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("adb command timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Ok(Output {
        status,
        stdout: stdout_reader.join().unwrap_or_default(),
        stderr: stderr_reader.join().unwrap_or_default(),
    })
}

fn read_all(pipe: Option<impl std::io::Read>) -> Vec<u8> {
    let mut buffer = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
    }
    buffer
}

pub(crate) fn select_device(requested: Option<&str>) -> Result<String> {
    if let Some(device) = requested.filter(|device| !device.is_empty()) {
        let mut command = Command::new(adb_program());
        command.args(["-s", device, "get-state"]);
        let state = output_with_timeout(command, Duration::from_secs(10))
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
        if state.as_deref() == Some("device") {
            return Ok(device.to_owned());
        }
        let online = parse_devices(&adb(&["devices"])?);
        let online = if online.is_empty() {
            "none".to_owned()
        } else {
            online.join(", ")
        };
        bail!("device '{device}' is not connected; online devices: {online}");
    }
    let output = adb(&["devices"])?;
    let devices = parse_devices(&output);
    match devices.len() {
        0 => {
            let unusable: Vec<String> = output
                .lines()
                .skip(1)
                .filter_map(|line| {
                    let mut fields = line.split_whitespace();
                    let serial = fields.next()?;
                    let state = fields.next()?;
                    (state != "device").then(|| format!("{serial} is {state}"))
                })
                .collect();
            if unusable.is_empty() {
                bail!(
                    "no Android device or emulator connected; start one or pass --device <serial>"
                );
            }
            bail!(
                "no usable Android device: {}; authorize USB debugging or run 'adb reconnect', or pass --device <serial>",
                unusable.join(", ")
            );
        }
        1 => Ok(devices.into_iter().next().expect("length checked")),
        _ => bail!(
            "multiple devices connected: {}; pass --device <serial>",
            devices.join(", ")
        ),
    }
}

fn parse_devices(output: &str) -> Vec<String> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            let state = fields.next()?;
            (state == "device").then(|| serial.to_owned())
        })
        .collect()
}

pub(crate) fn device_abi(device: &str) -> Result<Abi> {
    let output = adb(&["-s", device, "shell", "getprop", "ro.product.cpu.abi"])?;
    let raw = output.lines().next().unwrap_or_default().trim();
    Abi::parse(raw).with_context(|| format!("device {device} reports unsupported ABI '{raw}'"))
}

pub(crate) fn install(device: &str, apk: &Path) -> Result<()> {
    install_impl(device, apk, false)
}

pub(crate) fn install_quiet(device: &str, apk: &Path) -> Result<()> {
    install_impl(device, apk, true)
}

fn install_impl(device: &str, apk: &Path, quiet: bool) -> Result<()> {
    if apk
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with("-unsigned.apk"))
    {
        bail!(
            "{} is unsigned and cannot be installed; add a signingConfig to android/app/build.gradle.kts, or use a debug build (winia sign only signs packaged APKs)",
            apk.display()
        );
    }
    if !quiet {
        println!("Installing {}", apk.display());
    }
    let mut command = Command::new(adb_program());
    command.args(["-s", device, "install", "-r"]).arg(apk);
    let output = output_with_timeout(command, Duration::from_secs(180))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() || !text.contains("Success") {
        bail!("adb install failed for {}:\n{}", apk.display(), text.trim());
    }
    Ok(())
}

pub(crate) fn adb_forward(device: &str, host_port: u16, device_port: u16) -> Result<()> {
    let host_mapping = format!("tcp:{host_port}");
    let device_mapping = format!("tcp:{device_port}");
    let list = adb_output(
        &["-s", device, "forward", "--list"],
        Duration::from_secs(10),
    )?;
    let listing = String::from_utf8_lossy(&list.stdout);
    if listing.lines().any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        fields.len() >= 3
            && fields[0] == device
            && fields[1] == host_mapping
            && fields[2] == device_mapping
    }) {
        return Ok(());
    }
    // Re-point an existing mapping for this host port instead of failing on it: the device-side
    // debug port can change between app restarts.
    let _ = adb_output(
        &["-s", device, "forward", "--remove", &host_mapping],
        Duration::from_secs(10),
    );
    let output = adb_output(
        &["-s", device, "forward", &host_mapping, &device_mapping],
        Duration::from_secs(15),
    )?;
    if !output.status.success() {
        bail!(
            "adb forward failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub(crate) fn adb_remove_forward(device: &str, host_port: u16) -> Result<()> {
    let mapping = format!("tcp:{host_port}");
    let output = adb_output(
        &["-s", device, "forward", "--remove", &mapping],
        Duration::from_secs(10),
    )?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr);
        if !text.contains("not found") && !text.contains("cannot remove") {
            bail!("adb forward --remove failed: {}", text.trim());
        }
    }
    Ok(())
}

pub(crate) fn push_android_debug_info(device: &str, package: &str, json: &str) -> Result<()> {
    let local = std::env::temp_dir().join(format!("winia-debug-info-{}.json", std::process::id()));
    std::fs::write(&local, json).with_context(|| format!("writing {}", local.display()))?;
    let shell =
        format!("run-as {package} sh -c 'mkdir -p files; cat > files/winia-debug-info.json'");
    let stdin =
        std::fs::File::open(&local).with_context(|| format!("opening {}", local.display()))?;
    let mut command = Command::new(adb_program());
    command.args(["-s", device, "shell", &shell]).stdin(stdin);
    let output = output_with_timeout(command, Duration::from_secs(15))
        .context("running adb shell run-as")?;
    let _ = std::fs::remove_file(&local);
    if !output.status.success() {
        bail!(
            "adb run-as metadata write failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub(crate) fn clear_logcat(device: &str) {
    let _ = adb_output(&["-s", device, "logcat", "-c"], Duration::from_secs(10));
}

pub(crate) fn app_pid(device: &str, package: &str) -> Option<u32> {
    let output = adb_output(
        &["-s", device, "shell", "pidof", package],
        Duration::from_secs(10),
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
}

pub(crate) fn logcat_pid(device: &str, pid: u32) -> Result<String> {
    let output = adb_output(
        &["-s", device, "logcat", "-d", "--pid", &pid.to_string()],
        Duration::from_secs(30),
    )
    .context("running adb logcat --pid")?;
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

pub(crate) fn logcat_dump(device: &str) -> Result<String> {
    let output = adb_output(
        &["-s", device, "logcat", "-d", "-s", "RustStdoutStderr:I"],
        Duration::from_secs(30),
    )
    .context("running adb logcat")?;
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

pub(crate) fn screencap(device: &str) -> Result<Vec<u8>> {
    let output = adb_output(
        &["-s", device, "exec-out", "screencap", "-p"],
        Duration::from_secs(60),
    )
    .context("running adb screencap")?;
    if !output.status.success() {
        bail!(
            "adb screencap failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

pub(crate) fn debug_server_port(device: &str) -> Result<Option<u16>> {
    let output = adb_output(
        &["-s", device, "logcat", "-d", "-s", "RustStdoutStderr:I"],
        Duration::from_secs(30),
    )
    .context("running adb logcat")?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines().rev() {
        if let Some(index) = line.find("localhost:") {
            let digits: String = line[index + "localhost:".len()..]
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect();
            if let Ok(port) = digits.parse::<u16>()
                && port > 0
            {
                return Ok(Some(port));
            }
        }
    }
    Ok(None)
}

pub(crate) fn launch(device: &str, package: &str, component: &str, args: &[String]) -> Result<()> {
    launch_impl(device, package, component, args, false)
}

pub(crate) fn launch_quiet(
    device: &str,
    package: &str,
    component: &str,
    args: &[String],
) -> Result<()> {
    launch_impl(device, package, component, args, true)
}

fn launch_impl(
    device: &str,
    package: &str,
    component: &str,
    args: &[String],
    quiet: bool,
) -> Result<()> {
    let mut stop = Command::new(adb_program());
    stop.args(["-s", device, "shell", "am", "force-stop", package]);
    let _ = output_with_timeout(stop, Duration::from_secs(10));

    let mut start = Command::new(adb_program());
    start.args(["-s", device, "shell", "am", "start", "-W", "-n"]);
    start.arg(shell_quote(component));
    if !args.is_empty() {
        // `adb shell` joins argv and the device shell parses the line again, so values with spaces
        // or metacharacters must be quoted for that shell, not for adb.
        start
            .arg("--es")
            .arg("winia.args")
            .arg(shell_quote(&args.join(" ")));
        for (index, value) in args.iter().enumerate() {
            start
                .arg("--es")
                .arg(format!("winia.arg{index}"))
                .arg(shell_quote(value));
        }
    }
    let output = output_with_timeout(start, Duration::from_secs(30))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() || text.contains("Error") {
        let logcat = logcat_dump(device).unwrap_or_default();
        let tail: Vec<&str> = logcat.lines().rev().take(20).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        bail!(
            "could not start {component}:\n{}\nlogcat (last 20 lines):\n{}",
            text.trim(),
            tail.join("\n")
        );
    }
    if !quiet {
        println!("Launched {component} on {device}");
    }
    Ok(())
}

/// Resolve the launcher activity from the generated manifest, falling back to
/// the template's `.MainActivity`.
pub(crate) fn activity_component(root: &Path, package: &str) -> String {
    let manifest = root.join("android/app/src/main/AndroidManifest.xml");
    if let Ok(text) = std::fs::read_to_string(&manifest)
        && let Some(name) = launcher_activity_name(&text)
    {
        // am expects pkg/class; a bare fully-qualified class is rejected by ComponentName.
        return format!("{package}/{name}");
    }
    format!("{package}/.MainActivity")
}

/// Quote a value for the device shell: single quotes keep spaces and metacharacters literal.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Prefer the activity with MAIN/LAUNCHER; fall back to the first one.
fn launcher_activity_name(manifest: &str) -> Option<String> {
    let mut rest = manifest;
    while let Some(start) = rest.find("<activity") {
        let element = &rest[start..];
        let Some(open_end) = element.find('>') else {
            break;
        };
        // A self-closing <activity ... /> has no closing tag; skip it instead of ending the scan.
        if element[..open_end].trim_end().ends_with('/') {
            rest = &element[open_end + 1..];
            continue;
        }
        let alias = element.starts_with("<activity-alias");
        let close_tag = if alias {
            "</activity-alias>"
        } else {
            "</activity>"
        };
        let Some(close) = element.find(close_tag) else {
            break;
        };
        let block = &element[..close];
        if block.contains("android.intent.action.MAIN")
            && block.contains("android.intent.category.LAUNCHER")
            && let Some(name) = activity_name_in(block)
        {
            return Some(name);
        }
        rest = &element[close + close_tag.len()..];
    }
    first_activity_name(manifest)
}

fn activity_name_in(block: &str) -> Option<String> {
    let marker = "android:name=\"";
    let start = block.find(marker)? + marker.len();
    let value = &block[start..];
    let end = value.find('"')?;
    Some(value[..end].to_owned())
}

fn first_activity_name(manifest: &str) -> Option<String> {
    let mut rest = manifest;
    while let Some(start) = rest.find("<activity") {
        rest = &rest[start..];
        let end = rest.find('>')?;
        let element = &rest[..end];
        let marker = "android:name=\"";
        if let Some(name_start) = element.find(marker) {
            let value = &element[name_start + marker.len()..];
            if let Some(quote) = value.find('"') {
                return Some(value[..quote].to_owned());
            }
        }
        rest = &rest[end + 1..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_the_launcher_activity() {
        let manifest = r#"
<manifest><application>
  <activity android:name=".Splash" android:exported="false">
    <intent-filter><action android:name="android.intent.action.VIEW" /></intent-filter>
  </activity>
  <activity android:name=".MainActivity" android:exported="true">
    <intent-filter>
      <action android:name="android.intent.action.MAIN" />
      <category android:name="android.intent.category.LAUNCHER" />
    </intent-filter>
  </activity>
</application></manifest>"#;
        assert_eq!(
            launcher_activity_name(manifest).as_deref(),
            Some(".MainActivity")
        );
    }

    #[test]
    fn reads_the_activity_name_from_the_manifest() {
        let manifest = r#"<manifest><application>
            <activity android:name=".Launcher" android:exported="true" />
        </application></manifest>"#;
        assert_eq!(first_activity_name(manifest), Some(".Launcher".to_owned()));
        assert!(first_activity_name("<manifest />").is_none());
    }

    #[test]
    fn activity_alias_is_used_when_it_is_the_launcher() {
        let manifest = r#"<manifest><application>
  <activity android:name=".MainActivity" />
  <activity-alias android:name=".Alias" android:targetActivity=".MainActivity">
    <intent-filter>
      <action android:name="android.intent.action.MAIN" />
      <category android:name="android.intent.category.LAUNCHER" />
    </intent-filter>
  </activity-alias>
</application></manifest>"#;
        assert_eq!(launcher_activity_name(manifest).as_deref(), Some(".Alias"));
    }

    #[test]
    fn absolute_activity_names_keep_the_component_slash() {
        let root = std::env::temp_dir().join(format!("winia-component-{}", std::process::id()));
        let directory = root.join("android/app/src/main");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("AndroidManifest.xml"),
            "<manifest><application><activity android:name=\"com.example.Main\" /></application></manifest>",
        )
        .unwrap();
        assert_eq!(
            activity_component(&root, "com.example"),
            "com.example/com.example.Main"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parses_only_ready_devices() {
        let output = "List of devices attached\n\
            emulator-5554\tdevice\n\
            192.0.0.172:5555\tdevice\n\
            10.0.0.2:5555\toffline\n\
            10.0.0.3:5555\tunauthorized\n";
        assert_eq!(
            parse_devices(output),
            vec!["emulator-5554", "192.0.0.172:5555"]
        );
    }
}
