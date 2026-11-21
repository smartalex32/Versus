//! Public-interface checks for lossless mapping across small, ambiguous inputs.

use versus::core::git_merge::{GitMergeModel, MergeChoice};

#[test]
fn choosing_one_source_across_all_regions_reconstructs_that_source_exactly() {
    // Repetition, adjacent changes, empty sides, EOF insertions, Unicode and
    // distinct line terminators exercise ambiguous alignment boundaries.
    let snapshots = [
        "",
        "a\n",
        "b\n",
        "a\nb\n",
        "b\na\n",
        "a\na\n",
        "a\nb\nc\n",
        "A\nb\nc\n",
        "a\nB\nc\n",
        "a\r\nb\r\n",
        "a\rb\r",
        "α\nβ\n",
        "a\nb",
    ];
    for incoming in snapshots {
        for current in snapshots {
            for initial in snapshots {
                for (choice, expected) in [
                    (MergeChoice::Incoming, incoming),
                    (MergeChoice::Current, current),
                ] {
                    let mut model =
                        GitMergeModel::new(None, incoming.into(), current.into(), initial.into())
                            .unwrap();
                    for _ in 0..20 {
                        if model.result() == expected {
                            break;
                        }
                        let region = model.regions().iter().find(|region| {
                            let source = match choice {
                                MergeChoice::Incoming => &incoming[region.incoming.clone()],
                                _ => &current[region.current.clone()],
                            };
                            source != &model.result()[region.result.clone()]
                        }).cloned().unwrap_or_else(|| panic!(
                            "No actionable region: {incoming:?}, {current:?}, {initial:?}, {choice:?}; result {:?}", model.result(),
                        ));
                        model.choose(region.id, model.revision(), choice).unwrap();
                    }
                    assert_eq!(
                        model.result(),
                        expected,
                        "{incoming:?}, {current:?}, {initial:?}, {choice:?}"
                    );
                }
            }
        }
    }
}
