//! Self-update from the public GitHub Releases of the repository.
//!
//! No signing keys or update manifest: the latest release is read from the
//! GitHub API, the installer for this OS / CPU is picked from its assets by
//! file name, downloaded over HTTPS and run.
//!
//! - Windows: the NSIS `-setup.exe` (or the `.msi`) runs in passive mode and
//!   starts the app again when it is done.
//! - macOS: the `.dmg` is mounted and its `.app` replaces the running bundle.
//! - Linux: a running AppImage is replaced by the new one; a `.deb` / `.rpm`
//!   install is upgraded through the package manager (`pkexec` asks for the
//!   password). Then the new version starts.
//!
//! Only a release without any installer for this system falls back to
//! opening its page.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const REPO: &str = "Jyni0/Trajectory";

/// A release newer than the running build.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    /// The release on github.com, for installing by hand.
    pub page_url: String,
    /// Installer this app can apply itself; `None` = only the page helps.
    pub asset_name: Option<String>,
    #[serde(skip)]
    pub asset_url: Option<String>,
}

/// What `install` did.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallOutcome {
    /// The new version is in place / installing; the app exits now.
    Restarting,
    /// No installer for this setup: the release page was opened instead.
    OpenedPage,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// How this copy of the app is installed, which decides the asset to use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Windows,
    MacOs,
    /// Running from an AppImage (it can be swapped in place).
    LinuxAppImage,
    /// Installed from a `.deb` (dpkg-based distro).
    LinuxDeb,
    /// Installed from an `.rpm`.
    LinuxRpm,
}

impl Target {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Target::Windows
        } else if cfg!(target_os = "macos") {
            Target::MacOs
        } else if std::env::var_os("APPIMAGE").is_some() {
            Target::LinuxAppImage
        } else if Path::new("/usr/bin/dpkg").exists() {
            Target::LinuxDeb
        } else {
            Target::LinuxRpm
        }
    }

    /// File-name endings of the installers it can apply, preferred first.
    fn extensions(self) -> &'static [&'static str] {
        match self {
            Target::Windows => &["-setup.exe", ".msi"],
            Target::MacOs => &[".dmg"],
            Target::LinuxAppImage => &[".appimage"],
            Target::LinuxDeb => &[".deb"],
            Target::LinuxRpm => &[".rpm"],
        }
    }
}

const ARCH_X64: &[&str] = &["x64", "amd64", "x86_64"];
const ARCH_ARM64: &[&str] = &["aarch64", "arm64"];

/// The version in a release tag: `v2.8.0`, `Release-2.8.0` → `2.8.0`.
pub fn tag_version(tag: &str) -> &str {
    tag.find(|c: char| c.is_ascii_digit()).map_or("", |i| &tag[i..])
}

/// `1.10.0` > `1.9.2`; any prefix before the number and any `-pre` /
/// `+build` part are ignored.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        let core = tag_version(v).split(['-', '+']).next().unwrap_or("");
        core.split('.').map(|p| p.parse().unwrap_or(0)).collect()
    }
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The asset for `target` on CPU `arch` (`std::env::consts::ARCH` values).
pub fn pick_asset<'a>(names: &[&'a str], target: Target, arch: &str) -> Option<&'a str> {
    let (mine, other) = match arch {
        "aarch64" => (ARCH_ARM64, ARCH_X64),
        _ => (ARCH_X64, ARCH_ARM64),
    };
    // Whole words only, so "x64" does not match inside other words.
    let has = |name: &str, tokens: &[&str]| {
        let lower = name.to_ascii_lowercase();
        let words: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).collect();
        tokens.iter().any(|t| if t.contains('_') { lower.contains(t) } else { words.contains(t) })
    };
    for ext in target.extensions() {
        let candidates: Vec<&str> =
            names.iter().copied().filter(|n| n.to_ascii_lowercase().ends_with(ext)).filter(|n| !has(n, other)).collect();
        // This CPU first, then universal / unlabelled builds.
        if let Some(n) = candidates.iter().find(|n| has(n, mine)).or_else(|| candidates.first()) {
            return Some(n);
        }
    }
    None
}

/// Reads a GitHub "latest release" answer; `None` when it is not newer.
pub fn parse_release(json: &str, current: &str, target: Target, arch: &str) -> AppResult<Option<UpdateInfo>> {
    let release: Release = serde_json::from_str(json)?;
    let version = tag_version(&release.tag_name).to_string();
    if version.is_empty() || release.draft || release.prerelease || !is_newer(&version, current) {
        return Ok(None);
    }
    let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
    let asset = pick_asset(&names, target, arch).and_then(|n| release.assets.iter().find(|a| a.name == n));
    Ok(Some(UpdateInfo {
        version,
        notes: release.body.filter(|b| !b.trim().is_empty()),
        page_url: release.html_url,
        asset_name: asset.map(|a| a.name.clone()),
        asset_url: asset.map(|a| a.browser_download_url.clone()),
    }))
}

fn client(timeout: Option<Duration>) -> AppResult<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        // GitHub's API rejects requests without a User-Agent.
        .user_agent(concat!("Trajectory/", env!("CARGO_PKG_VERSION")));
    if let Some(t) = timeout {
        b = b.timeout(t);
    }
    b.build().map_err(|e| AppError::Network(e.to_string()))
}

