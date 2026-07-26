//! Lifecycle Defender — unified health, repair, and clean-uninstall subsystem.
//!
//! Design authority: `docs/DEFENDER_DESIGN.md`. Phase 1 ships the read-only
//! surface: the install receipt ledger (`footprint`), the check registry
//! (`checks`), advisory-feed evaluation (`advisories`), and the pool-free
//! `clotocore doctor` CLI (`doctor`). Phase 2 adds the non-destructive
//! `repair` verb and the clean-update first-boot phase. Phase 3 splits purge
//! in two: `purge` enumerates and produces the plan, and `purge_exec` is the
//! plan-bound executor that consumes it. `purge_exec` is the only code here
//! that deletes anything, it removes exactly what a plan lists (§8.5), and it
//! is reachable only through the uninstall flow (§8.2).

pub mod advisories;
pub mod checks;
pub mod doctor;
pub mod footprint;
pub mod purge;
pub mod purge_exec;
pub mod repair;
