//! The KiCad plugin as a package for KiCad's Plugin and Content Manager, one
//! per platform: `kicad-plugin/` with a copy of the platform's traxide as each
//! action's entry point, which traxide tells apart by its name. *Install from File* unpacks `plugins/` and `resources/` into
//! KiCad's `3rdparty` folder.

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The identifier of `plugin.json` and `metadata.json`.
pub const IDENTIFIER: &str = "com.github.h3ll5ur7er.traxide";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
    Macos,
}

impl Platform {
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "linux" => Self::Linux,
            "windows" => Self::Windows,
            "macos" => Self::Macos,
            _ => bail!("no platform {name}: linux, windows or macos"),
        })
    }

    /// The name KiCad's package manager gives the platform.
    fn pcm_name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::Macos => "macos",
        }
    }

    /// The file of the executable `name` on the platform.
    fn executable(self, name: &str) -> String {
        match self {
            Self::Windows => format!("{name}.exe"),
            Self::Linux | Self::Macos => name.to_owned(),
        }
    }
}

/// `cargo xtask kicad-package PLATFORM TRAXIDE PACKAGE.zip`
pub fn run(args: &[String]) -> Result<()> {
    let [platform, traxide, zip] = args else {
        bail!("usage: cargo xtask kicad-package linux|windows|macos TRAXIDE PACKAGE.zip");
    };
    package(
        Platform::parse(platform)?,
        Path::new(traxide),
        Path::new(zip),
    )
}

