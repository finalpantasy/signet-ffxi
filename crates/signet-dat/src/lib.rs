//! Read-only access to a user's own FFXI install. Ships and embeds no game data.

pub mod actor_mesh;
pub mod anim;
pub mod chunk;
pub mod collision;
pub mod crypt;
pub mod environment;
pub mod install;
pub mod mmb;
pub mod modelmap;
pub mod modeltable;
pub mod png;
pub mod skeleton;
pub mod texture;
pub mod zones;

pub use chunk::{Chunk, ChunkError, chunks};
pub use install::{DatLocation, Install};

/// Usual install locations, tried in order when no path is given.
pub const DEFAULT_INSTALL_CANDIDATES: &[&str] = &[
    r"C:\Program Files (x86)\PlayOnline\SquareEnix\FINAL FANTASY XI",
    r"C:\Program Files (x86)\Steam\steamapps\common\FFXINA\SquareEnix\FINAL FANTASY XI",
];

/// Install root from `SIGNET_FFXI_ROOT`, else the first usual location that holds `VTABLE.DAT`.
pub fn find_install_root() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("SIGNET_FFXI_ROOT") {
        return Some(p.into());
    }
    DEFAULT_INSTALL_CANDIDATES
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.join("VTABLE.DAT").exists())
}
