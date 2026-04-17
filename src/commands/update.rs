use crate::errors::XmasterError;
use serde::Deserialize;

const REPO: &str = "paperfoot/xmaster";

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Map the current compile-time target to the release asset suffix.
fn asset_name() -> &'static str {
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "xmaster-aarch64-darwin"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "xmaster-x86_64-darwin"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "xmaster-x86_64-linux"
    } else if cfg!(target_os = "windows") {
        "xmaster-x86_64-windows.exe"
    } else {
        "xmaster"
    }
}

pub async fn execute(check: bool) -> Result<(), XmasterError> {
    let current = env!("CARGO_PKG_VERSION");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(format!("xmaster/{current}"))
        .build()
        .map_err(XmasterError::Http)?;

    let release: GhRelease = client
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .send()
        .await?
        .json()
        .await
        .map_err(|e| XmasterError::Config(format!("Failed to check for updates: {e}")))?;

    let latest = release.tag_name.trim_start_matches('v');

    if latest == current {
        eprintln!("Already up to date (v{current})");
        return Ok(());
    }

    // Compare versions — only upgrade, never downgrade.
    let current_parts: Vec<u32> = current.split('.').filter_map(|s| s.parse().ok()).collect();
    let latest_parts: Vec<u32> = latest.split('.').filter_map(|s| s.parse().ok()).collect();
    if latest_parts <= current_parts {
        eprintln!("Already up to date (v{current}, latest release is v{latest})");
        return Ok(());
    }

    if check {
        eprintln!("Update available: v{current} -> v{latest}");
        eprintln!("Run `xmaster update` to install");
        return Ok(());
    }

    // Find the right asset for this platform.
    let target = asset_name();
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == target)
        .ok_or_else(|| {
            XmasterError::Config(format!(
                "No release asset found for this platform ({target}). \
                 Install via `cargo install xmaster` or Homebrew instead."
            ))
        })?;

    eprintln!("Downloading {target} v{latest}...");
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await?
        .bytes()
        .await?;

    // Replace the running binary.
    let current_exe = std::env::current_exe()
        .map_err(|e| XmasterError::Config(format!("Cannot locate current binary: {e}")))?;

    // Write to a temp file next to the binary, then atomic rename.
    let tmp = current_exe.with_extension("tmp");
    std::fs::write(&tmp, &bytes)
        .map_err(|e| XmasterError::Config(format!("Failed to write update: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| XmasterError::Config(format!("Failed to set permissions: {e}")))?;
    }

    std::fs::rename(&tmp, &current_exe)
        .map_err(|e| XmasterError::Config(format!("Failed to replace binary: {e}")))?;

    eprintln!("Updated: v{current} -> v{latest}");
    eprintln!("Run `xmaster skill update` to sync the skill file.");
    Ok(())
}
