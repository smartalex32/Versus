//! Small, versioned on-disk history of compared source pairs.
//!
//! This deliberately uses a compact binary format instead of a text format so
//! that Unix paths which are not valid UTF-8 can be restored exactly.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::selection::ComparisonMode;

/// The maximum number of comparisons retained for reopening.
pub const MAX_RECENT_COMPARISONS: usize = 20;

const MAGIC: &[u8; 4] = b"VREC";
const VERSION: u8 = 1;
const MAX_FILE_BYTES: u64 = 128 * 1024;
const MAX_PATH_BYTES: usize = 64 * 1024;

/// One file or folder source pair that has been compared.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentComparison {
    pub mode: ComparisonMode,
    pub paths: [PathBuf; 2],
}

/// Comparisons ordered from most recently used to least recently used.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecentComparisons {
    entries: Vec<RecentComparison>,
}

impl RecentComparisons {
    /// Builds a bounded, de-duplicated list. `entries` must be newest first.
    pub fn from_entries(entries: impl IntoIterator<Item = RecentComparison>) -> Self {
        let mut recent = Self::default();
        for entry in entries {
            if recent.entries.len() == MAX_RECENT_COMPARISONS {
                break;
            }
            if !recent.entries.contains(&entry) {
                recent.entries.push(entry);
            }
        }
        recent
    }

    pub fn entries(&self) -> &[RecentComparison] {
        &self.entries
    }

    /// Records a comparison, moving an existing identical comparison to front.
    pub fn record(&mut self, mode: ComparisonMode, paths: [PathBuf; 2]) {
        let entry = RecentComparison { mode, paths };
        self.entries.retain(|existing| existing != &entry);
        self.entries.insert(0, entry);
        self.entries.truncate(MAX_RECENT_COMPARISONS);
    }

    /// Loads the per-user comparison history. A missing history is empty.
    pub fn load() -> io::Result<Self> {
        match config_path() {
            Some(path) => Self::load_from(path),
            None => Ok(Self::default()),
        }
    }

    /// Loads history from an explicit path, primarily for tests and embedding.
    pub fn load_from(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        match fs::metadata(path) {
            Ok(metadata) if metadata.len() > MAX_FILE_BYTES => {
                return invalid("recent history is too large");
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        }

        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return invalid("recent history is too large");
        }
        decode(&bytes)
    }

    /// Saves the history at the normal per-user location.
    pub fn save(&self) -> io::Result<()> {
        match config_path() {
            Some(path) => self.save_to(path),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no per-user configuration directory is available",
            )),
        }
    }

    /// Saves history to an explicit path using a temporary file and replacement.
    pub fn save_to(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        if let Some(parent) = parent {
            fs::create_dir_all(parent)?;
        }

        let bytes = encode(&self.entries)?;
        let temp_path = temporary_path(path)?;
        let result = (|| -> io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            replace_file(&temp_path, path)?;
            if let Some(parent) = parent {
                // Best-effort durability of the directory entry on platforms that support it.
                let _ = File::open(parent).and_then(|directory| directory.sync_all());
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        result
    }
}

/// The standard per-user location for the history file, when one is available.
pub fn config_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|home| home.join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        });
    #[cfg(not(any(unix, target_os = "windows")))]
    let base: Option<PathBuf> = None;

    base.map(|directory| directory.join("Versus").join("recent-comparisons.v1"))
}

fn encode(entries: &[RecentComparison]) -> io::Result<Vec<u8>> {
    let mut output = Vec::with_capacity(16);
    output.extend_from_slice(MAGIC);
    output.push(VERSION);
    output.push(entries.len().min(MAX_RECENT_COMPARISONS) as u8);
    for entry in entries.iter().take(MAX_RECENT_COMPARISONS) {
        output.push(match entry.mode {
            ComparisonMode::Folder => 0,
            ComparisonMode::File => 1,
        });
        for path in &entry.paths {
            encode_path(&mut output, path)?;
        }
        if output.len() as u64 > MAX_FILE_BYTES {
            return invalid("recent history is too large");
        }
    }
    Ok(output)
}

fn decode(bytes: &[u8]) -> io::Result<RecentComparisons> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(4)? != MAGIC || cursor.take_byte()? != VERSION {
        return invalid("unrecognized recent history format");
    }
    let count = cursor.take_byte()? as usize;
    if count > MAX_RECENT_COMPARISONS {
        return invalid("recent history contains too many entries");
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let mode = match cursor.take_byte()? {
            0 => ComparisonMode::Folder,
            1 => ComparisonMode::File,
            _ => return invalid("recent history contains an invalid comparison mode"),
        };
        entries.push(RecentComparison {
            mode,
            paths: [decode_path(&mut cursor)?, decode_path(&mut cursor)?],
        });
    }
    if !cursor.remaining().is_empty() {
        return invalid("recent history has trailing data");
    }
    Ok(RecentComparisons::from_entries(entries))
}

#[cfg(unix)]
fn encode_path(output: &mut Vec<u8>, path: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() > MAX_PATH_BYTES || bytes.len() > u32::MAX as usize {
        return invalid("recent history path is too long");
    }
    output.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    output.extend_from_slice(bytes);
    Ok(())
}

#[cfg(target_os = "windows")]
fn encode_path(output: &mut Vec<u8>, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    if units.len() > MAX_PATH_BYTES / 2 || units.len() > u32::MAX as usize {
        return invalid("recent history path is too long");
    }
    output.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for unit in units {
        output.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

fn decode_path(cursor: &mut Cursor<'_>) -> io::Result<PathBuf> {
    let length = u32::from_le_bytes(cursor.take(4)?.try_into().expect("four byte length")) as usize;
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        if length > MAX_PATH_BYTES {
            return invalid("recent history path is too long");
        }
        return Ok(PathBuf::from(OsString::from_vec(
            cursor.take(length)?.to_vec(),
        )));
    }
    #[cfg(target_os = "windows")]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        if length > MAX_PATH_BYTES / 2 {
            return invalid("recent history path is too long");
        }
        let raw = cursor.take(length.checked_mul(2).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "recent history path is too long",
            )
        })?)?;
        let units = raw
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
        return Ok(PathBuf::from(OsString::from_wide(
            &units.collect::<Vec<_>>(),
        )));
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = (length, cursor);
        invalid("this platform cannot decode recent history")
    }
}

fn temporary_path(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "recent history path has no file name",
        )
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    Ok(path.with_file_name(format!(".{}.{}.tmp", name.to_string_lossy(), nonce)))
}

#[cfg(not(target_os = "windows"))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let mut source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let success = unsafe {
        MoveFileExW(
            source_wide.as_mut_ptr(),
            destination_wide.as_mut_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn invalid<T>(message: &'static str) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidData, message))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        let end = self.offset.checked_add(length).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "recent history is truncated")
        })?;
        let value = self.bytes.get(self.offset..end).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "recent history is truncated")
        })?;
        self.offset = end;
        Ok(value)
    }
    fn take_byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }
}

#[cfg(test)]
mod tests {
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
}
