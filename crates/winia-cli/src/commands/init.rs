//! 'winia init' — add Winia to an existing Cargo project.
//!
//! Unlike 'winia new', this never rewrites the user's sources: it adds the framework dependency
//! to Cargo.toml, writes winia.toml when absent, creates assets/, and — when Android is requested —
//! generates the Gradle project and src/lib.rs only if they do not exist yet.

use crate::cli::{InitArgs, Platform};
use crate::commands::new;
use crate::config::WiniaConfig;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

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
    let package_version = package_version(&root, &doc).unwrap_or_else(|| "0.1.0".to_owned());

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
    if android && root.join("android").exists() && !args.force {
        bail!(
            "{} already contains android/; pass --force to regenerate it, or run 'winia android sync' to update only the managed Gradle files",
            root.display()
        );
    }

    let mut config = WiniaConfig::default();
    config.app.name = app_name.clone();
    config.app.id = app_id.clone();
    config.app.version = package_version;
    config.android.package = app_id.clone();
    config.window.title = app_name.clone();
    config.window.width = 1000.0;
    config.window.height = 700.0;
    config.validate()?;
    if android {
        // Validate the lib path before touching Cargo.toml or winia.toml so an escaping
        // manifest cannot leave a half-initialized project.
        checked_lib_path(&root, &cargo_lib_path(&root))?;
    }

    let android_feature = new::ANDROID_BACKEND;
    let desktop_features = ["gl"];
    let android_features = [android_feature, "vulkan"];

    let plain = dependency_spec(&doc, None);
    let desktop_spec = dependency_spec(&doc, Some(false));
    let android_spec = dependency_spec(&doc, Some(true));
    if let Some((_, mut spec)) = plain {
        // A single dependency for all targets cannot coexist with target-specific ones:
        // Cargo requires one canonical source per dependency. Merge every required feature.
        merge_dependency_features(&mut spec, &["gl", android_feature, "vulkan"])?;
        insert_dependency_spec(&mut doc, None, "winia", spec)?;
    } else if desktop_spec.is_some() || android_spec.is_some() {
        merge_existing_target_dependency(
            &mut doc,
            false,
            "winia",
            &desktop_features,
            android_spec.as_ref().map(|(_, spec)| spec),
        )?;
        merge_existing_target_dependency(
            &mut doc,
            true,
            "winia",
            &android_features,
            desktop_spec.as_ref().map(|(_, spec)| spec),
        )?;
    } else {
        let winia_android = new::winia_dependency(args.winia_path.clone(), &android_features)?;
        let winia_desktop = new::winia_dependency(args.winia_path.clone(), &desktop_features)?;
        set_target_dependency(
            &mut doc,
            "cfg(not(target_os = \"android\"))",
            "winia",
            &winia_desktop,
        )?;
        set_target_dependency(
            &mut doc,
            "cfg(target_os = \"android\")",
            "winia",
            &winia_android,
        )?;
    }
    if android {
        // The Android entry point names winit directly, so the fork must be a dependency.
        set_target_dependency(
            &mut doc,
            "cfg(target_os = \"android\")",
            "winit",
            &new::winit_dependency(),
        )?;
        ensure_crate_types(&mut doc)?;
    }
    std::fs::write(&cargo_toml, doc.to_string())
        .with_context(|| format!("writing {}", cargo_toml.display()))?;
    println!(
        "Updated Cargo.toml (winia dependency{})",
        if android { " + lib crate types" } else { "" }
    );

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
    let lib_name = cargo_lib_name(root).unwrap_or_else(|| package.replace('-', "_"));
    let app_id = if config.android.package.trim().is_empty() {
        config.app.id.clone()
    } else {
        config.android.package.clone()
    };
    let package_path = app_id.replace('.', "/");
    let app_name_kotlin = new::kotlin_string(&config.app.name);
    let vars: Vec<(&str, &str)> = vec![
        ("__APP_NAME__", config.app.name.as_str()),
        ("__APP_NAME_KOTLIN__", app_name_kotlin.as_str()),
        ("__PACKAGE_NAME__", package.as_str()),
        ("__LIB_NAME__", lib_name.as_str()),
        ("__APP_ID__", app_id.as_str()),
        ("__PACKAGE_PATH__", package_path.as_str()),
        ("__VERSION__", config.app.version.as_str()),
    ];
    // Respect an existing [lib] path: a crate whose library lives in src/core.rs must not grow a
    // second, unused src/lib.rs. Reject paths that escape the project before creating anything.
    let lib_path = cargo_lib_path(root);
    let lib_file = checked_lib_path(root, &lib_path)?;
    let lib_path = lib_file
        .strip_prefix(root)
        .unwrap_or(&lib_file)
        .display()
        .to_string();
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
    // 'winia init' writes src/winia_config.rs, so the library must declare the module or the
    // generated window values are silently unused.
    if let Ok(existing) = std::fs::read_to_string(&lib_file)
        && !existing.contains("mod winia_config")
    {
        let target = std::path::absolute(root.join(crate::generated::FILE))
            .with_context(|| "resolving the generated config path")?;
        let module_path = relative_module_path(&lib_file, &target)?;
        std::fs::write(
            &lib_file,
            format!("{existing}\n#[path = \"{module_path}\"]\npub mod winia_config;\n"),
        )
        .with_context(|| format!("writing {}", lib_file.display()))?;
    }
    for file in new::android_files() {
        new::write_rendered(root, &file, &vars)?;
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
    let backend = new::ANDROID_BACKEND;
    let features = [backend, "vulkan"];
    if let Some((_, mut plain)) = dependency_spec(&doc, None) {
        // A plain [dependencies] winia applies to every target; adding a target-specific entry
        // would make Cargo reject the manifest for having two sources.
        merge_dependency_features(&mut plain, &features)?;
        insert_dependency_spec(&mut doc, None, "winia", plain)?;
    } else if present && replace {
        // Keep the existing path/version; only the backend feature changes.
        set_dependency_features(&mut doc, cfg, "winia", &features, true)?;
        // The direct winia-winit dependency must use the same backend, but keep its own
        // default-features setting: winia decides those, not this rewrite.
        let _ = set_dependency_features(&mut doc, cfg, "winit", &[backend], false);
    } else if !present {
        let spec = new::winia_dependency(winia_path, &features)?;
        set_target_dependency(&mut doc, cfg, "winia", &spec)?;
    }
    let winit_present = doc
        .get("target")
        .and_then(|target| target.get(cfg))
        .and_then(|target| target.get("dependencies"))
        .and_then(|dependencies| dependencies.get("winit"))
        .is_some();
    if !winit_present {
        set_target_dependency(&mut doc, cfg, "winit", &new::winit_dependency())?;
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

/// The library target name declared in Cargo.toml, or None when it defaults to the package name.
fn cargo_lib_name(root: &Path) -> Option<String> {
    let parsed = std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok());
    parsed
        .as_ref()
        .and_then(|value| value.get("lib"))
        .and_then(|lib| lib.get("name"))
        .and_then(|name| name.as_str())
        .map(str::to_owned)
}

/// Resolve the manifest-declared lib path and keep it inside the project root.
fn checked_lib_path(root: &Path, lib_path: &str) -> Result<PathBuf> {
    let root = normalize_path(
        &std::path::absolute(root).with_context(|| format!("resolving {}", root.display()))?,
    );
    let candidate = Path::new(lib_path);
    if candidate.is_absolute() {
        bail!(
            "[lib] path must be relative to the project, got '{}'",
            candidate.display()
        );
    }
    let absolute = normalize_path(
        &std::path::absolute(root.join(candidate))
            .with_context(|| format!("resolving [lib] path '{}'", candidate.display()))?,
    );
    if !absolute.starts_with(&root) || absolute == root {
        bail!(
            "[lib] path '{}' escapes the project root; refusing to write outside {}",
            lib_path,
            root.display()
        );
    }
    Ok(absolute)
}

/// Lexically remove '.' and '..' components; absolute() alone does not guarantee that a path
/// such as root/../victim stops being reported as a child of root.
fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Relative path from the module file to the generated config file, in Rust #[path] form.
fn relative_module_path(module: &Path, target: &Path) -> Result<String> {
    use std::path::Component;
    let from = module
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", module.display()))?;
    let from_components: Vec<Component<'_>> = from.components().collect();
    let target_components: Vec<Component<'_>> = target.components().collect();
    let common = from_components
        .iter()
        .zip(&target_components)
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0 {
        bail!(
            "cannot express {} relative to {}",
            target.display(),
            from.display()
        );
    }
    let mut parts = Vec::new();
    for _ in common..from_components.len() {
        parts.push("..".to_owned());
    }
    for component in &target_components[common..] {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    Ok(parts.join("/"))
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

/// The package version, following `version.workspace = true` into the workspace manifest.
fn package_version(root: &Path, doc: &DocumentMut) -> Option<String> {
    let item = doc.get("package")?.get("version")?;
    if let Some(value) = item.as_str() {
        return Some(value.to_owned());
    }
    let inherits = item
        .get("workspace")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !inherits {
        return None;
    }
    let mut directory = Some(root);
    while let Some(current) = directory {
        let manifest = current.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest)
            && let Ok(value) = text.parse::<toml::Value>()
            && let Some(version) = value
                .get("workspace")
                .and_then(|workspace| workspace.get("package"))
                .and_then(|package| package.get("version"))
                .and_then(|version| version.as_str())
        {
            return Some(version.to_owned());
        }
        directory = current.parent();
    }
    None
}

/// A parent table that renders only when it has to, so the file shows just the leaf headers.
fn implicit_table() -> Item {
    let mut table = Table::new();
    table.set_implicit(true);
    Item::Table(table)
}

/// Locate the winia dependency for all targets (None) or a target-specific table.
fn dependency_spec(doc: &DocumentMut, android: Option<bool>) -> Option<(String, Item)> {
    match android {
        None => doc
            .get("dependencies")
            .and_then(|dependencies| dependencies.get("winia"))
            .cloned()
            .map(|item| ("dependencies".to_owned(), item)),
        Some(android) => {
            for (key, entry) in doc.get("target")?.as_table()? {
                let mentions_android = key.contains("android");
                let is_desktop = mentions_android && key.contains("not(");
                if mentions_android
                    && (is_desktop != android)
                    && let Some(item) = entry.get("dependencies").and_then(|deps| deps.get("winia"))
                {
                    return Some((key.to_owned(), item.clone()));
                }
            }
            None
        }
    }
}

fn canonical_target(android: bool) -> &'static str {
    if android {
        "cfg(target_os = \"android\")"
    } else {
        "cfg(not(target_os = \"android\"))"
    }
}

/// Merge a missing target's dependency from the other target, preserving its source (path,
/// version, git) and other keys. This avoids Cargo's "different source paths depending on the
/// build target" error when a project already declares winia for only one target.
fn merge_existing_target_dependency(
    doc: &mut DocumentMut,
    android: bool,
    name: &str,
    features: &[&str],
    fallback: Option<&Item>,
) -> Result<()> {
    if let Some((key, mut spec)) = dependency_spec(doc, Some(android)) {
        merge_dependency_features(&mut spec, features)?;
        insert_dependency_spec(doc, Some(&key), name, spec)?;
    } else if let Some(fallback) = fallback {
        // The target had no winia entry: copy the other target's source, then use exactly the
        // features this target needs instead of inheriting desktop-only features such as gl.
        let spec = fallback.clone();
        let cfg = canonical_target(android);
        insert_dependency_spec(doc, Some(cfg), name, spec)?;
        set_dependency_features(doc, cfg, name, features, true)?;
    }
    Ok(())
}

fn merge_dependency_features(spec: &mut Item, features: &[&str]) -> Result<()> {
    match spec {
        Item::Value(Value::String(version)) => {
            let mut table = InlineTable::new();
            table.insert("version", Value::from(version.value().clone()));
            add_inline_features(&mut table, features)?;
            *spec = Item::Value(Value::InlineTable(table));
        }
        Item::Value(Value::InlineTable(table)) => add_inline_features(table, features)?,
        Item::Table(table) => {
            let mut array = table
                .get("features")
                .and_then(|item| item.as_array())
                .cloned()
                .unwrap_or_default();
            for feature in features {
                if !array.iter().any(|value| value.as_str() == Some(*feature)) {
                    array.push(*feature);
                }
            }
            table.insert("features", Item::Value(Value::Array(array)));
        }
        _ => bail!(
            "the existing winia dependency has an unsupported shape; edit Cargo.toml manually"
        ),
    }
    Ok(())
}

fn add_inline_features(table: &mut InlineTable, features: &[&str]) -> Result<()> {
    let mut array = table
        .get("features")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for feature in features {
        if !array.iter().any(|value| value.as_str() == Some(*feature)) {
            array.push(*feature);
        }
    }
    table.insert("features", Value::Array(array));
    Ok(())
}

fn insert_dependency_spec(
    doc: &mut DocumentMut,
    target: Option<&str>,
    name: &str,
    spec: Item,
) -> Result<()> {
    match target {
        None => {
            let dependencies = doc.entry("dependencies").or_insert_with(implicit_table);
            let dependencies = dependencies
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("Cargo.toml [dependencies] is not a table"))?;
            dependencies.insert(name, spec);
        }
        Some(cfg) => {
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
            let dependencies = dependencies.as_table_mut().ok_or_else(|| {
                anyhow::anyhow!("Cargo.toml target.{cfg}.dependencies is not a table")
            })?;
            dependencies.insert(name, spec);
        }
    }
    Ok(())
}

