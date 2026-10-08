use super::{DirectoryDiff, DirectoryEntry, DirectoryEntryKind, DirectoryEntryState};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeSide {
    pub exists: bool,
    pub kind: Option<DirectoryEntryKind>,
    pub size: Option<u64>,
}

impl TreeSide {
    fn missing() -> Self {
        Self {
            exists: false,
            kind: None,
            size: None,
        }
    }

    fn directory() -> Self {
        Self {
            exists: true,
            kind: Some(DirectoryEntryKind::Directory),
            size: None,
        }
    }
}

/// A node that can be rendered in both folder panes at the same relative path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeNode {
    pub relative_path: PathBuf,
    pub name: OsString,
    pub left: TreeSide,
    pub right: TreeSide,
    pub state: DirectoryEntryState,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    pub fn is_directory(&self) -> bool {
        matches!(self.left.kind, Some(DirectoryEntryKind::Directory))
            || matches!(self.right.kind, Some(DirectoryEntryKind::Directory))
            || !self.children.is_empty()
    }

    pub fn is_expandable(&self) -> bool {
        !self.children.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FolderTree {
    root: TreeNode,
    expanded: BTreeSet<PathBuf>,
}

/// A row to render in either pane. Both panes use the same row sequence.
#[derive(Clone, Copy, Debug)]
pub struct VisibleTreeRow<'a> {
    pub node: &'a TreeNode,
    pub depth: usize,
}

impl FolderTree {
    pub fn from_diff(diff: &DirectoryDiff) -> Self {
        let mut draft = DraftNode::default();
        for entry in &diff.entries {
            draft.insert(entry);
        }
        let mut root = draft.finish(PathBuf::new(), OsString::new());
        // The compared roots are valid directories even though they are not part
        // of DirectoryDiff.entries. Exposing their totals keeps the tree model
        // internally consistent, including for empty folders.
        let left_had_root_error = root.left.exists && root.left.kind.is_none();
        let right_had_root_error = root.right.exists && root.right.kind.is_none();
        root.left = TreeSide {
            exists: true,
            kind: Some(DirectoryEntryKind::Directory),
            size: (!left_had_root_error)
                .then(|| aggregate_child_size(&root.children, |child| &child.left))
                .flatten(),
        };
        root.right = TreeSide {
            exists: true,
            kind: Some(DirectoryEntryKind::Directory),
            size: (!right_had_root_error)
                .then(|| aggregate_child_size(&root.children, |child| &child.right))
                .flatten(),
        };
        Self {
            root,
            expanded: BTreeSet::new(),
        }
    }

    pub fn root(&self) -> &TreeNode {
        &self.root
    }

    pub fn is_expanded(&self, relative_path: impl AsRef<Path>) -> bool {
        self.expanded.contains(relative_path.as_ref())
    }

    pub fn set_expanded(&mut self, relative_path: impl Into<PathBuf>, expanded: bool) {
        let relative_path = relative_path.into();
        if expanded {
            self.expanded.insert(relative_path);
        } else {
            self.expanded.remove(&relative_path);
        }
    }

    pub fn toggle_expanded(&mut self, relative_path: impl Into<PathBuf>) -> bool {
        let relative_path = relative_path.into();
        if !self.expanded.insert(relative_path.clone()) {
            self.expanded.remove(&relative_path);
            false
        } else {
            true
        }
    }

    pub fn expand_all(&mut self) {
        let mut paths = Vec::new();
        collect_expandable_paths(&self.root, &mut paths);
        self.expanded.extend(paths);
    }

    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }

    pub fn visible_rows(&self) -> Vec<VisibleTreeRow<'_>> {
        let mut rows = Vec::new();
        self.collect_visible(&self.root, 0, &mut rows);
        rows
    }

    /// Returns every non-root node in the same depth-first order as
    /// [`Self::visible_rows`], regardless of the current expansion state.
    pub fn all_rows(&self) -> Vec<VisibleTreeRow<'_>> {
        let mut rows = Vec::new();
        Self::collect_all(&self.root, 0, &mut rows);
        rows
    }

    /// Reveals `relative_path` by expanding every expandable ancestor. Returns
    /// `true` when the target exists in this tree.
    pub fn expand_parents(&mut self, relative_path: impl AsRef<Path>) -> bool {
        let relative_path = relative_path.as_ref();
        let Some(target) = find_node(&self.root, relative_path) else {
            return false;
        };
        let mut ancestor = target.relative_path.parent();
        while let Some(path) = ancestor {
            if !path.as_os_str().is_empty() {
                self.expanded.insert(path.to_path_buf());
            }
            ancestor = path.parent();
        }
        true
    }

    fn collect_visible<'a>(
        &'a self,
        parent: &'a TreeNode,
        depth: usize,
        rows: &mut Vec<VisibleTreeRow<'a>>,
    ) {
        for child in &parent.children {
            rows.push(VisibleTreeRow { node: child, depth });
            if self.is_expanded(&child.relative_path) {
                self.collect_visible(child, depth + 1, rows);
            }
        }
    }

    fn collect_all<'a>(parent: &'a TreeNode, depth: usize, rows: &mut Vec<VisibleTreeRow<'a>>) {
        for child in &parent.children {
            rows.push(VisibleTreeRow { node: child, depth });
            Self::collect_all(child, depth + 1, rows);
        }
    }
}

