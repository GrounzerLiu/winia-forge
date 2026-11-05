//! 'winia init' — add Winia to an existing Cargo project.
//!
//! Unlike 'winia new', this never rewrites the user's sources: it adds the framework dependency
//! to Cargo.toml, writes winia.toml when absent, creates assets/, and — when Android is requested —
//! generates the Gradle project and src/lib.rs only if they do not exist yet.

use crate::cli::{InitArgs, Platform};
use crate::commands::new;
use crate::config::{AndroidActivity, WiniaConfig};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Table};

/// Written only when the project has no src/lib.rs. The blank template starts from a real UI;
/// an existing project usually already has one, so the entry point is left empty.
const ANDROID_ENTRY: &str = r#"//! Android entry point added by winia init.

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: winit::platform::android::activity::AndroidApp) {
    winia::app::run_app_android(android_app, |_ctx| {
        // Build the application UI here.
    });
}
"#;

pub fn run(args: InitArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let root: PathBuf = args.path.clone().unwrap_or(cwd);
    let cargo_toml = root.join("Cargo.toml");
    if !cargo_toml.is_file() {
        bail!(
            "no Cargo.toml in {}; create a Rust project first or use winia new",
            root.display()
        );
    }
    if root.join("winia.toml").is_file() {
        bail!(
            "{} already contains winia.toml; this project is already initialized",
            root.display()
        );
    }

    let text = std::fs::read_to_string(&cargo_toml)
        .with_context(|| format!("reading {}", cargo_toml.display()))?;
    let mut doc: DocumentMut = text
        .parse()
        .with_context(|| format!("parsing {}", cargo_toml.display()))?;
    let package_name = doc["package"]["name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Cargo.toml has no [package] name"))?
        .to_owned();
    let package_version = doc["package"]["version"]
        .as_str()
        .unwrap_or("0.1.0")
        .to_owned();

    let package = new::package_name(&package_name)?;
    let lib_name = package.replace('-', "_");
    let app_id = format!("com.example.{lib_name}");
    let app_name = args
        .name
        .clone()
        .unwrap_or_else(|| title_case(&package_name));

    let platforms = if args.platforms.is_empty() {
        vec![Platform::Windows, Platform::Macos, Platform::Linux]
    } else {
        args.platforms.clone()
    };
    if platforms.contains(&Platform::Ios) {
        bail!("ios support is not implemented yet; choose windows, macos, linux, or android");
    }
    let android = platforms.contains(&Platform::Android);

    let activity = AndroidActivity::Game;
    let winia_android = new::winia_dependency(
        args.winia_path.clone(),
        &[new::android_feature(activity), "vulkan"],
    )?;
    let winia_desktop = new::winia_dependency(args.winia_path.clone(), &["gl"])?;

    set_target_dependency(
        &mut doc,
        "cfg(not(target_os = \"android\"))",
        &winia_desktop,
    )?;
    set_target_dependency(&mut doc, "cfg(target_os = \"android\")", &winia_android)?;
    if android {
        // The Android entry point names winit directly, so the fork must be a dependency.
        set_target_dependency(
            &mut doc,
            "cfg(target_os = \"android\")",
            &new::winit_dependency(activity),
        )?;
        ensure_crate_types(&mut doc)?;
    }
    std::fs::write(&cargo_toml, doc.to_string())
        .with_context(|| format!("writing {}", cargo_toml.display()))?;
    println!(
        "Updated Cargo.toml (winia dependency{})",
        if android { " + lib crate types" } else { "" }
    );

    let mut config = WiniaConfig::default();
    config.app.name = app_name.clone();
    config.app.id = app_id.clone();
    config.app.version = package_version;
    config.android.package = app_id.clone();
    config.window.title = app_name;
    config.window.width = 1000.0;
    config.window.height = 700.0;
    config.save(&root.join("winia.toml"))?;
    crate::generated::write(&root, &config)?;
    println!("Wrote winia.toml");

    std::fs::create_dir_all(root.join("assets"))
        .with_context(|| format!("creating {}", root.join("assets").display()))?;

    if android {
        scaffold_android(&root, &config)?;
    }

    println!("Platforms: {}", platform_list(&platforms).join(", "));
    println!("Next:");
    println!("  winia doctor");
    println!("  winia run");
    Ok(())
}

