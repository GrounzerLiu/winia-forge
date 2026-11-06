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
];

/// GameActivity needs a Java subclass; this file is always generated.
const ANDROID_GAME_ACTIVITY: TemplateFile = TemplateFile {
    template: ANDROID_ACTIVITY,
    destination: "android/app/src/main/java/__PACKAGE_PATH__/MainActivity.java",
};

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

const RESERVED_CRATE_NAMES: &[&str] = &["std", "core", "alloc", "test", "proc_macro"];
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "try", "yield", "gen",
];

fn inside_git_worktree(path: &Path) -> bool {
    let mut current = path.parent();
    while let Some(directory) = current {
        if directory.join(".git").exists() {
            return true;
        }
        current = directory.parent();
    }
    false
}

/// A Cargo package name that is also a usable Rust identifier prefix: the template writes
/// `use <name>::ui;`, so it must start with a letter and cannot be a keyword.
fn validate_project_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        bail!("project name must not be empty");
    };
    if !first.is_ascii_alphabetic() {
        bail!("project name '{name}' must start with an ASCII letter");
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("project name '{name}' may only contain letters, digits, '-' and '_'");
    }
    let normalized = name.trim().to_lowercase().replace(' ', "-");
    if RUST_KEYWORDS.contains(&normalized.as_str()) {
        bail!("project name '{name}' is a Rust keyword; choose another name");
    }
    if RESERVED_CRATE_NAMES.contains(&normalized.as_str()) {
        bail!(
            "project name '{name}' collides with the Rust {normalized} crate; choose another name"
        );
    }
    if WINDOWS_RESERVED_NAMES.contains(&normalized.as_str()) {
        bail!("project name '{name}' is a reserved Windows device name; choose another name");
    }
    Ok(())
}

pub fn run(args: NewArgs) -> Result<()> {
    validate_project_name(&args.name)?;
    if args.platforms.contains(&Platform::Ios) {
        bail!("ios support is not implemented yet; choose windows, macos, linux, or android");
    }
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

    let app_name_kotlin = kotlin_string(&args.name);
    let vars: Vec<(&str, &str)> = vec![
        ("__APP_NAME__", args.name.as_str()),
        ("__APP_NAME_KOTLIN__", app_name_kotlin.as_str()),
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
    config.save(&root.join("winia.toml"))?;
    crate::generated::write(&root, &config)?;

    if platforms.contains(&Platform::Android) {
        for file in android_files() {
            write_rendered(&root, &file, &vars)?;
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
        if inside_git_worktree(&root) {
            eprintln!(
                "note: {} is inside an existing git worktree; skipped 'git init'",
                root.display()
            );
        } else {
            match std::process::Command::new("git")
                .arg("init")
                .current_dir(&root)
                .status()
            {
                Ok(status) if status.success() => {}
                Ok(status) => eprintln!(
                    "warning: 'git init' failed with {status}; the project is not a repository"
                ),
                Err(error) => eprintln!("warning: could not run 'git init': {error}"),
            }
        }
    }

    println!("Created {} at {}", args.name, root.display());
    println!("Next:");
    println!("  cd \"{}\"", root.display());
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

/// The first published Winia release. A caret requirement keeps generated projects on 0.1.x.
pub(crate) const WINIA_VERSION: &str = "0.1";

/// Render the Cargo dependency value for winia. A local checkout passed through
/// `--winia-path` or `WINIA_PATH` wins; otherwise the published crate is used.
pub(crate) fn winia_dependency(explicit: Option<PathBuf>, features: &[&str]) -> Result<String> {
    let path = explicit.or_else(|| std::env::var_os("WINIA_PATH").map(PathBuf::from));
    let features = features
        .iter()
        .map(|feature| format!("\"{feature}\""))
        .collect::<Vec<_>>()
        .join(", ");
    match path {
        Some(path) => {
            // Cargo resolves a path dependency relative to the manifest that contains it, while the
            // user passed this path relative to their working directory. Store an absolute path so
            // both interpretations agree.
            let path = std::path::absolute(&path).unwrap_or(path);
            // A missing path is left for cargo to report; a readable manifest that is a workspace
            // is the common mistake worth catching here.
            let manifest = path.join("Cargo.toml");
            if let Ok(text) = std::fs::read_to_string(&manifest)
                && !text.contains("[package]")
            {
                bail!(
                    "{} is a workspace manifest, not the winia crate; pass the crate directory instead (for example {})",
                    manifest.display(),
                    path.join("winia").display()
                );
            }
            Ok(format!(
                "{{ path = \"{}\", default-features = false, features = [{}] }}",
                path.display().to_string().replace('\\', "/"),
                features
            ))
        }
        None => Ok(format!(
            "{{ version = \"{WINIA_VERSION}\", default-features = false, features = [{}] }}",
            features
        )),
    }
}

/// Escape a value for a Kotlin string literal (quotes, backslashes and $ templates).
pub(crate) fn kotlin_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
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
pub(crate) fn android_files() -> Vec<TemplateFile> {
    let mut files: Vec<TemplateFile> = ANDROID_FILES
        .iter()
        .map(|file| TemplateFile {
            template: file.template,
            destination: file.destination,
        })
        .collect();
    files.push(ANDROID_GAME_ACTIVITY);
    files
}

/// Version of the published winit fork; the template and 'winia init' must pin the same one.
pub(crate) const WINIT_VERSION: &str = "=0.31.0-beta.4";

/// The only supported Android backend. The generated Java activity is GameActivity-based.
pub(crate) const ANDROID_BACKEND: &str = "android-game-activity";

/// The direct winit dependency an Android project needs: the generated entry point takes
/// `winit::platform::android::activity::AndroidApp`.
pub(crate) fn winit_dependency() -> String {
    format!(
        "{{ package = \"winia-winit\", version = \"{WINIT_VERSION}\", features = [\"{ANDROID_BACKEND}\"] }}"
    )
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
    fn rejects_names_that_cannot_be_rust_identifiers() {
        for name in [
            "123app",
            "type",
            "my app",
            "app!",
            "",
            "Type",
            "SELF",
            "std",
            "core",
            "alloc",
            "test",
            "proc_macro",
            "con",
            "nul",
            "com1",
        ] {
            assert!(
                validate_project_name(name).is_err(),
                "{name:?} should be rejected"
            );
        }
        for name in ["my-app", "my_app", "App2"] {
            assert!(
                validate_project_name(name).is_ok(),
                "{name:?} should be accepted"
            );
        }
    }

    #[test]
    fn android_files_always_include_game_activity() {
        let files = android_files();
        assert!(
            files
                .iter()
                .any(|file| file.destination.ends_with("MainActivity.java"))
        );
        assert!(winit_dependency().contains("android-game-activity"));
    }

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
        // Use an absolute path from this host so the test also runs on Linux/macOS CI.
        let source = std::env::temp_dir().join("winia-path");
        let expected = std::path::absolute(&source)
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/");
        let dependency = winia_dependency(Some(source), &["gl"]).unwrap();
        assert!(
            dependency.contains(&format!("path = \"{expected}\"")),
            "{dependency}"
        );
        assert!(dependency.contains("features = [\"gl\"]"));
        assert!(dependency.contains("default-features = false"));
    }
}
