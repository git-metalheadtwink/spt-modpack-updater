use crate::progress::ProgressEvent;
use crate::types::UpdateStatus;
use anyhow::{Context, Result};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

include!(concat!(env!("OUT_DIR"), "/config.rs"));

const PROTECTED: &[&str] = &[
    "BepInEx/config/BepInEx.cfg",
    "BepInEx/config/com.bepis.bepinex.configurationmanager.cfg",
];

// ── git binary ────────────────────────────────────────────────────────────────

fn git_bin() -> PathBuf {
    let local = PathBuf::from("git/git.exe");
    if local.exists() { return local; }
    PathBuf::from("git")
}

// ── low-level runners ─────────────────────────────────────────────────────────

fn run_git(path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(git_bin())
        .args(args)
        .current_dir(path)
        .output()
        .with_context(|| format!("Failed to execute git {:?}", args))?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Git error: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_git_stream_tx(path: &Path, args: &[&str], tx: &Sender<ProgressEvent>) -> Result<()> {
    let mut child = Command::new(git_bin())
        .args(args)
        .current_dir(path)
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to execute git {:?}", args))?;

    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);

    for line in reader.lines().flatten() {
        for raw in line.split('\r') {
            let s = raw.trim();
            if s.is_empty() || s.contains("[new branch]") { continue; }

            if let Some((phase, cur, tot)) = parse_progress(s) {
                let _ = tx.send(ProgressEvent::Phase { name: phase, current: cur, total: tot });
            } else if s.starts_with("fatal") || s.starts_with("error") || s.contains("error:") {
                let _ = tx.send(ProgressEvent::Log(format!("! {}", s)));
            } else {
                let _ = tx.send(ProgressEvent::Log(s.to_string()));
            }
        }
    }

    let status = child.wait()?;
    if !status.success() {
        return Err(anyhow::anyhow!("Git command failed: {:?}", args));
    }
    Ok(())
}

fn parse_progress(s: &str) -> Option<(String, u64, u64)> {
    const PHASES: &[&str] = &[
        "Counting objects", "Compressing objects", "Receiving objects",
        "Resolving deltas", "Updating files", "Unpacking objects",
        "Enumerating objects", "Writing objects",
    ];

    let stripped = s.strip_prefix("remote:").map(|x| x.trim()).unwrap_or(s);
    let phase = PHASES.iter().find(|p| stripped.starts_with(*p)).map(|p| p.to_string())?;

    let after_colon = stripped.split_once(':').map(|(_, r)| r)?.trim();
    let lp = after_colon.find('(')?;
    let rp = after_colon[lp + 1..].find(')')? + lp + 1;
    let inner = &after_colon[lp + 1..rp];
    let frac_end = inner.find(',').unwrap_or(inner.len());
    let (cur_s, tot_s) = inner[..frac_end].split_once('/')?;
    let cur = cur_s.trim().parse::<u64>().ok()?;
    let tot = tot_s.trim().parse::<u64>().ok()?;
    Some((phase, cur, tot))
}

// ── protected-file helpers ────────────────────────────────────────────────────

fn backup_protected(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    PROTECTED.iter().filter_map(|rel| {
        let full = path.join(rel);
        fs::read(&full).ok().map(|data| (PathBuf::from(rel), data))
    }).collect()
}

fn restore_protected(path: &Path, saved: Vec<(PathBuf, Vec<u8>)>, tx: &Sender<ProgressEvent>) {
    for (rel, data) in saved {
        let full = path.join(&rel);
        if let Some(parent) = full.parent() { let _ = fs::create_dir_all(parent); }
        match fs::write(&full, &data) {
            Ok(()) => { let _ = tx.send(ProgressEvent::Log(format!("Protected: {}", rel.display()))); }
            Err(e) => { let _ = tx.send(ProgressEvent::Log(format!("! Restore failed {}: {}", rel.display(), e))); }
        }
    }
}

// Maps a git phase name to a (start%, end%) range of the overall 0-100 operation progress.
pub fn phase_range(phase: &str) -> (f64, f64) {
    match phase {
        "Enumerating objects"              => (0.0,  4.0),
        "Counting objects"                 => (4.0,  8.0),
        "Compressing objects"              => (8.0,  13.0),
        "Receiving objects" | "Unpacking objects" => (13.0, 68.0),
        "Resolving deltas"                 => (68.0, 86.0),
        "Updating files" | "Writing objects" => (86.0, 100.0),
        _                                  => (0.0,  100.0),
    }
}

// ── public background-thread functions ────────────────────────────────────────

pub fn run_check_update(path: &Path, branch: &str, tx: &Sender<ProgressEvent>) {
    if !path.join(".git").exists() {
        let _ = tx.send(ProgressEvent::StatusResult(UpdateStatus::NotInitialized));
        let _ = tx.send(ProgressEvent::Done);
        return;
    }

    let _ = tx.send(ProgressEvent::Log("Fetching from remote...".to_string()));

    if let Err(e) = run_git_stream_tx(path, &["fetch", "origin", "--progress"], tx) {
        let _ = tx.send(ProgressEvent::Error(e.to_string()));
        return;
    }

    let local = match run_git(path, &["rev-parse", "HEAD"]) {
        Ok(s) => s.trim().to_string(),
        Err(e) => { let _ = tx.send(ProgressEvent::Error(e.to_string())); return; }
    };
    let remote = match run_git(path, &["rev-parse", &format!("origin/{}", branch)]) {
        Ok(s) => s.trim().to_string(),
        Err(e) => { let _ = tx.send(ProgressEvent::Error(e.to_string())); return; }
    };

    let status = if local == remote {
        UpdateStatus::UpToDate { hash: local }
    } else {
        UpdateStatus::Available { local, remote }
    };

    let _ = tx.send(ProgressEvent::StatusResult(status));
    let _ = tx.send(ProgressEvent::Done);
}

