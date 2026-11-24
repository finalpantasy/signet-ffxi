//! Server model ids to DAT file ids.
//!
//! Derived from the install itself (chunk kinds, mesh and texture names, DAT sizes) and from the
//! client's own resource table, checked against known look data; see
//! `docs/REPLACEMENT_CLIENT.md` ("Entity models").
//!
//! * **Monster/NPC model** (look kinds 0, 5 and 6, one u16 model id `m`): the model is one DAT
//!   holding the skeleton, meshes and animations. `m` in 0..1484 is file `1300 + m`; `m` in
//!   1500..3000 is file `50295 + m`; `m` in 3000.. (Trust NPCs) is file `99907 + (m - 3000)`.
//! * **Humanoid look** (race 1..8, face, head/body/hands/legs/feet/main/sub/ranged model ids):
//!   each race owns one skeleton DAT `S` (skeleton plus idle, walk and run animations; Taru male
//!   and female share one). Faces, armor and weapons come from the extent lists in
//!   [`crate::modeltable`]: ids 0..255 of an armor slot sit next to the skeleton, ids 256 and up
//!   continue in further extents elsewhere in the install.
//! * Weapons are plain actor meshes (`wep*`). Their bone table names a bone of the race skeleton
//!   (right hand 68 for hand-to-hand, 5/6/10/... for the weapon-carry bones that the animations
//!   move into the hand or onto the back, 85/68 for a held bow).

use crate::modeltable::RACE_TABLE;

/// Skeleton DAT of each race, indexed by race 1..=8 (Hume M/F, Elvaan M/F, Taru M/F, Mithra, Galka).
pub const RACE_SKELETONS: [u32; 8] = [7072, 10248, 13424, 16600, 19776, 19776, 23176, 26352];

pub const SLOT_HEAD: u8 = 1;
pub const SLOT_BODY: u8 = 2;
pub const SLOT_HANDS: u8 = 3;
pub const SLOT_LEGS: u8 = 4;
pub const SLOT_FEET: u8 = 5;
pub const SLOT_MAIN: u8 = 6;
pub const SLOT_SUB: u8 = 7;
pub const SLOT_RANGED: u8 = 8;

/// First model id of the Trust NPC range.
pub const TRUST_FIRST: u16 = 3000;
/// The chocobo DAT (skeleton, body and saddle meshes). The look's face byte (32 and up) and gear
/// bytes are not decoded: every chocobo look draws this one DAT. GUESSED.
pub const CHOCOBO_FILE: u32 = 39275;

/// DAT file id of a monster or NPC model id, if the id is in a mapped range.
pub fn mob_file(model: u16) -> Option<u32> {
    let m = u32::from(model);
    match m {
        0..=1483 => Some(1300 + m),
        1500..=2999 => Some(50295 + m),
        3000..=3999 => Some(99907 + (m - 3000)),
        _ => None,
    }
}

/// Block of [`RACE_TABLE`] for a race number: 1..=8 are the playable races; 29 and 30 are the two
/// mannequin blocks (which race each is for is GUESSED from their texture names: Mithra, Hume F).
fn race_block(race: u8) -> Option<usize> {
    match race {
        1..=8 => Some(usize::from(race) - 1),
        29 => Some(8),
        30 => Some(9),
        _ => None,
    }
}

/// Skeleton (and animation) DAT of a race number 1..=8 (29 and 30, the mannequins, borrow the
/// Mithra and Hume female skeletons: GUESSED).
pub fn race_skeleton(race: u8) -> Option<u32> {
    match race {
        29 => Some(RACE_SKELETONS[6]),
        30 => Some(RACE_SKELETONS[1]),
        _ => RACE_SKELETONS.get(usize::from(race).checked_sub(1)?).copied(),
    }
}

/// File id of model `id` inside a slot's extent list (see [`crate::modeltable`]).
fn walk(extents: &[(u32, u32)], id: u32) -> Option<u32> {
    let mut rest = id;
    for &(first, count) in extents {
        if rest < count {
            return (first != 0).then_some(first + rest);
        }
        rest -= count;
    }
    None
}

/// Face DAT: `face` is the server's face byte. The table has 32 faces per race (Taru female has
/// her own block); bytes of 32 and up have no file.
pub fn face_file(race: u8, face: u8) -> Option<u32> {
    walk(&RACE_TABLE[race_block(race)?][0], u32::from(face))
}

/// Equipment DAT for a slot (1 head .. 8 ranged) and the model id with the slot nibble already
/// removed. Model 0 of an armor slot is the empty/naked piece. Ids of 256 and up live in the
/// later extents of the slot (see [`crate::modeltable`]); an id past the last extent has no file.
pub fn equip_file(race: u8, slot: u8, model: u16) -> Option<u32> {
    if !(SLOT_HEAD..=SLOT_RANGED).contains(&slot) {
        return None;
    }
    walk(&RACE_TABLE[race_block(race)?][usize::from(slot)], u32::from(model))
}

/// The model id of a look value as the server sends it: the slot nibble (0x1000 head .. 0x8000 ranged) is dropped.
pub fn equip_model(raw: u16) -> u16 {
    raw & 0x0FFF
}

/// What the server's look data names: one model id, or a race with equipment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModelRef {
    /// Look kind 0 (and, GUESSED, kinds 5 "misc" and 6 "automaton"): a monster or NPC model id.
    Mob(u16),
    /// Look kind 1 or a player: race 1..=8, face byte, head, body, hands, legs, feet, main, sub, ranged
    /// (raw values; the slot nibble is dropped here).
    Humanoid { race: u8, face: u8, parts: [u16; 8] },
    /// Look kind 7: every chocobo draws the one chocobo DAT (the face and gear bytes are not decoded).
    Chocobo,
}

