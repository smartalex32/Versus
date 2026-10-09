use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressStage {
    Scanning,
    Reading,
    ComparingFiles,
    ComparingLines,
    Highlighting,
    Finished,
}

#[derive(Clone, Copy, Debug)]
pub struct ProgressSnapshot {
    pub stage: ProgressStage,
    pub completed: u64,
    pub total: Option<u64>,
    pub started: Instant,
}

impl ProgressSnapshot {
    /// An estimate for this measured stage, not a promise about later work.
    pub fn remaining_at(&self, now: Instant) -> Option<Duration> {
        let total = self.total?;
        let elapsed = now.saturating_duration_since(self.started);
        if self.completed == 0
            || self.completed >= total
            || elapsed < Duration::from_millis(250)
            || self.stage == ProgressStage::Finished
        {
            return None;
        }
        let seconds =
            elapsed.as_secs_f64() * (total - self.completed) as f64 / self.completed as f64;
        Duration::try_from_secs_f64(seconds).ok()
    }
}

/// A small shared snapshot. Filesystem workers publish progress without touching UI state.
#[derive(Debug)]
pub struct ComparisonProgress(Mutex<ProgressSnapshot>);

impl Default for ComparisonProgress {
    fn default() -> Self {
        Self(Mutex::new(ProgressSnapshot {
            stage: ProgressStage::Scanning,
            completed: 0,
            total: None,
            started: Instant::now(),
        }))
    }
}

impl ComparisonProgress {
    pub fn begin(&self, stage: ProgressStage, total: Option<u64>) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = ProgressSnapshot {
            stage,
            completed: 0,
            total,
            started: Instant::now(),
        };
    }

    pub fn advance(&self, amount: u64) {
        let mut snapshot = self.0.lock().unwrap_or_else(|error| error.into_inner());
        snapshot.completed = snapshot.completed.saturating_add(amount);
        if let Some(total) = snapshot.total {
            snapshot.completed = snapshot.completed.min(total);
        }
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        *self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_only_measured_work_and_resets_between_stages() {
        let progress = ComparisonProgress::default();
        progress.advance(50);
        let scan = progress.snapshot();
        assert!(
            scan.remaining_at(scan.started + Duration::from_secs(2))
                .is_none()
        );
        progress.begin(ProgressStage::Reading, Some(200));
        progress.advance(50);
        let reading = progress.snapshot();
        assert_eq!(reading.completed, 50);
        assert_eq!(
            reading.remaining_at(reading.started + Duration::from_secs(2)),
            Some(Duration::from_secs(6))
        );
        assert!(
            reading
                .remaining_at(reading.started + Duration::from_millis(10))
                .is_none()
        );
        progress.advance(u64::MAX);
        let done = progress.snapshot();
        assert_eq!(done.completed, 200);
        assert!(
            done.remaining_at(done.started + Duration::from_secs(2))
                .is_none()
        );
        progress.begin(ProgressStage::ComparingLines, None);
        assert_eq!(progress.snapshot().completed, 0);
    }
}