pub fn run_update(path: &Path, branch: &str, tx: &Sender<ProgressEvent>) {
    // First-time install if no .git
    if !path.join(".git").exists() {
        let _ = tx.send(ProgressEvent::Log("Initializing repository...".to_string()));

        if let Err(e) = run_git(path, &["init"]) { let _ = tx.send(ProgressEvent::Error(e.to_string())); return; }
        if let Err(e) = run_git(path, &["checkout", "-B", branch]) { let _ = tx.send(ProgressEvent::Error(e.to_string())); return; }
        if let Err(e) = run_git(path, &["remote", "add", "origin", DEFAULT_REMOTE]) { let _ = tx.send(ProgressEvent::Error(e.to_string())); return; }

        let _ = tx.send(ProgressEvent::Log("Downloading modpack...".to_string()));
        if let Err(e) = run_git_stream_tx(path, &["fetch", "origin", "--progress"], tx) { let _ = tx.send(ProgressEvent::Error(e.to_string())); return; }
    } else {
        let _ = tx.send(ProgressEvent::Log("Fetching updates...".to_string()));
        if let Err(e) = run_git_stream_tx(path, &["fetch", "origin", "--progress"], tx) { let _ = tx.send(ProgressEvent::Error(e.to_string())); return; }
    }

    let saved = backup_protected(path);
    if !saved.is_empty() { let _ = tx.send(ProgressEvent::Log(format!("Protecting {} config file(s)...", saved.len()))); }

    let _ = tx.send(ProgressEvent::Log("Applying files...".to_string()));
    if let Err(e) = run_git(path, &["reset", "--hard", &format!("origin/{}", branch)]) { let _ = tx.send(ProgressEvent::Error(e.to_string())); return; }

    if let Err(e) = run_git_stream_tx(path, &["clean", "-fd"], tx) { let _ = tx.send(ProgressEvent::Error(e.to_string())); return; }

    restore_protected(path, saved, tx);

    // Show manifest changelog if present
    if let Ok(m) = crate::manifest::read_manifest(path) {
        let _ = tx.send(ProgressEvent::Log(String::new()));
        let _ = tx.send(ProgressEvent::Log(format!("  {}", m.description)));
        if !m.changelog.is_empty() {
            let _ = tx.send(ProgressEvent::Log("  Changelog:".to_string()));
            for entry in &m.changelog {
                let _ = tx.send(ProgressEvent::Log(format!("    • {}", entry)));
            }
        }
    }

    let _ = tx.send(ProgressEvent::Done);
}

pub fn run_repair(path: &Path, tx: &Sender<ProgressEvent>) {
    let src = path.join(".BACKUP").join("BepInEx").join("config");
    let dst = path.join("BepInEx").join("config");

    if !src.exists() {
        let _ = tx.send(ProgressEvent::Error(".BACKUP/BepInEx/config not found".to_string()));
        return;
    }

    if let Err(e) = fs::create_dir_all(&dst) {
        let _ = tx.send(ProgressEvent::Error(e.to_string()));
        return;
    }

    let mut count = 0u32;
    match fs::read_dir(&src) {
        Err(e) => { let _ = tx.send(ProgressEvent::Error(e.to_string())); return; }
        Ok(entries) => {
            for entry in entries.flatten() {
                if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) { continue; }
                let name = entry.file_name();
                let dst_file = dst.join(&name);
                match fs::copy(entry.path(), &dst_file) {
                    Ok(_) => {
                        let _ = tx.send(ProgressEvent::Log(format!("Restored: {}", name.to_string_lossy())));
                        count += 1;
                    }
                    Err(e) => { let _ = tx.send(ProgressEvent::Log(format!("! {}: {}", name.to_string_lossy(), e))); }
                }
            }
        }
    }

    if count == 0 {
        let _ = tx.send(ProgressEvent::Log("No files found in .BACKUP/BepInEx/config/".to_string()));
    }

    let _ = tx.send(ProgressEvent::Done);
}

pub fn run_fetch_branches(path: &Path, tx: &Sender<ProgressEvent>) {
    let _ = tx.send(ProgressEvent::Log("Fetching branch list...".to_string()));

    let output = if path.join(".git").exists() {
        run_git(path, &["ls-remote", "--heads", "origin"])
    } else {
        run_git(path, &["ls-remote", "--heads", DEFAULT_REMOTE])
    };

    match output {
        Err(e) => { let _ = tx.send(ProgressEvent::Error(e.to_string())); }
        Ok(out) => {
            let branches: Vec<String> = out.lines()
                .filter_map(|l| l.split('\t').nth(1)?.strip_prefix("refs/heads/").map(|b| b.to_string()))
                .collect();

            if branches.is_empty() {
                let _ = tx.send(ProgressEvent::Error("No branches found on remote".to_string()));
            } else {
                let _ = tx.send(ProgressEvent::BranchList(branches));
                // No Done sent — BranchList itself transitions the mode to SelectBranch.
                // Sending Done here would race and immediately overwrite SelectBranch → Idle.
            }
        }
    }
}
