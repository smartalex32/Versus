//! Explicit, offline find/replace processing for merge folders.  Searching is
//! intentionally separate from comparison and only reads files selected by a
//! find operation.

use super::{
    CompareError, CompareErrorKind, WorkingFile, load_working_file, resolve_working_file_identity,
};
use regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

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

/// Recursively searches selected roots.  Existing working copies are searched
/// first so unsaved edits are included.  Directory symlinks are never followed.
pub fn search_roots(
    roots: &[Option<PathBuf>; 2],
    scope: SearchScope,
    query: &SearchQuery,
    cancellation: Option<&AtomicBool>,
    working_overrides: &BTreeMap<PathBuf, WorkingFile>,
) -> Result<SearchResults, CompareError> {
    let matcher = query.compile()?;
    let mut output = SearchResults::default();
    let mut next_id = 1;
    let working_by_key: BTreeMap<PathBuf, &WorkingFile> = working_overrides
        .iter()
        .map(|(_, file)| (file.identity_key(), file))
        .collect();
    for &side in sides(scope) {
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
        let mut files = Vec::new();
        if metadata.is_file() {
            files.push(root.clone());
        } else if metadata.is_dir() {
            collect_files(root, &mut files, cancellation, &mut output)?;
        } else {
            continue;
        }
        for path in files {
            if cancelled(cancellation) {
                output.cancelled = true;
                return Ok(output);
            }
            let identity = resolve_working_file_identity(&path)?;
            let working_file = match working_by_key.get(&identity) {
                Some(working) => (*working).clone(),
                None => match load_working_file(Some(path.as_path())) {
                    Ok(working) => working,
                    Err(error)
                        if error.kind == CompareErrorKind::InvalidText
                            || error.kind == CompareErrorKind::TooLarge =>
                    {
                        output.skipped_binary.push(path);
                        continue;
                    }
                    Err(error) => return Err(error),
                },
            };
            let text = &working_file.text;
            let hits: Vec<SearchHit> = matcher
                .find_iter(&text)
                .map(|found| {
                    let hit = SearchHit {
                        id: next_id,
                        path: path.clone(),
                        side,
                        range: found.range(),
                        line: line_for_offset(&text, found.start()),
                        matched: found.as_str().to_owned(),
                    };
                    next_id += 1;
                    hit
                })
                .collect();
            if !hits.is_empty() {
                output.files.push(SearchFileResults {
                    path,
                    side,
                    hits,
                    working_file,
                });
            }
        }
    }
    Ok(output)
}

fn collect_files(
    root: &Path,
    output: &mut Vec<PathBuf>,
    cancellation: Option<&AtomicBool>,
    results: &mut SearchResults,
) -> Result<(), CompareError> {
    let mut todo = vec![root.to_path_buf()];
    while let Some(directory) = todo.pop() {
        if cancelled(cancellation) {
            results.cancelled = true;
            return Ok(());
        }
        for item in fs::read_dir(&directory).map_err(|error| CompareError::io(&directory, error))? {
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
                output.push(path);
            }
        }
    }
    Ok(())
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
fn line_for_offset(text: &str, offset: usize) -> usize {
    text[..offset].bytes().filter(|byte| *byte == b'\n').count() + 1
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
    let working_by_key: BTreeMap<PathBuf, &WorkingFile> = working_overrides
        .iter()
        .map(|(_, file)| (file.identity_key(), file))
        .collect();
    for file in &results.files {
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
