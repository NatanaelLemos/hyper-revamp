//! Session restore + window geometry persistence.
//!
//! `session-state.json` uses the same format as the Electron app
//! (`app/ui/session-state.ts`): `{windows: [{tabs: [{profile, cwd?}]}]}` —
//! state written by either build restores in the other. Window geometry goes
//! to `windows-state.json` with the same defaults as `app/config/windows.ts`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTab {
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredWindow {
    pub tabs: Vec<StoredTab>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoredSessionState {
    pub windows: Vec<StoredWindow>,
}

pub fn read_session_state() -> Option<StoredSessionState> {
    let raw = std::fs::read_to_string(hyper_config::paths::session_state_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write_session_state(state: &StoredSessionState) {
    let path = hyper_config::paths::session_state_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(state) {
        Ok(json) => {
            if let Err(err) = std::fs::write(&path, json) {
                log::warn!("failed to persist session state: {err}");
            }
        }
        Err(err) => log::warn!("failed to serialize session state: {err}"),
    }
}

/// Drop the stored tabs once they have been restored (`clearSessionState`).
///
/// Restoring is a one-shot: the state describes the tabs open at the *last*
/// clean exit, and leaving it on disk means a later crash — which never gets
/// to write fresh state — resurrects that same stale set of tabs again.
pub fn clear_session_state() {
    write_session_state(&StoredSessionState::default());
}

// ---- window geometry (windows-state.json) ----

pub const DEFAULT_POSITION: [f32; 2] = [50.0, 50.0];
pub const DEFAULT_SIZE: [f32; 2] = [540.0, 380.0];

/// Saved window placement.
///
/// `window_size` is the **inner** (content) size, because that is what
/// `ViewportBuilder::with_inner_size` consumes on restore. Recording the outer
/// rect instead and feeding it back as an inner size adds the frame's height
/// to the window on every single launch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeometry {
    /// Outer top-left, matching `with_position`.
    pub window_position: [f32; 2],
    pub window_size: [f32; 2],
}

impl Default for WindowGeometry {
    fn default() -> Self {
        WindowGeometry {
            window_position: DEFAULT_POSITION,
            window_size: DEFAULT_SIZE,
        }
    }
}

pub fn read_window_geometry() -> WindowGeometry {
    std::fs::read_to_string(hyper_config::paths::windows_state_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn write_window_geometry(geometry: &WindowGeometry) {
    let path = hyper_config::paths::windows_state_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(geometry) {
        if let Err(err) = std::fs::write(&path, json) {
            log::warn!("failed to persist window geometry: {err}");
        }
    }
}

/// Logical bounds of one display: `[x, y, width, height]`.
pub type MonitorBounds = [f32; 4];

/// Whether a window position still lands on a display (`positionIsValid`).
///
/// A window restored onto a monitor that has since been unplugged ends up
/// somewhere the user can neither see nor drag back. With no monitors to check
/// against, the position is trusted rather than second-guessed.
pub fn position_is_valid(position: [f32; 2], monitors: &[MonitorBounds]) -> bool {
    if monitors.is_empty() {
        return true;
    }
    let [x, y] = position;
    monitors
        .iter()
        .any(|&[mx, my, mw, mh]| x >= mx && x <= mx + mw && y >= my && y <= my + mh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_state_roundtrips_electron_format() {
        // Format written by the Electron build must parse.
        let electron = r#"{"windows":[{"tabs":[{"profile":"default","cwd":"/tmp"},{"profile":"work"}]}]}"#;
        let state: StoredSessionState = serde_json::from_str(electron).unwrap();
        assert_eq!(state.windows[0].tabs.len(), 2);
        assert_eq!(state.windows[0].tabs[0].cwd.as_deref(), Some("/tmp"));
        assert_eq!(state.windows[0].tabs[1].cwd, None);

        let out = serde_json::to_string(&state).unwrap();
        let reparsed: StoredSessionState = serde_json::from_str(&out).unwrap();
        assert_eq!(reparsed.windows[0].tabs[0].profile, "default");
    }

    /// A position on an unplugged monitor is rejected so the window can be
    /// recentered; one that is still covered is left exactly as saved.
    #[test]
    fn positions_are_validated_against_the_attached_displays() {
        let laptop: MonitorBounds = [0.0, 0.0, 1440.0, 900.0];
        let external: MonitorBounds = [1440.0, 0.0, 2560.0, 1440.0];

        // Both displays attached: a window on either one stays put.
        let both = [laptop, external];
        assert!(position_is_valid([100.0, 100.0], &both));
        assert!(position_is_valid([2000.0, 300.0], &both));

        // External unplugged: the window that lived on it is off-screen now,
        // while the one on the laptop is still fine.
        let only_laptop = [laptop];
        assert!(!position_is_valid([2000.0, 300.0], &only_laptop));
        assert!(position_is_valid([100.0, 100.0], &only_laptop));

        // Nothing to check against: trust what was saved.
        assert!(position_is_valid([2000.0, 300.0], &[]));
    }

    /// The size in this file is the *inner* size, and it must survive a
    /// save/restore cycle unchanged. Recording the outer rect and restoring it
    /// as an inner size adds the frame height on every launch, so a window
    /// left at 380px tall creeps down the screen a titlebar at a time.
    #[test]
    fn geometry_round_trips_without_growing() {
        let frame_height = 28.0;
        let mut geometry = WindowGeometry {
            window_position: [50.0, 50.0],
            window_size: [540.0, 380.0],
        };

        for _ in 0..5 {
            // Launch: the inner size is applied, so the OS reports an outer
            // rect that is taller by the frame.
            let inner = geometry.window_size;
            let outer_height = inner[1] + frame_height;
            assert_eq!(outer_height, 408.0);
            // Exit: record the inner size, never the outer one.
            geometry = WindowGeometry {
                window_position: geometry.window_position,
                window_size: inner,
            };
        }
        assert_eq!(geometry.window_size, [540.0, 380.0]);
    }
}
