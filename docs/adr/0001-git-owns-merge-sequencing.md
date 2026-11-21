# Git owns merge sequencing and staging

Versus handles one file per Git mergetool invocation and reports success only
after the user chooses Save and Resolve and the result is saved successfully.
Git owns selecting the next conflicted file and staging successful resolutions.
We considered a repository-wide conflict workspace, but chose Git's standard
per-file contract to reuse its sequencing and repository state handling without
introducing another owner of the merge operation.

Cancellation reports failure; Git may restore its pre-tool copy of the file, so
this workflow does not promise persistent drafts after cancellation. See the
[workflow design](../design/git-tool-workflows.md) for the result editing model.
