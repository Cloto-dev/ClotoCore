//! Lifecycle Defender — unified health, repair, and clean-uninstall subsystem.
//!
//! Design authority: `docs/DEFENDER_DESIGN.md`. Phase 1 ships the read-only
//! surface: the install receipt ledger (`footprint`), the check registry
//! (`checks`), advisory-feed evaluation (`advisories`), and the pool-free
//! `clotocore doctor` CLI (`doctor`). Phase 2 adds the non-destructive
//! `repair` verb and the clean-update first-boot phase. The `purge` verb is
//! Phase 3 — nothing in this module deletes user data.

pub mod advisories;
pub mod checks;
pub mod doctor;
pub mod footprint;
pub mod repair;
