use anyhow::Result;
use std::path::PathBuf;

#[cfg(windows)]
extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

#[cfg(windows)]
unsafe extern "system" fn ctrl_handler(_ctrl_type: u32) -> i32 {
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    std::process::exit(0);
}

mod git;
mod manifest;
mod progress;
mod spt;
mod tui;
mod types;
mod ui;

mod build_cfg {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}
use build_cfg::DEFAULT_BRANCH;

const BRANCH_FILE: &str = ".updater-branch";

fn is_spt_folder(path: &std::path::Path) -> bool {
    // Must have the game exe, plus at least one SPT-specific directory.
    path.join("EscapeFromTarkov.exe").is_file()
        && (path.join("BepInEx").is_dir() || path.join("SPT").is_dir())
}

fn abort_not_spt(path: &std::path::Path) -> ! {
    const R: &str = "\x1b[31m";
    const Y: &str = "\x1b[33m";
    const W: &str = "\x1b[37m";
    const D: &str = "\x1b[90m";
    const Z: &str = "\x1b[0m";

    println!();
    println!("  {R}✗  This does not look like an SPT installation folder.{Z}");
    println!("  {D}    {}{Z}", path.display());
    println!();
    println!("  {W}Expected to find:{Z}");
    println!("  {D}  •{Z} {Y}EscapeFromTarkov.exe{Z}");
    println!("  {D}  •{Z} {Y}BepInEx/{Z}  {D}or{Z}  {Y}SPT/{Z}  {D}directory{Z}");
    println!();
    println!("  {W}Options:{Z}");
    println!("  {D}  •{Z} Move this program into your SPT folder and run it there.");
    println!("  {D}  •{Z} Or run with {Y}--path <SPT folder>{Z} to specify the location.");
    println!();
    println!("  {D}Press Enter to exit...{Z}");
    let _ = std::io::stdin().read_line(&mut String::new());
    std::process::exit(1);
}

fn load_branch(path: &std::path::Path) -> String {
    std::fs::read_to_string(path.join(BRANCH_FILE))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BRANCH.to_string())
}

fn main() -> Result<()> {
    #[cfg(windows)]
    {
        let _ = enable_ansi_support::enable_ansi_support();
        unsafe { SetConsoleCtrlHandler(Some(ctrl_handler), 1); }
    }

    print!("\x1b]0;spt-modpack-updater\x07");
    let _ = std::io::Write::flush(&mut std::io::stdout());

    // Resolve game path: --path arg or current directory.
    let game_path: PathBuf = std::env::args()
        .position(|a| a == "--path" || a == "-p")
        .and_then(|i| std::env::args().nth(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    if !is_spt_folder(&game_path) {
        abort_not_spt(&game_path);
    }

    let spt_version = spt::detect_spt_version(&game_path).ok();
    let initial_branch = load_branch(&game_path);

    tui::run(tui::Config { game_path, spt_version, initial_branch })
}