/// Write the Android project (Gradle files, wrapper, entry point) for a project that already has
/// winia.toml. Shared by 'winia init --platforms android' and 'winia android init'.
pub(crate) fn scaffold_android(root: &Path, config: &WiniaConfig) -> Result<()> {
    let package = cargo_package_name(root, config);
    let lib_name = package.replace('-', "_");
    let app_id = if config.android.package.trim().is_empty() {
        config.app.id.clone()
    } else {
        config.android.package.clone()
    };
    let package_path = app_id.replace('.', "/");
    let vars: Vec<(&str, &str)> = vec![
        ("__APP_NAME__", config.app.name.as_str()),
        ("__PACKAGE_NAME__", package.as_str()),
        ("__LIB_NAME__", lib_name.as_str()),
        ("__APP_ID__", app_id.as_str()),
        ("__PACKAGE_PATH__", package_path.as_str()),
        ("__VERSION__", config.app.version.as_str()),
        (
            "__MAIN_ACTIVITY__",
            match config.android.activity {
                AndroidActivity::Game => ".MainActivity",
                AndroidActivity::Native => "android.app.NativeActivity",
            },
        ),
    ];
    // Respect an existing [lib] path: a crate whose library lives in src/core.rs must not grow a
    // second, unused src/lib.rs.
    let lib_path = cargo_lib_path(root);
    let lib_file = root.join(&lib_path);
    if !lib_file.is_file() {
        if let Some(parent) = lib_file.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&lib_file, ANDROID_ENTRY)
            .with_context(|| format!("writing {}", lib_file.display()))?;
        println!("Wrote {lib_path} (android_main entry point)");
    } else {
        let existing = std::fs::read_to_string(&lib_file)
            .with_context(|| format!("reading {}", lib_file.display()))?;
        if !existing.contains("fn android_main") {
            // Inner doc comments cannot be appended after items; drop them for the append.
            let entry: String = ANDROID_ENTRY
                .lines()
                .filter(|line| !line.trim_start().starts_with("//!"))
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&lib_file, format!("{existing}\n\n{entry}"))
                .with_context(|| format!("writing {}", lib_file.display()))?;
            println!("Appended android_main to {lib_path}");
        }
    }
    for file in new::android_files(config.android.activity) {
        new::write_rendered(root, &file, &vars)?;
    }
    if config.android.activity == AndroidActivity::Native {
        // Switching back to NativeActivity leaves the GameActivity subclass behind.
        let stale = root
            .join("android/app/src/main/java")
            .join(&package_path)
            .join("MainActivity.java");
        let _ = std::fs::remove_file(stale);
    }
    new::write_android_binaries(root)?;
    println!("Wrote android/ Gradle project");
    Ok(())
}

/// Add the Android dependency and the cdylib crate type to Cargo.toml, leaving an existing entry
/// alone so a hand-written dependency spec survives.
pub(crate) fn ensure_android_cargo(
    root: &Path,
    winia_path: Option<PathBuf>,
    activity: AndroidActivity,
    replace: bool,
) -> Result<()> {
    let path = root.join("Cargo.toml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut doc: DocumentMut = text
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    let cfg = "cfg(target_os = \"android\")";
    let present = doc
        .get("target")
        .and_then(|target| target.get(cfg))
        .and_then(|target| target.get("dependencies"))
        .and_then(|dependencies| dependencies.get("winia"))
        .is_some();
    let backend = new::android_feature(activity);
    let features = [backend, "vulkan"];
    if present && replace {
        // Keep the existing path/version; only the backend feature changes.
        set_dependency_features(&mut doc, cfg, "winia", &features, true)?;
        // The direct winia-winit dependency must use the same backend, but keep its own
        // default-features setting: winia decides those, not this rewrite.
        let _ = set_dependency_features(&mut doc, cfg, "winit", &[backend], false);
    } else if !present {
        let spec = new::winia_dependency(winia_path, &features)?;
        set_target_dependency(&mut doc, cfg, &spec)?;
    }
    let winit_present = doc
        .get("target")
        .and_then(|target| target.get(cfg))
        .and_then(|target| target.get("dependencies"))
        .and_then(|dependencies| dependencies.get("winit"))
        .is_some();
    if !winit_present {
        set_target_dependency(&mut doc, cfg, &new::winit_dependency(activity))?;
    }
    ensure_crate_types(&mut doc)?;
    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Replace the feature list of an existing winia dependency, leaving its path or version alone.
fn set_dependency_features(
    doc: &mut DocumentMut,
    filter: &str,
    name: &str,
    features: &[&str],
    default_features_off: bool,
) -> Result<()> {
    use toml_edit::{Array, Item, Value};
    let dependency = doc["target"][filter]["dependencies"]
        .get_mut(name)
        .ok_or_else(|| anyhow::anyhow!("{name} dependency disappeared while rewriting it"))?;
    let array: Array = features
        .iter()
        .map(|feature| Value::from(*feature))
        .collect();
    match dependency {
        Item::Value(Value::InlineTable(table)) => {
            table.insert("features", Value::Array(array));
            if default_features_off {
                table.insert("default-features", Value::from(false));
            }
        }
        Item::Table(table) => {
            table.insert("features", toml_edit::value(array));
            if default_features_off {
                table.insert("default-features", toml_edit::value(false));
            }
        }
        Item::Value(Value::String(version)) => {
            // A bare version string becomes an inline table so it can carry features.
            let mut table = toml_edit::InlineTable::new();
            table.insert("version", Value::from(version.value().clone()));
            if default_features_off {
                table.insert("default-features", Value::from(false));
            }
            table.insert("features", Value::Array(array));
            *dependency = Item::Value(Value::InlineTable(table));
        }
        _ => anyhow::bail!("winia dependency has an unsupported shape in Cargo.toml"),
    }
    Ok(())
}

/// The library target path declared in Cargo.toml, or src/lib.rs.
fn cargo_lib_path(root: &Path) -> String {
    let parsed = std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok());
    parsed
        .as_ref()
        .and_then(|value| value.get("lib"))
        .and_then(|lib| lib.get("path"))
        .and_then(|path| path.as_str())
        .unwrap_or("src/lib.rs")
        .to_owned()
}

/// The Cargo package name; falls back to the last segment of the app id.
fn cargo_package_name(root: &Path, config: &WiniaConfig) -> String {
    let parsed = std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok());
    parsed
        .as_ref()
        .and_then(|value| value.get("package"))
        .and_then(|package| package.get("name"))
        .and_then(|name| name.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| config.app.id.rsplit('.').next().unwrap_or("app").to_owned())
}