/// Asks GitHub for the latest release of `REPO`.
pub async fn check(api_base: &str) -> AppResult<Option<UpdateInfo>> {
    let url = format!("{api_base}/repos/{REPO}/releases/latest");
    let res = client(Some(Duration::from_secs(20)))?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    // No release published yet.
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let json = res.error_for_status()?.text().await?;
    parse_release(&json, env!("CARGO_PKG_VERSION"), Target::current(), std::env::consts::ARCH)
}

/// Downloads `url` into `dest`, reporting `(downloaded, total)` bytes.
pub async fn download(url: &str, dest: &Path, mut progress: impl FnMut(u64, Option<u64>)) -> AppResult<()> {
    use tokio::io::AsyncWriteExt;
    let mut res = client(None)?.get(url).send().await?.error_for_status()?;
    let total = res.content_length();
    let mut file = tokio::fs::File::create(dest).await?;
    let mut done = 0u64;
    while let Some(chunk) = res.chunk().await? {
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush().await?;
    Ok(())
}

/// A fresh folder in the temp dir for this download.
pub fn work_dir(version: &str) -> AppResult<PathBuf> {
    let dir = std::env::temp_dir().join(format!("trajectory-update-{version}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Starts the downloaded installer / swaps the app in place. On success the
/// caller exits the app (Windows installers start it again themselves).
pub fn apply(installer: &Path) -> AppResult<()> {
    let name = installer.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
    if name.ends_with(".exe") {
        // Passive NSIS install of a newer version; `/R` relaunches the app.
        std::process::Command::new(installer).args(["/P", "/UPDATE", "/R"]).spawn()?;
        Ok(())
    } else if name.ends_with(".msi") {
        std::process::Command::new("msiexec.exe")
            .arg("/i")
            .arg(installer)
            .args(["/passive", "/promptrestart", "AUTOLAUNCHAPP=True"])
            .spawn()?;
        Ok(())
    } else if name.ends_with(".dmg") {
        apply_dmg(installer)
    } else if name.ends_with(".appimage") {
        apply_appimage(installer)
    } else if name.ends_with(".deb") {
        apply_package(&["dpkg", "-i"], installer)
    } else if name.ends_with(".rpm") {
        apply_package(&["rpm", "-U", "--replacepkgs"], installer)
    } else {
        Err(AppError::Invalid(format!("don't know how to install {name}")))
    }
}

fn run(cmd: &mut std::process::Command) -> AppResult<()> {
    let out = cmd.output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(AppError::Io(std::io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_string())))
    }
}

/// The `.app` bundle the running executable lives in.
fn current_bundle() -> AppResult<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
        .ok_or_else(|| AppError::Invalid("the app is not running from an .app bundle".into()))
}

/// Mounts the image, copies its `.app` next to the running one and swaps them.
fn apply_dmg(dmg: &Path) -> AppResult<()> {
    use std::process::Command;
    let bundle = current_bundle()?;
    let mount = dmg.with_extension("mount");
    std::fs::create_dir_all(&mount)?;
    run(Command::new("hdiutil").arg("attach").arg(dmg).args(["-nobrowse", "-readonly", "-mountpoint"]).arg(&mount))?;
    let result = (|| -> AppResult<()> {
        let app = std::fs::read_dir(&mount)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or_else(|| AppError::NotFound("no .app in the disk image".into()))?;
        let staged = bundle.with_extension("app.new");
        let old = bundle.with_extension("app.old");
        let _ = std::fs::remove_dir_all(&staged);
        let _ = std::fs::remove_dir_all(&old);
        run(Command::new("ditto").arg(&app).arg(&staged))?;
        std::fs::rename(&bundle, &old)?;
        if let Err(e) = std::fs::rename(&staged, &bundle) {
            let _ = std::fs::rename(&old, &bundle);
            return Err(e.into());
        }
        let _ = std::fs::remove_dir_all(&old);
        Ok(())
    })();
    let _ = run(Command::new("hdiutil").arg("detach").arg(&mount).arg("-quiet"));
    result?;
    // Start the new copy; the running one exits right after.
    run(Command::new("open").arg("-n").arg(&bundle))
}

/// Replaces the running AppImage file and starts the new one.
fn apply_appimage(new: &Path) -> AppResult<()> {
    let current = PathBuf::from(std::env::var_os("APPIMAGE").ok_or_else(|| AppError::Invalid("not running as an AppImage".into()))?);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))?;
    }
    // Copy next to the target first so the final rename stays on one filesystem.
    let staged = current.with_extension("new");
    std::fs::copy(new, &staged)?;
    std::fs::rename(&staged, &current)?;
    std::process::Command::new(&current).spawn()?;
    Ok(())
}

/// Upgrades the installed package as root (`pkexec` shows the system's
/// password prompt), then starts the upgraded binary.
fn apply_package(cmd: &[&str], package: &Path) -> AppResult<()> {
    let exe = std::env::current_exe()?;
    run(std::process::Command::new("pkexec").args(cmd).arg(package)).map_err(|e| match e {
        AppError::Io(io) => AppError::Invalid(format!("the package could not be installed: {io}")),
        e => e,
    })?;
    // The package replaced the file at the same path.
    let exe = PathBuf::from(exe.to_string_lossy().trim_end_matches(" (deleted)"));
    std::process::Command::new(exe).spawn()?;
    Ok(())
}
