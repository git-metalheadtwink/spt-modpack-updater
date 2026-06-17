#[derive(Debug, Clone)]
pub enum UpdateStatus {
    NotInitialized,
    UpToDate { hash: String },
    Available { local: String, remote: String },
    Error(String),
}

impl UpdateStatus {
    pub fn color(&self) -> &'static str {
        match self {
            UpdateStatus::UpToDate { .. }    => "\x1b[32m",       // green
            UpdateStatus::Available { .. }   => "\x1b[33m",       // gold
            UpdateStatus::NotInitialized     => "\x1b[38;5;117m", // blue
            UpdateStatus::Error(_)           => "\x1b[31m",       // red
        }
    }

    pub fn text(&self) -> String {
        match self {
            UpdateStatus::NotInitialized => "Not installed — UPDATE to install".to_string(),
            UpdateStatus::UpToDate { hash } => format!("Up to date ({})", &hash[..hash.len().min(8)]),
            UpdateStatus::Available { local, remote } => format!(
                "Update available  {} \u{2192} {}",
                &local[..local.len().min(8)],
                &remote[..remote.len().min(8)]
            ),
            UpdateStatus::Error(e) => format!("Check failed: {}", e),
        }
    }

    pub fn has_update(&self) -> bool {
        matches!(self, UpdateStatus::Available { .. } | UpdateStatus::NotInitialized)
    }
}
