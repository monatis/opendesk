//! Waiting for UI changes instead of sleeping: wait for an element to reach
//! a state, and an event log of what changed in an app since the last read.

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xa11y::{App, AppExt, ElementData, ElementState};

use super::local::LocalComputer;

/// Longest a single wait may block.
pub const MAX_WAIT: Duration = Duration::from_secs(120);
/// Events kept between reads; older ones are dropped (and counted).
const MAX_EVENTS: usize = 2000;

pub fn parse_state(s: &str) -> Option<ElementState> {
    Some(match s.trim().to_lowercase().as_str() {
        "attached" | "exists" | "appears" => ElementState::Attached,
        "detached" | "gone" | "disappears" => ElementState::Detached,
        "visible" => ElementState::Visible,
        "hidden" => ElementState::Hidden,
        "enabled" => ElementState::Enabled,
        "disabled" => ElementState::Disabled,
        "focused" => ElementState::Focused,
        "unfocused" => ElementState::Unfocused,
        _ => return None,
    })
}

pub const STATES: &str =
    "attached, detached, visible, hidden, enabled, disabled, focused, unfocused";

fn describe_element(d: &ElementData) -> String {
    let mut s = format!("{:?}", d.role);
    if let Some(n) = d.name.as_deref().filter(|n| !n.is_empty()) {
        s.push_str(&format!(" \"{n}\""));
    }
    if let Some(v) = d.value.as_deref().filter(|v| !v.is_empty()) {
        s.push_str(&format!(" value=\"{v}\""));
    }
    s
}

fn app_or_foreground(app_name: Option<&str>) -> Result<App> {
    match app_name {
        Some(name) => LocalComputer::find_app(name),
        None => {
            App::foreground(Duration::from_secs(2)).map_err(|e| anyhow!("no foreground app: {e}"))
        }
    }
}

/// One accessibility event, flattened for transport and display.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EventRecord {
    /// Milliseconds since the watch started.
    pub t_ms: u64,
    /// e.g. `FocusChanged`, `ValueChanged`, `StateChanged { flag: Checked, value: true }`.
    pub kind: String,
    pub role: String,
    pub name: String,
    pub value: String,
    pub app: String,
}

impl std::fmt::Display for EventRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "+{}ms {}", self.t_ms, self.kind)?;
        if !self.role.is_empty() {
            write!(f, " on {}", self.role)?;
        }
        if !self.name.is_empty() {
            write!(f, " \"{}\"", self.name)?;
        }
        if !self.value.is_empty() {
            write!(f, " value=\"{}\"", self.value)?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct Buffer {
    events: VecDeque<EventRecord>,
    dropped: usize,
}

pub struct EventWatcher {
    app: String,
    stop: Arc<AtomicBool>,
    buf: Arc<Mutex<Buffer>>,
}

impl Drop for EventWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// What a read returns: the events since the last read, and how many were
/// dropped because the buffer filled up.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EventBatch {
    pub app: String,
    pub events: Vec<EventRecord>,
    pub dropped: usize,
}

impl EventBatch {
    pub fn describe(&self) -> String {
        let mut lines = vec![format!(
            "{} event(s) from {}{}",
            self.events.len(),
            self.app,
            if self.dropped > 0 {
                format!(" ({} older event(s) dropped)", self.dropped)
            } else {
                String::new()
            }
        )];
        lines.extend(self.events.iter().map(|e| format!("  {e}")));
        lines.join("\n")
    }
}

impl LocalComputer {
    /// Block until the element matching `selector` reaches `state`, or fail
    /// after `timeout` (capped at [`MAX_WAIT`]).
    pub fn ui_wait(
        &self,
        app_name: Option<&str>,
        selector: &str,
        state: &str,
        timeout: Duration,
    ) -> Result<String> {
        let st = parse_state(state)
            .ok_or_else(|| anyhow!("unknown state '{state}'; use one of: {STATES}"))?;
        let app = app_or_foreground(app_name)?;
        let timeout = timeout.min(MAX_WAIT);
        let start = Instant::now();
        let found = app
            .locator(selector)
            .wait_for_state(st, timeout)
            .map_err(|e| {
                anyhow!(
                    "'{selector}' did not become {state} within {:.1} s: {e}",
                    timeout.as_secs_f64()
                )
            })?;
        let ms = start.elapsed().as_millis();
        Ok(match found {
            Some(el) => format!("{} is {state} after {ms} ms", describe_element(&el)),
            None => format!("'{selector}' is {state} after {ms} ms"),
        })
    }

