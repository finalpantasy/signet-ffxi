//! Preserve disk-profile order and explicit per-addon engines across grouped UI fields.
use crate::runtime_options::{AddonSpec, LuaEngine};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Selection {
    order: Vec<String>,
    engines: BTreeMap<String, LuaEngine>,
}
impl Selection {
    pub fn from_specs(specs: &[AddonSpec]) -> Self {
        Self {
            order: specs.iter().map(|s| s.name.to_lowercase()).collect(),
            engines: specs
                .iter()
                .map(|s| (s.name.to_lowercase(), s.engine))
                .collect(),
        }
    }
    pub fn engine(&self, name: &str, default: LuaEngine) -> LuaEngine {
        self.engines
            .get(&name.to_lowercase())
            .copied()
            .unwrap_or(default)
    }
    pub fn set(&mut self, name: &str, engine: LuaEngine) {
        self.engines.insert(name.to_lowercase(), engine);
    }
    pub fn apply(&self, specs: &mut [AddonSpec]) {
        for spec in specs.iter_mut() {
            spec.engine = self.engine(&spec.name, spec.engine);
        }
        specs.sort_by_key(|s| {
            self.order
                .iter()
                .position(|n| n == &s.name.to_lowercase())
                .unwrap_or(usize::MAX)
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Ecosystem;
    #[test]
    fn grouped_fields_preserve_mixed_engines_and_original_dispatch_order() {
        let a = AddonSpec {
            name: "Neighbor".into(),
            ecosystem: Ecosystem::Native,
            engine: LuaEngine::Lua51,
        };
        let b = AddonSpec {
            name: "Selected".into(),
            ecosystem: Ecosystem::Windower4,
            engine: LuaEngine::LuaJit,
        };
        let original = vec![a.clone(), b.clone()];
        let state = Selection::from_specs(&original);
        let mut grouped = vec![
            AddonSpec {
                engine: LuaEngine::Lua51,
                ..b
            },
            a,
        ];
        state.apply(&mut grouped);
        assert_eq!(grouped, original);
    }
    #[test]
    fn changing_one_engine_preserves_neighbor_and_appends_new_selections() {
        let old = vec![
            AddonSpec {
                name: "Neighbor".into(),
                ecosystem: Ecosystem::Native,
                engine: LuaEngine::Lua51,
            },
            AddonSpec {
                name: "Selected".into(),
                ecosystem: Ecosystem::Windower4,
                engine: LuaEngine::LuaJit,
            },
        ];
        let mut state = Selection::from_specs(&old);
        state.set("SELECTED", LuaEngine::Lua51);
        let mut next = vec![
            old[1].clone(),
            AddonSpec {
                name: "New".into(),
                ecosystem: Ecosystem::Ashita4,
                engine: LuaEngine::LuaJit,
            },
            old[0].clone(),
        ];
        state.apply(&mut next);
        assert_eq!(
            next.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Neighbor", "Selected", "New"]
        );
        assert_eq!(next[0], old[0]);
        assert_eq!(next[1].engine, LuaEngine::Lua51);
        assert_eq!(next[2].engine, LuaEngine::LuaJit);
    }
}
