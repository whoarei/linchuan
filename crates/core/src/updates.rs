use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const RELEASES: &str = "https://github.com/whoarei/linchuan/releases";
#[derive(Clone, Copy)]
pub enum Package {
    Windows,
    AppImage,
    Deb,
    Android,
    Manual,
}
#[derive(Serialize)]
pub struct ReleaseInfo {
    pub current_version: String,
    pub version: String,
    pub available: bool,
    pub notes: String,
    pub release_url: String,
    pub download_url: String,
    pub in_app: bool,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    body: Option<String>,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub async fn check(current: &str, package: Package) -> Result<ReleaseInfo, String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("Linchuan/{current}"))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get("https://api.github.com/repos/whoarei/linchuan/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| "无法连接更新服务，请检查网络后重试".to_string())?;
    if matches!(response.status().as_u16(), 403 | 429) {
        return Err("更新服务请求过于频繁，请稍后重试".into());
    }
    let release = response
        .error_for_status()
        .map_err(|_| "更新服务暂不可用，请稍后重试".to_string())?
        .json::<Release>()
        .await
        .map_err(|_| "更新服务返回的版本信息无效".to_string())?;
    describe(current, package, release).map_err(|e| e.to_string())
}

fn describe(current: &str, package: Package, release: Release) -> anyhow::Result<ReleaseInfo> {
    let installed = semver::Version::parse(current)?;
    let latest = semver::Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )?;
    anyhow::ensure!(
        !release.draft && !release.prerelease && latest.pre.is_empty(),
        "更新服务未返回正式版本"
    );
    let version = latest.to_string();
    let release_url = format!("{RELEASES}/tag/{}", release.tag_name);
    let suffix = match package {
        Package::Windows => "x64-setup.exe",
        Package::AppImage => "amd64.AppImage",
        Package::Deb => "amd64.deb",
        Package::Android => "android_arm64.apk",
        Package::Manual => "",
    };
    let asset_name = format!("Linchuan_{version}_{suffix}");
    let expected_url = |name: &str| format!("{RELEASES}/download/{}/{name}", release.tag_name);
    let installer = release.assets.iter().find(|asset| {
        asset.name == asset_name && asset.browser_download_url == expected_url(&asset.name)
    });
    let has_manifest = release.assets.iter().any(|asset| {
        asset.name == "latest.json" && asset.browser_download_url == expected_url("latest.json")
    });
    Ok(ReleaseInfo {
        current_version: current.into(),
        available: latest > installed,
        version,
        notes: release
            .body
            .unwrap_or_default()
            .chars()
            .take(16000)
            .collect(),
        download_url: installer
            .map(|a| a.browser_download_url.clone())
            .unwrap_or_else(|| release_url.clone()),
        release_url,
        in_app: has_manifest
            && installer.is_some()
            && matches!(package, Package::Windows | Package::AppImage | Package::Deb),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn release(version: &str) -> Release {
        let tag = format!("v{version}");
        serde_json::from_value(json!({"tag_name":tag,"body":"更新说明","assets":[
            {"name":format!("Linchuan_{version}_x64-setup.exe"),"browser_download_url":format!("{RELEASES}/download/{tag}/Linchuan_{version}_x64-setup.exe")},
            {"name":"latest.json","browser_download_url":format!("{RELEASES}/download/{tag}/latest.json")}
        ]})).unwrap()
    }
    #[test]
    fn stable_versions_compare_numerically_without_downgrades() {
        let update = describe("0.2.0", Package::Windows, release("0.10.0")).unwrap();
        assert!(update.available && update.in_app);
        assert!(update
            .download_url
            .ends_with("Linchuan_0.10.0_x64-setup.exe"));
        assert!(
            !describe("0.10.0", Package::Windows, release("0.2.0"))
                .unwrap()
                .available
        );
        assert!(
            !describe("0.2.0", Package::Windows, release("0.2.0"))
                .unwrap()
                .available
        );
        assert!(describe("0.2.0", Package::Windows, release("0.3.0-beta.1")).is_err());
    }
    #[test]
    fn rc_installations_can_upgrade_to_stable_without_receiving_rc_updates() {
        assert!(
            describe("0.4.0-rc.2", Package::Windows, release("0.4.0"))
                .unwrap()
                .available
        );
        assert!(
            !describe("0.4.0-rc.2", Package::Windows, release("0.3.0"))
                .unwrap()
                .available
        );
        assert!(describe("0.3.0", Package::Windows, release("0.4.0-rc.1")).is_err());
    }
    #[test]
    fn missing_packages_and_untrusted_urls_fall_back_to_the_release_page() {
        let android = describe("0.2.0", Package::Android, release("0.3.0")).unwrap();
        assert!(!android.in_app);
        assert_eq!(android.download_url, android.release_url);
        let mut untrusted = release("0.3.0");
        untrusted.assets[0].browser_download_url = "https://example.com/app.exe".into();
        let update = describe("0.2.0", Package::Windows, untrusted).unwrap();
        assert!(!update.in_app);
        assert_eq!(update.download_url, update.release_url);
    }
}
