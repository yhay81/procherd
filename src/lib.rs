pub mod cli;
pub mod error;
pub mod leases;
pub mod logs;
pub mod model;
pub mod readiness;
pub mod store;
pub mod supervisor;

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_detach;

pub use cli::run;
