//! Bounded Result-coordinate regions and immutable source projections.

use super::ledger::ranges_overlap;
use super::regions::{Hunk, close_ranges, hunks, map_range, union_ranges};
use super::*;
use std::time::{Duration, Instant};

impl GitMergeModel {
    pub(super) fn regenerate_regions(&mut self) {
        let parsed = conflicts::parse(&self.result);
        let Ok(parsed) = parsed else {
            self.malformed_markers = true;
            self.mapping_available = false;
            for conflict in &mut self.conflicts {
                conflict.wrapper = None;
            }
            self.publish_conflict_regions();
            return;
        };
        self.malformed_markers = false;
        self.sync_conflicts(parsed);

        let deadline = Instant::now() + Duration::from_millis(150);
        let incoming = hunks(&self.incoming, &self.result, deadline);
        let current = hunks(&self.current, &self.result, deadline);
        let base_hunks = self
            .base
            .as_deref()
            .map(|base| hunks(base, &self.result, deadline));
        self.mapping_available = incoming.is_some()
            && current.is_some()
            && base_hunks.as_ref().is_none_or(Option::is_some);
        if !self.mapping_available {
            self.publish_conflict_regions();
            return;
        }
        let incoming = incoming.expect("mapping availability checked");
        let current = current.expect("mapping availability checked");
        let base_hunks = base_hunks.flatten();
        let mut result_ranges =
            union_ranges(&[&incoming, &current, base_hunks.as_deref().unwrap_or(&[])]);
        for conflict in &self.conflicts {
            result_ranges.push(conflict.envelope.clone());
        }
        let result_ranges = close_ranges(result_ranges);

        let mut regions = Vec::new();
        for (index, result) in result_ranges.into_iter().enumerate() {
            let conflict_ids: Vec<_> = self
                .conflicts
                .iter()
                .filter(|conflict| ranges_overlap(&result, &conflict.envelope))
                .map(|conflict| conflict.id)
                .collect();
            if !conflict_ids.is_empty() {
                regions.push(InternalRegion {
                    public: MergeRegion {
                        id: REGULAR_REGION_ID_START + index,
                        incoming: map_result_to_source(&result, &incoming, self.incoming.len()),
                        current: map_result_to_source(&result, &current, self.current.len()),
                        base: base_hunks.as_deref().map(|hunks| {
                            map_result_to_source(
                                &result,
                                hunks,
                                self.base.as_deref().unwrap_or("").len(),
                            )
                        }),
                        result,
                        unresolved: conflict_ids.iter().any(|id| {
                            self.conflicts
                                .iter()
                                .any(|conflict| conflict.id == *id && !conflict.resolved)
                        }),
                    },
                    kind: RegionKind::Conflicts(conflict_ids),
                });
                continue;
            }
            let base = base_hunks.as_deref().map(|hunks| {
                map_result_to_source(&result, hunks, self.base.as_deref().unwrap_or("").len())
            });
            regions.push(InternalRegion {
                public: MergeRegion {
                    id: REGULAR_REGION_ID_START + index,
                    incoming: map_result_to_source(&result, &incoming, self.incoming.len()),
                    current: map_result_to_source(&result, &current, self.current.len()),
                    result,
                    base,
                    unresolved: false,
                },
                kind: RegionKind::Regular,
            });
        }
        self.public_regions = regions.iter().map(|region| region.public.clone()).collect();
        self.regions = regions;
    }

    fn publish_conflict_regions(&mut self) {
        self.regions = self
            .conflicts
            .iter()
            .enumerate()
            .map(|(index, conflict)| InternalRegion {
                public: MergeRegion {
                    id: REGULAR_REGION_ID_START + index,
                    incoming: 0..0,
                    current: 0..0,
                    result: conflict.envelope.clone(),
                    base: self.base.as_ref().map(|_| 0..0),
                    unresolved: !conflict.resolved,
                },
                kind: RegionKind::Conflicts(vec![conflict.id]),
            })
            .collect();
        self.public_regions = self
            .regions
            .iter()
            .map(|region| region.public.clone())
            .collect();
    }
}

fn map_result_to_source(
    result: &Range<usize>,
    source_to_result: &[Hunk],
    source_len: usize,
) -> Range<usize> {
    let reversed: Vec<_> = source_to_result
        .iter()
        .map(|hunk| Hunk {
            base: hunk.target.clone(),
            target: hunk.base.clone(),
        })
        .collect();
    map_range(result, &reversed, source_len)
}
