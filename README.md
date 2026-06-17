# SPT Modpack Updater

A lightweight Windows TUI app that keeps an SPT (Single Player Tarkov) modpack in sync with a Git remote. Drop it in your SPT folder and run it — it handles first-time install, updates, config file protection, and branch switching.

---

## Features

- **UPDATE** — First-time install or pull latest changes from the configured remote
- **REPAIR** — Restore protected BepInEx config files from `.BACKUP/BepInEx/config/`
- **BRANCH** — Switch between remote branches without touching the rest of your setup
- **Protected files** — `BepInEx/config/BepInEx.cfg` and the Configuration Manager cfg survive every reset/clean automatically
- **SPT folder guard** — Refuses to run outside a valid SPT installation (requires `EscapeFromTarkov.exe` + `BepInEx/` or `SPT/`)
- **Safe quit** — Pressing Esc during an active operation asks for confirmation before exiting
- **No flicker** — Built on [ratatui](https://github.com/ratatui-org/ratatui) with double-buffered rendering

---

## Requirements

- Windows 10 / 11
- [Git for Windows](https://git-scm.com/download/win) installed and on PATH
- A valid SPT installation folder

---

## Usage

Place `spt-modpack-updater.exe` in your SPT folder and double-click it.

Alternatively, run from anywhere with:

```
spt-modpack-updater.exe --path "C:\path\to\SPT"
```

### Controls

| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate menu |
| `Enter` | Select / confirm |
| `Esc` | Quit (prompts if operation is running) |

---

## Configuration

Copy `updater-config-example.json` to `updater-config.json` and fill in your values before building:

```json
{
  "remote_url": "https://github.com/your-org/your-modpack.git",
  "branch": "main",
  "modpack_name": "My SPT Modpack"
}
```

`updater-config.json` is excluded from version control — it contains your private remote URL.

---

## Building

Requires [Rust](https://rustup.rs) and a Windows build environment.

```
cargo build --release
```

The release binary will be at `target/release/spt-modpack-updater.exe`.

> `updater-config.json` must exist before building — `build.rs` reads it to bake the remote URL and branch into the binary.

---

## Protected Files

The following files are backed up in memory before every `git reset --hard` and restored immediately after, so your personal settings are never overwritten by an update:

- `BepInEx/config/BepInEx.cfg`
- `BepInEx/config/com.bepis.bepinex.configurationmanager.cfg`

The **REPAIR** action copies all files from `.BACKUP/BepInEx/config/` back into `BepInEx/config/`, for cases where a fresh install or manual reset wiped them.

---

## Branch Switching

Select **BRANCH** from the main menu to fetch and display all branches available on the remote. Selecting one saves it to `.updater-branch` in your SPT folder and runs an update check against it immediately. The chosen branch persists across restarts.
