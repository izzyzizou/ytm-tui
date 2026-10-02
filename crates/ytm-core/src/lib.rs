//! The daemon's brain. One task owns [`state::PlayerState`]; everything else sends it
//! [`reducer::Input`]s. [`reducer::reduce`] is pure and returns [`reducer::Effect`]s that the
//! daemon executes — so all player logic is unit-testable without audio or network.

pub mod client;
pub mod config;
pub mod daemon;
pub mod lrc;
pub mod lyrics;
pub mod paths;
pub mod protocol;
pub mod reducer;
pub mod state;
