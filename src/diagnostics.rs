//! Small, dependency-free diagnostics for the desktop application.
//!
//! Logging is deliberately best-effort: UI callers enqueue a short event with
//! `try_send`, so a slow or unavailable filesystem can never block interaction.
//! The caller owns [`LogGuard`], which requests a bounded flush at shutdown.

use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    panic::PanicHookInfo,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const LOG_FILE: &str = "versus.log";
const PREVIOUS_LOG_FILE: &str = "versus.log.1";
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const CHANNEL_CAPACITY: usize = 256;
const SHUTDOWN_WAIT: Duration = Duration::from_millis(500);
const EVENT_LIMIT: usize = 2_048;

static LOGGER: OnceLock<Arc<Logger>> = OnceLock::new();
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Keeps the background writer alive for the desktop application's lifetime.
/// Dropping the guard requests a flush, but never waits indefinitely for storage.
pub(super) struct LogGuard {
    logger: Arc<Logger>,
    active: bool,
}

struct Logger {
    sender: SyncSender<LogEntry>,
    stopping: Arc<AtomicBool>,
    last_error: Arc<Mutex<Option<String>>>,
    completion: Mutex<Option<mpsc::Receiver<()>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug)]
struct LogEntry {
    level: &'static str,
    event: String,
}

/// Starts logging beside the executable. On Linux AppImages, the outer
/// executable is preferred when `APPIMAGE` identifies an existing file.
///
/// This intentionally does not fall back to a home or temporary directory: a
/// caller can surface the returned error in the application's status area.
pub(super) fn initialize() -> Result<LogGuard, String> {
    if let Some(logger) = LOGGER.get() {
        return Ok(LogGuard {
            logger: Arc::clone(logger),
            active: false,
        });
    }

    let directory = executable_directory()?;
    let logger = Arc::new(Logger::start_in(&directory, CHANNEL_CAPACITY)?);
    // If another startup path won the race, use that logger and discard this
    // newly-created writer cleanly.
    match LOGGER.set(Arc::clone(&logger)) {
        Ok(()) => {
            install_panic_hook();
            Ok(LogGuard {
                logger,
                active: true,
            })
        }
        Err(unused) => {
            request_stop(&unused);
            Ok(LogGuard {
                logger: Arc::clone(LOGGER.get().expect("logger set by competing initializer")),
                active: false,
            })
        }
    }
}

/// Queues an informational, metadata-only diagnostic event.
pub(super) fn info(event: impl AsRef<str>) {
    record("INFO", event.as_ref());
}

/// Queues an error diagnostic event.
pub(super) fn error(event: impl AsRef<str>) {
    record("ERROR", event.as_ref());
}

/// Returns a writer failure once, without waiting for the writer thread.
/// Callers can surface this in their existing status area.
pub(super) fn take_error() -> Option<String> {
    LOGGER.get().and_then(|logger| {
        logger
            .last_error
            .try_lock()
            .ok()
            .and_then(|mut error| error.take())
    })
}

/// Runs a worker operation without allowing a panic to tear down the UI.
/// The operation name and error are logged; callers can display the returned
/// text in their existing status area.
pub(super) fn catch_worker<T, E>(
    operation: &str,
    work: impl FnOnce() -> Result<T, E>,
) -> Result<T, String>
where
    E: std::fmt::Display,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error_value)) => {
            let message = format!("{operation}: {error_value}");
            error(&message);
            Err(message)
        }
        Err(payload) => {
            let message = format!("{operation}: worker panicked ({})", panic_message(payload));
            error(&message);
            Err(message)
        }
    }
}

fn record(level: &'static str, event: &str) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    if logger.stopping.load(Ordering::Relaxed) {
        return;
    }
    let entry = LogEntry {
        level,
        event: bounded(event),
    };
    match logger.sender.try_send(entry) {
        Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
    }
}

fn executable_directory() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        let candidate = PathBuf::from(appimage);
        if candidate.is_file() {
            return parent_directory(&candidate);
        }
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not determine executable path for diagnostics: {error}"))?;
    parent_directory(&executable)
}

fn parent_directory(path: &Path) -> Result<PathBuf, String> {
    path.parent().map(Path::to_path_buf).ok_or_else(|| {
        format!(
            "could not determine diagnostics directory beside executable {}",
            path.display()
        )
    })
}

