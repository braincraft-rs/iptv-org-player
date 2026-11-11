//! Scheduled (one-off) recordings and the rule deciding what to do with them.

use super::Preset;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Running,
    Done,
    /// The window passed while the app was not running.
    Missed,
    Failed,
    Cancelled,
}

impl Status {
    pub fn key(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Running => "running",
            Status::Done => "done",
            Status::Missed => "missed",
            Status::Failed => "failed",
            Status::Cancelled => "cancelled",
        }
    }

    pub fn from_key(key: &str) -> Status {
        match key {
            "running" => Status::Running,
            "done" => Status::Done,
            "missed" => Status::Missed,
            "failed" => Status::Failed,
            "cancelled" => Status::Cancelled,
            _ => Status::Pending,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Pending => "Scheduled",
            Status::Running => "Recording",
            Status::Done => "Done",
            Status::Missed => "Missed (app was closed)",
            Status::Failed => "Failed",
            Status::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Scheduled {
    pub id: i64,
    pub channel_id: String,
    pub channel_name: String,
    /// Start time, unix seconds (UTC).
    pub start_ts: i64,
    pub duration_secs: i64,
    pub preset: Preset,
    pub status: Status,
    pub note: String,
}

impl Scheduled {
    pub fn end_ts(&self) -> i64 {
        self.start_ts + self.duration_secs
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Due {
    /// Not yet, or not pending anymore.
    Nothing,
    /// Start now and record for this many more seconds (shorter than the
    /// planned duration if the app started late inside the window).
    Start { remaining_secs: i64 },
    /// The whole window is over.
    Missed,
}

pub fn due(s: &Scheduled, now_ts: i64) -> Due {
    if s.status != Status::Pending {
        return Due::Nothing;
    }
    if now_ts >= s.end_ts() {
        Due::Missed
    } else if now_ts >= s.start_ts {
        Due::Start {
            remaining_secs: s.end_ts() - now_ts,
        }
    } else {
        Due::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sched(start: i64, dur: i64, status: Status) -> Scheduled {
        Scheduled {
            id: 1,
            channel_id: "c".into(),
            channel_name: "C".into(),
            start_ts: start,
            duration_secs: dur,
            preset: Preset::Small,
            status,
            note: String::new(),
        }
    }

    #[test]
    fn waits_until_start_then_starts_then_misses() {
        let s = sched(1000, 600, Status::Pending);
        assert_eq!(due(&s, 999), Due::Nothing);
        assert_eq!(
            due(&s, 1000),
            Due::Start {
                remaining_secs: 600
            }
        );
        assert_eq!(
            due(&s, 1300),
            Due::Start {
                remaining_secs: 300
            }
        );
        assert_eq!(due(&s, 1600), Due::Missed);
        assert_eq!(due(&s, 9999), Due::Missed);
    }

    #[test]
    fn only_pending_schedules_are_due() {
        for st in [
            Status::Running,
            Status::Done,
            Status::Cancelled,
            Status::Failed,
        ] {
            assert_eq!(due(&sched(0, 10, st), 5), Due::Nothing);
        }
    }

    #[test]
    fn status_keys_round_trip() {
        for st in [
            Status::Pending,
            Status::Running,
            Status::Done,
            Status::Missed,
            Status::Failed,
            Status::Cancelled,
        ] {
            assert_eq!(Status::from_key(st.key()), st);
        }
    }
}