/// Packages `traxide`, built for `platform`, into the zip file `zip`.
pub fn package(platform: Platform, traxide: &Path, zip: &Path) -> Result<()> {
    let staging = zip.with_extension("staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    assemble(platform, traxide, &staging)?;
    let zip = std::path::absolute(zip)?;
    if zip.exists() {
        std::fs::remove_file(&zip)?;
    }
    // Info-ZIP keeps the executable bit, which KiCad restores on Linux.
    let zipped = Command::new("zip")
        .args(["-qrX"])
        .arg(&zip)
        .args(["metadata.json", "plugins", "resources"])
        .current_dir(&staging)
        .status()
        .context("running zip: is it installed?")?;
    ensure!(zipped.success(), "zip failed");
    std::fs::remove_dir_all(&staging)?;
    Ok(())
}

/// Lays the package out in the folder `staging`.
fn assemble(platform: Platform, traxide: &Path, staging: &Path) -> Result<()> {
    let source = repository().join("kicad-plugin");
    let plugins = staging.join("plugins");
    let resources = staging.join("resources");
    std::fs::create_dir_all(&plugins)?;
    std::fs::create_dir_all(&resources)?;
    std::fs::copy(
        source.join("resources/icon.png"),
        resources.join("icon.png"),
    )?;
    let mut plugin = read_json(&source.join("plugins/plugin.json"))?;
    for action in plugin["actions"]
        .as_array_mut()
        .context("plugin.json has actions")?
    {
        let name = action["entrypoint"]
            .as_str()
            .context("each action has an entry point")?;
        let entry_point = platform.executable(name);
        let executable = plugins.join(&entry_point);
        std::fs::copy(traxide, &executable)
            .with_context(|| format!("copying {}", traxide.display()))?;
        make_executable(&executable)?;
        action["entrypoint"] = json!(entry_point);
        for icons in ["icons-light", "icons-dark"] {
            for icon in action[icons].as_array().context("each action has icons")? {
                let icon = icon.as_str().context("an icon's file name")?;
                std::fs::copy(source.join("plugins").join(icon), plugins.join(icon))?;
            }
        }
    }
    write_json(&plugins.join("plugin.json"), &plugin)?;

    let mut metadata = read_json(&source.join("metadata.json"))?;
    let version = &mut metadata["versions"][0];
    version["version"] = json!(env!("CARGO_PKG_VERSION"));
    version["platforms"] = json!([platform.pcm_name()]);
    write_json(&staging.join("metadata.json"), &metadata)
}

/// Downloaded build artifacts lose their mode, and KiCad starts the entry
/// point only if it is executable.
fn make_executable(file: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn read_json(path: &Path) -> Result<Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    Ok(std::fs::write(
        path,
        serde_json::to_string_pretty(value)? + "\n",
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assembled(platform: Platform) -> tempfile::TempDir {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let traxide = folder.path().join("built-traxide");
        std::fs::write(&traxide, "a binary").expect("written");
        assemble(platform, &traxide, &folder.path().join("package")).expect("assembled");
        folder
    }

    fn json_in(folder: &tempfile::TempDir, file: &str) -> Value {
        read_json(&folder.path().join("package").join(file)).expect("valid JSON")
    }

    #[test]
    fn each_button_starts_a_copy_of_the_platforms_traxide_named_for_it() {
        for (platform, extension) in [
            (Platform::Linux, ""),
            (Platform::Macos, ""),
            (Platform::Windows, ".exe"),
        ] {
            let folder = assembled(platform);
            let plugin = json_in(&folder, "plugins/plugin.json");
            assert_eq!(plugin["identifier"], IDENTIFIER);
            assert_eq!(plugin["runtime"]["type"], "exec");
            let actions = plugin["actions"].as_array().expect("actions");
            let buttons: Vec<(&str, &str)> = actions
                .iter()
                .map(|action| {
                    (
                        action["name"].as_str().expect("a name"),
                        action["entrypoint"].as_str().expect("an entry point"),
                    )
                })
                .collect();
            assert_eq!(
                buttons,
                [
                    ("Route board", format!("traxide{extension}").as_str()),
                    (
                        "Route selected nets",
                        format!("traxide-selected{extension}").as_str()
                    ),
                ]
            );
            for (_, entry_point) in buttons {
                let installed = folder.path().join("package/plugins").join(entry_point);
                assert_eq!(
                    std::fs::read_to_string(installed).expect("traxide in the package"),
                    "a binary"
                );
            }
        }
    }

    #[test]
    fn the_package_is_for_its_platform_and_this_version_of_traxide() {
        let folder = assembled(Platform::Windows);
        let metadata = json_in(&folder, "metadata.json");
        assert_eq!(metadata["identifier"], IDENTIFIER);
        let version = &metadata["versions"][0];
        assert_eq!(version["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(version["platforms"], json!(["windows"]));
        assert_eq!(version["kicad_version"], "9.0");
    }

    #[test]
    fn the_package_has_toolbar_icons_for_light_and_dark_themes_and_an_icon_for_the_package_manager()
    {
        let folder = assembled(Platform::Linux);
        let package = folder.path().join("package");
        let plugin = json_in(&folder, "plugins/plugin.json");
        for action in plugin["actions"].as_array().expect("actions") {
            for icons in ["icons-light", "icons-dark"] {
                for icon in action[icons].as_array().expect("icons") {
                    let icon = icon.as_str().expect("a file name");
                    assert!(package.join("plugins").join(icon).is_file(), "{icon}");
                }
            }
        }
        assert!(package.join("resources/icon.png").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn traxide_in_the_package_is_executable() {
        use std::os::unix::fs::PermissionsExt;
        let folder = assembled(Platform::Linux);
        for entry_point in ["traxide", "traxide-selected"] {
            let mode = std::fs::metadata(folder.path().join("package/plugins").join(entry_point))
                .expect("traxide in the package")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "{entry_point}");
        }
    }

    #[test]
    fn says_the_kicad_api_must_be_enabled_first() {
        let metadata = read_json(&repository().join("kicad-plugin/metadata.json")).expect("JSON");
        for field in ["description", "description_full"] {
            let text = metadata[field].as_str().expect("text");
            assert!(text.contains("KiCad API"), "{field}: {text}");
        }
    }
}