impl Logger {
    fn start_in(directory: &Path, capacity: usize) -> Result<Self, String> {
        let path = directory.join(LOG_FILE);
        rotate_if_needed(&path)?;
        let file = open_log(&path)?;
        let (sender, receiver) = mpsc::sync_channel::<LogEntry>(capacity);
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = Arc::clone(&stopping);
        let last_error = Arc::new(Mutex::new(None));
        let worker_error = Arc::clone(&last_error);
        let (completion_sender, completion) = mpsc::channel();
        let session = session_metadata();
        let worker = thread::Builder::new()
            .name("versus-diagnostics".into())
            .spawn(move || {
                let mut writer = Some(BufWriter::new(file));
                if let Err(error) = write_and_flush(
                    writer.as_mut().expect("writer is present"),
                    &session,
                    "INFO",
                    "diagnostics started",
                ) {
                    record_writer_error(&worker_error, error);
                    let _ = completion_sender.send(());
                    return;
                }
                'writer: loop {
                    match receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok(entry) => {
                            if let Err(error) = write_and_flush(
                                writer.as_mut().expect("writer is present"),
                                &session,
                                entry.level,
                                &entry.event,
                            ) {
                                record_writer_error(&worker_error, error);
                                break 'writer;
                            }
                            let current_writer = writer.take().expect("writer is present");
                            match rotate_running_log(&path, current_writer, &session) {
                                Ok(next_writer) => writer = Some(next_writer),
                                Err(error) => {
                                    record_writer_error(&worker_error, error);
                                    break 'writer;
                                }
                            }
                        }
                        Err(RecvTimeoutError::Timeout)
                            if worker_stopping.load(Ordering::Relaxed) =>
                        {
                            break;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                if let Some(mut writer) = writer {
                    let _ = writer.flush();
                }
                let _ = completion_sender.send(());
            })
            .map_err(|error| format!("could not start diagnostics writer: {error}"))?;
        Ok(Self {
            sender,
            stopping,
            last_error,
            completion: Mutex::new(Some(completion)),
            worker: Mutex::new(Some(worker)),
        })
    }
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        if self.active {
            request_stop(&self.logger);
        }
    }
}

fn request_stop(logger: &Logger) {
    if logger.stopping.swap(true, Ordering::Relaxed) {
        return;
    }
    let completed = logger
        .completion
        .lock()
        .ok()
        .and_then(|mut receiver| receiver.take())
        .is_some_and(|receiver| receiver.recv_timeout(SHUTDOWN_WAIT).is_ok());
    // Joining only after the completion signal avoids a potentially unbounded
    // wait if the operating system stalls while flushing the filesystem.
    if completed {
        if let Ok(mut worker) = logger.worker.lock() {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
        }
    }
}

fn rotate_if_needed(path: &Path) -> Result<(), String> {
    if !log_needs_rotation(path)? {
        return Ok(());
    }
    let previous = path.with_file_name(PREVIOUS_LOG_FILE);
    if previous.exists() {
        fs::remove_file(&previous).map_err(|error| {
            format!(
                "could not rotate diagnostics log {}: {error}",
                previous.display()
            )
        })?;
    }
    fs::rename(path, &previous).map_err(|error| {
        format!(
            "could not rotate diagnostics log {} to {}: {error}",
            path.display(),
            previous.display()
        )
    })
}

fn log_needs_rotation(path: &Path) -> Result<bool, String> {
    match path.metadata() {
        Ok(metadata) => Ok(metadata.len() >= MAX_LOG_BYTES),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "could not inspect diagnostics log {}: {error}",
            path.display()
        )),
    }
}

fn open_log(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("could not open diagnostics log {}: {error}", path.display()))
}

fn write_and_flush(
    writer: &mut BufWriter<File>,
    session: &str,
    level: &str,
    event: &str,
) -> Result<(), String> {
    write_entry(writer, session, level, event)
        .map_err(|error| format!("could not write diagnostics log: {error}"))?;
    // Persist useful crash evidence promptly, but always from the writer thread
    // rather than the UI thread.
    writer
        .flush()
        .map_err(|error| format!("could not flush diagnostics log: {error}"))
}

fn rotate_running_log(
    path: &Path,
    writer: BufWriter<File>,
    session: &str,
) -> Result<BufWriter<File>, String> {
    if !log_needs_rotation(path)? {
        return Ok(writer);
    }
    // Closing the old handle before rename permits rotation on Windows too.
    drop(writer);
    rotate_if_needed(path)?;
    let mut next = BufWriter::new(open_log(path)?);
    write_and_flush(&mut next, session, "INFO", "diagnostics rotated")?;
    Ok(next)
}

fn record_writer_error(last_error: &Mutex<Option<String>>, error: String) {
    if let Ok(mut stored) = last_error.lock() {
        if stored.is_none() {
            *stored = Some(error);
        }
    }
}

fn write_entry(
    writer: &mut BufWriter<File>,
    session: &str,
    level: &str,
    event: &str,
) -> std::io::Result<()> {
    writeln!(
        writer,
        "{} {} [{}] {}",
        unix_millis(),
        session,
        level,
        event
    )
}

fn session_metadata() -> String {
    format!(
        "session={} version={} pid={} platform={}-{}",
        SESSION_COUNTER.fetch_add(1, Ordering::Relaxed) + 1,
        option_env!("CARGO_PKG_VERSION").unwrap_or("unknown"),
        std::process::id(),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn bounded(event: &str) -> String {
    let mut end = event.len().min(EVENT_LIMIT);
    while end > 0 && !event.is_char_boundary(end) {
        end -= 1;
    }
    let mut result = event[..end].replace(['\n', '\r'], " ");
    if end < event.len() {
        result.push_str("…");
    }
    result
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        bounded(message)
    } else if let Some(message) = payload.downcast_ref::<String>() {
        bounded(message)
    } else {
        "non-string panic payload".to_owned()
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown location".to_owned());
        error(format!("panic at {location}: {}", panic_payload_ref(info)));
        previous(info);
    }));
}

fn panic_payload_ref(info: &PanicHookInfo<'_>) -> String {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        bounded(message)
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        bounded(message)
    } else {
        "non-string panic payload".to_owned()
    }
}

#[cfg(test)]
#[path = "../tests/unit/diagnostics/mod.rs"]
mod tests;
