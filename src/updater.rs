use std::io::{Read, Write};
use std::sync::mpsc::Sender;

use crate::progress::ProgressEvent;

const REPO: &str = "git-metalheadtwink/spt-modpack-updater";

pub fn check_self_update(tx: &Sender<ProgressEvent>) {
    match fetch_latest_release() {
        Ok(Some((version, url))) if is_newer(&version, env!("CARGO_PKG_VERSION")) => {
            let _ = tx.send(ProgressEvent::SelfUpdateAvailable { version, url });
        }
        _ => {}
    }
}

fn fetch_latest_release() -> anyhow::Result<Option<(String, String)>> {
    let api_url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        REPO
    );

    let resp = ureq::get(&api_url)
        .set("User-Agent", "spt-modpack-updater")
        .set("Accept", "application/vnd.github.v3+json")
        .call()?;

    let body = resp.into_string()?;
    let json: serde_json::Value = serde_json::from_str(&body)?;

    let tag = json["tag_name"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches('v')
        .to_string();

    if tag.is_empty() {
        return Ok(None);
    }

    let download_url = json["assets"]
        .as_array()
        .and_then(|assets| {
            assets.iter().find(|a| {
                a["name"]
                    .as_str()
                    .map(|n| n == "spt-modpack-updater.exe")
                    .unwrap_or(false)
            })
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .map(|s| s.to_string());

    Ok(download_url.map(|u| (tag, u)))
}

fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> {
        s.trim_start_matches('v')
            .split('.')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    parse(latest) > parse(current)
}

pub fn download_and_replace(url: &str, tx: &Sender<ProgressEvent>) -> anyhow::Result<()> {
    let resp = ureq::get(url)
        .set("User-Agent", "spt-modpack-updater")
        .call()?;

    let total = resp
        .header("content-length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let current_exe = std::env::current_exe()?;
    let tmp_path = current_exe.with_extension("exe.tmp");

    {
        let mut reader = resp.into_reader();
        let mut file = std::fs::File::create(&tmp_path)?;
        let mut buf = vec![0u8; 65536];
        let mut downloaded = 0u64;

        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            downloaded += n as u64;
            if total > 0 {
                let _ = tx.send(ProgressEvent::Phase {
                    name: "Downloading".into(),
                    current: downloaded,
                    total,
                });
            }
        }
    }

    // On Windows a running exe can be renamed, just not deleted.
    // Rename current → .old, then move the downloaded tmp into place.
    let old_path = current_exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&old_path);
    std::fs::rename(&current_exe, &old_path)?;
    std::fs::rename(&tmp_path, &current_exe)?;

    let _ = tx.send(ProgressEvent::Log(
        "Update installed. Relaunching...".into(),
    ));
    let _ = tx.send(ProgressEvent::Done);

    Ok(())
}

// Spawns the newly written exe and exits this process.
pub fn relaunch() -> ! {
    let exe = std::env::current_exe().expect("cannot determine current exe path");
    let _ = std::process::Command::new(exe).spawn();
    std::process::exit(0);
}

// Removes any leftover .old file from a previous self-update.
pub fn cleanup_old_exe() {
    if let Ok(exe) = std::env::current_exe() {
        let old = exe.with_extension("exe.old");
        let _ = std::fs::remove_file(old);
        let tmp = exe.with_extension("exe.tmp");
        let _ = std::fs::remove_file(tmp);
    }
}
