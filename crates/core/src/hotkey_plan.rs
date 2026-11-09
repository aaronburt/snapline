use std::collections::BTreeMap;
use std::fmt;

use crate::actions::ActionId;
use crate::chord::Chord;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Owner(String);

impl Owner {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for Owner {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Owner {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<ActionId> for Owner {
    fn from(value: ActionId) -> Self {
        Self::new(value.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlockReason {
    Internal,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    pub chord: Chord,
    pub owners: Vec<Owner>,
    pub reason: BlockReason,
}

impl Blocked {
    pub fn internal(chord: Chord, owners: Vec<Owner>) -> Self {
        Self {
            chord,
            owners,
            reason: BlockReason::Internal,
        }
    }

    pub fn external(chord: Chord, owner: Owner) -> Self {
        Self {
            chord,
            owners: vec![owner],
            reason: BlockReason::External,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HotkeyPlan {
    pub bind: Vec<(Owner, Chord)>,
    pub blocked: Vec<Blocked>,
}

impl HotkeyPlan {
    pub fn is_bound(&self, chord: &Chord) -> bool {
        self.bind.iter().any(|(_, bound)| bound == chord)
    }

    pub fn with_external(&self, failures: &[Chord]) -> Self {
        let mut bind = Vec::new();
        let mut blocked = self.blocked.clone();
        for (owner, chord) in &self.bind {
            if failures.contains(chord) {
                blocked.push(Blocked::external(*chord, owner.clone()));
            } else {
                bind.push((owner.clone(), *chord));
            }
        }
        blocked.sort_by(|left, right| {
            left.chord
                .cmp(&right.chord)
                .then(left.reason.cmp(&right.reason))
                .then(left.owners.cmp(&right.owners))
        });
        Self { bind, blocked }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    Free,
    Internal { owners: Vec<Owner> },
    External,
}

pub fn plan(entries: &[(Owner, Chord)]) -> HotkeyPlan {
    let mut grouped: BTreeMap<Chord, Vec<Owner>> = BTreeMap::new();
    for (owner, chord) in entries {
        let owners = grouped.entry(*chord).or_default();
        if !owners.contains(owner) {
            owners.push(owner.clone());
        }
    }
    let mut bind = Vec::new();
    let mut blocked = Vec::new();
    for (chord, mut owners) in grouped {
        owners.sort();
        if owners.len() == 1 {
            bind.push((owners[0].clone(), chord));
        } else {
            blocked.push(Blocked::internal(chord, owners));
        }
    }
    HotkeyPlan { bind, blocked }
}

pub fn preview(
    entries: &[(Owner, Chord)],
    candidate: &(Owner, Chord),
    external: &[Chord],
) -> Preview {
    let mut owners: Vec<Owner> = entries
        .iter()
        .filter(|(owner, chord)| chord == &candidate.1 && owner != &candidate.0)
        .map(|(owner, _)| owner.clone())
        .collect();
    if !owners.is_empty() {
        owners.sort();
        owners.dedup();
        return Preview::Internal { owners };
    }
    if external.contains(&candidate.1) {
        return Preview::External;
    }
    Preview::Free
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).expect("valid chord")
    }

    fn owner(text: &str) -> Owner {
        Owner::new(text)
    }

    #[test]
    fn a_single_owner_binds() {
        let entries = [(owner("capture_region"), chord("PrtSc"))];
        let result = plan(&entries);
        assert_eq!(result.bind, entries.to_vec());
        assert!(result.blocked.is_empty());
        assert!(result.is_bound(&chord("PrtSc")));
    }

    #[test]
    fn an_internal_clash_blocks_every_owner() {
        let entries = [
            (owner("capture_region"), chord("Ctrl+Shift+A")),
            (owner("workflow:annotate"), chord("Shift+Ctrl+A")),
            (owner("capture_window"), chord("Alt+PrtSc")),
        ];
        let result = plan(&entries);
        assert_eq!(
            result.bind,
            vec![(owner("capture_window"), chord("Alt+PrtSc"))]
        );
        assert_eq!(
            result.blocked,
            vec![Blocked::internal(
                chord("Ctrl+Shift+A"),
                vec![owner("capture_region"), owner("workflow:annotate")]
            )]
        );
    }

    #[test]
    fn a_duplicate_owner_counts_once() {
        let entries = [
            (owner("capture_region"), chord("PrtSc")),
            (owner("capture_region"), chord("PrtSc")),
        ];
        let result = plan(&entries);
        assert_eq!(result.bind, vec![(owner("capture_region"), chord("PrtSc"))]);
        assert!(result.blocked.is_empty());
    }

    #[test]
    fn output_ordering_is_deterministic() {
        let forward = [
            (owner("a"), chord("Ctrl+Shift+R")),
            (owner("b"), chord("PrtSc")),
            (owner("c"), chord("Alt+PrtSc")),
        ];
        let reversed = [
            (owner("c"), chord("Alt+PrtSc")),
            (owner("b"), chord("PrtSc")),
            (owner("a"), chord("Ctrl+Shift+R")),
        ];
        assert_eq!(plan(&forward), plan(&reversed));
        assert_eq!(
            plan(&forward)
                .bind
                .iter()
                .map(|(_, value)| value.to_string())
                .collect::<Vec<_>>(),
            vec!["PrtSc", "Alt+PrtSc", "Ctrl+Shift+R"]
        );
    }

    #[test]
    fn external_failures_move_bindings_to_blocked() {
        let entries = [
            (owner("capture_region"), chord("PrtSc")),
            (owner("capture_window"), chord("Alt+PrtSc")),
        ];
        let result = plan(&entries).with_external(&[chord("PrtSc")]);
        assert_eq!(
            result.bind,
            vec![(owner("capture_window"), chord("Alt+PrtSc"))]
        );
        assert_eq!(
            result.blocked,
            vec![Blocked::external(chord("PrtSc"), owner("capture_region"))]
        );
    }

    #[test]
    fn external_failures_keep_existing_internal_blocks() {
        let entries = [
            (owner("a"), chord("Ctrl+Shift+A")),
            (owner("b"), chord("Ctrl+Shift+A")),
            (owner("c"), chord("Alt+PrtSc")),
        ];
        let result = plan(&entries).with_external(&[chord("Alt+PrtSc")]);
        assert!(result.bind.is_empty());
        assert_eq!(
            result.blocked,
            vec![
                Blocked::external(chord("Alt+PrtSc"), owner("c")),
                Blocked::internal(chord("Ctrl+Shift+A"), vec![owner("a"), owner("b")]),
            ]
        );
    }

    #[test]
    fn preview_reports_free_internal_and_external() {
        let entries = [
            (owner("capture_region"), chord("PrtSc")),
            (owner("capture_window"), chord("Alt+PrtSc")),
        ];
        assert_eq!(
            preview(
                &entries,
                &(owner("capture_screen"), chord("Ctrl+PrtSc")),
                &[]
            ),
            Preview::Free
        );
        assert_eq!(
            preview(
                &entries,
                &(owner("capture_screen"), chord("Alt+PrtSc")),
                &[]
            ),
            Preview::Internal {
                owners: vec![owner("capture_window")]
            }
        );
        assert_eq!(
            preview(&entries, &(owner("capture_screen"), chord("PrtSc")), &[]),
            Preview::Internal {
                owners: vec![owner("capture_region")]
            }
        );
        assert_eq!(
            preview(&entries, &(owner("capture_region"), chord("PrtSc")), &[]),
            Preview::Free
        );
        assert_eq!(
            preview(
                &entries,
                &(owner("capture_screen"), chord("Ctrl+PrtSc")),
                &[chord("Ctrl+PrtSc")]
            ),
            Preview::External
        );
    }

    const PROPTEST_MODIFIERS: &[&str] = &["Ctrl", "Alt", "Shift"];

    const PROPTEST_KEYS: &[&str] = &[
        "PrtSc", "A", "B", "C", "R", "S", "T", "0", "9", "F1", "F12", "F24", "Pause", "Insert",
        "Delete", "Home", "End", "PageUp", "PageDown", "Up", "Down", "Left", "Right",
    ];

    fn chord_strategy() -> impl Strategy<Value = Chord> {
        (
            prop::collection::vec(prop::sample::select(PROPTEST_MODIFIERS.to_vec()), 1..4),
            prop::sample::select(PROPTEST_KEYS.to_vec()),
        )
            .prop_map(|(modifiers, key)| {
                let mut text = String::new();
                for (index, modifier) in modifiers.iter().enumerate() {
                    if index > 0 {
                        text.push('+');
                    }
                    text.push_str(modifier);
                }
                text.push('+');
                text.push_str(key);
                Chord::parse(&text).expect("generated chord")
            })
    }

    fn owner_strategy() -> impl Strategy<Value = Owner> {
        "[a-z]{1,5}".prop_map(Owner::new)
    }

    proptest! {
        #[test]
        fn no_chord_is_ever_bound_twice(
            entries in prop::collection::vec((owner_strategy(), chord_strategy()), 0..40)
        ) {
            let result = plan(&entries);
            let bound: Vec<Chord> = result.bind.iter().map(|(_, value)| *value).collect();
            let total = bound.len();
            let mut unique = bound.clone();
            unique.sort();
            unique.dedup();
            prop_assert_eq!(unique.len(), total);
            for blocked in &result.blocked {
                prop_assert!(!bound.contains(&blocked.chord));
            }
        }

        #[test]
        fn external_failures_never_leave_a_binding(
            entries in prop::collection::vec((owner_strategy(), chord_strategy()), 0..40),
            failures in prop::collection::vec(chord_strategy(), 0..10)
        ) {
            let result = plan(&entries).with_external(&failures);
            for (_, value) in &result.bind {
                prop_assert!(!failures.contains(value));
            }
            let bound: Vec<Chord> = result.bind.iter().map(|(_, value)| *value).collect();
            let total = bound.len();
            let mut unique = bound;
            unique.sort();
            unique.dedup();
            prop_assert_eq!(unique.len(), total);
        }
    }
}
