# Repo Notes

## Git push rules
- Local working branch: `dev2`
- **Never push to `dev2` on origin.** Always push to `dev` on origin.
- The push refspec is preconfigured: `git push` from `dev2` pushes to `origin/dev` via `remote.origin.push = refs/heads/dev2:refs/heads/dev`.
- Pulls still track `origin/dev2`.

## Build
- `rustup` is not on PATH; the only toolchain installed is `stable-x86_64-pc-windows-msvc` at `%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-msvc`.
- MSVC tools live at `C:\Program Files\Microsoft Visual Studio\18\Community`.
- The toolchain bin dir MUST be on PATH at build time, otherwise `proc-macro-error v1.0.4`'s build script (via `version_check`) can't locate `rustc` and panics with `Option::unwrap() on a None value` at build.rs:7.
- `.cargo/config.toml` sets `target-dir = "target-CLI"`, so release artifacts land in `target-CLI/release/`, not `target/release/`.
- Build command (PowerShell):
  ```powershell
  $tc = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin"
  $vcvars = "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat"
  cmd /c "`"$vcvars`" >nul && set `"PATH=$tc;%PATH%`" && `"$tc\cargo.exe`" build --release"
  Copy-Item "target-CLI\release\spt-modpack-updater.exe" "spt-modpack-updater.exe" -Force
  Copy-Item "target-CLI\release\spt-modpack-updater.exe" "D:\SPT\spt-modpack-updater.exe" -Force
  ```
- **After every recompile, copy `target-CLI\release\spt-modpack-updater.exe` to BOTH:**
  - `D:\SPT\spt-modpack-updater\spt-modpack-updater.exe` (repo root)
  - `D:\SPT\spt-modpack-updater.exe` (SPT install root)

## Design notes (don't undo these)
- `updater-config.json` is now **tracked** (removed from `.gitignore`). The real config lives in git so a working-tree wipe is recoverable via `git checkout`. Current real config (as of 2026-06-12): `https://github.com/git-metalheadtwink/spt-modpack-test.git`, branch `main`, name `My SPT Modpack`. Never overwrite it with `updater-config-example.json` (which contains placeholders like `YOUR_USERNAME/YOUR_MODPACK_REPO`).
- `cleanup_untracked` was removed from `src/git.rs`. Reason: it walked the filesystem with walkdir and deleted any file not in `git ls-files` output, but `git ls-files -o -i --exclude-standard` does NOT recurse into nested git repos (anything containing its own `.git/`), so files like `spt-modpack-updater/Cargo.toml` were classed as not-allowed and wiped. `git reset --hard origin/<branch>` already handles tracked-file removals and `git clean -fd` already handles untracked-file removal while respecting `.gitignore`. Don't re-add walkdir-based mirror logic.
- `walkdir` is intentionally NOT a dependency anymore.
- `run_git_stream` in `src/git.rs` parses git progress frames (split on `\r` AND `\n`) and renders them through `ui::progress` as a pink bar. The recognized phases live in the `PHASES` array in `parse_progress`. Add new phase names there if git ever introduces one.
- `ui::progress(phase, current, total)` overwrites the same line via `\r`. The trailing spaces in the format string exist to fully wipe a longer previous line — keep them.