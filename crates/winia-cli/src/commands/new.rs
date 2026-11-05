//! 'winia new' — create a new Winia application from an embedded template.
//!
//! The blank template is a minimal desktop + Android application: a shared UI
//! module, a desktop binary, an `android_main` entry point, and a GameActivity
//! Gradle project. Android support is only scaffolded when the platform list
//! includes `android`; the template files are compiled into the binary so no
//! network access is required to create a project.

use crate::cli::{NewArgs, Platform};
use crate::config::WiniaConfig;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

const VERSION: &str = "0.1.0";

const CARGO_TOML: &str = include_str!("../../templates/blank/Cargo.toml.tmpl");
const MAIN_RS: &str = include_str!("../../templates/blank/src/main.rs.tmpl");
const LIB_RS: &str = include_str!("../../templates/blank/src/lib.rs.tmpl");
const UI_RS: &str = include_str!("../../templates/blank/src/ui.rs.tmpl");
const README: &str = include_str!("../../templates/blank/README.md.tmpl");
const GITIGNORE: &str = include_str!("../../templates/blank/.gitignore");
const BUILD_RS: &str = include_str!("../../templates/blank/build.rs.tmpl");

const ANDROID_SETTINGS: &str =
    include_str!("../../templates/blank/android/settings.gradle.kts.tmpl");
const ANDROID_BUILD: &str = include_str!("../../templates/blank/android/build.gradle.kts.tmpl");
const ANDROID_GRADLE_PROPERTIES: &str =
    include_str!("../../templates/blank/android/gradle.properties.tmpl");
const ANDROID_VERSIONS: &str =
    include_str!("../../templates/blank/android/gradle/libs.versions.toml.tmpl");
const ANDROID_WRAPPER_PROPERTIES: &str =
    include_str!("../../templates/blank/android/gradle/wrapper/gradle-wrapper.properties.tmpl");
const ANDROID_APP_BUILD: &str =
    include_str!("../../templates/blank/android/app/build.gradle.kts.tmpl");
const ANDROID_MANIFEST: &str =
    include_str!("../../templates/blank/android/app/src/main/AndroidManifest.xml.tmpl");
const ANDROID_DEBUG_MANIFEST: &str =
    include_str!("../../templates/blank/android/app/src/debug/AndroidManifest.xml.tmpl");
const ANDROID_ACTIVITY: &str = include_str!(
    "../../templates/blank/android/app/src/main/java/__PACKAGE_PATH__/MainActivity.java.tmpl"
);

const ANDROID_GRADLEW: &[u8] = include_bytes!("../../templates/blank/android/gradlew");
const ANDROID_GRADLEW_BAT: &[u8] = include_bytes!("../../templates/blank/android/gradlew.bat");
const ANDROID_WRAPPER_JAR: &[u8] =
    include_bytes!("../../templates/blank/android/gradle/wrapper/gradle-wrapper.jar");

pub(crate) struct TemplateFile {
    pub(crate) template: &'static str,
    pub(crate) destination: &'static str,
}

const COMMON_FILES: &[TemplateFile] = &[
    TemplateFile {
        template: CARGO_TOML,
        destination: "Cargo.toml",
    },
    TemplateFile {
        template: MAIN_RS,
        destination: "src/main.rs",
    },
    TemplateFile {
        template: LIB_RS,
        destination: "src/lib.rs",
    },
    TemplateFile {
        template: UI_RS,
        destination: "src/ui.rs",
    },
    TemplateFile {
        template: README,
        destination: "README.md",
    },
    TemplateFile {
        template: GITIGNORE,
        destination: ".gitignore",
    },
    TemplateFile {
        template: BUILD_RS,
        destination: "build.rs",
    },
];

const ANDROID_FILES: &[TemplateFile] = &[
    TemplateFile {
        template: ANDROID_SETTINGS,
        destination: "android/settings.gradle.kts",
    },
    TemplateFile {
        template: ANDROID_BUILD,
        destination: "android/build.gradle.kts",
    },
    TemplateFile {
        template: ANDROID_GRADLE_PROPERTIES,
        destination: "android/gradle.properties",
    },
    TemplateFile {
        template: ANDROID_VERSIONS,
        destination: "android/gradle/libs.versions.toml",
    },
    TemplateFile {
        template: ANDROID_WRAPPER_PROPERTIES,
        destination: "android/gradle/wrapper/gradle-wrapper.properties",
    },
    TemplateFile {
        template: ANDROID_APP_BUILD,
        destination: "android/app/build.gradle.kts",
    },
    TemplateFile {
        template: ANDROID_MANIFEST,
        destination: "android/app/src/main/AndroidManifest.xml",
    },
    TemplateFile {
        template: ANDROID_DEBUG_MANIFEST,
        destination: "android/app/src/debug/AndroidManifest.xml",
    },
    TemplateFile {
        template: ANDROID_ACTIVITY,
        destination: "android/app/src/main/java/__PACKAGE_PATH__/MainActivity.java",
    },
];

