//! Public compatibility targets and explicit capability reporting. A target
//! describes the intended ecosystem; it is never a claim of complete support.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    #[default]
    Windower4,
    Ashita4,
    Windower5,
    Native,
}
impl Ecosystem {
    pub fn name(self) -> &'static str {
        match self {
            Self::Windower4 => "windower4",
            Self::Ashita4 => "ashita4",
            Self::Windower5 => "windower5",
            Self::Native => "native",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Implementation {
    ProbeSubset,
    Planned,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Capability {
    pub id: &'static str,
    pub implementation: Implementation,
    pub api_callable: bool,
    pub in_game_verified: bool,
    pub detail: &'static str,
}
pub const CAPABILITIES: &[Capability] = &[
    Capability {
        id: "api.commands.input",
        implementation: Implementation::ProbeSubset,
        api_callable: true,
        in_game_verified: false,
        detail: "Direct game-input queue, separate from console scripts; native binding not live-validated",
    },
    Capability {
        id: "api.events",
        implementation: Implementation::ProbeSubset,
        api_callable: true,
        in_game_verified: false,
        detail: "Addon-local registration, removal, error isolation; selected render/input events",
    },
    Capability {
        id: "api.commands.queue",
        implementation: Implementation::ProbeSubset,
        api_callable: true,
        in_game_verified: false,
        detail: "Dialect-tagged command queues; no game execution",
    },
    Capability {
        id: "api.chat.queue",
        implementation: Implementation::ProbeSubset,
        api_callable: true,
        in_game_verified: false,
        detail: "Binary-safe chat queues; no game output",
    },
    Capability {
        id: "lua.scheduler",
        implementation: Implementation::ProbeSubset,
        api_callable: true,
        in_game_verified: false,
        detail: "Lua 5.1 schedule, sleep, cancellation, addon-local clocks",
    },
    Capability {
        id: "compat.windower4.lua",
        implementation: Implementation::ProbeSubset,
        api_callable: false,
        in_game_verified: false,
        detail: "Selected API subset only",
    },
    Capability {
        id: "compat.windower4.plugins",
        implementation: Implementation::ProbeSubset,
        api_callable: false,
        in_game_verified: false,
        detail: "Factory and metadata only; host ABI incomplete",
    },
    Capability {
        id: "compat.ashita4.lua",
        implementation: Implementation::ProbeSubset,
        api_callable: false,
        in_game_verified: false,
        detail: "Selected event/command adapters; optional isolated LuaJIT FFI; full libraries pending",
    },
    Capability {
        id: "compat.ashita4.plugins",
        implementation: Implementation::ProbeSubset,
        api_callable: false,
        in_game_verified: false,
        detail: "SDK 4.30 factories/metadata only; initialization and host services pending",
    },
    Capability {
        id: "compat.windower5.packages",
        implementation: Implementation::Planned,
        api_callable: false,
        in_game_verified: false,
        detail: "Separate LuaJIT package and shared-data interfaces required",
    },
    Capability {
        id: "graphics.backend",
        implementation: Implementation::ProbeSubset,
        api_callable: false,
        in_game_verified: false,
        detail: "One selected D3D8 provider below our COM/event layer",
    },
    Capability {
        id: "graphics.ui",
        implementation: Implementation::Planned,
        api_callable: false,
        in_game_verified: false,
        detail: "Interactive UI, legacy drawing and ImGui adapters",
    },
    Capability {
        id: "graphics.postprocessing",
        implementation: Implementation::Planned,
        api_callable: false,
        in_game_verified: false,
        detail: "Explicit renderer-specific postprocessing integration",
    },
    Capability {
        id: "game.state",
        implementation: Implementation::Planned,
        api_callable: false,
        in_game_verified: false,
        detail: "Validated memory, inventory, entities, party, jobs and resources",
    },
    Capability {
        id: "game.packets",
        implementation: Implementation::Planned,
        api_callable: false,
        in_game_verified: false,
        detail: "Ordered packet events, edits, blocking and queueing",
    },
    Capability {
        id: "game.commands",
        implementation: Implementation::Planned,
        api_callable: false,
        in_game_verified: false,
        detail: "Command routing, parsing, binds, aliases, scripts and game dispatch",
    },
    Capability {
        id: "game.framerate",
        implementation: Implementation::Planned,
        api_callable: false,
        in_game_verified: false,
        detail: "Pacing and game timing fixes need live-client validation",
    },
    Capability {
        id: "platform.ipc",
        implementation: Implementation::Planned,
        api_callable: false,
        in_game_verified: false,
        detail: "Cross-client messages and addon interoperability",
    },
    Capability {
        id: "platform.packages",
        implementation: Implementation::Planned,
        api_callable: false,
        in_game_verified: false,
        detail: "Versioned packages, dependencies, updates and rollback",
    },
];
pub fn supports_api(id: &str) -> bool {
    CAPABILITIES
        .iter()
        .any(|capability| capability.id == id && capability.api_callable)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn support_does_not_imply_unimplemented_game_features() {
        let ids = CAPABILITIES
            .iter()
            .map(|item| item.id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), CAPABILITIES.len());
        assert!(supports_api("api.events"));
        assert!(!supports_api("game.state"));
        assert!(!supports_api("unknown"));
        assert!(CAPABILITIES.iter().all(|item| !item.in_game_verified));
    }
}
