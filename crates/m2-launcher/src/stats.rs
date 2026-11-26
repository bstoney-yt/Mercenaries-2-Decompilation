//! What the launcher itself keeps of the play across every save: the
//! total time the game ran, how many times it was started, and when it was
//! last played. Kept in `m2-launcher-stats.toml` beside the launcher's
//! settings. None of this is in a save or the 360's record: it is the
//! launcher's own (**invented**), and the Stats page says so.
//!
//! A session is the game's run from the launcher starting it to its
//! exit; it is written when the game exits. The launcher stays (hidden,
//! when it is not kept open) until then. A game the launcher did not start,
//! or one still running when the launcher is killed, is not counted.

use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

pub const STATS_FILE: &str = "m2-launcher-stats.toml";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherStats {
    /// Seconds the game ran, every session together.
    pub total_seconds: u64,
    /// Sessions counted.
    pub sessions: u32,
    /// When the last one ended, seconds since 1970 (UTC).
    pub last_played: Option<u64>,
    /// The longest one, seconds.
    pub longest_seconds: u64,
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl LauncherStats {
    pub fn read(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(
            path,
            toml::to_string_pretty(self).expect("stats always serialise"),
        )
    }

    /// Counts one session from `start` to `end`.
    pub fn add_session(&mut self, start: SystemTime, end: SystemTime) {
        let secs = end.duration_since(start).map_or(0, |d| d.as_secs());
        self.total_seconds = self.total_seconds.saturating_add(secs);
        self.sessions = self.sessions.saturating_add(1);
        self.longest_seconds = self.longest_seconds.max(secs);
        self.last_played = Some(unix(end));
    }

    /// When the last session ended.
    pub fn last_played_at(&self) -> Option<SystemTime> {
        self.last_played
            .map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s))
    }
}

/// Reads `path`, counts the session and writes it back.
pub fn record_session(path: &Path, start: SystemTime, end: SystemTime) -> std::io::Result<()> {
    let mut s = LauncherStats::read(path);
    s.add_session(start, end);
    s.write(path)
}

/// A long time as the page writes it: `H:MM:SS` (the save list's form).
pub fn duration_text(secs: u64) -> String {
    format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_add_up_and_round_trip_through_the_file() {
        let d = std::env::temp_dir().join(format!("m2-launcher-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join(STATS_FILE);
        // No file yet reads as nothing played.
        assert_eq!(LauncherStats::read(&path), LauncherStats::default());
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        record_session(&path, t, t + Duration::from_secs(3725)).unwrap();
        record_session(
            &path,
            t + Duration::from_secs(7200),
            t + Duration::from_secs(7260),
        )
        .unwrap();
        let s = LauncherStats::read(&path);
        assert_eq!(
            s,
            LauncherStats {
                total_seconds: 3785,
                sessions: 2,
                last_played: Some(1_800_007_260),
                longest_seconds: 3725,
            }
        );
        assert_eq!(s.last_played_at(), Some(t + Duration::from_secs(7260)));
        // An end before its start (the clock set back) counts no time.
        let mut s2 = s.clone();
        s2.add_session(t + Duration::from_secs(10), t);
        assert_eq!((s2.total_seconds, s2.sessions), (3785, 3));
        // A file missing fields, or broken, still reads.
        std::fs::write(&path, "sessions = 4\n").unwrap();
        assert_eq!(LauncherStats::read(&path).sessions, 4);
        std::fs::write(&path, "not [toml").unwrap();
        assert_eq!(LauncherStats::read(&path), LauncherStats::default());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn durations_read_as_hours_minutes_seconds() {
        assert_eq!(duration_text(0), "0:00:00");
        assert_eq!(duration_text(3725), "1:02:05");
        assert_eq!(duration_text(100 * 3600 + 59), "100:00:59");
    }
}
