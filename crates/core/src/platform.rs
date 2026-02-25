//! Platform-specific service management, permissions, and binary update operations.
//! Each platform module exposes the same public interface, selected at compile time via #[cfg].

#[cfg(unix)]
mod linux;
#[cfg(unix)]
pub use linux::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
