use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=updater-config.json");

    let config_path = Path::new("updater-config.json");

    let config = if config_path.exists() {
        fs::read_to_string(config_path).expect("Failed to read updater-config.json")
    } else {
        println!("cargo:warning=updater-config.json not found! Using example.");
        fs::read_to_string("updater-config-example.json")
            .expect("Neither updater-config.json nor updater-config-example.json found!")
    };

    let config: serde_json::Value = serde_json::from_str(&config)
        .expect("Invalid JSON in updater config");

    let remote_url = config["remote_url"].as_str().expect("remote_url missing");
    let branch = config["branch"].as_str().expect("branch missing");
    let modpack_name = config["modpack_name"].as_str().unwrap_or("SPT Modpack");

    let out_dir = env::var_os("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("config.rs");

    let content = format!(
        r#"
#[allow(dead_code)] pub const DEFAULT_REMOTE: &str = "{}";
#[allow(dead_code)] pub const DEFAULT_BRANCH: &str = "{}";
#[allow(dead_code)] pub const MODPACK_NAME:   &str = "{}";
"#,
        remote_url, branch, modpack_name
    );

    fs::write(dest_path, content).expect("Failed to write config.rs");

    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();

        // Version info (set_resource_file would silently ignore set_manifest_file,
        // so we set these fields through the winres API instead)
        res.set("FileDescription",  "SPT Modpack Updater");
        res.set("ProductName",      "SPT Modpack Updater");
        res.set("FileVersion",      "1.0.0.0");
        res.set("ProductVersion",   "1.0.0.0");
        res.set("InternalName",     "spt-modpack-updater");
        res.set("OriginalFilename", "spt-modpack-updater.exe");
        res.set("CompanyName",      "MetalheadTwink");

        // Icon appears on both the exe file and the console window title bar.
        res.set_icon("icon.ico");

        // Embedding this manifest tells Windows the exe is NOT an installer and
        // does not need elevation, which suppresses both the PCA dialog and UAC.
        res.set_manifest_file("app.manifest");

        res.compile()
            .expect("failed to compile windows resources");
    }
}
