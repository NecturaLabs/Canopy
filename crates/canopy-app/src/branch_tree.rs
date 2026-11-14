//! The sidebar's Branches as a tree: names grouped into folders by their `/` prefixes.
//!
//! The tree is built once per snapshot ([`build`]); each frame only walks it for the rows the
//! open folders show ([`visible`]).

use std::collections::HashSet;

use canopy_core::BranchInfo;

/// The key a folder's closed state is saved under in `RepoTab::collapsed`: folders are open
/// unless listed, like the sidebar's sections.
pub fn folder_key(prefix: &str) -> String {
    format!("folder:{prefix}")
}

/// One row of the tree, in display order: a folder is followed by everything inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Folder {
        /// The path so far, without the trailing slash: `feature/auth` for a folder inside
        /// `feature`.
        path: String,
        /// The last segment, as the row shows it.
        name: String,
        depth: usize,
        /// Branches anywhere inside.
        branches: usize,
        /// The rows after this one that belong to it.
        rows_inside: usize,
        /// The checked-out branch is inside.
        has_head: bool,
    },
    Branch {
        /// Index into the snapshot's `branches`.
        index: usize,
        /// The last segment of the name, as the row shows it.
        name: String,
        depth: usize,
    },
}

impl Node {
    pub fn depth(&self) -> usize {
        match self {
            Self::Folder { depth, .. } | Self::Branch { depth, .. } => *depth,
        }
    }
}

/// A row an open folder shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown {
    /// Index into the tree's nodes.
    pub node: usize,
    /// Position, among the shown rows, of the folder this one is in.
    pub parent: Option<usize>,
}

/// Groups `branches` by the `/` prefixes of their names. In each folder the branches come
/// first, then the folders, each in name order (case-insensitive).
pub fn build(branches: &[BranchInfo]) -> Vec<Node> {
    let items: Vec<(usize, &str)> = branches
        .iter()
        .enumerate()
        .map(|(i, b)| (i, b.name.as_str()))
        .collect();
    let mut out = Vec::with_capacity(branches.len());
    level(branches, items, "", 0, &mut out);
    out
}

fn level(
    branches: &[BranchInfo],
    mut items: Vec<(usize, &str)>,
    prefix: &str,
    depth: usize,
    out: &mut Vec<Node>,
) {
    items.sort_by(|a, b| {
        a.1.to_lowercase()
            .cmp(&b.1.to_lowercase())
            .then_with(|| a.1.cmp(b.1))
    });
    let (leaves, nested): (Vec<_>, Vec<_>) = items.into_iter().partition(|(_, rest)| {
        // A name that starts or ends with a slash cannot exist; keep it a plain row anyway.
        rest.split_once('/')
            .is_none_or(|(head, tail)| head.is_empty() || tail.is_empty())
    });
    for (index, rest) in leaves {
        out.push(Node::Branch {
            index,
            name: rest.to_owned(),
            depth,
        });
    }
    let mut folders: Vec<(&str, Vec<(usize, &str)>)> = Vec::new();
    for (index, rest) in nested {
        let Some((head, tail)) = rest.split_once('/') else {
            continue;
        };
        match folders.iter_mut().find(|(name, _)| *name == head) {
            Some((_, inside)) => inside.push((index, tail)),
            None => folders.push((head, vec![(index, tail)])),
        }
    }
    for (name, inside) in folders {
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let at = out.len();
        let count = inside.len();
        let has_head = inside.iter().any(|(i, _)| branches[*i].is_head);
        out.push(Node::Folder {
            path: path.clone(),
            name: name.to_owned(),
            depth,
            branches: count,
            rows_inside: 0,
            has_head,
        });
        level(branches, inside, &path, depth + 1, out);
        let inside_rows = out.len() - at - 1;
        if let Node::Folder { rows_inside, .. } = &mut out[at] {
            *rows_inside = inside_rows;
        }
    }
}

