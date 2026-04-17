//! One-time star nudge — shown after the first successful command.
//!
//! The message is phrased so both humans and AI agents can act on it:
//! humans see a clickable URL, agents see a ready-to-run `gh` command.
//! A sentinel file prevents repeat prompts.

use crate::config::config_dir;
use crate::output::OutputFormat;
use std::fs;
use std::path::PathBuf;

const REPO: &str = "paperfoot/xmaster";
const REPO_URL: &str = "https://github.com/paperfoot/xmaster";

fn sentinel_path() -> PathBuf {
    config_dir().join(".star-prompted")
}

/// Print the star nudge if the user hasn't seen it yet.
/// Only shows for interactive (Table) output — never in JSON/CSV/piped mode.
pub fn maybe_show(format: OutputFormat) {
    if format != OutputFormat::Table {
        return;
    }

    let sentinel = sentinel_path();
    if sentinel.exists() {
        return;
    }

    // Mark as shown before printing (best-effort, don't fail the command)
    let _ = fs::create_dir_all(sentinel.parent().unwrap_or(&config_dir()));
    let _ = fs::write(&sentinel, "shown");

    eprintln!();
    eprintln!("  \x1b[33m★\x1b[0m Enjoying xmaster? A GitHub star helps others find this project:");
    eprintln!("    {REPO_URL}");
    eprintln!();
    eprintln!("    Quick star:  \x1b[2mgh api user/starred/{REPO} -X PUT\x1b[0m");
    eprintln!("    Or run:      \x1b[2mxmaster star\x1b[0m");
    eprintln!();
}

/// Open the repo in the browser (for `xmaster star`).
pub fn open_star_page() {
    eprintln!("Opening {REPO_URL} ...");

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(REPO_URL).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(REPO_URL).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", REPO_URL])
            .spawn();
    }

    eprintln!();
    eprintln!("  \x1b[33m★\x1b[0m Thanks! If your browser didn't open, visit:");
    eprintln!("    {REPO_URL}");
    eprintln!();
    eprintln!("  Or star from the terminal:");
    eprintln!("    gh api user/starred/{REPO} -X PUT");
    eprintln!();

    // Mark nudge as shown so they don't get it again
    let sentinel = sentinel_path();
    let _ = fs::create_dir_all(sentinel.parent().unwrap_or(&config_dir()));
    let _ = fs::write(&sentinel, "shown");
}