impl ModelRef {
    /// Number of face, armor and weapon ids of a humanoid look that have no file (0 for other looks).
    pub fn missing_parts(&self) -> usize {
        match *self {
            ModelRef::Humanoid { race, face, parts } => {
                usize::from(face_file(race, face).is_none())
                    + parts.iter().enumerate().filter(|&(i, raw)| equip_file(race, i as u8 + 1, equip_model(*raw)).is_none()).count()
            }
            _ => 0,
        }
    }

    /// Extra DATs that only hold animations: for a humanoid with a weapon, the race's weapon
    /// animation set (the DAT right after the skeleton DAT; `wlk1`, `idl1`, `run1` and so on, which
    /// key the weapon mounts). Decoded 2026-11-23 on the Mithra (file 23177 follows 23176).
    pub fn anim_files(&self) -> Vec<u32> {
        match *self {
            ModelRef::Humanoid { race, parts, .. } if parts[5..8].iter().any(|&raw| equip_model(raw) != 0) => {
                race_skeleton(race).map(|f| vec![f + 1]).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    /// DAT file ids to load, skeleton owner first: the skeleton DAT, the face, head to feet, then
    /// main, sub and ranged weapons. Pieces whose id has no file are left out (the character is
    /// drawn without them). None when the race or monster id has no mapping.
    pub fn files(&self) -> Option<Vec<u32>> {
        match *self {
            ModelRef::Mob(m) => mob_file(m).map(|f| vec![f]),
            ModelRef::Chocobo => Some(vec![CHOCOBO_FILE]),
            ModelRef::Humanoid { race, face, parts } => {
                let mut v = vec![race_skeleton(race)?];
                // A face byte with no file falls back to face 0.
                v.extend(face_file(race, face).or_else(|| face_file(race, 0)));
                for (i, raw) in parts.iter().enumerate() {
                    // Weapon slots (6..=8): model 0 is the empty hand, not a mesh to draw.
                    if i >= 5 && equip_model(*raw) == 0 {
                        continue;
                    }
                    v.extend(equip_file(race, i as u8 + 1, equip_model(*raw)));
                }
                Some(v)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mob_ranges() {
        assert_eq!(mob_file(292), Some(1592)); // Slime Mold: texture "slm_"
        assert_eq!(mob_file(2083), Some(52378)); // Yagudo
        assert_eq!(mob_file(1490), None);
        assert_eq!(mob_file(3010), Some(99917)); // Zeid (Trust): a Galka
        assert_eq!(mob_file(3086), Some(99993)); // Zeid II: texture "zeid"
        assert_eq!(mob_file(5000), None);
    }

    #[test]
    fn humanoid_blocks() {
        assert_eq!(face_file(1, 0), Some(7080));
        assert_eq!(face_file(1, 31), Some(7111));
        assert_eq!(face_file(1, 32), None);
        assert_eq!(face_file(6, 0), Some(22952)); // Taru female has her own faces
        assert_eq!(equip_file(1, SLOT_BODY, 0), Some(7368));
        assert_eq!(equip_file(1, SLOT_FEET, 8), Some(8144));
        assert_eq!(equip_file(5, SLOT_BODY, 3), equip_file(6, SLOT_BODY, 3));
        assert_eq!(equip_file(8, SLOT_MAIN, 268), Some(26352 + 1320 + 268));
        assert_eq!(race_skeleton(0), None);
        assert_eq!(race_skeleton(9), None);
        assert_eq!(equip_file(9, SLOT_BODY, 0), None);
    }

    #[test]
    fn extended_ids() {
        // Hume male body: ids 256..303 then 304..319, then 320..575, 576..607, 608..671.
        assert_eq!(equip_file(1, SLOT_BODY, 255), Some(7623));
        assert_eq!(equip_file(1, SLOT_BODY, 256), Some(63387));
        assert_eq!(equip_file(1, SLOT_BODY, 304), Some(63435));
        assert_eq!(equip_file(1, SLOT_BODY, 320), Some(71503));
        assert_eq!(equip_file(1, SLOT_BODY, 576), Some(98819));
        assert_eq!(equip_file(1, SLOT_BODY, 608), Some(103025));
        assert_eq!(equip_file(1, SLOT_BODY, 672), None);
        // Main hand: ids 0..511 beside the skeleton, 512..639 in the next extent.
        assert_eq!(equip_file(1, SLOT_MAIN, 512), Some(63643));
        assert_eq!(equip_file(1, SLOT_MAIN, 640), Some(72527));
        // Sub hand and ranged have their own extents.
        assert_eq!(equip_file(1, SLOT_SUB, 0), Some(41199));
        assert_eq!(equip_file(1, SLOT_RANGED, 255), Some(9416 + 255));
        assert_eq!(equip_file(1, SLOT_RANGED, 256), None);
    }

    #[test]
    fn humanoid_files_list_weapons() {
        let r = ModelRef::Humanoid { race: 7, face: 3, parts: [0, 0, 0, 0, 0, 5, 1, 2] };
        let f = r.files().unwrap();
        assert_eq!(f[0], 23176);
        assert_eq!(f[1], 23184 + 3);
        assert_eq!(f.len(), 10);
        assert_eq!(f[7], 24496 + 5);
    }
}
