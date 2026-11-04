//! Terminal pane split layout.
//!
//! A split node owns only pane geometry. Tabs remain owned by `State`, so
//! closing a tab collapses exactly one leaf and switching tabs never duplicates
//! a session. Ratios are clamped to keep both panes usable.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Axis {
    /// Children side by side (first on the left).
    Horizontal,
    /// Children stacked (first on top).
    Vertical,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SplitNode {
    Leaf {
        tab: String,
    },
    Split {
        id: u64,
        axis: Axis,
        ratio: f32,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

impl SplitNode {
    pub fn leaf(tab: String) -> Self {
        Self::Leaf { tab }
    }

    pub fn contains(&self, tab: &str) -> bool {
        match self {
            Self::Leaf { tab: candidate } => candidate == tab,
            Self::Split { first, second, .. } => first.contains(tab) || second.contains(tab),
        }
    }

    pub fn replace_tab(&mut self, old: &str, new: &str) -> bool {
        match self {
            Self::Leaf { tab } => {
                if tab == old {
                    *tab = new.to_string();
                    true
                } else {
                    false
                }
            }
            Self::Split { first, second, .. } => {
                first.replace_tab(old, new) || second.replace_tab(old, new)
            }
        }
    }

    pub fn split_leaf(&mut self, target: &str, id: u64, axis: Axis, new_tab: &str) -> bool {
        match self {
            Self::Leaf { tab } if tab == target => {
                let first = Box::new(Self::Leaf {
                    tab: target.to_string(),
                });
                let second = Box::new(Self::Leaf {
                    tab: new_tab.to_string(),
                });
                *self = Self::Split {
                    id,
                    axis,
                    ratio: 0.5,
                    first,
                    second,
                };
                true
            }
            Self::Leaf { .. } => false,
            Self::Split { first, second, .. } => {
                first.split_leaf(target, id, axis, new_tab)
                    || second.split_leaf(target, id, axis, new_tab)
            }
        }
    }

    pub fn remove_tab(&mut self, tab: &str) {
        if self.contains(tab) {
            let old = std::mem::replace(self, Self::Leaf { tab: String::new() });
            if let Some(node) = remove_from(old, tab) {
                *self = node;
            } else {
                *self = Self::Leaf { tab: String::new() };
            }
        }
    }

    pub fn set_ratio(&mut self, id: u64, ratio: f32) -> bool {
        match self {
            Self::Leaf { .. } => false,
            Self::Split {
                id: candidate,
                ratio: value,
                first,
                second,
                ..
            } => {
                if *candidate == id {
                    *value = ratio.clamp(0.15, 0.85);
                    true
                } else {
                    first.set_ratio(id, ratio) || second.set_ratio(id, ratio)
                }
            }
        }
    }

    pub fn ratio(&self, id: u64) -> Option<f32> {
        match self {
            Self::Leaf { .. } => None,
            Self::Split {
                id: candidate,
                ratio,
                first,
                second,
                ..
            } => {
                if *candidate == id {
                    Some(*ratio)
                } else {
                    first.ratio(id).or_else(|| second.ratio(id))
                }
            }
        }
    }

    pub fn leaves(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.walk_leaves(&mut |tab| {
            if !tab.is_empty() {
                out.push(tab.to_string());
            }
        });
        out
    }

    pub fn walk_leaves(&self, visit: &mut impl FnMut(&str)) {
        match self {
            Self::Leaf { tab } => visit(tab),
            Self::Split { first, second, .. } => {
                first.walk_leaves(visit);
                second.walk_leaves(visit);
            }
        }
    }

    /// Pane sizes in the same coordinate space used by the dock. The divider
    /// itself consumes six pixels, matching the rendered drag handle.
    pub fn sizes(&self, width: f32, height: f32) -> Vec<(String, (f32, f32))> {
        let mut out = Vec::new();
        self.collect_sizes(width, height, &mut out);
        out
    }

    fn collect_sizes(&self, width: f32, height: f32, out: &mut Vec<(String, (f32, f32))>) {
        match self {
            Self::Leaf { tab } => out.push((tab.clone(), (width.max(1.), height.max(1.)))),
            Self::Split {
                axis,
                ratio,
                first,
                second,
                ..
            } => {
                const DIVIDER: f32 = 6.;
                match axis {
                    Axis::Horizontal => {
                        let usable = (width - DIVIDER).max(2.);
                        let first_width = usable * ratio.clamp(0.15, 0.85);
                        first.collect_sizes(first_width, height, out);
                        second.collect_sizes(usable - first_width, height, out);
                    }
                    Axis::Vertical => {
                        let usable = (height - DIVIDER).max(2.);
                        let first_height = usable * ratio.clamp(0.15, 0.85);
                        first.collect_sizes(width, first_height, out);
                        second.collect_sizes(width, usable - first_height, out);
                    }
                }
            }
        }
    }

    /// Extent available to one split divider, used to translate pointer deltas
    /// into ratio changes for nested layouts.
    pub fn extent(&self, id: u64, width: f32, height: f32) -> Option<(Axis, f32)> {
        match self {
            Self::Leaf { .. } => None,
            Self::Split {
                id: candidate,
                axis,
                ratio,
                first,
                second,
            } => {
                if *candidate == id {
                    Some((
                        *axis,
                        match axis {
                            Axis::Horizontal => width,
                            Axis::Vertical => height,
                        }
                        .max(1.),
                    ))
                } else {
                    let divider = 6.0;
                    match axis {
                        Axis::Horizontal => {
                            let usable = (width - divider).max(2.);
                            let first_width = usable * ratio.clamp(0.15, 0.85);
                            first
                                .extent(id, first_width, height)
                                .or_else(|| second.extent(id, usable - first_width, height))
                        }
                        Axis::Vertical => {
                            let usable = (height - divider).max(2.);
                            let first_height = usable * ratio.clamp(0.15, 0.85);
                            first
                                .extent(id, width, first_height)
                                .or_else(|| second.extent(id, width, usable - first_height))
                        }
                    }
                }
            }
        }
    }
}

fn remove_from(node: SplitNode, tab: &str) -> Option<SplitNode> {
    match node {
        SplitNode::Leaf { tab: candidate } if candidate == tab => None,
        SplitNode::Leaf { .. } => Some(node),
        SplitNode::Split {
            id,
            axis,
            ratio,
            first,
            second,
        } => {
            let first = remove_from(*first, tab);
            let second = remove_from(*second, tab);
            match (first, second) {
                (None, None) => None,
                (Some(node), None) | (None, Some(node)) => Some(node),
                (Some(first), Some(second)) => Some(SplitNode::Split {
                    id,
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> SplitNode {
        SplitNode::leaf("a".into())
    }

    #[test]
    fn splits_and_collapses_without_losing_sessions() {
        let mut node = root();
        assert!(node.split_leaf("a", 1, Axis::Horizontal, "b"));
        assert!(node.contains("a") && node.contains("b"));
        assert!(node.split_leaf("b", 2, Axis::Vertical, "c"));
        assert_eq!(node.leaves(), vec!["a", "b", "c"]);
        node.set_ratio(1, 0.7);
        node.remove_tab("b");
        assert_eq!(node.leaves(), vec!["a", "c"]);
        node.remove_tab("a");
        assert_eq!(node.leaves(), vec!["c"]);
        node.remove_tab("c");
        assert!(node.leaves().is_empty());
    }

    #[test]
    fn sizes_include_the_divider_and_keep_both_panes_positive() {
        let mut node = root();
        node.split_leaf("a", 1, Axis::Horizontal, "b");
        let sizes = node.sizes(800., 300.);
        assert_eq!(sizes.len(), 2);
        assert!((sizes[0].1.0 + sizes[1].1.0 - 794.).abs() < 0.01);
        node.split_leaf("b", 2, Axis::Vertical, "c");
        let sizes = node.sizes(800., 300.);
        assert_eq!(sizes.len(), 3);
        assert!(
            sizes
                .iter()
                .all(|(_, (width, height))| *width > 0. && *height > 0.)
        );
    }

    #[test]
    fn ratio_is_clamped_to_usable_bounds() {
        let mut node = root();
        node.split_leaf("a", 1, Axis::Horizontal, "b");
        assert!(node.set_ratio(1, 0.99));
        let sizes = node.sizes(100., 100.);
        assert!(sizes[0].1.0 >= 12.);
        assert!(sizes[1].1.0 >= 12.);
    }

    #[test]
    fn replacing_a_visible_tab_keeps_the_leaf_shape() {
        let mut node = root();
        node.split_leaf("a", 1, Axis::Horizontal, "b");
        assert!(node.replace_tab("b", "c"));
        assert_eq!(node.leaves(), vec!["a", "c"]);
        assert!(!node.replace_tab("missing", "x"));
    }
}
