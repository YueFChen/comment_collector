//! Opt-in, process-local scheduler.
//! Scheduling state survives restarts; network cursors deliberately do not.
use crate::{CommentCollector, storage, valid_level_id};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use wonderland_plugin_sdk::PluginFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CollectionMode {
    Full,
    Incremental,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MonitorConfig {
    pub enabled: bool,
    pub level_ids: Vec<String>,
    pub incremental_interval_secs: u32,
    pub full_interval_secs: u32,
    pub max_parallel: u32,
    pub request_interval_ms: u32,
    pub overlap_pages: u32,
    pub incremental_max_pages: u32,
}
impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            level_ids: vec![],
            incremental_interval_secs: 1800,
            full_interval_secs: 86_400,
            max_parallel: 2,
            request_interval_ms: 500,
            overlap_pages: 2,
            incremental_max_pages: 50,
        }
    }
}
impl MonitorConfig {
    pub fn validate(&self) -> Result<(), PluginFailure> {
        let unique: HashSet<_> = self.level_ids.iter().collect();
        if self.level_ids.len() > 100
            || unique.len() != self.level_ids.len()
            || self.level_ids.iter().any(|id| !valid_level_id(id))
            || !(60..=86_400).contains(&self.incremental_interval_secs)
            || !(300..=2_592_000).contains(&self.full_interval_secs)
            || self.full_interval_secs < self.incremental_interval_secs
            || !(1..=4).contains(&self.max_parallel)
            || !(250..=60_000).contains(&self.request_interval_ms)
            || !(1..=10).contains(&self.overlap_pages)
            || !(1..=500).contains(&self.incremental_max_pages)
            || self.incremental_max_pages < self.overlap_pages
        {
            return Err(PluginFailure::InvalidInput);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct MonitorLevelStatus {
    pub level_id: String,
    pub running: bool,
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub last_attempt_at: u64,
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub last_success_at: u64,
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub last_full_success_at: u64,
    pub consecutive_failures: u32,
    pub last_error: String,
    pub last_mode: Option<CollectionMode>,
    /// A paused full traversal must finish before returning to incremental scans.
    #[serde(default)]
    pub needs_full_recovery: bool,
    /// Computed from the current schedule; null while paused or running.
    #[serde(default)]
    #[cfg_attr(feature = "bindings", ts(type = "number | null"))]
    pub next_collect_at: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct MonitorStatus {
    pub config: MonitorConfig,
    pub levels: Vec<MonitorLevelStatus>,
}

#[derive(Default)]
pub(crate) struct MonitorRuntime {
    pub initialized: bool,
    pub statuses: HashMap<String, MonitorLevelStatus>,
    pub active: HashMap<String, Arc<AtomicBool>>,
    pub requested: HashMap<String, CollectionMode>,
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), PluginFailure> {
    use std::io::Write;
    let bytes = serde_json::to_vec(value).map_err(storage)?;
    let temp = path.with_extension("tmp");
    let mut file = fs::File::create(&temp).map_err(storage)?;
    file.write_all(&bytes).map_err(storage)?;
    file.sync_all().map_err(storage)?;
    drop(file);
    fs::rename(temp, path).map_err(storage)
}
pub(crate) fn run_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{ns:020}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

impl CommentCollector {
    pub fn monitor_config(&self) -> Result<MonitorConfig, PluginFailure> {
        let _guard = self.monitor_config_lock.lock().map_err(storage)?;
        self.read_monitor_config()
    }
    fn read_monitor_config(&self) -> Result<MonitorConfig, PluginFailure> {
        let path = self.root.join("monitor-config.json");
        match fs::read(path) {
            Ok(bytes) => {
                let config: MonitorConfig = serde_json::from_slice(&bytes).map_err(storage)?;
                config.validate()?;
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(MonitorConfig::default())
            }
            Err(error) => Err(storage(error)),
        }
    }
    pub fn configure_monitor(&self, config: MonitorConfig) -> Result<MonitorConfig, PluginFailure> {
        // Serialize config writes and job admission under the same lock ordering.
        let _guard = self.monitor_config_lock.lock().map_err(storage)?;
        self.configure_monitor_locked(config)
    }
    /// Add one target without replacing settings or targets from another caller.
    pub fn add_monitor(&self, level_id: &str) -> Result<MonitorConfig, PluginFailure> {
        if !valid_level_id(level_id) {
            return Err(PluginFailure::InvalidInput);
        }
        let _guard = self.monitor_config_lock.lock().map_err(storage)?;
        let mut config = self.read_monitor_config()?;
        if config.level_ids.iter().any(|id| id == level_id) {
            return Ok(config);
        }
        // The first target starts monitoring. Adding to a paused list preserves
        // the user's pause instead of unexpectedly restarting existing targets.
        if config.level_ids.is_empty() {
            config.enabled = true;
        }
        config.level_ids.push(level_id.into());
        self.configure_monitor_locked(config)
    }
    pub fn remove_monitor(&self, level_id: &str) -> Result<MonitorConfig, PluginFailure> {
        if !valid_level_id(level_id) {
            return Err(PluginFailure::InvalidInput);
        }
        let _guard = self.monitor_config_lock.lock().map_err(storage)?;
        let mut config = self.read_monitor_config()?;
        config.level_ids.retain(|id| id != level_id);
        if config.level_ids.is_empty() {
            config.enabled = false;
        }
        self.configure_monitor_locked(config)
    }
    fn configure_monitor_locked(
        &self,
        config: MonitorConfig,
    ) -> Result<MonitorConfig, PluginFailure> {
        config.validate()?;
        atomic_json(&self.root.join("monitor-config.json"), &config)?;
        self.bbs
            .set_request_interval(config.request_interval_ms as u64);
        let mut monitor = self.monitor.lock().map_err(storage)?;
        for (id, cancel) in &monitor.active {
            if !config.enabled || !config.level_ids.contains(id) {
                cancel.store(true, Ordering::SeqCst);
            }
        }
        monitor
            .requested
            .retain(|id, _| config.enabled && config.level_ids.contains(id));
        Ok(config)
    }
    fn initialize_monitor(&self, monitor: &mut MonitorRuntime) -> Result<(), PluginFailure> {
        if monitor.initialized {
            return Ok(());
        }
        match fs::read(self.root.join("monitor-state.json")) {
            Ok(bytes) => {
                let statuses: Vec<MonitorLevelStatus> =
                    serde_json::from_slice(&bytes).map_err(storage)?;
                for mut status in statuses {
                    if !valid_level_id(&status.level_id) {
                        return Err(PluginFailure::InvalidInput);
                    }
                    // A killed process cannot have an in-flight job after restart.
                    if status.running {
                        status.last_error = "上次采集进程中断，将重新扫描".into();
                        status.consecutive_failures = status.consecutive_failures.max(1);
                        status.needs_full_recovery = true;
                    }
                    status.running = false;
                    monitor.statuses.insert(status.level_id.clone(), status);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage(error)),
        }
        monitor.initialized = true;
        Ok(())
    }
    fn save_monitor_state(&self, monitor: &MonitorRuntime) -> Result<(), PluginFailure> {
        let statuses: Vec<_> = monitor.statuses.values().collect();
        atomic_json(&self.root.join("monitor-state.json"), &statuses)
    }
    pub fn monitor_status(&self) -> Result<MonitorStatus, PluginFailure> {
        let config = self.monitor_config()?;
        let mut monitor = self.monitor.lock().map_err(storage)?;
        self.initialize_monitor(&mut monitor)?;
        let timestamp = now();
        let levels = config
            .level_ids
            .iter()
            .map(|id| {
                let mut status =
                    monitor
                        .statuses
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| MonitorLevelStatus {
                            level_id: id.clone(),
                            ..Default::default()
                        });
                status.next_collect_at = if config.enabled && !status.running {
                    Some(if monitor.requested.contains_key(id) {
                        timestamp
                    } else {
                        next_attempt_at(&config, &status).max(timestamp)
                    })
                } else {
                    None
                };
                status
            })
            .collect();
        Ok(MonitorStatus { config, levels })
    }
    pub fn request_monitor_run(
        &self,
        level_id: &str,
        mode: CollectionMode,
    ) -> Result<(), PluginFailure> {
        let _guard = self.monitor_config_lock.lock().map_err(storage)?;
        let config = self.read_monitor_config()?;
        if !config.enabled || !config.level_ids.iter().any(|id| id == level_id) {
            return Err(PluginFailure::InvalidInput);
        }
        let mut monitor = self.monitor.lock().map_err(storage)?;
        self.initialize_monitor(&mut monitor)?;
        if monitor.active.contains_key(level_id) {
            return Err(PluginFailure::Busy);
        }
        let mode = if mode == CollectionMode::Incremental
            && monitor.statuses.get(level_id).is_none_or(|s| {
                s.last_full_success_at == 0 || s.needs_full_recovery || s.consecutive_failures > 0
            }) {
            CollectionMode::Full
        } else {
            mode
        };
        monitor.requested.insert(level_id.into(), mode);
        Ok(())
    }
    /// Called once after the Core host connection is available. No OS service is installed.
    pub fn start_monitoring(self: &Arc<Self>) {
        if self.monitor_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let collector = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let Some(collector) = collector.upgrade() else {
                    break;
                };
                if let Err(error) = collector.monitor_tick() {
                    tracing::warn!(%error, "监测调度失败");
                }
                drop(collector);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }
    fn monitor_tick(self: &Arc<Self>) -> Result<(), PluginFailure> {
        let _guard = self.monitor_config_lock.lock().map_err(storage)?;
        let config = self.read_monitor_config()?;
        if !config.enabled {
            return Ok(());
        }
        self.bbs
            .set_request_interval(config.request_interval_ms as u64);
        let mut monitor = self.monitor.lock().map_err(storage)?;
        self.initialize_monitor(&mut monitor)?;
        let timestamp = now();
        // Least recently attempted first prevents starvation with more levels than slots.
        let mut ids = config.level_ids.clone();
        ids.sort_by_key(|id| {
            monitor
                .statuses
                .get(id)
                .map(|s| s.last_attempt_at)
                .unwrap_or(0)
        });
        for id in ids {
            if monitor.active.len() >= config.max_parallel as usize
                || self.active_jobs.load(Ordering::SeqCst) >= config.max_parallel
            {
                break;
            }
            if monitor.active.contains_key(&id) {
                continue;
            }
            let status = monitor
                .statuses
                .entry(id.clone())
                .or_insert_with(|| MonitorLevelStatus {
                    level_id: id.clone(),
                    ..Default::default()
                })
                .clone();
            let explicitly_requested = monitor.requested.contains_key(&id);
            let Some(mode) = monitor
                .requested
                .remove(&id)
                .or_else(|| due_mode(&config, &status, timestamp))
            else {
                continue;
            };
            let token = Arc::new(AtomicBool::new(false));
            monitor.active.insert(id.clone(), token.clone());
            let state = monitor.statuses.get_mut(&id).expect("inserted");
            state.running = true;
            state.last_attempt_at = timestamp;
            state.last_mode = Some(mode);
            // Persist admission before network; failure must not silently spawn an untracked job.
            if let Err(error) = self.save_monitor_state(&monitor) {
                monitor.active.remove(&id);
                *monitor.statuses.get_mut(&id).expect("inserted") = status;
                if explicitly_requested {
                    monitor.requested.insert(id.clone(), mode);
                }
                return Err(error);
            }
            let collector = self.clone();
            let options = config.clone();
            let previous_status = status;
            tokio::spawn(async move {
                // Supervise panics as well as ordinary errors, otherwise a panicked
                // worker would occupy its per-level scheduling slot until restart.
                let worker = collector.clone();
                let worker_id = id.clone();
                let cancel_token = token.clone();
                let result = tokio::spawn(async move {
                    worker
                        .collect_mode(&worker_id, mode, &options, token, |_| {})
                        .await
                })
                .await
                .unwrap_or_else(|error| {
                    Err(PluginFailure::Other(format!("采集任务异常结束：{error}")))
                });
                if let Ok(mut monitor) = collector.monitor.lock() {
                    monitor.active.remove(&id);
                    let status = monitor.statuses.get_mut(&id).expect("admitted status");
                    status.running = false;
                    match result {
                        Err(PluginFailure::Busy) => {
                            *status = previous_status;
                            // Foreground contention is not a network failure. Retain an
                            // explicit request and retry admission on the next tick.
                            if explicitly_requested && !cancel_token.load(Ordering::SeqCst) {
                                monitor.requested.insert(id.clone(), mode);
                            }
                        }
                        Err(_) if cancel_token.load(Ordering::SeqCst) => {
                            *status = previous_status;
                            status.needs_full_recovery |= mode == CollectionMode::Full;
                            status.last_error = "监测已暂停，已抓取部分已保留".into();
                        }
                        Ok(_) => {
                            status.last_success_at = now();
                            if mode == CollectionMode::Full {
                                status.last_full_success_at = status.last_success_at;
                                status.needs_full_recovery = false;
                            }
                            status.consecutive_failures = 0;
                            status.last_error.clear();
                        }
                        Err(error) => {
                            status.consecutive_failures =
                                status.consecutive_failures.saturating_add(1);
                            status.last_error = error.to_string().chars().take(2048).collect();
                        }
                    }
                    if let Err(error) = collector.save_monitor_state(&monitor) {
                        tracing::error!(%error, "保存监测状态失败");
                    }
                }
            });
        }
        Ok(())
    }
}

pub(crate) fn due_mode(
    config: &MonitorConfig,
    status: &MonitorLevelStatus,
    now: u64,
) -> Option<CollectionMode> {
    if status.running {
        return None;
    }
    if status.last_attempt_at > 0 && now < next_attempt_at(config, status) {
        return None;
    }
    if status.needs_full_recovery
        || status.consecutive_failures > 0
        || status.last_full_success_at == 0
        || now.saturating_sub(status.last_full_success_at) >= config.full_interval_secs as u64
    {
        Some(CollectionMode::Full)
    } else {
        Some(CollectionMode::Incremental)
    }
}

fn next_attempt_at(config: &MonitorConfig, status: &MonitorLevelStatus) -> u64 {
    if status.last_attempt_at == 0 {
        return 0;
    }
    // Shared by admission and the displayed deadline, including failure cooldown.
    let wait = if status.consecutive_failures > 0 {
        (config.incremental_interval_secs as u64)
            .saturating_mul(1u64 << status.consecutive_failures.min(6))
            .min(86_400)
    } else {
        config.incremental_interval_secs as u64
    };
    status.last_attempt_at.saturating_add(wait)
}