fn platform_list(platforms: &[Platform]) -> Vec<String> {
    platforms
        .iter()
        .map(|platform| match platform {
            Platform::Windows => "windows".to_owned(),
            Platform::Macos => "macos".to_owned(),
            Platform::Linux => "linux".to_owned(),
            Platform::Android => "android".to_owned(),
            Platform::Ios => "ios".to_owned(),
        })
        .collect()
}

/// "my-app" becomes "My App"; existing capitalisation is preserved.
fn title_case(package: &str) -> String {
    package
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A parent table that renders only when it has to, so the file shows just the leaf headers.
fn implicit_table() -> Item {
    let mut table = Table::new();
    table.set_implicit(true);
    Item::Table(table)
}

fn set_target_dependency(doc: &mut DocumentMut, cfg: &str, spec: &str) -> Result<()> {
    let target = doc.entry("target").or_insert_with(implicit_table);
    let target = target
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("Cargo.toml [target] is not a table"))?;
    let entry = target.entry(cfg).or_insert_with(implicit_table);
    let entry = entry
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("Cargo.toml target.{cfg} is not a table"))?;
    let dependencies = entry
        .entry("dependencies")
        .or_insert(Item::Table(Table::new()));
    let dependencies = dependencies
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("Cargo.toml target.{cfg}.dependencies is not a table"))?;
    dependencies.insert("winia", dependency_item(spec)?);
    Ok(())
}

fn dependency_item(spec: &str) -> Result<Item> {
    let parsed: DocumentMut = format!("[dependencies]\nwinia = {spec}\n")
        .parse()
        .with_context(|| format!("building the winia dependency from {spec}"))?;
    Ok(parsed["dependencies"]["winia"].clone())
}

fn ensure_crate_types(doc: &mut DocumentMut) -> Result<()> {
    let lib = doc.entry("lib").or_insert(Item::Table(Table::new()));
    let lib = lib
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("Cargo.toml [lib] is not a table"))?;
    let mut crate_types = Array::new();
    crate_types.push("rlib");
    crate_types.push("cdylib");
    lib.insert("crate-type", Item::Value(crate_types.into()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_cargo_gains_the_winit_fork_used_by_the_entry_point() {
        let root = std::env::temp_dir().join(format!("winia-init-winit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[target.'cfg(target_os = \"android\")'.dependencies]\nwinia = { path = \"../winia\", default-features = false, features = [\"android-game-activity\", \"vulkan\"] }\n",
        )
        .unwrap();

        ensure_android_cargo(&root, None, AndroidActivity::Game, false).unwrap();
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(
            manifest.contains("package = \"winia-winit\""),
            "winit fork missing from {manifest}"
        );
        assert!(manifest.contains("crate-type"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn switching_backend_updates_both_dependencies() {
        let root = std::env::temp_dir().join(format!("winia-init-switch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[target.'cfg(target_os = \"android\")'.dependencies]\nwinia = { path = \"../winia\", default-features = false, features = [\"android-game-activity\", \"vulkan\"] }\nwinit = { package = \"winia-winit\", version = \"=0.31.0-beta.4\", features = [\"android-game-activity\"] }\n",
        )
        .unwrap();

        ensure_android_cargo(&root, None, AndroidActivity::Native, true).unwrap();
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("android-native-activity"));
        assert!(!manifest.contains("android-game-activity"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn title_cases_package_names() {
        assert_eq!(title_case("my-app"), "My App");
        assert_eq!(title_case("winia_blank"), "Winia Blank");
        assert_eq!(title_case("simple"), "Simple");
    }

    #[test]
    fn adds_target_dependencies_without_clobbering_others() {
        let mut doc: DocumentMut = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n"
            .parse()
            .unwrap();
        set_target_dependency(&mut doc, "cfg(unix)", "{ version = \"0.1\" }").unwrap();
        set_target_dependency(&mut doc, "cfg(unix)", "{ version = \"0.2\" }").unwrap();
        let text = doc.to_string();
        assert!(text.contains("[target.\"cfg(unix)\".dependencies]"));
        assert!(text.contains("version = \"0.2\""));
        assert!(!text.contains("version = \"0.1\""));
    }
}
