use super::*;

#[test]
fn measured_progress_reports_completion_and_resets_between_stages() {
    let progress = ComparisonProgress::default();
    progress.advance(50);
    assert_eq!(progress.snapshot().fraction(), None);
    progress.begin(ProgressStage::Reading, Some(200));
    assert_eq!(progress.snapshot().fraction(), Some(0.0));
    progress.advance(50);
    assert_eq!(progress.snapshot().fraction(), Some(0.25));
    progress.advance(u64::MAX);
    assert_eq!(progress.snapshot().completed, 200);
    assert_eq!(progress.snapshot().fraction(), Some(1.0));
    progress.begin(ProgressStage::ComparingLines, None);
    assert_eq!(progress.snapshot().completed, 0);
    assert_eq!(progress.snapshot().fraction(), None);
    progress.begin(ProgressStage::Finished, Some(0));
    assert_eq!(progress.snapshot().fraction(), None);
}
