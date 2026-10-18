//! Deterministic packet-event fixtures. No sockets or native game transport.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_PACKET: usize = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Incoming,
    Outgoing,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packet {
    pub direction: Direction,
    pub sequence: u64,
    pub id: u16,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Observe {},
    Patch { offset: usize, bytes: Vec<u8> },
    Block {},
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub owner: String,
    pub direction: Direction,
    pub id: Option<u16>,
    pub action: Action,
}
#[derive(Debug, Serialize)]
pub struct ResultPacket {
    pub original: Packet,
    pub bytes: Vec<u8>,
    pub blocked: bool,
    pub observers: Vec<String>,
}
#[derive(Default)]
pub struct Pipeline {
    rules: Vec<Rule>,
    last: BTreeMap<Direction, u64>,
}
impl Pipeline {
    pub fn register(&mut self, rule: Rule) -> Result<(), String> {
        if self.rules.len() >= 256
            || rule.owner.is_empty()
            || rule.owner.len() > 256
            || rule.id.is_some_and(|id| id > 0x1ff)
        {
            return Err("Invalid packet handler owner/capacity".into());
        }
        if let Action::Patch { offset, bytes } = &rule.action
            && (bytes.is_empty()
                || bytes.len() > MAX_PACKET
                || offset
                    .checked_add(bytes.len())
                    .is_none_or(|n| n > MAX_PACKET))
        {
            return Err("Packet patch exceeds bounds".into());
        }
        self.rules.push(rule);
        Ok(())
    }
    pub fn remove_owner(&mut self, owner: &str) {
        self.rules.retain(|r| r.owner != owner);
    }
    pub fn process(&mut self, packet: Packet) -> Result<ResultPacket, String> {
        if packet.bytes.is_empty()
            || packet.bytes.len() > MAX_PACKET
            || packet.id > 0x1ff
            || packet.sequence == 0
            || self
                .last
                .get(&packet.direction)
                .is_some_and(|last| packet.sequence <= *last)
        {
            return Err("Malformed or replayed packet fixture".into());
        }
        let mut bytes = packet.bytes.clone();
        let mut blocked = false;
        let mut observers = Vec::new();
        for rule in &self.rules {
            if rule.direction != packet.direction || rule.id.is_some_and(|id| id != packet.id) {
                continue;
            }
            observers.push(rule.owner.clone());
            match &rule.action {
                Action::Observe {} => {}
                Action::Block {} => blocked = true,
                Action::Patch {
                    offset,
                    bytes: patch,
                } => {
                    let end = offset
                        .checked_add(patch.len())
                        .filter(|end| *end <= bytes.len())
                        .ok_or("Patch exceeds fixture payload")?;
                    bytes[*offset..end].copy_from_slice(patch);
                }
            }
        }
        // Failure does not consume a sequence or mutate the original fixture.
        self.last.insert(packet.direction, packet.sequence);
        Ok(ResultPacket {
            original: packet,
            bytes,
            blocked,
            observers,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn packet(sequence: u64) -> Packet {
        Packet {
            direction: Direction::Incoming,
            sequence,
            id: 0x28,
            bytes: vec![1, 2, 3, 4],
        }
    }
    #[test]
    fn ordered_edits_sticky_block_and_owner_removal() {
        let mut p = Pipeline::default();
        for (owner, action) in [
            (
                "first",
                Action::Patch {
                    offset: 1,
                    bytes: vec![9],
                },
            ),
            ("block", Action::Block {}),
            (
                "second",
                Action::Patch {
                    offset: 1,
                    bytes: vec![8, 7],
                },
            ),
        ] {
            p.register(Rule {
                owner: owner.into(),
                direction: Direction::Incoming,
                id: Some(0x28),
                action,
            })
            .unwrap();
        }
        let result = p.process(packet(1)).unwrap();
        assert_eq!(result.original.bytes, vec![1, 2, 3, 4]);
        assert_eq!(result.bytes, vec![1, 8, 7, 4]);
        assert!(result.blocked);
        assert_eq!(result.observers, vec!["first", "block", "second"]);
        p.remove_owner("block");
        assert!(!p.process(packet(2)).unwrap().blocked);
        assert!(p.process(packet(2)).is_err());
        let mut out = packet(1);
        out.direction = Direction::Outgoing;
        assert_eq!(p.process(out).unwrap().bytes, vec![1, 2, 3, 4]);
    }
    #[test]
    fn invalid_patch_is_transactional_and_malformed_frames_are_refused() {
        let mut p = Pipeline::default();
        p.register(Rule {
            owner: "bad".into(),
            direction: Direction::Incoming,
            id: None,
            action: Action::Patch {
                offset: 4,
                bytes: vec![1],
            },
        })
        .unwrap();
        assert!(p.process(packet(1)).is_err());
        p.remove_owner("bad");
        assert!(p.process(packet(1)).is_ok());
        for mut bad in [packet(2), packet(2), packet(0)] {
            bad.bytes.clear();
            assert!(p.process(bad).is_err());
        }
        let mut bad = packet(2);
        bad.bytes = vec![0; MAX_PACKET + 1];
        assert!(p.process(bad).is_err());
        let mut bad = packet(2);
        bad.id = 0x200;
        assert!(p.process(bad).is_err());
    }
}
