//! Lifecycle Defender — unified health, repair, and clean-uninstall subsystem.
//!
//! Design authority: `docs/DEFENDER_DESIGN.md`. Phase 1 ships the read-only
//! surface: the install receipt ledger (`footprint`), the check registry
//! (`checks`), advisory-feed evaluation (`advisories`), and the pool-free
//! `clotocore doctor` CLI (`doctor`). Phase 2 adds the non-destructive
//! `repair` verb and the clean-update first-boot phase. Phase 3 adds `purge`,
//! whose enumeration half (`purge`) lands first: it produces the plan that is
//! the executor's only input. Nothing in this module deletes user data — the
//! plan-bound executor is the sole path that removes anything, and it is
//! reachable only through the uninstall flow (§8.2).

pub mod advisories;
pub mod checks;
pub mod doctor;
pub mod footprint;
pub mod purge;
pub mod repair;
