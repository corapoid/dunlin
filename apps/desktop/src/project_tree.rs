//! Persistent logical hierarchy only: no filesystem moves or context expansion.
use dunlin_domain::{Project, ProjectGroup};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum View {
    List,
    #[default]
    Tree,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Sort {
    #[default]
    Name,
    Modified,
}

pub(crate) fn descendants(groups: &[ProjectGroup], root: &str) -> HashSet<String> {
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for group in groups {
        if let Some(parent) = group.parent_id.as_deref() {
            children.entry(parent).or_default().push(&group.id);
        }
    }
    let mut seen = HashSet::from([root.to_string()]);
    let mut queue = vec![root];
    while let Some(id) = queue.pop() {
        for child in children.get(id).into_iter().flatten() {
            if seen.insert((*child).to_string()) {
                queue.push(child);
            }
        }
    }
    seen
}
pub(crate) fn group_path(groups: &[ProjectGroup], id: &str) -> String {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut next = Some(id);
    while let Some(id) = next {
        if !seen.insert(id) {
            break;
        }
        let Some(group) = groups.iter().find(|g| g.id == id) else {
            break;
        };
        names.push(group.name.as_str());
        next = group.parent_id.as_deref();
    }
    names.reverse();
    names.join(" / ")
}
pub(crate) fn sort_projects(
    projects: &[Project],
    indices: &mut [usize],
    sort: Sort,
    modified: &HashMap<String, u64>,
) {
    indices.sort_by(|a, b| {
        let (a, b) = (&projects[*a], &projects[*b]);
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| {
                if sort == Sort::Modified {
                    modified
                        .get(&b.id)
                        .copied()
                        .unwrap_or(0)
                        .cmp(&modified.get(&a.id).copied().unwrap_or(0))
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
}
pub(crate) fn visible_groups(
    groups: &[ProjectGroup],
    projects: &[Project],
    collapsed: &[String],
    sort: Sort,
    modified: &HashMap<String, u64>,
) -> Vec<(usize, usize)> {
    let by_id: HashMap<_, _> = groups
        .iter()
        .enumerate()
        .map(|(i, g)| (g.id.as_str(), i))
        .collect();
    // Resolve parent validity once per node, without recursion or repeatedly
    // walking an entire deep hierarchy on each render.
    let mut valid = vec![None; groups.len()];
    for index in 0..groups.len() {
        if valid[index].is_some() {
            continue;
        }
        let mut path = Vec::new();
        let mut seen = HashSet::new();
        let mut next = index;
        let result = loop {
            if let Some(result) = valid[next] {
                break result;
            }
            if !seen.insert(next) {
                break false;
            }
            path.push(next);
            match groups[next].parent_id.as_deref() {
                None => break true,
                Some(parent) => match by_id.get(parent) {
                    Some(index) => next = *index,
                    None => break false,
                },
            }
        };
        for index in path {
            valid[index] = Some(result);
        }
    }
    let mut children = vec![Vec::new(); groups.len()];
    let mut parents = vec![None; groups.len()];
    let mut roots = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        // Corrupt/orphaned persisted links remain visible as roots; never lose
        // access to their projects or recurse forever during recovery.
        let parent = if valid[index] == Some(true) {
            group
                .parent_id
                .as_deref()
                .and_then(|id| by_id.get(id).copied())
        } else {
            None
        };
        parents[index] = parent;
        if let Some(parent) = parent {
            children[parent].push(index);
        } else {
            roots.push(index);
        }
    }
    let mut times = vec![0; groups.len()];
    if sort == Sort::Modified {
        for project in projects {
            if let Some(index) = project.group_id.as_deref().and_then(|id| by_id.get(id)) {
                times[*index] = times[*index].max(modified.get(&project.id).copied().unwrap_or(0));
            }
        }
        let mut traversal = roots.clone();
        let mut cursor = 0;
        while cursor < traversal.len() {
            let index = traversal[cursor];
            traversal.extend_from_slice(&children[index]);
            cursor += 1;
        }
        for index in traversal.into_iter().rev() {
            if let Some(parent) = parents[index] {
                times[parent] = times[parent].max(times[index]);
            }
        }
    }
    for indices in children.iter_mut().chain(std::iter::once(&mut roots)) {
        indices.sort_by(|a, b| {
            (if sort == Sort::Modified {
                times[*b].cmp(&times[*a])
            } else {
                std::cmp::Ordering::Equal
            })
            .then_with(|| {
                groups[*a]
                    .name
                    .to_lowercase()
                    .cmp(&groups[*b].name.to_lowercase())
            })
            .then_with(|| groups[*a].id.cmp(&groups[*b].id))
        });
    }
    let mut stack: Vec<_> = roots.iter().rev().map(|i| (*i, 0)).collect();
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    while let Some((index, depth)) = stack.pop() {
        let group = &groups[index];
        if !seen.insert(group.id.clone()) {
            continue;
        }
        rows.push((index, depth));
        if !collapsed.contains(&group.id) {
            stack.extend(children[index].iter().rev().map(|i| (*i, depth + 1)));
        }
    }
    rows
}

impl crate::app::DunlinApp {
    pub(crate) fn open_subgroup_prompt(
        &mut self,
        parent: Option<String>,
        window: &mut dunlin_ui::Window,
        cx: &mut dunlin_ui::Context<Self>,
    ) {
        if parent
            .as_ref()
            .is_some_and(|id| !self.project_groups.iter().any(|g| &g.id == id))
        {
            return;
        }
        self.open_new_group_prompt(None, window, cx);
        if let Some(prompt) = &mut self.group_prompt {
            prompt.parent_id = parent;
        }
    }
    pub(crate) fn move_group(
        &mut self,
        id: &str,
        parent: Option<String>,
        cx: &mut dunlin_ui::Context<Self>,
    ) {
        let Some(index) = self.project_groups.iter().position(|g| g.id == id) else {
            return;
        };
        let mut group = self.project_groups[index].clone();
        group.parent_id = parent.clone();
        if let Err(error) = group.validate_parent(&self.project_groups) {
            self.file_notice(error, cx);
            return;
        }
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_project_group(&group)
        {
            self.file_notice(error.to_string(), cx);
            return;
        }
        self.project_groups[index] = group;
        if let Some(toast) = &mut self.toast {
            toast.action.member_moved(id, true);
        }
        if let Some(parent) = parent {
            self.collapsed_project_groups.retain(|id| id != &parent);
        }
        self.persist_settings(cx);
        self.set_notice(
            "Group moved · projects and existing agent scopes are unchanged",
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Sort, descendants, group_path, visible_groups};
    fn group(id: &str, parent: Option<&str>) -> dunlin_domain::ProjectGroup {
        dunlin_domain::ProjectGroup {
            id: id.into(),
            name: id.into(),
            tone: dunlin_domain::ProjectTone::Blue,
            parent_id: parent.map(str::to_string),
        }
    }
    #[test]
    fn nested_hierarchy_collapse_paths_and_cycle_protection_do_not_drop_members() {
        let groups = vec![
            group("root", None),
            group("child", Some("root")),
            group("leaf", Some("child")),
        ];
        assert_eq!(group_path(&groups, "leaf"), "root / child / leaf");
        assert_eq!(descendants(&groups, "root").len(), 3);
        assert_eq!(
            visible_groups(&groups, &[], &[], Sort::Name, &Default::default()),
            [(0, 0), (1, 1), (2, 2)]
        );
        assert_eq!(
            visible_groups(
                &groups,
                &[],
                &["root".into()],
                Sort::Name,
                &Default::default()
            ),
            [(0, 0)]
        );
        assert!(
            group("root", Some("leaf"))
                .validate_parent(&groups)
                .is_err()
        );
        assert!(
            group("child", Some("child"))
                .validate_parent(&groups)
                .is_err()
        );
        assert!(
            group("child", Some("missing"))
                .validate_parent(&groups)
                .is_err()
        );
    }
    #[test]
    fn old_groups_migrate_to_roots_and_orphans_stay_visible() {
        let legacy: dunlin_domain::ProjectGroup =
            serde_json::from_str(r#"{"id":"old","name":"Old","tone":"Blue"}"#).unwrap();
        assert!(legacy.parent_id.is_none());
        let groups = vec![
            group("orphan", Some("deleted")),
            group("loop", Some("loop")),
        ];
        assert_eq!(
            visible_groups(&groups, &[], &[], Sort::Name, &Default::default()).len(),
            2
        );
    }

    #[test]
    fn deep_reversed_hierarchies_and_cycles_are_iterative_and_keep_every_group_visible() {
        let mut groups: Vec<_> = (0..2048)
            .map(|i| {
                group(
                    &format!("g{i}"),
                    (i > 0).then(|| format!("g{}", i - 1)).as_deref(),
                )
            })
            .collect();
        groups.reverse();
        let rows = visible_groups(&groups, &[], &[], Sort::Modified, &Default::default());
        assert_eq!(rows.len(), 2048);
        assert_eq!(rows.last().unwrap().1, 2047);
        assert_eq!(descendants(&groups, "g0").len(), 2048);
        groups.last_mut().unwrap().parent_id = Some("g2047".into());
        let recovered = visible_groups(&groups, &[], &[], Sort::Name, &Default::default());
        assert_eq!(recovered.len(), 2048);
        assert!(recovered.iter().all(|(_, depth)| *depth == 0));
    }

    fn project(id: &str, name: &str, pinned: bool, group: Option<&str>) -> dunlin_domain::Project {
        dunlin_domain::Project {
            id: id.into(),
            name: name.into(),
            pinned,
            group_id: group.map(str::to_string),
            path: format!("/unused/{id}"),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            trusted: true,
            running_sessions: 0,
            remote: None,
            settings: Default::default(),
        }
    }
    #[test]
    fn sorting_preserves_pins_and_assignments_with_missing_dates_last() {
        let projects = vec![
            project("a", "Zulu", false, Some("child")),
            project("b", "Beta", false, None),
            project("c", "Alpha", false, None),
            project("d", "Pinned", true, None),
        ];
        let modified = std::collections::HashMap::from([("a".into(), 300), ("c".into(), 100)]);
        let mut indices = vec![0, 1, 2, 3];
        super::sort_projects(&projects, &mut indices, Sort::Name, &modified);
        assert_eq!(indices, [3, 2, 1, 0]);
        super::sort_projects(&projects, &mut indices, Sort::Modified, &modified);
        assert_eq!(indices, [3, 0, 2, 1]);
        assert_eq!(projects[0].group_id.as_deref(), Some("child"));
        let groups = vec![
            group("parent", None),
            group("child", Some("parent")),
            group("empty", None),
        ];
        assert_eq!(
            visible_groups(&groups, &projects, &[], Sort::Modified, &modified),
            [(0, 0), (1, 1), (2, 0)]
        );
    }
    #[test]
    fn tree_preferences_and_sidebar_width_roundtrip_without_replacing_existing_preferences() {
        let legacy: crate::app::StoredSettings =
            serde_json::from_str(r#"{"dark":false,"editor_minimap":false}"#).unwrap();
        assert!(!legacy.dark);
        assert!(!legacy.editor_minimap);
        assert_eq!(legacy.project_view, super::View::Tree);
        assert_eq!(legacy.sidebar_width, 278.);
        let mut settings = legacy;
        settings.project_view = super::View::List;
        settings.project_sort = Sort::Modified;
        settings.sidebar_width = 403.;
        let saved: crate::app::StoredSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(saved.project_view, super::View::List);
        assert_eq!(saved.project_sort, Sort::Modified);
        assert_eq!(saved.sidebar_width, 403.);
        assert!(!saved.dark);
        assert!(!saved.editor_minimap);
    }
}
