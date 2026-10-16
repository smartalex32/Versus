//! Test-only inspection helpers for projection regressions.

use super::*;

impl MergePresentation {
    pub(in crate::app::interactive_view) fn editor_projection(
        &self,
        side: usize,
    ) -> Option<&EditorProjection> {
        self.editor_projections[side].as_ref()
    }
}

impl MergeDecoration {
    pub(in crate::app::interactive_view) fn lines(
        &self,
    ) -> &BTreeMap<usize, (DirectoryEntryState, Vec<Range<usize>>)> {
        &self.lines
    }

    pub(in crate::app::interactive_view) fn lines_mut(
        &mut self,
    ) -> &mut BTreeMap<usize, (DirectoryEntryState, Vec<Range<usize>>)> {
        &mut self.lines
    }

    pub(in crate::app::interactive_view) fn endings_mut(
        &mut self,
    ) -> &mut BTreeMap<usize, versus::DisplayLineEnding> {
        &mut self.endings
    }

    pub(in crate::app::interactive_view) fn ending(
        &self,
        line: usize,
    ) -> Option<versus::DisplayLineEnding> {
        self.endings.get(&line).copied()
    }
}

impl EditorProjection {
    pub(in crate::app::interactive_view) fn text(&self) -> &str {
        &self.text
    }

    pub(in crate::app::interactive_view) fn source_lines(&self) -> &[usize] {
        &self.source_lines
    }
}
