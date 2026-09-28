//! Persistent storage and daemon for scheduled tasks.

use std::fs::{create_dir_all, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScheduleEntry {
    pub id: String,
    pub name: String,
    pub task: String,
    pub timing: String,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub interval_seconds: Option<u64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "now_ts")]
    pub created_at: f64,
    #[serde(default)]
    pub last_run: Option<f64>,
    #[serde(default)]
    pub last_status: Option<String>,
}

fn default_true() -> bool {
    true
}

fn now_ts() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn schedules_path(project_dir: &Path) -> PathBuf {
    let dir = project_dir.join(".opendesk");
    let _ = create_dir_all(&dir);
    dir.join("schedules.json")
}

pub struct ScheduleStore {
    path: PathBuf,
}

impl ScheduleStore {
    pub fn new(project_dir: &Path) -> Self {
        Self {
            path: schedules_path(project_dir),
        }
    }

    pub fn all(&self) -> Vec<ScheduleEntry> {
        if !self.path.exists() {
            return vec![];
        }
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return vec![],
        };
        serde_json::from_reader(file).unwrap_or_default()
    }

    pub fn save(&self, entries: &[ScheduleEntry]) -> Result<()> {
        let file = File::create(&self.path)?;
        serde_json::to_writer_pretty(file, entries)?;
        Ok(())
    }

    pub fn add(&self, name: &str, task: &str, timing: &str) -> Result<ScheduleEntry> {
        let mut entries = self.all();
        let entry = ScheduleEntry {
            id: format!("{:08x}", rand::random::<u32>()),
            name: name.to_string(),
            task: task.to_string(),
            timing: timing.to_string(),
            cron: None,
            interval_seconds: None,
            enabled: true,
            created_at: now_ts(),
            last_run: None,
            last_status: None,
        };
        entries.retain(|e| e.name != name);
        entries.push(entry.clone());
        self.save(&entries)?;
        Ok(entry)
    }

    pub fn remove(&self, name: &str) -> Result<bool> {
        let mut entries = self.all();
        let before = entries.len();
        entries.retain(|e| e.name != name);
        if entries.len() < before {
            self.save(&entries)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

pub async fn run_scheduler_daemon(project_dir: &Path) -> Result<()> {
    let store = ScheduleStore::new(project_dir);
    let entries = store.all();
    if entries.is_empty() {
        println!("No schedules found in {}. Add one with the learn tool or schedule tool.", project_dir.display());
        println!("Waiting for schedules... (Ctrl-C to stop)");
    } else {
        println!("Starting scheduler with {} task(s)... (Ctrl-C to stop)", entries.len());
    }

    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let entries = store.all();
        for entry in entries {
            if entry.enabled {
                info!("Scheduled task triggered: {} -> {}", entry.name, entry.task);
            }
        }
    }
}
