//! Explicit, offline find/replace processing for merge folders.  Searching is
//! intentionally separate from comparison and only reads files selected by a
//! find operation.

use super::{
    CompareError, CompareErrorKind, WorkingFile, load_working_file, resolve_working_file_identity,
    working_file_key,
};
use regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const SEARCH_HIT_BATCH_SIZE: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchScope {
    Left,
    Right,
    Both,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchQuery {
    pub pattern: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}
impl SearchQuery {
    pub fn plain(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            case_sensitive: false,
            whole_word: false,
            regex: false,
        }
    }
    fn compile(&self) -> Result<Regex, CompareError> {
        let mut source = if self.regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };
        if self.whole_word {
            source = format!(r"(?u)\b(?:{source})\b");
        }
        if !self.case_sensitive {
            source = format!("(?i:{source})");
        }
        Regex::new(&source).map_err(|error| CompareError {
            path: None,
            kind: CompareErrorKind::InvalidText,
            message: format!("invalid search expression: {error}"),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub id: u64,
    pub path: PathBuf,
    pub side: usize,
    pub range: Range<usize>,
    pub line: usize,
    pub matched: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchFileResults {
    pub path: PathBuf,
    pub side: usize,
    pub hits: Vec<SearchHit>,
    /// Snapshot retained to make later explicit saving check the bytes that
    /// were present when search ran.
    pub working_file: WorkingFile,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchResults {
    pub files: Vec<SearchFileResults>,
    pub skipped_binary: Vec<PathBuf>,
    pub cancelled: bool,
}

/// A search result made available as soon as one file has been read.  This
/// allows callers to present useful results while a large directory is still
/// being traversed instead of waiting for a complete file list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchEvent {
    File(SearchFileResults),
    /// Additional matches for the preceding file event. The file snapshot is
    /// retained only by `File`, so large searches do not clone file text for
    /// every batch.
    Hits {
        path: PathBuf,
        side: usize,
        hits: Vec<SearchHit>,
    },
    /// A binary, invalid-text, or oversized file that search intentionally did
    /// not inspect.
    Skipped(PathBuf),
    Complete {
        cancelled: bool,
    },
}

/// Recursively searches selected roots. Dirty working copies are searched first
/// so unsaved edits are included; clean snapshots are refreshed from disk.
/// Directory symlinks are never followed.
pub fn search_roots(
    roots: &[Option<PathBuf>; 2],
    scope: SearchScope,
    query: &SearchQuery,
    cancellation: Option<&AtomicBool>,
    working_overrides: &BTreeMap<PathBuf, WorkingFile>,
) -> Result<SearchResults, CompareError> {
    let mut output = SearchResults::default();
    search_roots_streaming(
        roots,
        scope,
        query,
        cancellation,
        working_overrides,
        |event| {
            match event {
                SearchEvent::File(file) => output.files.push(file),
                SearchEvent::Hits { path, side, hits } => output
                    .files
                    .iter_mut()
                    .rev()
                    .find(|file| file.path == path && file.side == side)
                    .expect("search hit batches follow a file event")
                    .hits
                    .extend(hits),
                SearchEvent::Skipped(path) => output.skipped_binary.push(path),
                SearchEvent::Complete { cancelled } => output.cancelled = cancelled,
            }
            true
        },
    )?;
    Ok(output)
}

/// Recursively searches selected roots, emitting each matching file as soon as
/// it is processed. Directory enumeration and file loading are deliberately
/// interleaved: callers can render early matches without waiting for an entire
/// directory tree to be listed. Returning `false` from `emit` cancels search
/// cleanly, which is useful when an asynchronous UI receiver goes away.
pub fn search_roots_streaming<F>(
    roots: &[Option<PathBuf>; 2],
    scope: SearchScope,
    query: &SearchQuery,
    cancellation: Option<&AtomicBool>,
    working_overrides: &BTreeMap<PathBuf, WorkingFile>,
    mut emit: F,
) -> Result<(), CompareError>
where
    F: FnMut(SearchEvent) -> bool,
{
    let matcher = query.compile()?;
    let mut next_id = 1;
    // A clean working snapshot is only a view cache and may have become stale
    // since the previous Find. Re-read it from disk on every search; only
    // unsaved edits take precedence over the filesystem.
    let working_by_key: BTreeMap<PathBuf, &WorkingFile> = working_overrides
        .iter()
        .filter(|(_, file)| file.is_dirty())
        .map(|(_, file)| (file.identity_key(), file))
        .collect();
    let mut working_by_path = BTreeMap::new();
    for (map_path, file) in working_overrides.iter().filter(|(_, file)| file.is_dirty()) {
        let native_path = file.path.as_deref().unwrap_or(map_path);
        working_by_path
            .entry(working_file_key(native_path))
            .or_insert(file);
        working_by_path
            .entry(working_file_key(map_path))
            .or_insert(file);
        if let Ok(canonical_path) = fs::canonicalize(native_path) {
            working_by_path.entry(canonical_path).or_insert(file);
        }
    }
    for &side in sides(scope) {
        if cancelled(cancellation) {
            emit(SearchEvent::Complete { cancelled: true });
            return Ok(());
        }
        let Some(root) = roots[side].as_ref() else {
            continue;
        };
        let metadata = fs::symlink_metadata(root).map_err(|error| CompareError::io(root, error))?;
        if metadata.file_type().is_symlink() {
            return Err(CompareError {
                path: Some(root.clone()),
                kind: CompareErrorKind::InvalidPath,
                message: "search roots cannot be symlinks".into(),
            });
        }
        if metadata.is_file() {
            if !search_file(
                root,
                side,
                &matcher,
                &mut next_id,
                cancellation,
                &working_by_key,
                &working_by_path,
                &mut emit,
            )? {
                return Ok(());
            }
        } else if metadata.is_dir() {
            if !search_directory(
                root,
                side,
                &matcher,
                &mut next_id,
                cancellation,
                &working_by_key,
                &working_by_path,
                &mut emit,
            )? {
                return Ok(());
            }
        } else {
            continue;
        }
        if cancelled(cancellation) {
            emit(SearchEvent::Complete { cancelled: true });
            return Ok(());
        }
    }
    emit(SearchEvent::Complete { cancelled: false });
    Ok(())
}

fn search_directory<F>(
    root: &Path,
    side: usize,
    matcher: &Regex,
    next_id: &mut u64,
    cancellation: Option<&AtomicBool>,
    working_by_key: &BTreeMap<PathBuf, &WorkingFile>,
    working_by_path: &BTreeMap<PathBuf, &WorkingFile>,
    emit: &mut F,
) -> Result<bool, CompareError>
where
    F: FnMut(SearchEvent) -> bool,
{
    let mut todo = vec![root.to_path_buf()];
    while let Some(directory) = todo.pop() {
        if cancelled(cancellation) {
            emit(SearchEvent::Complete { cancelled: true });
            return Ok(false);
        }
        for item in fs::read_dir(&directory).map_err(|error| CompareError::io(&directory, error))? {
            if cancelled(cancellation) {
                emit(SearchEvent::Complete { cancelled: true });
                return Ok(false);
            }
            let item = item.map_err(|error| CompareError::io(&directory, error))?;
            let path = item.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|error| CompareError::io(&path, error))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                todo.push(path);
            } else if metadata.is_file() {
                if !search_file(
                    &path,
                    side,
                    matcher,
                    next_id,
                    cancellation,
                    working_by_key,
                    working_by_path,
                    emit,
                )? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn search_file<F>(
    path: &Path,
    side: usize,
    matcher: &Regex,
    next_id: &mut u64,
    cancellation: Option<&AtomicBool>,
    working_by_key: &BTreeMap<PathBuf, &WorkingFile>,
    working_by_path: &BTreeMap<PathBuf, &WorkingFile>,
    emit: &mut F,
) -> Result<bool, CompareError>
where
    F: FnMut(SearchEvent) -> bool,
{
    let identity = resolve_working_file_identity(path)?;
    let working_file = match working_by_key.get(&identity).copied().or_else(|| {
        working_by_path
            .get(&working_file_key(path))
            .copied()
            .or_else(|| {
                fs::canonicalize(path)
                    .ok()
                    .and_then(|canonical_path| working_by_path.get(&canonical_path).copied())
            })
    }) {
        Some(working) => (*working).clone(),
        None => match load_working_file(Some(path)) {
            Ok(working) => working,
            Err(error)
                if error.kind == CompareErrorKind::InvalidText
                    || error.kind == CompareErrorKind::TooLarge =>
            {
                return Ok(emit(SearchEvent::Skipped(path.to_path_buf())));
            }
            Err(error) => return Err(error),
        },
    };
    let text = &working_file.text;
    let mut hits = Vec::new();
    let mut emitted_file = false;
    let mut line = 1;
    let mut previous_start = 0;
    for found in matcher.find_iter(text) {
        if cancelled(cancellation) {
            emit(SearchEvent::Complete { cancelled: true });
            return Ok(false);
        }
        // Regex matches are ordered, so count only the text passed since the
        // prior hit instead of repeatedly scanning from the beginning.
        line += text[previous_start..found.start()]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        previous_start = found.start();
        hits.push(SearchHit {
            id: *next_id,
            path: path.to_path_buf(),
            side,
            range: found.range(),
            line,
            matched: found.as_str().to_owned(),
        });
        *next_id += 1;
        // Send the first match immediately so a sparse match near the start of
        // a very large file is visible without waiting for the rest of the
        // scan. Later events are bounded batches.
        if !emitted_file || hits.len() == SEARCH_HIT_BATCH_SIZE {
            let batch = std::mem::take(&mut hits);
            if emitted_file {
                if !emit(SearchEvent::Hits {
                    path: path.to_path_buf(),
                    side,
                    hits: batch,
                }) {
                    return Ok(false);
                }
            } else {
                // Keep the original snapshot for the rest of this scan while
                // passing one owned snapshot to consumers. This is the only
                // text clone for the file; later batches carry only hits.
                if !emit(SearchEvent::File(SearchFileResults {
                    path: path.to_path_buf(),
                    side,
                    hits: batch,
                    working_file: working_file.clone(),
                })) {
                    return Ok(false);
                }
                emitted_file = true;
            }
        }
    }
    if hits.is_empty() {
        return Ok(true);
    }
    if emitted_file {
        Ok(emit(SearchEvent::Hits {
            path: path.to_path_buf(),
            side,
            hits,
        }))
    } else {
        Ok(emit(SearchEvent::File(SearchFileResults {
            path: path.to_path_buf(),
            side,
            hits,
            working_file,
        })))
    }
}
fn cancelled(value: Option<&AtomicBool>) -> bool {
    value.is_some_and(|flag| flag.load(Ordering::Relaxed))
}
fn sides(scope: SearchScope) -> &'static [usize] {
    match scope {
        SearchScope::Left => &[0],
        SearchScope::Right => &[1],
        SearchScope::Both => &[0, 1],
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchChange {
    pub path: PathBuf,
    pub before: String,
    pub after: String,
    pub hit_ids: Vec<u64>,
    /// Original offsets of selected matches.  Consumers use these to preserve
    /// byte ranges when preparing the replacement transaction.
    pub ranges: Vec<Range<usize>>,
    /// Exact replacements in original-buffer coordinates.  These let history
    /// preserve multiple disjoint find-and-replace edits as one transaction.
    pub edits: Vec<(Range<usize>, String)>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchPreview {
    pub changes: Vec<SearchChange>,
    pub skipped: Vec<u64>,
    pub bases: BTreeMap<PathBuf, WorkingFile>,
}

/// Builds a non-mutating replacement preview.  IDs refer to original search
/// ranges, so selected matches remain safe when another selected match changes
/// length.  Regex replacements honor capture substitutions (`$1`, `${name}`).
pub fn preview_replace(
    results: &SearchResults,
    selected: &BTreeSet<u64>,
    query: &SearchQuery,
    replacement: &str,
    working_overrides: &BTreeMap<PathBuf, WorkingFile>,
) -> Result<SearchPreview, CompareError> {
    let matcher = query.compile()?;
    let mut preview = SearchPreview::default();
    let mut grouped: BTreeMap<PathBuf, Vec<&SearchHit>> = BTreeMap::new();
    let mut snapshots: BTreeMap<PathBuf, WorkingFile> = BTreeMap::new();
    let mut native_paths: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    // Search refreshes clean snapshots from disk. Only unsaved edits may
    // supersede the snapshot that produced these hit ranges; a clean editor
    // cache must not reinstate an older fingerprint or text here.
    let working_by_key: BTreeMap<PathBuf, &WorkingFile> = working_overrides
        .iter()
        .filter(|(_, file)| file.is_dirty())
        .map(|(_, file)| (file.identity_key(), file))
        .collect();
    for file in &results.files {
        // Cached matches on the other side are not replacement participants.
        // Their newer edits must not block a preview scoped to this side.
        if !file.hits.iter().any(|hit| selected.contains(&hit.id)) {
            continue;
        }
        let key = file.working_file.identity_key();
        let base = working_by_key
            .get(&key)
            .map(|file| (*file).clone())
            .unwrap_or_else(|| file.working_file.clone());
        if base.text != file.working_file.text {
            return Err(CompareError {
                path: Some(file.path.clone()),
                kind: CompareErrorKind::Io,
                message: "file changed after find; run find again before replacing".into(),
            });
        }
        snapshots.entry(key.clone()).or_insert(base);
        native_paths
            .entry(key.clone())
            .or_insert_with(|| file.path.clone());
        let hits = grouped.entry(key).or_default();
        hits.extend(file.hits.iter().filter(|hit| selected.contains(&hit.id)));
    }
    for (identity, mut hits) in grouped {
        if hits.is_empty() {
            continue;
        }
        let base = snapshots
            .remove(&identity)
            .expect("search snapshot is grouped with its file");
        let path = native_paths
            .remove(&identity)
            .expect("search path is grouped with its file");
        let before = base.text.clone();
        hits.sort_by_key(|hit| std::cmp::Reverse(hit.range.start));
        let mut after = before.clone();
        let mut ids = Vec::new();
        let mut ranges = Vec::new();
        let mut edits = Vec::new();
        let mut processed_ranges = BTreeSet::new();
        for hit in hits {
            if !processed_ranges.insert((hit.range.start, hit.range.end)) {
                continue;
            }
            if hit.range.end > after.len()
                || !after.is_char_boundary(hit.range.start)
                || !after.is_char_boundary(hit.range.end)
            {
                preview.skipped.push(hit.id);
                continue;
            }
            let matched = &after[hit.range.clone()];
            // A stale preview must never replace a different range.
            if matched != hit.matched {
                preview.skipped.push(hit.id);
                continue;
            }
            let replacement =
                replacement_text(&matcher, &before, &hit.range, replacement, query.regex)?;
            after.replace_range(hit.range.clone(), &replacement);
            ids.push(hit.id);
            ranges.push(hit.range.clone());
            edits.push((hit.range.clone(), replacement));
        }
        if after != before {
            ranges.sort_by_key(|range| range.start);
            edits.sort_by_key(|(range, _)| range.start);
            preview.bases.insert(path.clone(), base);
            preview.changes.push(SearchChange {
                path,
                before,
                after,
                hit_ids: ids,
                ranges,
                edits,
            });
        }
    }
    Ok(preview)
}

fn replacement_text(
    matcher: &Regex,
    text: &str,
    range: &Range<usize>,
    replacement: &str,
    regex: bool,
) -> Result<String, CompareError> {
    if !regex {
        return Ok(replacement.to_owned());
    }
    let captures = matcher
        .captures_at(text, range.start)
        .ok_or_else(|| CompareError {
            path: None,
            kind: CompareErrorKind::InvalidText,
            message: "replacement no longer matches the selected expression".into(),
        })?;
    if captures.get(0).map(|found| found.range()) != Some(range.clone()) {
        return Err(CompareError {
            path: None,
            kind: CompareErrorKind::InvalidText,
            message: "replacement no longer matches the selected expression".into(),
        });
    }
    let mut output = String::new();
    captures.expand(replacement, &mut output);
    Ok(output)
}

/// Applies a reviewed preview to working copies.  Returned changes are suitable
/// for one transactional undo entry; disk files are never written here.
pub fn apply_preview(
    working: &mut BTreeMap<PathBuf, WorkingFile>,
    preview: &SearchPreview,
) -> Result<Vec<SearchChange>, CompareError> {
    // Validate every source first.  A later stale file must not leave an
    // earlier file inserted into the working map.
    for change in &preview.changes {
        let base = preview
            .bases
            .get(&change.path)
            .ok_or_else(|| CompareError {
                path: Some(change.path.clone()),
                kind: CompareErrorKind::InvalidPath,
                message: "replace preview is missing its file snapshot".into(),
            })?;
        let identity = base.identity_key();
        let matching: Vec<&WorkingFile> = working
            .iter()
            .filter(|(_, file)| file.identity_key() == identity)
            .map(|(_, file)| file)
            .collect();
        if matching.is_empty() {
            if base.text != change.before {
                return Err(CompareError {
                    path: Some(change.path.clone()),
                    kind: CompareErrorKind::Io,
                    message: "file changed after replace preview; run find again".into(),
                });
            }
        } else if matching.iter().any(|file| file.text != change.before) {
            return Err(CompareError {
                path: Some(change.path.clone()),
                kind: CompareErrorKind::Io,
                message: "file changed after replace preview; run find again".into(),
            });
        }
    }
    for change in &preview.changes {
        let base = &preview.bases[&change.path];
        let identity = base.identity_key();
        let exists = working.values().any(|file| file.identity_key() == identity);
        if !exists {
            working.insert(
                base.path.clone().unwrap_or_else(|| change.path.clone()),
                base.clone(),
            );
        }
        for file in working.values_mut() {
            if file.identity_key() == identity {
                file.text.clone_from(&change.after);
            }
        }
    }
    Ok(preview.changes.clone())
}

#[cfg(test)]
#[path = "../../tests/unit/core/search_replace/mod.rs"]
mod tests;
