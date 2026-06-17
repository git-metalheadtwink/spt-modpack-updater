#[derive(Debug, Clone)]
pub enum UpdateStatus {
    NotInitialized,
    UpToDate { hash: String },
    Available { local: String, remote: String },
    Error(String),
}