/// The rows to show: everything except what a closed folder holds.
pub fn visible(nodes: &[Node], closed: &HashSet<String>) -> Vec<Shown> {
    let mut out = Vec::with_capacity(nodes.len());
    // The open folders around the next row: (depth, position among the shown rows).
    let mut around: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < nodes.len() {
        let depth = nodes[i].depth();
        while around.last().is_some_and(|(d, _)| *d >= depth) {
            around.pop();
        }
        out.push(Shown {
            node: i,
            parent: around.last().map(|(_, at)| *at),
        });
        if let Node::Folder {
            path, rows_inside, ..
        } = &nodes[i]
        {
            if closed.contains(&folder_key(path)) {
                i += rows_inside;
            } else {
                around.push((depth, out.len() - 1));
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str, is_head: bool) -> BranchInfo {
        BranchInfo {
            name: name.to_owned(),
            target: None,
            is_head,
            upstream: None,
            ahead: 0,
            behind: 0,
            worktree: None,
        }
    }

    fn names(branches: &[BranchInfo], nodes: &[Node], shown: &[Shown]) -> Vec<String> {
        shown
            .iter()
            .map(|s| match &nodes[s.node] {
                Node::Folder { name, depth, .. } => format!("{}{name}/", "  ".repeat(*depth)),
                Node::Branch { index, depth, .. } => {
                    format!("{}{}", "  ".repeat(*depth), branches[*index].name)
                }
            })
            .collect()
    }

    fn sample() -> Vec<BranchInfo> {
        [
            "main",
            "feature/auth",
            "feature/graph",
            "feature/ui/dark",
            "feature/ui/light",
            "release/1.0",
            "develop",
            "Zeta",
        ]
        .into_iter()
        .map(|n| branch(n, n == "feature/ui/dark"))
        .collect()
    }

    #[test]
    fn names_group_into_nested_folders_with_branches_before_folders() {
        let branches = sample();
        let nodes = build(&branches);
        let all = visible(&nodes, &HashSet::new());
        assert_eq!(
            names(&branches, &nodes, &all),
            [
                "develop",
                "main",
                "Zeta",
                "feature/",
                "  feature/auth",
                "  feature/graph",
                "  ui/",
                "    feature/ui/dark",
                "    feature/ui/light",
                "release/",
                "  release/1.0",
            ]
        );
    }

    #[test]
    fn a_branch_without_a_prefix_stays_at_the_top_level() {
        let branches = [branch("main", false)];
        let nodes = build(&branches);
        assert_eq!(
            nodes,
            [Node::Branch {
                index: 0,
                name: "main".into(),
                depth: 0
            }]
        );
    }

    #[test]
    fn a_closed_folder_hides_what_it_holds_and_parents_are_positions_among_shown_rows() {
        let branches = sample();
        let nodes = build(&branches);
        let closed: HashSet<String> = [folder_key("feature/ui")].into();
        let shown = visible(&nodes, &closed);
        assert_eq!(
            names(&branches, &nodes, &shown),
            [
                "develop",
                "main",
                "Zeta",
                "feature/",
                "  feature/auth",
                "  feature/graph",
                "  ui/",
                "release/",
                "  release/1.0",
            ]
        );
        // `feature/auth` sits in `feature/` (shown row 3); `release/1.0` in `release/` (row 7).
        assert_eq!(shown[4].parent, Some(3));
        assert_eq!(shown[6].parent, Some(3));
        assert_eq!(shown[8].parent, Some(7));
        assert_eq!(shown[0].parent, None);
        assert_eq!(shown[7].parent, None);

        let closed: HashSet<String> = [folder_key("feature")].into();
        let shown = visible(&nodes, &closed);
        assert_eq!(
            names(&branches, &nodes, &shown),
            [
                "develop",
                "main",
                "Zeta",
                "feature/",
                "release/",
                "  release/1.0"
            ]
        );
        assert_eq!(shown[5].parent, Some(4));
    }

    #[test]
    fn a_folder_knows_its_branches_and_whether_the_checked_out_one_is_inside() {
        let branches = sample();
        let nodes = build(&branches);
        let feature = nodes
            .iter()
            .find(|n| matches!(n, Node::Folder { path, .. } if path == "feature"))
            .expect("folder");
        assert!(matches!(
            feature,
            Node::Folder {
                branches: 4,
                rows_inside: 5,
                has_head: true,
                ..
            }
        ));
        let release = nodes
            .iter()
            .find(|n| matches!(n, Node::Folder { path, .. } if path == "release"))
            .expect("folder");
        assert!(matches!(
            release,
            Node::Folder {
                has_head: false,
                ..
            }
        ));
    }

    #[test]
    fn an_empty_repository_has_no_rows() {
        assert_eq!(build(&[]).len(), 0);
        assert_eq!(visible(&[], &HashSet::new()).len(), 0);
    }
}
