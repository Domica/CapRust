//! Real-time ETA. Ported from jub0t/Concat#64 + #175.

use std::time::Instant;

pub struct ExportProgressTracker {
    started_at: Instant,
}

impl ExportProgressTracker {
    pub fn start() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }

    pub fn eta_string(&self, progress: f32) -> Option<String> {
        if progress <= 0.02 {
            return None;
        }
        let elapsed = self.started_at.elapsed().as_secs_f32();
        let remaining = elapsed / progress * (1.0 - progress);
        Some(format_eta(remaining))
    }

    pub fn elapsed_secs(&self) -> f32 {
        self.started_at.elapsed().as_secs_f32()
    }
}

fn format_eta(seconds: f32) -> String {
    let total = seconds.round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn eta_at_50_percent() {
        let tracker = ExportProgressTracker {
            started_at: Instant::now() - Duration::from_secs(60),
        };
        let eta = tracker.eta_string(0.5).unwrap();
        assert!(eta == "1:00" || eta == "0:59" || eta == "1:01", "got {eta}");
    }

    #[test]
    fn no_eta_below_2_percent() {
        let tracker = ExportProgressTracker::start();
        assert!(tracker.eta_string(0.01).is_none());
    }
}
