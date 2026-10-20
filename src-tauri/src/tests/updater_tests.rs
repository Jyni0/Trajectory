//! Picking the newer release and the right installer from GitHub assets.

use crate::updater::{is_newer, parse_release, pick_asset, tag_version, Target};

const ASSETS: &[&str] = &[
    "Trajectory_2.8.0_x64-setup.exe",
    "Trajectory_2.8.0_x64_en-US.msi",
    "Trajectory_2.8.0_aarch64.dmg",
    "Trajectory_2.8.0_x64.dmg",
    "Trajectory_2.8.0_amd64.AppImage",
    "Trajectory_2.8.0_amd64.deb",
    "Trajectory-2.8.0-1.x86_64.rpm",
];

#[test]
fn compares_versions_numerically() {
    assert!(is_newer("2.10.0", "2.9.9"));
    assert!(is_newer("v2.8.0", "2.7.3"));
    assert!(is_newer("Release-2.8.0", "2.7.3"));
    assert_eq!(tag_version("Release-2.5.0"), "2.5.0");
    assert_eq!(tag_version("v2.5.0"), "2.5.0");
    assert!(is_newer("3", "2.99.0"));
    assert!(!is_newer("2.7.3", "2.7.3"));
    assert!(!is_newer("2.7.2", "2.7.3"));
    assert!(!is_newer("2.7.3-beta.1", "2.7.3"));
}

#[test]
fn picks_the_installer_for_this_platform_and_cpu() {
    assert_eq!(pick_asset(ASSETS, Target::Windows, "x86_64"), Some("Trajectory_2.8.0_x64-setup.exe"));
    assert_eq!(pick_asset(&["Trajectory_2.8.0_x64_en-US.msi"], Target::Windows, "x86_64"), Some("Trajectory_2.8.0_x64_en-US.msi"));
    assert_eq!(pick_asset(ASSETS, Target::MacOs, "aarch64"), Some("Trajectory_2.8.0_aarch64.dmg"));
    assert_eq!(pick_asset(ASSETS, Target::MacOs, "x86_64"), Some("Trajectory_2.8.0_x64.dmg"));
    assert_eq!(pick_asset(ASSETS, Target::LinuxAppImage, "x86_64"), Some("Trajectory_2.8.0_amd64.AppImage"));
    // No build for this CPU: nothing, rather than one that cannot run.
    assert_eq!(pick_asset(ASSETS, Target::LinuxAppImage, "aarch64"), None);
    assert_eq!(pick_asset(ASSETS, Target::LinuxDeb, "x86_64"), Some("Trajectory_2.8.0_amd64.deb"));
    assert_eq!(pick_asset(ASSETS, Target::LinuxRpm, "x86_64"), Some("Trajectory-2.8.0-1.x86_64.rpm"));
    assert_eq!(pick_asset(ASSETS, Target::LinuxRpm, "aarch64"), None);
    assert_eq!(pick_asset(&["Trajectory_2.8.0_universal.dmg"], Target::MacOs, "aarch64"), Some("Trajectory_2.8.0_universal.dmg"));
}

fn release(tag: &str, prerelease: bool) -> String {
    let assets: Vec<_> = ASSETS
        .iter()
        .map(|n| serde_json::json!({ "name": n, "browser_download_url": format!("https://github.com/Jyni0/Trajectory/releases/download/{tag}/{n}") }))
        .collect();
    serde_json::json!({
        "tag_name": tag,
        "html_url": format!("https://github.com/Jyni0/Trajectory/releases/tag/{tag}"),
        "body": "Fixes",
        "draft": false,
        "prerelease": prerelease,
        "assets": assets,
    })
    .to_string()
}

#[test]
fn reads_the_latest_release() {
    let info = parse_release(&release("v2.8.0", false), "2.7.3", Target::Windows, "x86_64").unwrap().unwrap();
    assert_eq!(info.version, "2.8.0");
    assert_eq!(info.notes.as_deref(), Some("Fixes"));
    assert_eq!(info.asset_name.as_deref(), Some("Trajectory_2.8.0_x64-setup.exe"));
    assert!(info.asset_url.unwrap().ends_with("/v2.8.0/Trajectory_2.8.0_x64-setup.exe"));

    let linux = parse_release(&release("v2.8.0", false), "2.7.3", Target::LinuxDeb, "x86_64").unwrap().unwrap();
    assert_eq!(linux.asset_name.as_deref(), Some("Trajectory_2.8.0_amd64.deb"));

    // No build for this system in the release: only its page is left.
    let mac = parse_release(&release("v2.8.0", false), "2.7.3", Target::MacOs, "aarch64").unwrap().unwrap();
    assert!(mac.asset_name.is_some());
    let arm_linux = parse_release(&release("v2.8.0", false), "2.7.3", Target::LinuxAppImage, "aarch64").unwrap().unwrap();
    assert_eq!(arm_linux.asset_url, None);
    assert_eq!(arm_linux.page_url, "https://github.com/Jyni0/Trajectory/releases/tag/v2.8.0");

    let named = parse_release(&release("Release-2.8.0", false), "2.7.3", Target::Windows, "x86_64").unwrap().unwrap();
    assert_eq!(named.version, "2.8.0");

    assert_eq!(parse_release(&release("v2.7.3", false), "2.7.3", Target::Windows, "x86_64").unwrap(), None);
    assert_eq!(parse_release(&release("v2.9.0", true), "2.7.3", Target::Windows, "x86_64").unwrap(), None);
}
