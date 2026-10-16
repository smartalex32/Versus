use super::*;
use std::fs;

fn test_path(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("versus-recent-{name}-{unique}"))
}
fn comparison(index: usize) -> RecentComparison {
    RecentComparison {
        mode: ComparisonMode::File,
        paths: [
            PathBuf::from(format!("left-{index}")),
            PathBuf::from(format!("right-{index}")),
        ],
    }
}

#[test]
fn records_newest_first_deduplicates_and_stays_bounded() {
    let mut recent = RecentComparisons::default();
    for index in 0..=MAX_RECENT_COMPARISONS {
        let item = comparison(index);
        recent.record(item.mode, item.paths);
    }
    assert_eq!(recent.entries().len(), MAX_RECENT_COMPARISONS);
    assert_eq!(recent.entries()[0], comparison(MAX_RECENT_COMPARISONS));
    let item = comparison(5);
    recent.record(item.mode, item.paths);
    assert_eq!(recent.entries()[0], comparison(5));
    assert_eq!(
        recent
            .entries()
            .iter()
            .filter(|entry| **entry == comparison(5))
            .count(),
        1
    );
}

#[test]
fn round_trips_file_folder_and_unicode_paths() {
    let path = test_path("roundtrip");
    let entries = RecentComparisons::from_entries([
        RecentComparison {
            mode: ComparisonMode::File,
            paths: [
                PathBuf::from("résumé-left.txt"),
                PathBuf::from("日本語-right.txt"),
            ],
        },
        RecentComparison {
            mode: ComparisonMode::Folder,
            paths: [PathBuf::from("left-folder"), PathBuf::from("right-folder")],
        },
    ]);
    entries.save_to(&path).expect("save history");
    assert_eq!(
        RecentComparisons::load_from(&path).expect("load history"),
        entries
    );
    fs::remove_file(path).expect("remove history");
}

#[cfg(unix)]
#[test]
fn round_trips_non_utf8_paths() {
    use std::os::unix::ffi::OsStringExt;
    let path = test_path("non-utf8");
    let raw = PathBuf::from(std::ffi::OsString::from_vec(vec![b'a', 0xFF]));
    let entries = RecentComparisons::from_entries([RecentComparison {
        mode: ComparisonMode::File,
        paths: [raw.clone(), PathBuf::from("right")],
    }]);
    entries.save_to(&path).expect("save history");
    assert_eq!(
        RecentComparisons::load_from(&path)
            .expect("load history")
            .entries()[0]
            .paths[0],
        raw
    );
    fs::remove_file(path).expect("remove history");
}

#[test]
fn rejects_corrupt_history_and_treats_missing_history_as_empty() {
    let path = test_path("corrupt");
    assert!(
        RecentComparisons::load_from(&path)
            .expect("missing is empty")
            .entries()
            .is_empty()
    );
    fs::write(&path, b"not a history").expect("write corrupt history");
    let error = RecentComparisons::load_from(&path).expect_err("corrupt history fails safely");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    fs::remove_file(path).expect("remove history");
}

#[test]
fn refuses_to_save_paths_the_loader_would_reject() {
    let path = test_path("oversized");
    let oversized = PathBuf::from("x".repeat(MAX_PATH_BYTES + 1));
    let entries = RecentComparisons::from_entries([RecentComparison {
        mode: ComparisonMode::File,
        paths: [oversized, PathBuf::from("right")],
    }]);
    let error = entries
        .save_to(&path)
        .expect_err("oversized path is rejected");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(!path.exists());

    let mut entries = RecentComparisons::default();
    for index in 0..MAX_RECENT_COMPARISONS {
        let long_path = PathBuf::from(format!("{index}{}", "x".repeat(10_000)));
        entries.record(ComparisonMode::File, [long_path.clone(), long_path]);
    }
    assert_eq!(
        entries.save_to(&path).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(!path.exists());
}
