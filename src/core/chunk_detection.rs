//! Pure structural and paragraph chunk detection for editable text.

use super::interactive::{ChunkKind, InteractiveChunk};
use super::text_lines::lines;
use std::ops::Range;

pub(super) fn detect_chunks(text: &str) -> Vec<InteractiveChunk> {
    let lines = lines(text);
    let mut result = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim().is_empty() {
            index += 1;
            continue;
        };
        let start = index;
        let structural = structural_kind(lines[index]);
        index += 1;
        if let Some(StructuralKind::Python { indent }) = structural {
            while index < lines.len() {
                if !lines[index].trim().is_empty() && leading_indent(lines[index]) <= indent {
                    break;
                };
                index += 1;
            }
        } else if structural.is_some() {
            let mut lexical = LexState::default();
            let mut braces = brace_delta(lines[start], &mut lexical);
            let mut opened = braces > 0;
            while index < lines.len() {
                if !opened
                    && (lines[index].trim().is_empty() || structural_kind(lines[index]).is_some())
                {
                    break;
                };
                braces += brace_delta(lines[index], &mut lexical);
                opened |= braces > 0;
                index += 1;
                if opened && braces <= 0 {
                    break;
                }
            }
        } else {
            while index < lines.len()
                && !lines[index].trim().is_empty()
                && structural_kind(lines[index]).is_none()
            {
                index += 1;
            }
        }
        let range = start..index;
        result.push(InteractiveChunk {
            id: automatic_id(&range),
            range,
            kind: if structural.is_some() {
                ChunkKind::Structural
            } else {
                ChunkKind::Paragraph
            },
        });
    }
    result
}

#[derive(Clone, Copy)]
enum StructuralKind {
    Python { indent: usize },
    Braced,
}

fn structural_kind(line: &str) -> Option<StructuralKind> {
    let indent = leading_indent(line);
    let line = line.trim_start();
    if ["def ", "async def "]
        .iter()
        .any(|prefix| line.starts_with(prefix))
        || (line.starts_with("class ") && line.trim_end().ends_with(':'))
    {
        return Some(StructuralKind::Python { indent });
    };
    if [
        "fn ",
        "pub fn ",
        "async fn ",
        "class ",
        "struct ",
        "enum ",
        "impl ",
        "function ",
        "interface ",
        "type ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
        || looks_like_braced_declaration(line)
    {
        Some(StructuralKind::Braced)
    } else {
        None
    }
}

fn leading_indent(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn looks_like_braced_declaration(line: &str) -> bool {
    let line = line.trim_start();
    if ["if", "for", "while", "switch", "catch", "match"]
        .iter()
        .any(|keyword| line.starts_with(keyword) && line[keyword.len()..].starts_with([' ', '(']))
    {
        return false;
    };
    let Some(open) = line.find('(') else {
        return false;
    };
    let Some(close) = line[open..].find(')') else {
        return false;
    };
    line[..open].split_whitespace().count() >= 2
        && (line[open + close + 1..].contains('{') || line.trim_end().ends_with(')'))
}

#[derive(Default)]
struct LexState {
    block_comment: bool,
    quote: Option<u8>,
    escaped: bool,
}

fn brace_delta(line: &str, state: &mut LexState) -> isize {
    let bytes = line.as_bytes();
    let mut depth = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if state.block_comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                state.block_comment = false;
                index += 2
            } else {
                index += 1
            };
            continue;
        }
        if let Some(quote) = state.quote {
            if state.escaped {
                state.escaped = false
            } else if byte == b'\\' {
                state.escaped = true
            } else if byte == quote {
                state.quote = None
            };
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            break;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            state.block_comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            state.quote = Some(byte)
        } else if byte == b'{' {
            depth += 1
        } else if byte == b'}' {
            depth -= 1
        };
        index += 1;
    }
    if state.quote.is_some() && !state.escaped {
        state.quote = None
    };
    state.escaped = false;
    depth
}

fn automatic_id(range: &Range<usize>) -> u64 {
    (1u64 << 63) | ((range.start as u64) << 31) | range.end as u64
}
