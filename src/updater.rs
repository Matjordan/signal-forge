//! Official stable-release discovery and verified, atomic Linux executable updates.
//! Network and archive work run outside the UI. No downloaded installer is executed.
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

pub const RELEASES_URL: &str =
    "https://api.github.com/repos/Matjordan/signal-forge/releases?per_page=30";
const RELEASE_ROOT: &str = "https://github.com/Matjordan/signal-forge/releases/";
const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;
const MAX_BINARY: u64 = 128 * 1024 * 1024;
pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Deserialize, Debug)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}
#[derive(Deserialize, Debug)]
pub struct Release {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub html_url: String,
    pub assets: Vec<Asset>,
}
#[derive(Clone, Debug)]
pub struct Offer {
    pub version: Version,
    pub notes_url: String,
    pub archive: Asset,
    pub checksum: Asset,
}
impl Offer {
    fn validate(&self) -> Result<()> {
        let name = format!("signal-forge-{}-linux-x86_64.tar.gz", self.version);
        let root = format!("{RELEASE_ROOT}download/v{}/", self.version);
        if self.archive.name != name
            || self.checksum.name != format!("{name}.sha256")
            || self.archive.browser_download_url != format!("{root}{name}")
            || self.checksum.browser_download_url != format!("{root}{name}.sha256")
            || self.notes_url != format!("{RELEASE_ROOT}tag/v{}", self.version)
            || !self.version.pre.is_empty()
            || !self.version.build.is_empty()
        {
            return Err(
                "Release metadata does not match the official stable Linux x86_64 package.".into(),
            );
        }
        Ok(())
    }
}
/// Ignore drafts, prereleases, malformed tags and releases without both platform assets.
pub fn select_release(
    releases: Vec<Release>,
    current: &Version,
    os: &str,
    arch: &str,
) -> Option<Offer> {
    if os != "linux" || arch != "x86_64" {
        return None;
    }
    releases
        .into_iter()
        .filter_map(|r| {
            if r.draft || r.prerelease {
                return None;
            }
            let version = Version::parse(r.tag_name.strip_prefix('v')?).ok()?;
            if version <= *current {
                return None;
            }
            let name = format!("signal-forge-{version}-linux-x86_64.tar.gz");
            let offer = Offer {
                version,
                notes_url: r.html_url,
                archive: r.assets.iter().find(|a| a.name == name)?.clone(),
                checksum: r
                    .assets
                    .iter()
                    .find(|a| a.name == format!("{name}.sha256"))?
                    .clone(),
            };
            offer.validate().ok()?;
            Some(offer)
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}
fn client(timeout: Duration) -> Result<Client> {
    Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(timeout)
        .user_agent(concat!("signal-forge/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("Too many release redirects")
            } else if attempt.url().scheme() != "https"
                || !matches!(
                    attempt.url().host_str(),
                    Some(
                        "github.com"
                            | "api.github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.error("Release redirected outside GitHub HTTPS hosts")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}
pub fn check() -> Result<Option<Offer>> {
    if std::env::consts::OS != "linux" || std::env::consts::ARCH != "x86_64" {
        return Ok(None);
    }
    let response = client(Duration::from_secs(10))?
        .get(RELEASES_URL)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Release metadata is too large.".into());
    }
    let releases = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(select_release(
        releases,
        &Version::parse(env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())?,
        std::env::consts::OS,
        std::env::consts::ARCH,
    ))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    Downloading { received: u64, total: Option<u64> },
    Verifying,
    Installing,
    Restarting,
}
pub struct PreparedUpdate {
    binary: tempfile::TempPath,
    target: PathBuf,
}
pub struct InstalledUpdate {
    pub target: PathBuf,
    pub backup: PathBuf,
}
/// Staging in the executable's directory proves directory write access before downloading.
pub fn prepare(
    offer: &Offer,
    target: &Path,
    mut progress: impl FnMut(Progress),
) -> Result<PreparedUpdate> {
    offer.validate()?;
    let target =
        fs::canonicalize(target).map_err(|e| format!("Locate installed executable: {e}"))?;
    let metadata = fs::metadata(&target).map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.permissions().mode() & 0o222 == 0
        || metadata.permissions().mode() & 0o6000 != 0
    {
        return Err("This executable cannot safely self-update. Install the release into a user-writable location.".into());
    }
    let parent = target
        .parent()
        .ok_or("Executable has no parent directory")?;
    let mut archive = NamedTempFile::new_in(parent).map_err(|e| format!("Installation directory is not writable: {e}. Download and install the release manually."))?;
    let http = client(Duration::from_secs(120))?;
    let mut checksum = String::new();
    http.get(&offer.checksum.browser_download_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .take(4097)
        .read_to_string(&mut checksum)
        .map_err(|e| e.to_string())?;
    if checksum.len() > 4096 {
        return Err("Checksum file is too large.".into());
    }
    let expected = parse_checksum(&checksum, &offer.archive.name)?;
    let mut response = http
        .get(&offer.archive.browser_download_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let total = response.content_length();
    if total.is_some_and(|n| n > MAX_ARCHIVE) {
        return Err("Release archive exceeds the size limit.".into());
    }
    let mut digest = Sha256::new();
    let mut received = 0;
    let mut buffer = [0u8; 64 * 1024];
    progress(Progress::Downloading { received, total });
    loop {
        let n = response.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > MAX_ARCHIVE {
            return Err("Release archive exceeds the size limit.".into());
        }
        digest.update(&buffer[..n]);
        archive.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        progress(Progress::Downloading { received, total });
    }
    progress(Progress::Verifying);
    verify_digest(&expected, &format!("{:x}", digest.finalize()))?;
    let archive_file = archive.reopen().map_err(|e| e.to_string())?;
    let binary = extract_binary(archive_file, parent, &offer.version)?;
    binary
        .as_file()
        .set_permissions(metadata.permissions())
        .map_err(|e| e.to_string())?;
    let binary = binary.into_temp_path();
    validate_binary(&binary, &offer.version)?;
    Ok(PreparedUpdate { binary, target })
}
pub fn parse_checksum(text: &str, name: &str) -> Result<String> {
    let fields: Vec<_> = text.split_whitespace().collect();
    if fields.len() != 2
        || fields[1].trim_start_matches('*') != name
        || fields[0].len() != 64
        || !fields[0].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Published checksum has an unexpected format or filename.".into());
    }
    Ok(fields[0].to_ascii_lowercase())
}
pub fn verify_digest(expected: &str, actual: &str) -> Result<()> {
    if expected != actual {
        return Err(
            "SHA-256 verification failed; the existing application was not changed.".into(),
        );
    }
    Ok(())
}
/// Never unpack archive paths or execute its install.sh. Copy only the exact regular binary.
pub fn extract_binary(
    reader: impl Read,
    directory: &Path,
    version: &Version,
) -> Result<NamedTempFile> {
    let expected = format!("signal-forge-{version}-linux-x86_64/bin/signal-forge");
    let mut archive = tar::Archive::new(GzDecoder::new(reader).take(512 * 1024 * 1024));
    let mut binary = None;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if entry.path().map_err(|e| e.to_string())?.as_ref() != Path::new(&expected) {
            continue;
        }
        if binary.is_some() || !entry.header().entry_type().is_file() || entry.size() > MAX_BINARY {
            return Err("Archive contains an invalid or duplicate executable.".into());
        }
        let mut file = NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        binary = Some(file);
    }
    binary.ok_or_else(|| "Release archive does not contain the expected executable.".into())
}
fn validate_binary(path: &Path, version: &Version) -> Result<()> {
    let mut header = [0u8; 20];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|e| e.to_string())?;
    if &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 || header[18..20] != [62, 0] {
        return Err("Replacement is not a Linux x86_64 ELF executable.".into());
    }
    let mut child = Command::new(path)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Replacement cannot run on this system: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let mut output = String::new();
            child
                .stdout
                .take()
                .ok_or("Missing version output")?
                .take(1024)
                .read_to_string(&mut output)
                .map_err(|e| e.to_string())?;
            if !status.success() || output.trim() != format!("Signal Forge {version}") {
                return Err("Replacement did not report the expected release version.".into());
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Replacement version check timed out.".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
impl PreparedUpdate {
    /// Linux rename replaces the directory entry while the running inode stays usable.
    pub fn install(self) -> Result<InstalledUpdate> {
        let parent = self
            .target
            .parent()
            .ok_or("Missing installation directory")?
            .to_path_buf();
        let mut backup = tempfile::Builder::new()
            .prefix(".signal-forge-previous-")
            .tempfile_in(&parent)
            .map_err(|e| e.to_string())?;
        std::io::copy(
            &mut File::open(&self.target).map_err(|e| e.to_string())?,
            &mut backup,
        )
        .map_err(|e| e.to_string())?;
        backup
            .as_file()
            .set_permissions(
                fs::metadata(&self.target)
                    .map_err(|e| e.to_string())?
                    .permissions(),
            )
            .map_err(|e| e.to_string())?;
        backup.as_file().sync_all().map_err(|e| e.to_string())?;
        let (_, backup_path) = backup.keep().map_err(|e| e.to_string())?;
        if let Err(error) = self.binary.persist(&self.target) {
            let _ = fs::remove_file(&backup_path);
            return Err(format!("Install failed; existing executable kept: {error}"));
        }
        let installed = InstalledUpdate {
            target: self.target,
            backup: backup_path,
        };
        if let Err(error) = File::open(&parent).and_then(|f| f.sync_all()) {
            installed.rollback()?;
            return Err(format!(
                "Installation could not be synced and was rolled back: {error}"
            ));
        }
        Ok(installed)
    }
}
impl InstalledUpdate {
    pub fn rollback(&self) -> Result<()> {
        fs::rename(&self.backup, &self.target).map_err(|e| {
            format!(
                "Restore {} from {}: {e}",
                self.target.display(),
                self.backup.display()
            )
        })
    }
    /// Called only after the old workbench is dropped, saving its workspace and releasing I/O.
    pub fn restart(self) -> Result<()> {
        match Command::new(&self.target).spawn() {
            Ok(mut child) => {
                std::thread::sleep(Duration::from_millis(500));
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    self.rollback()?;
                    Command::new(&self.target)
                        .spawn()
                        .map_err(|e| e.to_string())?;
                    return Err("New process exited during startup; restored and restarted the previous version.".into());
                }
                log::info!(
                    "Updated executable restarted; recovery copy: {}",
                    self.backup.display()
                );
                Ok(())
            }
            Err(error) => {
                self.rollback()?;
                Command::new(&self.target)
                    .spawn()
                    .map_err(|e| e.to_string())?;
                Err(format!(
                    "Restart failed ({error}); restored and restarted the previous version."
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(version: &str) -> Release {
        let name = format!("signal-forge-{version}-linux-x86_64.tar.gz");
        Release {
            tag_name: format!("v{version}"),
            draft: false,
            prerelease: false,
            html_url: format!("{RELEASE_ROOT}tag/v{version}"),
            assets: vec![
                Asset {
                    name: name.clone(),
                    browser_download_url: format!("{RELEASE_ROOT}download/v{version}/{name}"),
                },
                Asset {
                    name: format!("{name}.sha256"),
                    browser_download_url: format!(
                        "{RELEASE_ROOT}download/v{version}/{name}.sha256"
                    ),
                },
            ],
        }
    }
    #[test]
    fn versions_are_semantic_and_select_newest_compatible_stable() {
        let current = Version::parse("0.1.9").unwrap();
        let mut beta = release("1.0.0-beta.1");
        beta.prerelease = true;
        let mut draft = release("2.0.0");
        draft.draft = true;
        let mut wrong = release("3.0.0");
        wrong.assets.clear();
        let offer = select_release(
            vec![release("0.1.8"), release("0.1.10"), beta, draft, wrong],
            &current,
            "linux",
            "x86_64",
        )
        .unwrap();
        assert_eq!(offer.version, Version::parse("0.1.10").unwrap());
        assert!(select_release(vec![release("0.1.9")], &current, "linux", "x86_64").is_none());
        assert!(select_release(vec![release("1.0.0")], &current, "linux", "aarch64").is_none());
        assert!(select_release(vec![release("1.0.0")], &current, "windows", "x86_64").is_none());
    }
    #[test]
    fn unofficial_urls_and_mismatched_assets_are_rejected() {
        let mut r = release("1.0.0");
        r.assets[0].browser_download_url = "https://example.com/binary".into();
        assert!(select_release(vec![r], &Version::new(0, 1, 0), "linux", "x86_64").is_none());
        let mut r = release("1.0.0");
        r.assets[1].browser_download_url =
            r.assets[1].browser_download_url.replace("https:", "http:");
        assert!(select_release(vec![r], &Version::new(0, 1, 0), "linux", "x86_64").is_none());
    }
    #[test]
    fn read_only_executable_is_rejected_before_any_download() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("signal-forge");
        fs::write(&target, b"existing executable").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o555)).unwrap();
        let offer = select_release(
            vec![release("1.0.0")],
            &Version::new(0, 1, 0),
            "linux",
            "x86_64",
        )
        .unwrap();
        let error = prepare(&offer, &target, |_| panic!("Download must not start"))
            .err()
            .unwrap();
        assert!(error.contains("cannot safely self-update"));
        assert_eq!(fs::read(&target).unwrap(), b"existing executable");
    }
    #[test]
    fn checksums_require_exact_filename_and_matching_bytes() {
        let digest = format!("{:x}", Sha256::digest(b"archive"));
        assert_eq!(
            parse_checksum(&format!("{digest}  release.tar.gz\n"), "release.tar.gz").unwrap(),
            digest
        );
        assert!(parse_checksum(&format!("{digest} wrong.tar.gz"), "release.tar.gz").is_err());
        assert!(parse_checksum("abcd release.tar.gz", "release.tar.gz").is_err());
        assert!(verify_digest(&digest, &format!("{:x}", Sha256::digest(b"corrupt"))).is_err());
    }
    fn archive(kind: tar::EntryType, duplicate: bool) -> Vec<u8> {
        let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut tar = tar::Builder::new(gzip);
        for _ in 0..if duplicate { 2 } else { 1 } {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(kind);
            header.set_mode(0o755);
            let bytes: &[u8] = if kind.is_file() { b"binary" } else { b"" };
            header.set_size(bytes.len() as u64);
            if kind.is_symlink() {
                header.set_link_name("/tmp/arbitrary").unwrap();
            }
            header.set_cksum();
            tar.append_data(
                &mut header,
                "signal-forge-1.0.0-linux-x86_64/bin/signal-forge",
                bytes,
            )
            .unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }
    #[test]
    fn extracts_only_regular_expected_binary_and_rejects_duplicates_links_missing() {
        let dir = tempfile::tempdir().unwrap();
        let version = Version::new(1, 0, 0);
        let binary = extract_binary(
            &archive(tar::EntryType::Regular, false)[..],
            dir.path(),
            &version,
        )
        .unwrap();
        assert_eq!(fs::read(binary.path()).unwrap(), b"binary");
        assert!(extract_binary(
            &archive(tar::EntryType::Symlink, false)[..],
            dir.path(),
            &version
        )
        .is_err());
        assert!(extract_binary(
            &archive(tar::EntryType::Regular, true)[..],
            dir.path(),
            &version
        )
        .is_err());
        assert!(extract_binary(
            &archive(tar::EntryType::Regular, false)[..],
            dir.path(),
            &Version::new(2, 0, 0)
        )
        .is_err());
        assert!(extract_binary(&b"not gzip"[..], dir.path(), &version).is_err());
    }
    fn fixture(directory: &Path, name: &str, version: &str) -> PathBuf {
        let path = directory.join(name);
        let source = directory.join(format!("{name}.rs"));
        let marker = directory.join(format!("{name}.started"));
        fs::write(&source, format!(r#"fn main() {{ if std::env::args().any(|a| a == "--version") {{ println!("Signal Forge {version}"); }} else {{ std::fs::write({marker:?}, "{version}").unwrap(); std::thread::sleep(std::time::Duration::from_secs(2)); }} }}"#, marker=marker.to_str().unwrap())).unwrap();
        assert!(Command::new("rustc")
            .arg("--crate-name")
            .arg("fixture")
            .arg(&source)
            .arg("-o")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        path
    }
    fn prepared(source: &Path, target: &Path) -> PreparedUpdate {
        let mut binary = NamedTempFile::new_in(target.parent().unwrap()).unwrap();
        std::io::copy(&mut File::open(source).unwrap(), &mut binary).unwrap();
        binary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))
            .unwrap();
        PreparedUpdate {
            binary: binary.into_temp_path(),
            target: target.to_path_buf(),
        }
    }
    #[test]
    fn validates_elf_and_exact_reported_version_before_installing() {
        let dir = tempfile::tempdir().unwrap();
        let binary = fixture(dir.path(), "new", "1.0.0");
        validate_binary(&binary, &Version::new(1, 0, 0)).unwrap();
        assert!(validate_binary(&binary, &Version::new(2, 0, 0)).is_err());
        fs::write(dir.path().join("invalid"), b"not an executable").unwrap();
        assert!(validate_binary(&dir.path().join("invalid"), &Version::new(1, 0, 0)).is_err());
    }
    #[test]
    fn atomic_install_keeps_backup_and_restart_runs_new_version_without_touching_data() {
        let dir = tempfile::tempdir().unwrap();
        let old = fixture(dir.path(), "old", "0.1.0");
        let new = fixture(dir.path(), "new", "1.0.0");
        let target = dir.path().join("signal-forge");
        fs::copy(&old, &target).unwrap();
        let config = dir.path().join("workspace.json");
        fs::write(&config, b"user workspace").unwrap();
        let installed = prepared(&new, &target).install().unwrap();
        assert_eq!(
            fs::read(&installed.backup).unwrap(),
            fs::read(&old).unwrap()
        );
        assert_eq!(fs::read(&target).unwrap(), fs::read(&new).unwrap());
        installed.restart().unwrap();
        assert_eq!(fs::read(dir.path().join("new.started")).unwrap(), b"1.0.0");
        assert!(!dir.path().join("old.started").exists());
        assert_eq!(fs::read(&config).unwrap(), b"user workspace");
    }
    #[test]
    fn failed_replacement_keeps_original_and_does_not_restart() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("signal-forge");
        fs::write(&target, b"old application").unwrap();
        let staged = prepared(&target, &target);
        fs::remove_file(&staged.binary).unwrap();
        assert!(staged.install().is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old application");
    }
    #[test]
    fn restart_failure_restores_and_relaunches_previous_executable() {
        let dir = tempfile::tempdir().unwrap();
        let old = fixture(dir.path(), "old", "0.1.0");
        let new = fixture(dir.path(), "new", "1.0.0");
        let target = dir.path().join("signal-forge");
        fs::copy(&old, &target).unwrap();
        let installed = prepared(&new, &target).install().unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(installed.restart().is_err());
        assert_eq!(fs::read(&target).unwrap(), fs::read(&old).unwrap());
        let deadline = Instant::now() + Duration::from_secs(3);
        while !dir.path().join("old.started").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(fs::read(dir.path().join("old.started")).unwrap(), b"0.1.0");
    }
}