fn set_target_dependency(doc: &mut DocumentMut, cfg: &str, name: &str, spec: &str) -> Result<()> {
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
    dependencies.insert(name, dependency_item(name, spec)?);
    Ok(())
}

fn dependency_item(name: &str, spec: &str) -> Result<Item> {
    let parsed: DocumentMut = format!("[dependencies]\n{name} = {spec}\n")
        .parse()
        .with_context(|| format!("building the {name} dependency from {spec}"))?;
    Ok(parsed["dependencies"][name].clone())
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

        ensure_android_cargo(&root, None, false).unwrap();
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(
            manifest.contains("winit = { package = \"winia-winit\""),
            "winit fork missing from {manifest}"
        );
        assert!(
            manifest.contains("winia = { path = \"../winia\""),
            "winia path dependency was clobbered: {manifest}"
        );
        assert!(manifest.contains("crate-type"));
        let _ = std::fs::remove_dir_all(root);
    }


    #[test]
    fn title_cases_package_names() {
        assert_eq!(title_case("my-app"), "My App");
        assert_eq!(title_case("winia_blank"), "Winia Blank");
        assert_eq!(title_case("simple"), "Simple");
    }

    #[test]
    fn lib_paths_must_stay_inside_the_project() {
        let root = std::env::temp_dir().join(format!("winia-lib-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        assert!(checked_lib_path(&root, "src/lib.rs").is_ok());
        assert!(checked_lib_path(&root, "../outside.rs").is_err());
        assert!(checked_lib_path(&root, "src/../../outside.rs").is_err());
        // Platform-neutral absolute path: "D:/outside.rs" is relative on Linux.
        let outside = std::env::temp_dir().join("winia-outside-lib.rs");
        assert!(checked_lib_path(&root, &outside.to_string_lossy()).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn generated_module_path_is_relative_to_the_lib_file() {
        let root = std::env::temp_dir().join(format!("winia-lib-mod-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/core")).unwrap();
        let root = std::path::absolute(&root).unwrap();
        let target = root.join("src/winia_config.rs");
        assert_eq!(
            relative_module_path(&root.join("src/lib.rs"), &target).unwrap(),
            "winia_config.rs"
        );
        assert_eq!(
            relative_module_path(&root.join("src/core/mod.rs"), &target).unwrap(),
            "../winia_config.rs"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn existing_target_source_is_copied_to_the_missing_target() {
        let mut doc: DocumentMut = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[target.'cfg(not(target_os = \"android\"))'.dependencies]\nwinia = { path = \"../winia\", default-features = false, features = [\"gl\"] }\n"
            .parse()
            .unwrap();
        let desktop = dependency_spec(&doc, Some(false));
        merge_existing_target_dependency(
            &mut doc,
            true,
            "winia",
            &["android-game-activity", "vulkan"],
            desktop.as_ref().map(|(_, spec)| spec),
        )
        .unwrap();
        let text = doc.to_string();
        assert!(text.contains("path = \"../winia\""), "{text}");
        assert!(text.contains("cfg(target_os = \"android\")"), "{text}");
        assert!(!text.contains("features = [\"gl\", \"android-game-activity\""));
        let _ = doc;
    }

    #[test]
    fn adds_target_dependencies_without_clobbering_others() {
        let mut doc: DocumentMut = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n"
            .parse()
            .unwrap();
        set_target_dependency(&mut doc, "cfg(unix)", "winia", "{ version = \"0.1\" }").unwrap();
        set_target_dependency(&mut doc, "cfg(unix)", "winia", "{ version = \"0.2\" }").unwrap();
        let text = doc.to_string();
        assert!(text.contains("[target.\"cfg(unix)\".dependencies]"));
        assert!(text.contains("version = \"0.2\""));
        assert!(!text.contains("version = \"0.1\""));
    }
}