fn find_node<'a>(root: &'a TreeNode, relative_path: &Path) -> Option<&'a TreeNode> {
    if relative_path.as_os_str().is_empty() {
        return Some(root);
    }
    let mut current = root;
    for component in relative_path.components() {
        current = current
            .children
            .iter()
            .find(|child| child.name == component.as_os_str())?;
    }
    Some(current)
}

fn aggregate_child_size(
    children: &[TreeNode],
    side: impl Fn(&TreeNode) -> &TreeSide,
) -> Option<u64> {
    children.iter().try_fold(0_u64, |total, child| {
        let side = side(child);
        match (&side.kind, side.size) {
            (Some(DirectoryEntryKind::File | DirectoryEntryKind::Directory), Some(size)) => {
                total.checked_add(size)
            }
            (Some(DirectoryEntryKind::File | DirectoryEntryKind::Directory), None) | (None, _)
                if side.exists =>
            {
                None
            }
            _ => Some(total),
        }
    })
}

fn collect_expandable_paths(node: &TreeNode, paths: &mut Vec<PathBuf>) {
    for child in &node.children {
        if child.is_expandable() {
            paths.push(child.relative_path.clone());
            collect_expandable_paths(child, paths);
        }
    }
}

#[derive(Default)]
struct DraftNode {
    entry: Option<DirectoryEntry>,
    children: BTreeMap<OsString, DraftNode>,
}

impl DraftNode {
    fn insert(&mut self, entry: &DirectoryEntry) {
        let mut node = self;
        for component in entry.relative_path.components() {
            let name = component.as_os_str().to_os_string();
            node = node.children.entry(name).or_default();
        }
        node.entry = Some(entry.clone());
    }

    fn finish(self, relative_path: PathBuf, name: OsString) -> TreeNode {
        let mut children: Vec<_> = self
            .children
            .into_iter()
            .map(|(child_name, child)| child.finish(relative_path.join(&child_name), child_name))
            .collect();
        children.sort_by(|a, b| {
            b.is_directory()
                .cmp(&a.is_directory())
                .then_with(|| a.name.cmp(&b.name))
        });

        let (mut left, mut right, direct_state) = match self.entry {
            Some(entry) => sides_from_entry(&entry),
            None => (TreeSide::missing(), TreeSide::missing(), None),
        };
        for child in &children {
            derive_parent_side(&mut left, &child.left);
            derive_parent_side(&mut right, &child.right);
        }
        let state = aggregate_state(direct_state, &left, &right, &children);
        TreeNode {
            relative_path,
            name,
            left,
            right,
            state,
            children,
        }
    }
}

fn sides_from_entry(entry: &DirectoryEntry) -> (TreeSide, TreeSide, Option<DirectoryEntryState>) {
    (
        TreeSide {
            exists: entry.left_exists,
            kind: entry.left_kind.clone(),
            size: entry.left_size,
        },
        TreeSide {
            exists: entry.right_exists,
            kind: entry.right_kind.clone(),
            size: entry.right_size,
        },
        Some(entry.state.clone()),
    )
}

fn derive_parent_side(parent: &mut TreeSide, child: &TreeSide) {
    if !parent.exists && child.exists {
        *parent = TreeSide::directory();
    }
}

fn aggregate_state(
    direct_state: Option<DirectoryEntryState>,
    left: &TreeSide,
    right: &TreeSide,
    children: &[TreeNode],
) -> DirectoryEntryState {
    use DirectoryEntryState::*;
    if let Some(state @ (LeftOnly | RightOnly | TypeMismatch | Error(_) | Different)) = direct_state
    {
        return state;
    }
    if let Some(error) = children.iter().find_map(|child| match &child.state {
        Error(error) => Some(error.clone()),
        _ => None,
    }) {
        return Error(error);
    }
    if children.iter().any(|child| child.state != Same) {
        return Different;
    }
    match (left.exists, right.exists) {
        (true, false) => LeftOnly,
        (false, true) => RightOnly,
        _ => Same,
    }
}
