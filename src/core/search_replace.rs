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
    /// protected merge ranges when presenting the replacement preview.
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
                message: "file changed after find; run find again before previewing replacements"
                    .into(),
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
mod tests {
    use super::*;
    use std::fs;
    fn temp(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("versus-search-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn replace_file(path: &Path, text: &str) {
        let replacement = path.with_extension("replacement");
        fs::write(&replacement, text).unwrap();
        #[cfg(windows)]
        fs::remove_file(path).unwrap();
        fs::rename(replacement, path).unwrap();
    }
    #[test]
    fn regex_preview_is_grouped_and_staged_without_saving() {
        let root = temp("regex");
        let file = root.join("a.txt");
        fs::write(&file, "a1\na2\n").unwrap();
        fs::write(root.join("bin"), [0, 1, 2]).unwrap();
        let query = SearchQuery {
            pattern: "a(\\d)".into(),
            case_sensitive: true,
            whole_word: false,
            regex: true,
        };
        let copies = BTreeMap::new();
        let results = search_roots(
            &[Some(root.clone()), None],
            SearchScope::Left,
            &query,
            None,
            &copies,
        )
        .unwrap();
        assert_eq!(results.files[0].hits.len(), 2);
        assert_eq!(results.skipped_binary.len(), 1);
        let selected = results.files[0].hits.iter().map(|hit| hit.id).collect();
        let preview = preview_replace(&results, &selected, &query, "b$1", &copies).unwrap();
        let mut copies = BTreeMap::new();
        let changes = apply_preview(&mut copies, &preview).unwrap();
        assert_eq!(changes[0].after, "b1\nb2\n");
        assert_eq!(fs::read_to_string(&file).unwrap(), "a1\na2\n");
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn both_scopes_keep_side_identity_but_preview_deduplicates_the_file() {
        let root = temp("both");
        let file = root.join("same.txt");
        fs::write(&file, "needle").unwrap();
        let query = SearchQuery::plain("needle");
        let results = search_roots(
            &[Some(root.clone()), Some(root.clone())],
            SearchScope::Both,
            &query,
            None,
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(results.files.len(), 2);
        assert_eq!(results.files[0].side, 0);
        assert_eq!(results.files[1].side, 1);
        let selected = results
            .files
            .iter()
            .flat_map(|file| file.hits.iter())
            .map(|hit| hit.id)
            .collect();
        let preview =
            preview_replace(&results, &selected, &query, "done", &BTreeMap::new()).unwrap();
        assert_eq!(preview.changes.len(), 1);
        assert_eq!(preview.changes[0].ranges, vec![0..6]);
        assert_eq!(preview.changes[0].edits, vec![(0..6, "done".into())]);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn stale_multifile_preview_does_not_partially_mutate_working_copies() {
        let root = temp("atomic-preview");
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        fs::write(&first, "replace").unwrap();
        fs::write(&second, "replace").unwrap();
        let query = SearchQuery::plain("replace");
        let results = search_roots(
            &[Some(root.clone()), None],
            SearchScope::Left,
            &query,
            None,
            &BTreeMap::new(),
        )
        .unwrap();
        let selected = results
            .files
            .iter()
            .flat_map(|file| file.hits.iter())
            .map(|hit| hit.id)
            .collect();
        let preview =
            preview_replace(&results, &selected, &query, "done", &BTreeMap::new()).unwrap();
        let mut working = BTreeMap::new();
        let mut stale = load_working_file(Some(second.as_path())).unwrap();
        stale.text = "changed after preview".into();
        working.insert(second.clone(), stale);
        assert!(apply_preview(&mut working, &preview).is_err());
        assert!(!working.contains_key(&first));
        assert_eq!(working[&second].text, "changed after preview");
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn streaming_emits_matching_files_before_a_later_root_fails() {
        let root = temp("streaming-early-result");
        let file = root.join("match.txt");
        fs::write(&file, "needle").unwrap();
        let missing = root.join("missing");
        let mut events = Vec::new();
        let error = search_roots_streaming(
            &[Some(file.clone()), Some(missing)],
            SearchScope::Both,
            &SearchQuery::plain("needle"),
            None,
            &BTreeMap::new(),
            |event| {
                events.push(event);
                true
            },
        )
        .unwrap_err();
        assert_eq!(error.path, Some(root.join("missing")));
        assert!(matches!(
            events.as_slice(),
            [SearchEvent::File(SearchFileResults { path, side: 0, .. })] if path == &file
        ));
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn streaming_stops_when_receiver_declines_an_event() {
        let root = temp("streaming-receiver-gone");
        fs::write(root.join("first.txt"), "needle").unwrap();
        fs::write(root.join("second.txt"), "needle").unwrap();
        let mut events = Vec::new();
        search_roots_streaming(
            &[Some(root.clone()), None],
            SearchScope::Left,
            &SearchQuery::plain("needle"),
            None,
            &BTreeMap::new(),
            |event| {
                events.push(event);
                false
            },
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SearchEvent::File(_)));
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn streaming_emits_bounded_hit_batches_before_a_large_file_finishes() {
        let root = temp("streaming-hit-batches");
        let file = root.join("many.txt");
        fs::write(&file, "needle\n".repeat(SEARCH_HIT_BATCH_SIZE * 2 + 3)).unwrap();
        let mut events = Vec::new();
        search_roots_streaming(
            &[Some(file.clone()), None],
            SearchScope::Left,
            &SearchQuery::plain("needle"),
            None,
            &BTreeMap::new(),
            |event| {
                let continue_search = !matches!(event, SearchEvent::File(_));
                events.push(event);
                continue_search
            },
        )
        .unwrap();
        assert!(matches!(
            events.as_slice(),
            [SearchEvent::File(SearchFileResults { hits, .. })]
                if hits.len() == 1
        ));
        // The receiver stopped at the first match, proving the remaining
        // matches were not required before an event was delivered.
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, SearchEvent::Complete { .. }))
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn streaming_aggregates_batches_and_can_cancel_between_them() {
        let root = temp("streaming-batch-cancel");
        let file = root.join("many.txt");
        fs::write(&file, "needle\n".repeat(SEARCH_HIT_BATCH_SIZE * 2 + 3)).unwrap();
        let cancellation = AtomicBool::new(false);
        let mut events = Vec::new();
        search_roots_streaming(
            &[Some(file), None],
            SearchScope::Left,
            &SearchQuery::plain("needle"),
            Some(&cancellation),
            &BTreeMap::new(),
            |event| {
                if matches!(event, SearchEvent::File(_)) {
                    cancellation.store(true, Ordering::Relaxed);
                }
                events.push(event);
                true
            },
        )
        .unwrap();
        assert!(matches!(
            events.as_slice(),
            [SearchEvent::File(SearchFileResults { hits, .. }), SearchEvent::Complete { cancelled: true }]
                if hits.len() == 1
        ));

        let full = search_roots(
            &[Some(root.join("many.txt")), None],
            SearchScope::Left,
            &SearchQuery::plain("needle"),
            None,
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            full.files[0]
                .hits
                .iter()
                .map(|hit| hit.id)
                .collect::<Vec<_>>(),
            (1..=(SEARCH_HIT_BATCH_SIZE * 2 + 3) as u64).collect::<Vec<_>>()
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn streaming_assigns_lines_incrementally_for_dense_utf8_matches() {
        let root = temp("streaming-utf8-lines");
        let file = root.join("lines.txt");
        fs::write(&file, "β needle\nneedle β needle\nneedle").unwrap();
        let mut events = Vec::new();
        search_roots_streaming(
            &[Some(file), None],
            SearchScope::Left,
            &SearchQuery::plain("needle"),
            None,
            &BTreeMap::new(),
            |event| {
                events.push(event);
                true
            },
        )
        .unwrap();
        let hits: Vec<_> = events
            .into_iter()
            .flat_map(|event| match event {
                SearchEvent::File(file) => file.hits,
                SearchEvent::Hits { hits, .. } => hits,
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(
            hits.iter().map(|hit| hit.line).collect::<Vec<_>>(),
            vec![1, 2, 2, 3]
        );
        assert_eq!(
            hits.iter().map(|hit| hit.id).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn streaming_preserves_side_specific_unique_ids_and_dirty_overrides() {
        let root = temp("streaming-sides");
        let file = root.join("same.txt");
        fs::write(&file, "disk needle").unwrap();
        let mut dirty = load_working_file(Some(file.as_path())).unwrap();
        dirty.text = "working needle".into();
        // An external atomic replacement can give the new on-disk file a
        // different resolved identity. Keep unsaved text selected by its
        // native path rather than silently searching the replacement.
        dirty.identity = Some(root.join("retired-identity"));
        replace_file(&file, "replacement needle");
        let mut overrides = BTreeMap::new();
        overrides.insert(file.clone(), dirty);
        let mut events = Vec::new();
        search_roots_streaming(
            &[Some(file.clone()), Some(file.clone())],
            SearchScope::Both,
            &SearchQuery::plain("needle"),
            None,
            &overrides,
            |event| {
                events.push(event);
                true
            },
        )
        .unwrap();
        let files: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                SearchEvent::File(file) => Some(file),
                _ => None,
            })
            .collect();
        assert_eq!(files.len(), 2);
        assert_eq!((files[0].side, files[1].side), (0, 1));
        assert_eq!(files[0].working_file.text, "working needle");
        assert_ne!(files[0].hits[0].id, files[1].hits[0].id);
        assert!(matches!(
            events.last(),
            Some(SearchEvent::Complete { cancelled: false })
        ));
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn clean_override_is_reloaded_from_disk_for_a_fresh_search() {
        let root = temp("streaming-fresh-disk");
        let file = root.join("content.txt");
        fs::write(&file, "old needle").unwrap();
        let clean = load_working_file(Some(file.as_path())).unwrap();
        replace_file(&file, "new needle");
        let mut overrides = BTreeMap::new();
        overrides.insert(file.clone(), clean);
        let results = search_roots(
            &[Some(file.clone()), None],
            SearchScope::Left,
            &SearchQuery::plain("needle"),
            None,
            &overrides,
        )
        .unwrap();
        assert_eq!(results.files[0].working_file.text, "new needle");
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn preview_only_validates_files_with_selected_replacement_targets() {
        let root = temp("selected-side-preview");
        fs::create_dir_all(&root).unwrap();
        let paths = [root.join("left.txt"), root.join("right.txt")];
        for path in &paths {
            fs::write(path, "match").unwrap();
        }
        let query = SearchQuery::plain("match");
        let results = search_roots(
            &paths.clone().map(Some),
            SearchScope::Both,
            &query,
            None,
            &BTreeMap::new(),
        )
        .unwrap();
        let mut dirty = load_working_file(Some(&paths[1])).unwrap();
        dirty.text = "new unsaved text".into();
        let overrides = BTreeMap::from([(paths[1].clone(), dirty)]);
        let left = BTreeSet::from([results.files[0].hits[0].id]);
        let preview = preview_replace(&results, &left, &query, "changed", &overrides).unwrap();
        assert_eq!(preview.changes.len(), 1);
        assert_eq!(preview.changes[0].path, paths[0]);
        assert_eq!(preview.changes[0].after, "changed");
        let right = BTreeSet::from([results.files[1].hits[0].id]);
        assert!(preview_replace(&results, &right, &query, "changed", &overrides).is_err());
        assert_eq!(fs::read_to_string(&paths[0]).unwrap(), "match");
        assert_eq!(fs::read_to_string(&paths[1]).unwrap(), "match");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preview_uses_fresh_search_snapshot_instead_of_a_clean_editor_cache() {
        let root = temp("preview-fresh-clean-cache");
        let file = root.join("content.txt");
        fs::write(&file, "old needle").unwrap();
        let clean_cache = load_working_file(Some(file.as_path())).unwrap();
        replace_file(&file, "new needle");
        let mut overrides = BTreeMap::new();
        overrides.insert(file.clone(), clean_cache);
        let query = SearchQuery::plain("needle");
        let results = search_roots(
            &[Some(file.clone()), None],
            SearchScope::Left,
            &query,
            None,
            &overrides,
        )
        .unwrap();
        let selected = results.files[0].hits.iter().map(|hit| hit.id).collect();
        let preview = preview_replace(&results, &selected, &query, "done", &overrides).unwrap();
        assert_eq!(preview.changes[0].before, "new needle");
        assert_eq!(preview.changes[0].after, "new done");
        assert_eq!(
            preview.bases.get(&file),
            Some(&results.files[0].working_file)
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn streaming_reports_cancellation_as_completion() {
        let root = temp("streaming-cancelled");
        let cancelled = AtomicBool::new(true);
        let mut events = Vec::new();
        search_roots_streaming(
            &[Some(root.clone()), None],
            SearchScope::Left,
            &SearchQuery::plain("needle"),
            Some(&cancelled),
            &BTreeMap::new(),
            |event| {
                events.push(event);
                true
            },
        )
        .unwrap();
        assert_eq!(events, vec![SearchEvent::Complete { cancelled: true }]);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn aliased_roots_share_unsaved_copy_and_coalesce_selected_matches() {
        let root = temp("aliases");
        let file = root.join("same.txt");
        fs::write(&file, "a b").unwrap();
        let alias = root.join("../").join(root.file_name().unwrap());
        let query = SearchQuery {
            pattern: "[ab]".into(),
            case_sensitive: true,
            whole_word: false,
            regex: true,
        };
        let mut copies = BTreeMap::new();
        let mut working = load_working_file(Some(file.as_path())).unwrap();
        working.text = "a b".into();
        copies.insert(file.clone(), working);
        let results = search_roots(
            &[Some(root.clone()), Some(alias)],
            SearchScope::Both,
            &query,
            None,
            &copies,
        )
        .unwrap();
        assert_eq!(results.files.len(), 2);
        assert_eq!(results.files[0].side, 0);
        assert_eq!(results.files[1].side, 1);
        let selected = results
            .files
            .iter()
            .flat_map(|file| file.hits.iter())
            .filter(|hit| {
                (hit.side == 0 && hit.matched == "a") || (hit.side == 1 && hit.matched == "b")
            })
            .map(|hit| hit.id)
            .collect();
        let preview = preview_replace(&results, &selected, &query, "x", &copies).unwrap();
        assert_eq!(preview.changes.len(), 1);
        assert_eq!(preview.changes[0].after, "x x");
        apply_preview(&mut copies, &preview).unwrap();
        assert_eq!(copies[&file].text, "x x");
        let _ = fs::remove_dir_all(root);
    }
    #[cfg(unix)]
    #[test]
    fn lexical_aliases_with_different_symlink_parents_stay_separate() {
        use std::os::unix::fs::symlink;
        let root = temp("physical-aliases");
        let external = root.join("external");
        let link = root.join("link");
        fs::create_dir_all(external.join("nested")).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("outside.txt"), "left").unwrap();
        fs::write(external.join("outside.txt"), "right").unwrap();
        symlink(external.join("nested"), &link).unwrap();
        // `link/..` is the external parent physically but normalizes to the
        // same lexical spelling as `root/outside.txt`.
        let left = link.join("../outside.txt");
        let right = root.join("outside.txt");
        let query = SearchQuery::plain("t");
        let results = search_roots(
            &[Some(left), Some(right)],
            SearchScope::Both,
            &query,
            None,
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(results.files.len(), 2);
        let selected = results
            .files
            .iter()
            .flat_map(|file| file.hits.iter())
            .map(|hit| hit.id)
            .collect();
        let preview = preview_replace(&results, &selected, &query, "T", &BTreeMap::new()).unwrap();
        assert_eq!(preview.changes.len(), 2);
        let _ = fs::remove_dir_all(root);
    }
}
