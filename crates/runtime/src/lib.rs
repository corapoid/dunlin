//! UI-independent session models and execution. Desktop adapters own rendering,
//! credentials, project trust decisions and application-specific persistence.
pub mod run;
pub mod transcript;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

pub fn session_status_from_stop(reason: &str) -> dunlin_domain::SessionStatus {
    use dunlin_domain::SessionStatus;
    match reason {
        "end_turn" => SessionStatus::Done,
        "cancelled" => SessionStatus::Interrupted,
        _ => SessionStatus::Failed,
    }
}