/// Build configuration owned by 'winia android sync'. Application code (manifest, activity,
/// resources) is deliberately not in this list: those files belong to the project.
const MANAGED_ANDROID: &[TemplateFile] = &[
    TemplateFile {
        template: ANDROID_SETTINGS,
        destination: "android/settings.gradle.kts",
    },
    TemplateFile {
        template: ANDROID_BUILD,
        destination: "android/build.gradle.kts",
    },
    TemplateFile {
        template: ANDROID_GRADLE_PROPERTIES,
        destination: "android/gradle.properties",
    },
    TemplateFile {
        template: ANDROID_VERSIONS,
        destination: "android/gradle/libs.versions.toml",
    },
    TemplateFile {
        template: ANDROID_WRAPPER_PROPERTIES,
        destination: "android/gradle/wrapper/gradle-wrapper.properties",
    },
    TemplateFile {
        template: ANDROID_APP_BUILD,
        destination: "android/app/build.gradle.kts",
    },
];

pub(crate) fn managed_android_files() -> &'static [TemplateFile] {
    MANAGED_ANDROID
}

pub fn run(args: NewArgs) -> Result<()> {
    if args.template != "blank" {
        bail!(
            "unknown template '{}'; the only built-in template is 'blank'",
            args.template
        );
    }

    let root = args
        .path
        .clone()
        .unwrap_or_else(|| PathBuf::from(&args.name));
    if root.exists() {
        let mut entries =
            std::fs::read_dir(&root).with_context(|| format!("reading {}", root.display()))?;
        if entries.next().is_some() {
            bail!("{} already exists and is not empty", root.display());
        }
    }

    let package = package_name(&args.name)?;
    let lib_name = package.replace('-', "_");
    let app_id = format!("com.example.{lib_name}");
    let package_path = app_id.replace('.', "/");
    let platforms = effective_platforms(&args.platforms);
    let winia_android = winia_dependency(
        args.winia_path.clone(),
        &["android-game-activity", "vulkan"],
    )?;
    let winia_desktop = winia_dependency(args.winia_path.clone(), &["gl"])?;

    let vars: Vec<(&str, &str)> = vec![
        ("__APP_NAME__", args.name.as_str()),
        ("__PACKAGE_NAME__", package.as_str()),
        ("__LIB_NAME__", lib_name.as_str()),
        ("__APP_ID__", app_id.as_str()),
        ("__PACKAGE_PATH__", package_path.as_str()),
        ("__VERSION__", VERSION),
        ("__WINIA_DEP_ANDROID__", winia_android.as_str()),
        ("__WINIA_DEP_DESKTOP__", winia_desktop.as_str()),
    ];

    std::fs::create_dir_all(root.join("src"))?;
    std::fs::create_dir_all(root.join("assets"))?;
    for file in COMMON_FILES {
        write_rendered(&root, file, &vars)?;
    }

    let mut config = WiniaConfig::default();
    config.app.name = args.name.clone();
    config.app.id = app_id.clone();
    config.android.package = app_id.clone();
    config.window.title = args.name.clone();
    config.window.width = 1000.0;
    config.window.height = 700.0;
    config.save(&root.join("winia.toml"))?;

    if platforms.contains(&Platform::Android) {
        for file in ANDROID_FILES {
            write_rendered(&root, file, &vars)?;
        }
        write_bytes(&root, "android/gradlew", ANDROID_GRADLEW)?;
        #[cfg(unix)]
        make_executable(&root.join("android/gradlew"))?;
        write_bytes(&root, "android/gradlew.bat", ANDROID_GRADLEW_BAT)?;
        write_bytes(
            &root,
            "android/gradle/wrapper/gradle-wrapper.jar",
            ANDROID_WRAPPER_JAR,
        )?;
        std::fs::create_dir_all(root.join("android/app/src/main/jniLibs/arm64-v8a"))?;
        std::fs::create_dir_all(root.join("android/app/src/main/jniLibs/x86_64"))?;
    }

    if !args.no_git {
        let _ = std::process::Command::new("git")
            .arg("init")
            .current_dir(&root)
            .status();
    }

    println!("Created {} at {}", args.name, root.display());
    println!("Next:");
    println!("  cd {}", root.display());
    println!("  winia doctor");
    println!("  winia run");
    if platforms.contains(&Platform::Android) {
        println!("  winia build --target android");
    }
    Ok(())
}

