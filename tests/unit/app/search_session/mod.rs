use super::*;
use versus::{FileFingerprint, SearchFileResults, SearchHit, TextEncoding};

fn file(path: &str, identity: &str, text: &str) -> WorkingFile {
    WorkingFile {
        path: Some(path.into()),
        identity: Some(identity.into()),
        text: text.into(),
        encoding: TextEncoding::Utf8 { bom: false },
        fingerprint: FileFingerprint::Missing,
    }
}

fn results(text: &str) -> SearchResults {
    let working = file("left.txt", "/physical/left.txt", text);
    SearchResults {
        files: vec![SearchFileResults {
            path: "left.txt".into(),
            side: 0,
            hits: vec![
                SearchHit {
                    id: 1,
                    path: "left.txt".into(),
                    side: 0,
                    range: 0..3,
                    line: 1,
                    matched: "foo".into(),
                },
                SearchHit {
                    id: 2,
                    path: "left.txt".into(),
                    side: 0,
                    range: 4..7,
                    line: 1,
                    matched: "foo".into(),
                },
            ],
            working_file: working,
        }],
        ..SearchResults::default()
    }
}

#[test]
fn next_previews_only_the_first_selected_hit_in_result_order() {
    let results = results("foo foo");
    let selected = BTreeSet::from([1, 2]);
    let mut working = BTreeMap::new();
    working.insert(
        "alias.txt".into(),
        file("alias.txt", "/physical/left.txt", "foo foo"),
    );
    let preview = preview_replacements(
        &results,
        &selected,
        &SearchQuery::plain("foo"),
        "bar",
        &working,
        true,
    )
    .unwrap();
    assert_eq!(preview.changes[0].after, "bar foo");
    assert_eq!(preview.changes[0].hit_ids, vec![1]);
}

#[test]
fn advancing_consumes_applied_hits_and_shifts_remaining_utf8_ranges() {
    let mut results = results("foo\ncafé\nfoo");
    results.files[0].hits = vec![
        SearchHit {
            id: 1,
            path: "left.txt".into(),
            side: 0,
            range: 0..3,
            line: 1,
            matched: "foo".into(),
        },
        SearchHit {
            id: 2,
            path: "left.txt".into(),
            side: 0,
            range: 10..13,
            line: 3,
            matched: "foo".into(),
        },
    ];
    results.files[0].working_file.text = "foo\ncafé\nfoo".into();
    let selected = BTreeSet::from([1, 2]);
    let mut working = BTreeMap::new();
    working.insert(
        "left.txt".into(),
        file("left.txt", "/physical/left.txt", "foo\ncafé\nfoo"),
    );
    let preview = preview_replacements(
        &results,
        &selected,
        &SearchQuery::plain("foo"),
        "quux",
        &working,
        true,
    )
    .unwrap();
    working.get_mut(&PathBuf::from("left.txt")).unwrap().text = "quux\ncafé\nfoo".into();
    let mut selected = selected;
    advance_results(&mut results, &mut selected, &preview, &working).unwrap();
    assert_eq!(results.files[0].hits[0].id, 2);
    assert_eq!(results.files[0].hits[0].range, 11..14);
    assert_eq!(results.files[0].hits[0].line, 3);
    assert_eq!(selected, BTreeSet::from([2]));

    let next = preview_replacements(
        &results,
        &selected,
        &SearchQuery::plain("foo"),
        "quux",
        &working,
        true,
    )
    .unwrap();
    assert_eq!(next.changes[0].hit_ids, vec![2]);
    assert_eq!(next.changes[0].after, "quux\ncafé\nquux");
}

#[test]
fn advancing_keeps_aliases_and_does_not_add_matches_from_replacement_text() {
    let mut results = results("foo foo");
    let alias = SearchFileResults {
        path: "alias.txt".into(),
        side: 1,
        hits: vec![SearchHit {
            id: 3,
            path: "alias.txt".into(),
            side: 1,
            range: 4..7,
            line: 1,
            matched: "foo".into(),
        }],
        working_file: file("alias.txt", "/physical/left.txt", "foo foo"),
    };
    results.files.push(alias);
    let mut selected = BTreeSet::from([1]);
    let mut working = BTreeMap::new();
    working.insert(
        "left.txt".into(),
        file("left.txt", "/physical/left.txt", "foo foo"),
    );
    let preview = preview_replacements(
        &results,
        &selected,
        &SearchQuery::plain("foo"),
        "foofoo",
        &working,
        true,
    )
    .unwrap();
    working.get_mut(&PathBuf::from("left.txt")).unwrap().text = "foofoo foo".into();
    advance_results(&mut results, &mut selected, &preview, &working).unwrap();
    assert_eq!(results.files.len(), 2);
    assert!(
        results
            .files
            .iter()
            .flat_map(|file| &file.hits)
            .all(|hit| hit.id != 1)
    );
    assert_eq!(
        results
            .files
            .iter()
            .flat_map(|file| &file.hits)
            .map(|hit| hit.id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([2, 3])
    );
    assert!(selected.is_empty());
}

#[test]
fn stale_working_copy_fails_without_changing_results() {
    let mut results = results("foo foo");
    let original = results.clone();
    let selected = BTreeSet::from([1]);
    let mut working = BTreeMap::new();
    working.insert(
        "left.txt".into(),
        file("left.txt", "/physical/left.txt", "foo foo"),
    );
    let preview = preview_replacements(
        &results,
        &selected,
        &SearchQuery::plain("foo"),
        "bar",
        &working,
        true,
    )
    .unwrap();
    working.get_mut(&PathBuf::from("left.txt")).unwrap().text = "changed".into();
    let mut selected = selected;
    assert!(advance_results(&mut results, &mut selected, &preview, &working).is_err());
    assert_eq!(results, original);
    assert_eq!(selected, BTreeSet::from([1]));
}
