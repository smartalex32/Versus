use super::*;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "versus-diagnostics-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create temporary directory");
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn start_for_test(directory: &Path, capacity: usize) -> LogGuard {
    LogGuard {
        logger: Arc::new(Logger::start_in(directory, capacity).expect("start logger")),
        active: true,
    }
}

fn wait_for(path: &Path, needle: &str) -> String {
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        let contents = fs::read_to_string(path).unwrap_or_default();
        if contents.contains(needle) || Instant::now() >= until {
            return contents;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn writes_session_metadata_and_events_to_injected_directory() {
    let directory = TempDirectory::new();
    let guard = start_for_test(&directory.0, 8);
    guard
        .logger
        .sender
        .try_send(LogEntry {
            level: "ERROR",
            event: "comparison worker failed: sample error".into(),
        })
        .expect("queue event");
    let contents = wait_for(&directory.0.join("versus.log"), "sample error");
    assert!(contents.contains("session="));
    assert!(contents.contains("version="));
    assert!(contents.contains("pid="));
    assert!(contents.contains("platform="));
    assert!(contents.contains("[ERROR] comparison worker failed: sample error"));
    drop(guard);
}

#[test]
fn rotates_large_existing_log() {
    let directory = TempDirectory::new();
    let current = directory.0.join(LOG_FILE);
    File::create(&current)
        .and_then(|file| file.set_len(MAX_LOG_BYTES))
        .expect("create large log");
    let guard = start_for_test(&directory.0, 8);
    let previous = directory.0.join(PREVIOUS_LOG_FILE);
    assert_eq!(
        fs::metadata(previous).expect("rotated log").len(),
        MAX_LOG_BYTES
    );
    drop(guard);
    assert!(
        fs::read_to_string(current)
            .expect("new log")
            .contains("session=")
    );
}

#[test]
fn rotates_after_a_long_running_session_reaches_the_limit() {
    let directory = TempDirectory::new();
    let guard = start_for_test(&directory.0, 8);
    let current = directory.0.join(LOG_FILE);
    File::options()
        .write(true)
        .open(&current)
        .and_then(|file| file.set_len(MAX_LOG_BYTES))
        .expect("grow current log");
    guard
        .logger
        .sender
        .try_send(LogEntry {
            level: "INFO",
            event: "trigger runtime rotation".into(),
        })
        .expect("queue event");
    let rotated = wait_for(&current, "diagnostics rotated");
    assert!(rotated.contains("session="));
    assert!(directory.0.join(PREVIOUS_LOG_FILE).is_file());
    drop(guard);
}

#[test]
fn reports_unwritable_log_target() {
    let directory = TempDirectory::new();
    let path = directory.0.join(LOG_FILE);
    fs::create_dir(&path).expect("create conflicting directory");
    let error = match Logger::start_in(&directory.0, 8) {
        Ok(_) => panic!("must reject directory target"),
        Err(error) => error,
    };
    assert!(error.contains("could not open diagnostics log"));
    assert!(error.contains("versus.log"));
}

#[test]
fn saturated_queue_never_blocks_caller() {
    let directory = TempDirectory::new();
    let guard = start_for_test(&directory.0, 0);
    let started = Instant::now();
    for _ in 0..50_000 {
        let _ = guard.logger.sender.try_send(LogEntry {
            level: "INFO",
            event: "rapid UI event".into(),
        });
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    drop(guard);
}

#[test]
fn worker_errors_and_panics_are_returned_as_messages() {
    let error = catch_worker("sample work", || -> Result<(), &str> {
        Err("disk unavailable")
    })
    .expect_err("error returned");
    assert_eq!(error, "sample work: disk unavailable");
    let panic = catch_worker("sample work", || -> Result<(), &str> { panic!("boom") })
        .expect_err("panic returned");
    assert_eq!(panic, "sample work: worker panicked (boom)");
}
