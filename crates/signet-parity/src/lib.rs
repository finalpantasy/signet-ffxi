//! Retail parity harness.
//!
//! * `pe`, `sig`, `layout`: byte-pattern signatures that locate the retail camera, the player
//!   entity and the projection constants inside FFXiMain.dll, in a live process image or in a
//!   statically unpacked image file. Signatures use wildcards over operands, so a patched build
//!   still resolves as long as the code shape is unchanged.
//! * `rows`: the 60 Hz CSV format shared by the retail probe and Signet's `--parity-log`.
//! * `compare`: the retail-versus-Signet metric comparator.
//! * `synth`: a synthetic retail-like recording used to test the comparator.
//!
//! Coordinates follow the game: x, y (down is positive, up is negative) and z. Everything here is
//! read-only analysis; nothing writes to a game process.

pub mod compare;
pub mod layout;
#[cfg(windows)]
pub mod live;
pub mod pe;
pub mod rows;
pub mod sig;
pub mod synth;
