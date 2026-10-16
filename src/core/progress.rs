use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressStage {
    Scanning,
    Reading,
    ComparingFiles,
    ComparingLines,
    Highlighting,
    BuildingTree,
    Finished,
}

#[derive(Clone, Copy, Debug)]
pub struct ProgressSnapshot {
    pub stage: ProgressStage,
    pub completed: u64,
    pub total: Option<u64>,
}

impl ProgressSnapshot {
    /// Completion for a measured stage. Unknown and empty totals have no ratio.
    pub fn fraction(&self) -> Option<f32> {
        self.total
            .filter(|total| *total > 0)
            .map(|total| (self.completed.min(total) as f64 / total as f64) as f32)
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
        }))
    }
}

impl ComparisonProgress {
    pub fn begin(&self, stage: ProgressStage, total: Option<u64>) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = ProgressSnapshot {
            stage,
            completed: 0,
            total,
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
#[path = "../../tests/unit/core/progress/mod.rs"]
mod tests;
