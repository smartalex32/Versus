//! Stateful, UI-independent three-way editing support for Git mergetool.
//!
//! The model deliberately owns no paths, UI state, or Git process state.  It
//! preserves Git's supplied result verbatim and exposes byte-accurate source to
//! result replacements for a focused three-pane view.

mod actions;
mod conflicts;
mod history;
mod ledger;
mod mapping;
mod regions;

#[cfg(test)]
#[path = "../../tests/unit/core/git_merge/mod.rs"]
mod tests;

use std::{fmt, ops::Range};

use history::{History, Snapshot};

const REGULAR_REGION_ID_START: usize = 1_000_000;

/// A source choice for a mapped Git merge region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeChoice {
    Incoming,
    Current,
    IncomingThenCurrent,
    CurrentThenIncoming,
}

/// One replacement mapping. All offsets are UTF-8 byte offsets in the named
/// snapshot. A missing Base means Git did not provide a common ancestor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeRegion {
    pub id: usize,
    pub incoming: Range<usize>,
    pub current: Range<usize>,
    pub result: Range<usize>,
    pub base: Option<Range<usize>>,
    pub unresolved: bool,
}

/// An action could not be safely applied to the current Result snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitMergeError {
    MalformedConflictMarkers,
    StaleRegion { expected: u64, actual: u64 },
    UnknownRegion(usize),
    RegionUnavailable(usize),
    ConflictStillHasMarkers(usize),
    BaseUnavailable,
    MappingUnavailable,
    InputTooLarge,
}

impl fmt::Display for GitMergeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedConflictMarkers => {
                write!(formatter, "Git conflict markers are malformed")
            }
            Self::StaleRegion { expected, actual } => write!(
                formatter,
                "region revision {expected} is stale; current revision is {actual}"
            ),
            Self::UnknownRegion(id) => write!(formatter, "Git merge region {id} no longer exists"),
            Self::RegionUnavailable(id) => {
                write!(formatter, "Git merge region {id} cannot be safely mapped")
            }
            Self::ConflictStillHasMarkers(id) => write!(
                formatter,
                "Git merge region {id} still contains conflict markers"
            ),
            Self::BaseUnavailable => write!(formatter, "Git did not supply a common ancestor"),
            Self::MappingUnavailable => {
                write!(formatter, "Git merge mapping exceeds its safe work limit")
            }
            Self::InputTooLarge => write!(formatter, "Git merge text exceeds the 32 MiB limit"),
        }
    }
}

impl std::error::Error for GitMergeError {}

#[derive(Clone, Debug)]
struct ConflictState {
    id: usize,
    wrapper: Option<Range<usize>>,
    envelope: Range<usize>,
    resolved: bool,
}

#[derive(Clone, Debug)]
enum RegionKind {
    Regular,
    Conflicts(Vec<usize>),
}

#[derive(Clone, Debug)]
struct InternalRegion {
    public: MergeRegion,
    kind: RegionKind,
}

/// The editable state for one Git mergetool invocation.
#[derive(Debug)]
pub struct GitMergeModel {
    base: Option<String>,
    incoming: String,
    current: String,
    result: String,
    conflicts: Vec<ConflictState>,
    regions: Vec<InternalRegion>,
    public_regions: Vec<MergeRegion>,
    malformed_markers: bool,
    mapping_available: bool,
    revision: u64,
    history: History,
}

impl GitMergeModel {
    /// Creates a model from immutable Git snapshots and Git's current partial
    /// merge result. A supplied empty Base remains distinct from no Base.
    pub fn new(
        base: Option<String>,
        incoming: String,
        current: String,
        result: String,
    ) -> Result<Self, GitMergeError> {
        if [
            base.as_deref(),
            Some(incoming.as_str()),
            Some(current.as_str()),
            Some(result.as_str()),
        ]
        .into_iter()
        .flatten()
        .any(|text| text.len() > super::DEFAULT_TEXT_SIZE_LIMIT as usize)
        {
            return Err(GitMergeError::InputTooLarge);
        }
        let parsed = conflicts::parse(&result)?;
        let conflict_states: Vec<ConflictState> = parsed
            .iter()
            .enumerate()
            .map(|(id, conflict)| ConflictState {
                id,
                wrapper: Some(conflict.wrapper.clone()),
                envelope: conflict.wrapper.clone(),
                resolved: false,
            })
            .collect();
        let initial = Snapshot {
            result: result.clone(),
            conflicts: conflict_states.clone(),
        };
        let mut model = Self {
            base,
            incoming,
            current,
            result,
            conflicts: conflict_states,
            regions: Vec::new(),
            public_regions: Vec::new(),
            malformed_markers: false,
            mapping_available: true,
            revision: 0,
            history: History::new(initial),
        };
        model.regenerate_regions();
        Ok(model)
    }

    pub fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }
    pub fn incoming(&self) -> &str {
        &self.incoming
    }
    pub fn current(&self) -> &str {
        &self.current
    }
    pub fn result(&self) -> &str {
        &self.result
    }
    pub fn regions(&self) -> &[MergeRegion] {
        &self.public_regions
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn unresolved_count(&self) -> usize {
        self.conflicts
            .iter()
            .filter(|conflict| !conflict.resolved)
            .count()
    }
    pub fn can_finish(&self) -> bool {
        !self.malformed_markers
            && self.unresolved_count() == 0
            && self
                .conflicts
                .iter()
                .all(|conflict| conflict.wrapper.is_none())
    }
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
    pub fn mapping_available(&self) -> bool {
        self.mapping_available
    }
}