fn effective_platforms(requested: &[Platform]) -> Vec<Platform> {
    if requested.is_empty() {
        vec![Platform::Windows, Platform::Macos, Platform::Linux]
    } else {
        requested.to_vec()
    }
}

pub(crate) fn package_name(name: &str) -> Result<String> {
    let package = name.trim().to_lowercase().replace(' ', "-");
    if package.is_empty() {
        bail!("application name must not be empty");
    }
    if !package
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("application name may only contain ASCII letters, digits, '-' and '_'");
    }
    Ok(package)
}

/// Render the Cargo dependency value for winia. The crate is not published yet, so
/// a local checkout passed through `--winia-path` or `WINIA_PATH` is required.
pub(crate) fn winia_dependency(explicit: Option<PathBuf>, features: &[&str]) -> Result<String> {
    let path = explicit.or_else(|| std::env::var_os("WINIA_PATH").map(PathBuf::from));
    let features = features
        .iter()
        .map(|feature| format!("\"{feature}\""))
        .collect::<Vec<_>>()
        .join(", ");
    match path {
        Some(path) => {
            // A missing path is left for cargo to report; a readable manifest that is a workspace
            // is the common mistake worth catching here.
            let manifest = path.join("Cargo.toml");
            if let Ok(text) = std::fs::read_to_string(&manifest) {
                if !text.contains("[package]") {
                    bail!(
                        "{} is a workspace manifest, not the winia crate; pass the crate directory instead (for example {})",
                        manifest.display(),
                        path.join("winia").display()
                    );
                }
            }
            Ok(format!(
                "{{ path = \"{}\", default-features = false, features = [{}] }}",
                path.display().to_string().replace('\\', "/"),
                features
            ))
        }
        None => bail!(
            "winia is not published on crates.io yet; pass --winia-path <path-to-winia-crate> or set WINIA_PATH"
        ),
    }
}

pub(crate) fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut rendered = template.to_owned();
    for (key, value) in vars {
        rendered = rendered.replace(key, value);
    }
    rendered
}

pub(crate) fn write_rendered(
    root: &Path,
    file: &TemplateFile,
    vars: &[(&str, &str)],
) -> Result<()> {
    let destination = render(file.destination, vars);
    let path = root.join(&destination);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&path, render(file.template, vars))
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
        .with_context(|| format!("marking {} executable", path.display()))
}

/// The Android project files the blank template installs, for 'winia init' to reuse.
pub(crate) fn android_files() -> &'static [TemplateFile] {
    ANDROID_FILES
}

/// Write the Gradle wrapper binaries and the jniLibs directories.
pub(crate) fn write_android_binaries(root: &Path) -> Result<()> {
    write_bytes(root, "android/gradlew", ANDROID_GRADLEW)?;
    #[cfg(unix)]
    make_executable(&root.join("android/gradlew"))?;
    write_bytes(root, "android/gradlew.bat", ANDROID_GRADLEW_BAT)?;
    write_bytes(
        root,
        "android/gradle/wrapper/gradle-wrapper.jar",
        ANDROID_WRAPPER_JAR,
    )?;
    std::fs::create_dir_all(root.join("android/app/src/main/jniLibs/arm64-v8a"))?;
    std::fs::create_dir_all(root.join("android/app/src/main/jniLibs/x86_64"))?;
    Ok(())
}

fn write_bytes(root: &Path, destination: &str, bytes: &[u8]) -> Result<()> {
    let path = root.join(destination);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_name_normalises_spaces_and_rejects_non_ascii() {
        assert_eq!(package_name("My App").unwrap(), "my-app");
        assert_eq!(package_name("Winia_Blank").unwrap(), "winia_blank");
        assert!(package_name("你好").is_err());
        assert!(package_name("   ").is_err());
    }

    #[test]
    fn render_replaces_placeholders_in_contents_and_paths() {
        let vars = [
            ("__APP_NAME__", "Demo"),
            ("__PACKAGE_PATH__", "com/example/demo"),
        ];
        assert_eq!(
            render("window title = \"__APP_NAME__\"", &vars),
            "window title = \"Demo\""
        );
        assert_eq!(
            render("__PACKAGE_PATH__/MainActivity.java", &vars),
            "com/example/demo/MainActivity.java"
        );
    }

    #[test]
    fn a_local_winia_path_is_written_with_forward_slashes() {
        let dependency =
            winia_dependency(Some(PathBuf::from("D:\\Projects\\winia\\winia")), &["gl"]).unwrap();
        assert!(dependency.contains("path = \"D:/Projects/winia/winia\""));
        assert!(dependency.contains("features = [\"gl\"]"));
        assert!(dependency.contains("default-features = false"));
    }
}
