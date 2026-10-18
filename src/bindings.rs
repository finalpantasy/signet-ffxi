//! Remappable, deterministic input bindings; does not read or inject OS input.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub key: u16,
    pub modifiers: Modifiers,
    pub command: String,
    pub block: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Trigger {
    pub command: String,
    pub blocked: bool,
}
#[derive(Default)]
pub struct Bindings {
    entries: BTreeMap<u16, Vec<Binding>>,
    held: BTreeSet<u16>,
    focused: bool,
}
impl Bindings {
    pub fn set(&mut self, bindings: Vec<Binding>) -> Result<(), String> {
        if bindings.len() > 256 {
            return Err("Too many input bindings".into());
        }
        let mut next: BTreeMap<u16, Vec<Binding>> = BTreeMap::new();
        for binding in bindings {
            if binding.key == 0
                || binding.key > 511
                || binding.command.is_empty()
                || binding.command.len() > 1023
                || binding.command.contains(['\0', '\r', '\n'])
            {
                return Err("Invalid input binding".into());
            }
            let entries = next.entry(binding.key).or_default();
            if entries.iter().any(|b| b.modifiers == binding.modifiers) {
                return Err("Conflicting input chord".into());
            }
            entries.push(binding);
        }
        self.entries = next;
        self.held.clear();
        Ok(())
    }
    pub fn focus(&mut self, focused: bool) {
        self.focused = focused;
        self.held.clear();
    }
    /// Hosts must consume matching repeated key messages even when no new action fires.
    pub fn is_blocking(&self, key: u16, modifiers: Modifiers) -> bool {
        self.focused
            && self
                .entries
                .get(&key)
                .is_some_and(|entries| entries.iter().any(|b| b.modifiers == modifiers && b.block))
    }
    pub fn key(&mut self, key: u16, down: bool, modifiers: Modifiers) -> Option<Trigger> {
        if !down {
            self.held.remove(&key);
            return None;
        }
        if !self.focused || !self.held.insert(key) {
            return None;
        }
        self.entries
            .get(&key)?
            .iter()
            .find(|b| b.modifiers == modifiers)
            .map(|b| Trigger {
                command: b.command.clone(),
                blocked: b.block,
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bindings_are_remappable_repeat_safe_and_clear_on_focus_loss() {
        let mut b = Bindings::default();
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::default()
        };
        b.set(vec![Binding {
            key: 65,
            modifiers: ctrl,
            command: "addon reload Example".into(),
            block: true,
        }])
        .unwrap();
        assert!(b.key(65, true, ctrl).is_none());
        b.focus(true);
        assert!(b.key(65, true, Modifiers::default()).is_none());
        b.key(65, false, ctrl);
        assert_eq!(
            b.key(65, true, ctrl).unwrap().command,
            "addon reload Example"
        );
        assert!(b.key(65, true, ctrl).is_none());
        assert!(b.is_blocking(65, ctrl));
        b.focus(false);
        assert!(!b.is_blocking(65, ctrl));
        b.focus(true);
        assert!(b.key(65, true, ctrl).unwrap().blocked);
        b.set(vec![Binding {
            key: 66,
            modifiers: ctrl,
            command: "new".into(),
            block: false,
        }])
        .unwrap();
        assert!(b.key(65, true, ctrl).is_none());
        assert!(!b.key(66, true, ctrl).unwrap().blocked);
    }
    #[test]
    fn invalid_remap_preserves_existing_configuration() {
        let binding = Binding {
            key: 65,
            modifiers: Modifiers::default(),
            command: "old".into(),
            block: true,
        };
        let mut b = Bindings::default();
        b.set(vec![binding.clone()]).unwrap();
        assert!(b.set(vec![binding.clone(), binding]).is_err());
        b.focus(true);
        assert_eq!(
            b.key(65, true, Modifiers::default()).unwrap().command,
            "old"
        );
    }
}
