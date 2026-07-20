use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult};

use crate::paths;

/// Watches the config file (via its parent dir, so atomic replaces are
/// seen) and the themes dir; sends `()` on the channel ~150ms debounced.
pub struct ConfigWatcher {
    pub rx: crossbeam_channel::Receiver<()>,
    _debouncer: notify_debouncer_full::Debouncer<
        notify::RecommendedWatcher,
        notify_debouncer_full::RecommendedCache,
    >,
}

pub fn watch() -> Option<ConfigWatcher> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let cfg_dir = paths::cfg_dir();
    let themes_dir = paths::themes_dir();
    let _ = std::fs::create_dir_all(&themes_dir);

    let mut debouncer = new_debouncer(
        Duration::from_millis(150),
        None,
        move |result: DebounceEventResult| {
            if let Ok(events) = result {
                // Only care about changes to the config file or themes.
                let relevant = events.iter().any(|e| {
                    e.paths.iter().any(|p| {
                        p.file_name()
                            .map(|n| n == "hyper-revamp.json")
                            .unwrap_or(false)
                            || p.parent()
                                .map(|d| d.ends_with("themes"))
                                .unwrap_or(false)
                    })
                });
                if relevant {
                    let _ = tx.send(());
                }
            }
        },
    )
    .map_err(|err| log::warn!("config watcher failed: {err}"))
    .ok()?;

    debouncer
        .watch(&cfg_dir, RecursiveMode::NonRecursive)
        .map_err(|err| log::warn!("watch {}: {err}", cfg_dir.display()))
        .ok()?;
    let _ = debouncer.watch(&themes_dir, RecursiveMode::NonRecursive);

    Some(ConfigWatcher {
        rx,
        _debouncer: debouncer,
    })
}
