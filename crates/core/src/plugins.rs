//! In-tree plugin implementations.
//!
//! Per ARCHITECTURE.md §1.4 (Data Sovereignty) — "The Kernel holds the data,
//! but does not interpret its contents." — plugin-specific schemas that the
//! kernel would otherwise be tempted to deserialize live here instead of in the
//! kernel's HTTP handlers. The kernel passes opaque `metadata` (JSON) down to
//! these in-tree plugins, which own the concrete schema and its interpretation.
//!
//! These run as direct in-process function calls (no event-bus round-trip), so
//! relocating schema ownership here carries no latency cost.

pub(crate) mod routing_default;
