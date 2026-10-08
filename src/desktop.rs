//! Application-owned Linux desktop assets, installed with rollback alongside updates.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};
use tempfile::NamedTempFile;

type Result<T> = std::result::Result<T, String>;
pub const ICON_SIZES: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];
pub const MANIFEST: &str = "share/signal-forge/install-manifest.json";
const LAUNCHER: &str = "share/applications/signal-forge.desktop";
const UNINSTALL: &str = "share/signal-forge/uninstall.sh";
const MAX_ASSET: usize = 8 * 1024 * 1024;
const MAX_ASSETS: usize = 32 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
struct Manifest {
    owner: String,
    version: u32,
    files: Vec<String>,
}
#[derive(Default)]
pub struct Assets {
    files: BTreeMap<String, Vec<u8>>,
}
/// Only our launcher, named icons, documentation and uninstall helper are owned.
/// Neither archive names nor a modified installed manifest can name arbitrary files.
pub fn owned_path(path: &str) -> bool {
    let clean = !path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    clean
        && (path == LAUNCHER
            || path == UNINSTALL
            || path == MANIFEST
            || ICON_SIZES.iter().any(|size| {
                path == format!("share/icons/hicolor/{size}x{size}/apps/signal-forge.png")
            })
            || (path.starts_with("share/doc/signal-forge/") && path.len() <= 512))
}
impl Assets {
    pub fn add(&mut self, path: String, bytes: Vec<u8>) -> Result<()> {
        if !owned_path(&path) || path == MANIFEST {
            return Err(format!("Unexpected installation asset: {path}"));
        }
        if bytes.len() > MAX_ASSET
            || self.files.len() >= 128
            || self.files.values().map(Vec::len).sum::<usize>() + bytes.len() > MAX_ASSETS
        {
            return Err("Installation assets exceed the size limit".into());
        }
        if self.files.insert(path, bytes).is_some() {
            return Err("Duplicate installation asset".into());
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        if !self.files.contains_key(LAUNCHER)
            || !self.files.contains_key(UNINSTALL)
            || ICON_SIZES.iter().any(|size| {
                !self.files.contains_key(&format!(
                    "share/icons/hicolor/{size}x{size}/apps/signal-forge.png"
                ))
            })
        {
            return Err("Release is missing its launcher, icons or uninstall helper".into());
        }
        Ok(())
    }
    pub fn from_package(root: &Path) -> Result<Self> {
        let manifest =
            read_manifest(&root.join(MANIFEST))?.ok_or("Package has no installation manifest")?;
        let mut assets = Self::default();
        for path in manifest.files {
            if path == MANIFEST {
                continue;
            }
            let source = checked_target(root, &path)?;
            let metadata = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
            if !metadata.is_file() || metadata.len() > MAX_ASSET as u64 {
                return Err(format!("Invalid package asset: {path}"));
            }
            assets.add(path, fs::read(source).map_err(|e| e.to_string())?)?;
        }
        assets.validate()?;
        validate_manifest(
            &fs::read(root.join(MANIFEST)).map_err(|e| e.to_string())?,
            &assets,
        )?;
        Ok(assets)
    }
    pub fn write_manifest(&self, root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("share/signal-forge")).map_err(|e| e.to_string())?;
        fs::write(root.join(MANIFEST), self.manifest_bytes()?).map_err(|e| e.to_string())
    }
    fn manifest_bytes(&self) -> Result<Vec<u8>> {
        let mut files: Vec<_> = self.files.keys().cloned().collect();
        files.push(MANIFEST.into());
        serde_json::to_vec_pretty(&Manifest {
            owner: "signal-forge".into(),
            version: 1,
            files,
        })
        .map_err(|e| e.to_string())
    }
    pub fn prepare(mut self, prefix: &Path) -> Result<PreparedAssets> {
        self.validate()?;
        let prefix = fs::canonicalize(prefix).map_err(|e| e.to_string())?;
        let previous = read_manifest(&checked_target(&prefix, MANIFEST)?)?;
        let launcher =
            String::from_utf8(self.files.remove(LAUNCHER).unwrap()).map_err(|e| e.to_string())?;
        self.files.insert(
            LAUNCHER.into(),
            render_launcher(&launcher, &prefix.join("bin/signal-forge"))?.into_bytes(),
        );
        self.files.insert(MANIFEST.into(), self.manifest_bytes()?);
        let mut changes = Vec::new();
        if let Some(previous) = previous {
            for path in previous.files {
                if !self.files.contains_key(&path) {
                    changes.push(stage_change(&prefix, &path, None)?);
                }
            }
        }
        // Publish the ownership manifest last, after its files have been installed.
        let manifest = self.files.remove(MANIFEST).unwrap();
        for (path, bytes) in self.files {
            changes.push(stage_change(&prefix, &path, Some(&bytes))?);
        }
        changes.push(stage_change(&prefix, MANIFEST, Some(&manifest))?);
        Ok(PreparedAssets { prefix, changes })
    }
}
/// The package manifest must describe exactly the staged, allowlisted asset set.
pub fn validate_manifest(bytes: &[u8], assets: &Assets) -> Result<()> {
    if bytes.len() > 64 * 1024 {
        return Err("Installation manifest exceeds size limit".into());
    }
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let expected: std::collections::BTreeSet<_> = assets
        .files
        .keys()
        .map(String::as_str)
        .chain(std::iter::once(MANIFEST))
        .collect();
    if manifest.owner != "signal-forge"
        || manifest.version != 1
        || manifest.files.len() != expected.len()
        || manifest
            .files
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>()
            != expected
    {
        return Err("Package manifest does not match its installation assets".into());
    }
    Ok(())
}
fn read_manifest(path: &Path) -> Result<Option<Manifest>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(meta) if meta.is_file() && meta.len() <= 64 * 1024 => {}
        _ => return Err("Invalid installation manifest file".into()),
    }
    let manifest: Manifest = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if manifest.owner != "signal-forge"
        || manifest.version != 1
        || manifest.files.len() > 129
        || manifest.files.iter().any(|p| !owned_path(p))
        || manifest
            .files
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != manifest.files.len()
    {
        return Err("Invalid installation ownership manifest".into());
    }
    Ok(Some(manifest))
}
fn checked_target(prefix: &Path, relative: &str) -> Result<PathBuf> {
    if !owned_path(relative) {
        return Err("Installation path is not application-owned".into());
    }
    checked_path(prefix, relative)
}
fn checked_path(prefix: &Path, relative: &str) -> Result<PathBuf> {
    let target = prefix.join(relative);
    let mut path = prefix.to_path_buf();
    for component in Path::new(relative).components() {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "Refusing a symlink in installation assets: {}",
                    path.display()
                ))
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
    }
    Ok(target)
}
pub fn render_launcher(source: &str, executable: &Path) -> Result<String> {
    let exe = executable
        .to_str()
        .ok_or("Installation path must be UTF-8")?;
    if exe.chars().any(char::is_control) {
        return Err("Installation path cannot contain control characters".into());
    }
    // Exec first quotes a single argument; Desktop Entry string escaping is a second layer.
    let quoted = exe
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    let exec = format!("Exec=\"{}\"", quoted.replace('\\', "\\\\"));
    let mut found = false;
    let mut lines = Vec::new();
    for line in source.lines() {
        if line.starts_with("Exec=") {
            if found {
                return Err("Duplicate launcher Exec".into());
            }
            found = true;
            lines.push(exec.clone());
        } else {
            lines.push(line.to_owned());
        }
    }
    if !found
        || !lines.iter().any(|line| line == "Name=Signal Forge")
        || !lines.iter().any(|line| line == "Icon=signal-forge")
    {
        return Err("Invalid Signal Forge launcher".into());
    }
    Ok(lines.join("\n") + "\n")
}
struct Change {
    target: PathBuf,
    staged: Option<NamedTempFile>,
    backup: Option<NamedTempFile>,
}
fn stage_change(prefix: &Path, relative: &str, bytes: Option<&[u8]>) -> Result<Change> {
    let target = if relative == "bin/signal-forge" {
        checked_path(prefix, relative)?
    } else {
        checked_target(prefix, relative)?
    };
    let parent = target.parent().unwrap();
    fs::create_dir_all(parent)
        .map_err(|e| format!("Desktop asset directory is not writable: {e}"))?;
    let backup = match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.permissions().mode() & 0o222 == 0 {
                return Err(format!(
                    "Desktop asset is not a writable regular file: {}",
                    target.display()
                ));
            }
            let mut backup = NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            std::io::copy(
                &mut fs::File::open(&target).map_err(|e| e.to_string())?,
                &mut backup,
            )
            .map_err(|e| e.to_string())?;
            backup
                .as_file()
                .set_permissions(metadata.permissions())
                .map_err(|e| e.to_string())?;
            backup.as_file().sync_all().map_err(|e| e.to_string())?;
            Some(backup)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let staged = if let Some(bytes) = bytes {
        let mut staged = NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        staged.write_all(bytes).map_err(|e| e.to_string())?;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(
                if relative == UNINSTALL || relative == "bin/signal-forge" {
                    0o755
                } else {
                    0o644
                },
            ))
            .map_err(|e| e.to_string())?;
        staged.as_file().sync_all().map_err(|e| e.to_string())?;
        Some(staged)
    } else {
        None
    };
    Ok(Change {
        target,
        staged,
        backup,
    })
}
pub struct PreparedAssets {
    prefix: PathBuf,
    changes: Vec<Change>,
}
pub struct InstalledAssets {
    prefix: PathBuf,
    changes: Vec<Change>,
}
impl PreparedAssets {
    pub fn install(self) -> Result<InstalledAssets> {
        let mut installed = InstalledAssets {
            prefix: self.prefix,
            changes: Vec::new(),
        };
        for mut change in self.changes {
            let result = match change.staged.take() {
                Some(file) => file
                    .persist(&change.target)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                None if change.target.exists() => {
                    fs::remove_file(&change.target).map_err(|e| e.to_string())
                }
                None => Ok(()),
            };
            if let Err(error) = result {
                installed.rollback()?;
                return Err(format!(
                    "Desktop installation failed and was rolled back: {error}"
                ));
            }
            installed.changes.push(change);
            if let Err(error) =
                fs::File::open(installed.changes.last().unwrap().target.parent().unwrap())
                    .and_then(|f| f.sync_all())
            {
                installed.rollback()?;
                return Err(format!(
                    "Desktop installation sync failed and was rolled back: {error}"
                ));
            }
        }
        refresh_caches(&installed.prefix);
        Ok(installed)
    }
}
impl InstalledAssets {
    pub fn rollback(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for change in self.changes.iter_mut().rev() {
            let result = if let Some(backup) = change.backup.take() {
                backup.persist(&change.target).map(|_| ()).map_err(|error| {
                    let message = error.error.to_string();
                    match error.file.keep() {
                        Ok((_, path)) => format!("{message}; recovery copy: {}", path.display()),
                        Err(error) => {
                            format!("{message}; recovery copy could not be preserved: {error}")
                        }
                    }
                })
            } else {
                match fs::remove_file(&change.target) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                    _ => Ok(()),
                }
            };
            if let Err(e) = result {
                errors.push(format!("{}: {e}", change.target.display()));
            }
        }
        refresh_caches(&self.prefix);
        if errors.is_empty() {
            self.changes.clear();
            Ok(())
        } else {
            Err(format!(
                "Desktop asset rollback failed: {}",
                errors.join("; ")
            ))
        }
    }
}
fn refresh_caches(prefix: &Path) {
    for (program, args) in [
        (
            "update-desktop-database",
            vec![prefix.join("share/applications").into_os_string()],
        ),
        (
            "gtk-update-icon-cache",
            vec![
                "-f".into(),
                "-t".into(),
                prefix.join("share/icons/hicolor").into_os_string(),
            ],
        ),
    ] {
        // These desktop utilities are optional; absence never prevents portable installs.
        let _ = Command::new(program)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
/// Detect installs made by the bundled installer, including pre-icon legacy installs.
pub fn installation_prefix(executable: &Path) -> Option<PathBuf> {
    let bin = executable.parent()?;
    let prefix = bin.parent()?;
    (executable.file_name()? == "signal-forge"
        && bin.file_name()? == "bin"
        && (prefix.join(LAUNCHER).is_file() || prefix.join(MANIFEST).is_file()))
    .then(|| prefix.to_path_buf())
}
pub fn uninstall(prefix: &Path) -> Result<()> {
    let prefix = fs::canonicalize(prefix).map_err(|e| e.to_string())?;
    let manifest = read_manifest(&checked_target(&prefix, MANIFEST)?)?
        .ok_or("No Signal Forge ownership manifest; refusing to guess installed files")?;
    let executable = checked_path(&prefix, "bin/signal-forge")?;
    let targets = manifest
        .files
        .iter()
        .map(|p| checked_target(&prefix, p))
        .collect::<Result<Vec<_>>>()?;
    for target in targets {
        match fs::remove_file(target) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
    }
    if fs::symlink_metadata(&executable).is_ok_and(|m| m.is_file()) {
        fs::remove_file(executable).map_err(|e| e.to_string())?;
    }
    refresh_caches(&prefix);
    Ok(())
}

/// Fresh/manual installation uses the same asset validation and rollback as self-update.
pub fn install_package(root: &Path, prefix: &Path) -> Result<()> {
    fs::create_dir_all(prefix).map_err(|e| e.to_string())?;
    let prefix = fs::canonicalize(prefix).map_err(|e| e.to_string())?;
    let mut assets = Assets::from_package(root)?.prepare(&prefix)?;
    let source = root.join("bin/signal-forge");
    let metadata = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 128 * 1024 * 1024 {
        return Err("Invalid packaged executable".into());
    }
    // The executable is the final transaction entry, including a backup on reinstall.
    assets.changes.push(stage_change(
        &prefix,
        "bin/signal-forge",
        Some(&fs::read(source).map_err(|e| e.to_string())?),
    )?);
    assets.install()?;
    Ok(())
}

fn bundled_assets() -> Assets {
    let mut assets = Assets::default();
    assets
        .add(
            LAUNCHER.into(),
            include_bytes!("../packaging/signal-forge.desktop").to_vec(),
        )
        .unwrap();
    assets
        .add(
            UNINSTALL.into(),
            include_bytes!("../packaging/uninstall.sh").to_vec(),
        )
        .unwrap();
    macro_rules! icon {
        ($size:literal) => {
            assets
                .add(
                    format!("share/icons/hicolor/{0}x{0}/apps/signal-forge.png", $size),
                    include_bytes!(concat!("../assets/icons/signal-forge-", $size, ".png"))
                        .to_vec(),
                )
                .unwrap();
        };
    }
    icon!(16);
    icon!(24);
    icon!(32);
    icon!(48);
    icon!(64);
    icon!(128);
    icon!(256);
    icon!(512);
    assets
}
/// The previous executable-only updater cannot install assets from its first
/// icon-bearing release. Repair only an identifiable installation on first launch.
pub fn repair_current_install() -> Result<()> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    repair_install(&executable)
}
pub fn repair_install(executable: &Path) -> Result<()> {
    let Some(prefix) = installation_prefix(executable) else {
        return Ok(());
    };
    let mut assets = bundled_assets();
    // Keep other package-owned files, particularly docs, during the legacy migration.
    if let Some(manifest) = read_manifest(&checked_target(&prefix, MANIFEST)?)? {
        for path in manifest.files {
            if path != MANIFEST && !assets.files.contains_key(&path) {
                let target = checked_target(&prefix, &path)?;
                if target.is_file() {
                    assets.add(path, fs::read(target).map_err(|e| e.to_string())?)?;
                }
            }
        }
    }
    // Documentation from the old shell installer is also application-owned.
    for relative in [
        "README.md",
        "linked-libraries.txt",
        "docs/capture-format.md",
        "docs/workbench.jpg",
        "docs/four-terminals.jpg",
        "docs/manual-smoke-test.md",
        "a_detailed_widescreen_dark_themed_desktop_applicat.png",
    ] {
        let path = format!("share/doc/signal-forge/{relative}");
        if !assets.files.contains_key(&path) {
            let target = checked_target(&prefix, &path)?;
            if target.is_file() {
                assets.add(path, fs::read(target).map_err(|e| e.to_string())?)?;
            }
        }
    }
    let launcher = render_launcher(
        std::str::from_utf8(include_bytes!("../packaging/signal-forge.desktop")).unwrap(),
        &executable,
    )?;
    let changed = assets.files.iter().any(|(path, data)| {
        let expected = if path == LAUNCHER {
            launcher.as_bytes()
        } else {
            data.as_slice()
        };
        checked_target(&prefix, path)
            .ok()
            .and_then(|target| fs::read(target).ok())
            .as_deref()
            != Some(expected)
    });
    if changed {
        assets.prepare(&prefix)?.install()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Assets {
        bundled_assets()
    }
    fn icon(prefix: &Path) -> PathBuf {
        prefix.join("share/icons/hicolor/256x256/apps/signal-forge.png")
    }
    #[test]
    fn installs_all_sizes_launcher_permissions_and_uninstalls_only_owned_files() {
        let directory = tempfile::tempdir().unwrap();
        let prefix = directory.path().join("custom prefix $quote\"% ");
        fs::create_dir_all(&prefix).unwrap();
        fixture().prepare(&prefix).unwrap().install().unwrap();
        for size in ICON_SIZES {
            let bytes = fs::read(prefix.join(format!(
                "share/icons/hicolor/{size}x{size}/apps/signal-forge.png"
            )))
            .unwrap();
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), size);
            assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), size);
        }
        let launcher = fs::read_to_string(prefix.join(LAUNCHER)).unwrap();
        assert!(launcher.contains("Name=Signal Forge\n"));
        assert!(launcher.contains("Icon=signal-forge\n"));
        assert!(launcher.contains("custom prefix \\\\$quote\\\\\"%% "));
        assert_eq!(
            fs::metadata(prefix.join(UNINSTALL))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            fs::metadata(icon(&prefix)).unwrap().permissions().mode() & 0o777,
            0o644
        );
        fs::write(prefix.join("share/applications/other.desktop"), "unrelated").unwrap();
        uninstall(&prefix).unwrap();
        assert!(!icon(&prefix).exists());
        assert!(!prefix.join(LAUNCHER).exists());
        assert!(!prefix.join(MANIFEST).exists());
        assert_eq!(
            fs::read_to_string(prefix.join("share/applications/other.desktop")).unwrap(),
            "unrelated"
        );
    }
    #[test]
    fn updating_replaces_icons_removes_stale_owned_docs_and_can_roll_back_exactly() {
        let prefix = tempfile::tempdir().unwrap();
        let mut old = fixture();
        old.add(
            "share/doc/signal-forge/obsolete.txt".into(),
            b"old doc".to_vec(),
        )
        .unwrap();
        old.prepare(prefix.path()).unwrap().install().unwrap();
        let before_icon = fs::read(icon(prefix.path())).unwrap();
        let before_manifest = fs::read(prefix.path().join(MANIFEST)).unwrap();
        let mut new = fixture();
        new.files.insert(
            "share/icons/hicolor/256x256/apps/signal-forge.png".into(),
            b"updated icon".to_vec(),
        );
        let mut installed = new.prepare(prefix.path()).unwrap().install().unwrap();
        assert_eq!(fs::read(icon(prefix.path())).unwrap(), b"updated icon");
        assert!(!prefix
            .path()
            .join("share/doc/signal-forge/obsolete.txt")
            .exists());
        installed.rollback().unwrap();
        assert_eq!(fs::read(icon(prefix.path())).unwrap(), before_icon);
        assert_eq!(
            fs::read(prefix.path().join(MANIFEST)).unwrap(),
            before_manifest
        );
        assert_eq!(
            fs::read(prefix.path().join("share/doc/signal-forge/obsolete.txt")).unwrap(),
            b"old doc"
        );
    }
    #[test]
    fn refuses_foreign_paths_symlinks_read_only_assets_and_incomplete_packages() {
        for path in [
            "/tmp/file",
            "share/doc/signal-forge/../../other",
            "share/doc/signal-forge/./a",
            "share/applications/other.desktop",
            "share/icons/hicolor/256x256/apps/other.png",
            "bin/signal-forge",
        ] {
            assert!(!owned_path(path), "{path}");
        }
        let prefix = tempfile::tempdir().unwrap();
        fixture().prepare(prefix.path()).unwrap().install().unwrap();
        fs::set_permissions(icon(prefix.path()), fs::Permissions::from_mode(0o444)).unwrap();
        assert!(fixture().prepare(prefix.path()).is_err());
        fs::set_permissions(icon(prefix.path()), fs::Permissions::from_mode(0o644)).unwrap();
        fs::remove_file(icon(prefix.path())).unwrap();
        std::os::unix::fs::symlink("/tmp/foreign-icon", icon(prefix.path())).unwrap();
        assert!(fixture().prepare(prefix.path()).is_err());
        assert!(Assets::default().validate().is_err());
        assert!(render_launcher("Exec=signal-forge", Path::new("/tmp/a\nb")).is_err());
        assert!(validate_manifest(
            br#"{"owner":"signal-forge","version":1,"files":["/tmp/other"]}"#,
            &fixture()
        )
        .is_err());
    }
    #[test]
    fn failed_mid_install_restores_previous_assets_and_does_not_publish_manifest() {
        let prefix = tempfile::tempdir().unwrap();
        fixture().prepare(prefix.path()).unwrap().install().unwrap();
        let previous = fs::read(icon(prefix.path())).unwrap();
        let manifest = fs::read(prefix.path().join(MANIFEST)).unwrap();
        let mut new = fixture();
        for bytes in new.files.values_mut() {
            bytes.extend_from_slice(b"updated");
        }
        // Keep valid launcher contents and force a later filesystem publication failure.
        new.files.insert(
            LAUNCHER.into(),
            include_bytes!("../packaging/signal-forge.desktop").to_vec(),
        );
        let mut staged = new.prepare(prefix.path()).unwrap();
        let index = staged.changes.len() - 1;
        let blocked = prefix.path().join("blocked");
        fs::create_dir(&blocked).unwrap();
        staged.changes[index].target = blocked;
        assert!(staged.install().is_err());
        assert_eq!(fs::read(icon(prefix.path())).unwrap(), previous);
        assert_eq!(fs::read(prefix.path().join(MANIFEST)).unwrap(), manifest);
    }
    #[test]
    fn legacy_executable_only_update_repairs_assets_once_and_leaves_portable_runs_alone() {
        let prefix = tempfile::tempdir().unwrap();
        fs::create_dir_all(prefix.path().join("bin")).unwrap();
        fs::create_dir_all(prefix.path().join("share/applications")).unwrap();
        fs::write(
            prefix.path().join(LAUNCHER),
            "[Desktop Entry]\nIcon=utilities-terminal\n",
        )
        .unwrap();
        let executable = prefix.path().join("bin/signal-forge");
        fs::write(&executable, "binary").unwrap();
        repair_install(&executable).unwrap();
        assert!(icon(prefix.path()).is_file());
        assert!(prefix.path().join(MANIFEST).is_file());
        let modified = fs::metadata(icon(prefix.path()))
            .unwrap()
            .modified()
            .unwrap();
        repair_install(&executable).unwrap();
        assert_eq!(
            fs::metadata(icon(prefix.path()))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        assert!(installation_prefix(&prefix.path().join("signal-forge")).is_none());
    }
}
