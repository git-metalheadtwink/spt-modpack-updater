use crate::types::UpdateStatus;

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Phase { name: String, current: u64, total: u64 },
    Log(String),
    StatusResult(UpdateStatus),
    BranchList(Vec<String>),
    Done,
    Error(String),
    SelfUpdateAvailable { version: String, url: String },
}
