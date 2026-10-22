use std::ops::Range;

use super::GitMergeError;

/// One syntactically complete Git conflict wrapper in the editable result.
#[derive(Clone, Debug)]
pub(super) struct ParsedConflict {
    pub wrapper: Range<usize>,
}

/// Parses Git's merge and diff3/zdiff3 marker formats.  Marker-looking lines
/// are deliberately rejected unless they form a complete wrapper: accepting a
/// damaged wrapper would allow a caller to report a false resolution.
pub(super) fn parse(text: &str) -> Result<Vec<ParsedConflict>, GitMergeError> {
    let lines = line_ranges(text);
    let mut parsed = Vec::new();
    let mut active: Option<ActiveConflict> = None;

    for range in lines {
        let line = text[range.clone()].trim_end_matches(['\r', '\n']);
        let marker = marker(line);
        if active.is_some() {
            match marker {
                Some((Marker::Base, width))
                    if width == active.as_ref().expect("active conflict exists").width
                        && active
                            .as_ref()
                            .expect("active conflict exists")
                            .base_start
                            .is_none()
                        && active
                            .as_ref()
                            .expect("active conflict exists")
                            .divider
                            .is_none() =>
                {
                    let active = active.as_mut().expect("active conflict exists");
                    active.base_start = Some(range.end);
                }
                Some((Marker::Divider, width))
                    if width == active.as_ref().expect("active conflict exists").width
                        && active
                            .as_ref()
                            .expect("active conflict exists")
                            .divider
                            .is_none() =>
                {
                    let active = active.as_mut().expect("active conflict exists");
                    active.divider = Some(range.end);
                }
                Some((Marker::End, width))
                    if width == active.as_ref().expect("active conflict exists").width
                        && active
                            .as_ref()
                            .expect("active conflict exists")
                            .divider
                            .is_some() =>
                {
                    let complete = active.take().expect("active conflict exists");
                    let _ = complete.divider.expect("divider checked");
                    parsed.push(ParsedConflict {
                        wrapper: complete.start..range.end,
                    });
                }
                Some(_) => return Err(GitMergeError::MalformedConflictMarkers),
                None => {}
            }
            continue;
        }

        match marker {
            Some((Marker::Start, width)) => {
                active = Some(ActiveConflict {
                    start: range.start,
                    width,
                    base_start: None,
                    divider: None,
                });
            }
            Some(_) => return Err(GitMergeError::MalformedConflictMarkers),
            None => {}
        }
    }

    if active.is_some() {
        return Err(GitMergeError::MalformedConflictMarkers);
    }
    Ok(parsed)
}

#[derive(Clone, Debug)]
struct ActiveConflict {
    start: usize,
    width: usize,
    base_start: Option<usize>,
    divider: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Marker {
    Start,
    Base,
    Divider,
    End,
}

fn marker(line: &str) -> Option<(Marker, usize)> {
    for (character, kind) in [
        ('<', Marker::Start),
        ('|', Marker::Base),
        ('=', Marker::Divider),
        ('>', Marker::End),
    ] {
        let width = line.chars().take_while(|&value| value == character).count();
        if width >= 7 {
            return Some((kind, width));
        }
    }
    None
}

fn line_ranges(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        if start == text.len() {
            return None;
        }
        let bytes = text.as_bytes();
        let mut end = start;
        while end < bytes.len() {
            let width = match bytes[end] {
                b'\n' => 1,
                b'\r' if bytes.get(end + 1) == Some(&b'\n') => 2,
                b'\r' => 1,
                _ => {
                    end += 1;
                    continue;
                }
            };
            end += width;
            break;
        }
        let range = start..end;
        start = end;
        Some(range)
    })
}