    /// Start (or restart) recording accessibility events from an app.
    pub fn events_start(&self, app_name: Option<&str>) -> Result<String> {
        self.events_stop();
        let app_name = app_name.map(str::to_string);
        let stop = Arc::new(AtomicBool::new(false));
        let buf: Arc<Mutex<Buffer>> = Arc::default();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<String>>(1);
        {
            let stop = stop.clone();
            let buf = buf.clone();
            std::thread::spawn(move || {
                // The subscription lives on this thread for its whole life.
                let sub = match app_or_foreground(app_name.as_deref())
                    .and_then(|app| Ok((app.name.clone(), app.subscribe()?)))
                {
                    Ok((name, sub)) => {
                        let _ = ready_tx.send(Ok(name));
                        sub
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(anyhow!("cannot watch events: {e}")));
                        return;
                    }
                };
                let started = Instant::now();
                while !stop.load(Ordering::SeqCst) {
                    let Some(ev) = sub.try_recv() else {
                        std::thread::sleep(Duration::from_millis(30));
                        continue;
                    };
                    let target = ev.target.as_ref();
                    let rec = EventRecord {
                        t_ms: ev.timestamp.saturating_duration_since(started).as_millis() as u64,
                        kind: format!("{:?}", ev.kind),
                        role: target.map(|t| format!("{:?}", t.role)).unwrap_or_default(),
                        name: target.and_then(|t| t.name.clone()).unwrap_or_default(),
                        value: target.and_then(|t| t.value.clone()).unwrap_or_default(),
                        app: ev.app_name.clone(),
                    };
                    if let Ok(mut b) = buf.lock() {
                        if b.events.len() >= MAX_EVENTS {
                            b.events.pop_front();
                            b.dropped += 1;
                        }
                        b.events.push_back(rec);
                    }
                }
            });
        }
        let app = ready_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| anyhow!("timed out subscribing to events"))??;
        if let Ok(mut w) = self.watcher.lock() {
            *w = Some(EventWatcher {
                app: app.clone(),
                stop,
                buf,
            });
        }
        Ok(app)
    }

    /// Events since the last read. With `wait`, blocks up to that long for
    /// the first one, so "act, then read" sees the reaction.
    pub fn events_read(&self, wait: Duration) -> Result<EventBatch> {
        let (app, buf) = {
            let w = self
                .watcher
                .lock()
                .map_err(|_| anyhow!("event watcher poisoned"))?;
            let Some(w) = w.as_ref() else {
                bail!("no event watch is running; start one first");
            };
            (w.app.clone(), w.buf.clone())
        };
        let deadline = Instant::now() + wait.min(MAX_WAIT);
        loop {
            let ready = buf.lock().map(|b| !b.events.is_empty()).unwrap_or(true);
            if ready || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let mut b = buf.lock().map_err(|_| anyhow!("event buffer poisoned"))?;
        Ok(EventBatch {
            app,
            events: b.events.drain(..).collect(),
            dropped: std::mem::take(&mut b.dropped),
        })
    }

    /// Stop recording events. Returns whether a watch was running.
    pub fn events_stop(&self) -> bool {
        self.watcher
            .lock()
            .map(|mut w| w.take().is_some())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_parse() {
        for s in STATES.split(", ") {
            assert!(parse_state(s).is_some(), "{s}");
        }
        assert!(matches!(
            parse_state("Appears"),
            Some(ElementState::Attached)
        ));
        assert!(parse_state("blue").is_none());
    }

    #[test]
    fn read_without_a_watch_is_an_error() {
        let c = LocalComputer::new();
        assert!(c.events_read(Duration::ZERO).is_err());
        assert!(!c.events_stop());
    }

    #[test]
    fn event_records_format_compactly() {
        let e = EventRecord {
            t_ms: 12,
            kind: "ValueChanged".into(),
            role: "TextField".into(),
            name: "Email".into(),
            value: "ada@example.com".into(),
            app: "chrome".into(),
        };
        assert_eq!(
            e.to_string(),
            "+12ms ValueChanged on TextField \"Email\" value=\"ada@example.com\""
        );
    }
}
