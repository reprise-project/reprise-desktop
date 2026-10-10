/// Each group at least two windows, no window in two of them.
#[derive(Clone, Debug)]
pub struct Groups<T> {
    groups: Vec<Group<T>>,
}

impl<T> Default for Groups<T> {
    fn default() -> Self {
        Groups { groups: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group<T> {
    pub members: Vec<T>,
    pub shown: usize,
}

impl<T: Copy + Eq> Group<T> {
    pub fn shown(&self) -> T {
        self.members[self.shown]
    }
}

impl<T: Copy + Eq> Groups<T> {
    fn index_of(&self, w: T) -> Option<usize> {
        self.groups.iter().position(|g| g.members.contains(&w))
    }

    pub fn group_of(&self, w: T) -> Option<&Group<T>> {
        self.index_of(w).map(|i| &self.groups[i])
    }

    pub fn shown_of(&self, w: T) -> T {
        self.group_of(w).map_or(w, Group::shown)
    }

    pub fn is_hidden(&self, w: T) -> bool {
        self.shown_of(w) != w
    }

    pub fn members_of(&self, w: T) -> Vec<T> {
        self.group_of(w)
            .map_or_else(|| vec![w], |g| g.members.clone())
    }

    pub fn select(&mut self, w: T) -> Option<T> {
        let gi = self.index_of(w)?;
        let g = &mut self.groups[gi];
        let was = g.shown();
        g.shown = g.members.iter().position(|&m| m == w)?;
        (was != w).then_some(was)
    }

    pub fn step(&self, w: T, backwards: bool) -> Option<T> {
        let g = self.group_of(w)?;
        let n = g.members.len();
        let i = if backwards {
            (g.shown + n - 1) % n
        } else {
            (g.shown + 1) % n
        };
        Some(g.members[i])
    }

    pub fn first_hidden(&self) -> Option<T> {
        self.groups.iter().find_map(|g| {
            let shown = g.shown();
            g.members.iter().copied().find(|&m| m != shown)
        })
    }

    pub fn leave(&mut self, w: T) -> Option<T> {
        let gi = self.index_of(w)?;
        let g = &mut self.groups[gi];
        let i = g.members.iter().position(|&m| m == w)?;
        g.members.remove(i);
        if g.shown > i || g.shown == g.members.len() {
            g.shown = g.shown.saturating_sub(1);
        }
        let now = g.shown();
        if g.members.len() < 2 {
            self.groups.remove(gi);
        }
        Some(now)
    }

    pub fn join(&mut self, w: T, target: T) {
        if w == target
            || self.index_of(target).is_some() && self.index_of(target) == self.index_of(w)
        {
            return;
        }
        let incoming = match self.index_of(w) {
            Some(gi) => self.groups.remove(gi).members,
            None => vec![w],
        };
        let gi = match self.index_of(target) {
            Some(gi) => gi,
            None => {
                self.groups.push(Group {
                    members: vec![target],
                    shown: 0,
                });
                self.groups.len() - 1
            }
        };
        let g = &mut self.groups[gi];
        g.members.extend(incoming);
        g.shown = g.members.iter().position(|&m| m == w).unwrap_or(0);
    }

    pub fn reorder(&mut self, w: T, slot: usize) {
        let Some(gi) = self.index_of(w) else {
            return;
        };
        let g = &mut self.groups[gi];
        let shown = g.shown();
        let Some(i) = g.members.iter().position(|&m| m == w) else {
            return;
        };
        let m = g.members.remove(i);
        let slot = slot.min(g.members.len());
        g.members.insert(slot, m);
        g.shown = g.members.iter().position(|&m| m == shown).unwrap_or(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(spec: &[(&[u32], usize)]) -> Groups<u32> {
        Groups {
            groups: spec
                .iter()
                .map(|(m, shown)| Group {
                    members: m.to_vec(),
                    shown: *shown,
                })
                .collect(),
        }
    }

    #[test]
    fn joining_a_lone_window_makes_a_group_showing_the_newcomer() {
        let mut g = Groups::default();
        g.join(2, 1);
        assert_eq!(g.members_of(1), vec![1, 2]);
        assert_eq!(g.shown_of(1), 2);
        assert!(g.is_hidden(1));
        assert!(!g.is_hidden(2));
        assert!(!g.is_hidden(3), "a window in no group shows");
        assert_eq!(g.members_of(3), vec![3]);
    }

    #[test]
    fn a_group_dropped_on_another_brings_every_tab_along() {
        let mut g = groups(&[(&[1, 2], 0), (&[3, 4], 1)]);
        g.join(4, 1);
        assert_eq!(g.members_of(1), vec![1, 2, 3, 4]);
        assert_eq!(g.shown_of(2), 4);
        assert_eq!(g.groups.len(), 1);
    }

    #[test]
    fn leaving_each_hidden_window_in_turn_ends_every_group() {
        let mut g = groups(&[(&[1, 2, 3], 1), (&[4, 5], 0)]);
        let mut left = Vec::new();
        while let Some(w) = g.first_hidden() {
            g.leave(w);
            left.push(w);
        }
        assert_eq!(left, [1, 3, 5], "every window but the shown ones");
        assert!(g.groups.is_empty());
        assert_eq!(g.first_hidden(), None);
    }

    #[test]
    fn joining_within_the_same_group_changes_nothing() {
        let mut g = groups(&[(&[1, 2], 0)]);
        g.join(2, 1);
        g.join(1, 1);
        assert_eq!(g.members_of(1), vec![1, 2]);
        assert_eq!(g.shown_of(1), 1);
    }

    #[test]
    fn leaving_the_shown_tab_shows_the_next_or_the_one_before_the_last() {
        let mut g = groups(&[(&[1, 2, 3], 1)]);
        assert_eq!(g.leave(2), Some(3));
        let mut g = groups(&[(&[1, 2, 3], 2)]);
        assert_eq!(g.leave(3), Some(2));
        let mut g = groups(&[(&[1, 2, 3], 2)]);
        assert_eq!(
            g.leave(1),
            Some(3),
            "a hidden tab leaving keeps the shown one"
        );
        assert_eq!(g.members_of(3), vec![2, 3]);
        assert_eq!(g.leave(9), None);
    }

    #[test]
    fn a_group_of_one_ends() {
        let mut g = groups(&[(&[1, 2], 1)]);
        assert_eq!(g.leave(2), Some(1));
        assert!(g.group_of(1).is_none());
        assert!(!g.is_hidden(1));
    }

    #[test]
    fn select_and_step_go_round_the_tabs() {
        let mut g = groups(&[(&[1, 2, 3], 0)]);
        assert_eq!(g.step(1, false), Some(2));
        assert_eq!(g.step(1, true), Some(3));
        assert_eq!(g.select(3), Some(1));
        assert_eq!(g.select(3), None, "already showing");
        assert_eq!(g.step(2, false), Some(1));
        assert_eq!(g.step(7, false), None);
    }

    #[test]
    fn reorder_moves_a_tab_and_keeps_the_shown_one() {
        let mut g = groups(&[(&[1, 2, 3], 1)]);
        g.reorder(1, 2);
        assert_eq!(g.members_of(1), vec![2, 3, 1]);
        assert_eq!(g.shown_of(1), 2);
        g.reorder(1, 0);
        assert_eq!(g.members_of(1), vec![1, 2, 3]);
        g.reorder(2, 9);
        assert_eq!(g.members_of(1), vec![1, 3, 2]);
    }
}
