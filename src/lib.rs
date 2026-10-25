pub mod app;
pub mod backup;
pub mod http;
pub mod model;
pub mod page;
pub mod paths;
pub mod providers;
pub mod settings;
pub mod sotf;
pub mod steam;
pub mod store;
pub mod vdf;

#[cfg(test)]
pub fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
