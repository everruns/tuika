//! The crate's own test scaffolding, compiled only under `#[cfg(test)]`.
//!
//! Per-module unit tests stay beside the code they cover, in each module's own
//! `#[cfg(test)] mod tests`. What lives here is everything that has no single
//! owning module — cross-module integration, solver property tests, golden
//! screen snapshots — plus the helpers they share. Keeping it in one subtree is
//! what lets the crate root read as public API and nothing else.
//!
//! Nothing here ships in the library. Public test helpers for *consumers* are a
//! different thing and live in [`crate::testing`].

mod fuzz;
mod integration;
mod proptests;
mod snapshots;
pub(crate) mod support;
