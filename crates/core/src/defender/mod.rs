//! Lifecycle Defender — unified health, repair, and clean-uninstall subsystem.
//!
//! Design authority: `docs/DEFENDER_DESIGN.md`. Phase 1 ships the read-only
//! surface: the install receipt ledger (`footprint`), the check registry
//! (`checks`), advisory-feed evaluation (`advisories`), and the pool-free
//! `clotocore doctor` CLI (`doctor`). The `repair` and `purge` verbs are
//! Phase 2/3 — nothing in this module deletes user data.

pub mod advisories;
pub mod checks;
pub mod doctor;
pub mod footprint;
