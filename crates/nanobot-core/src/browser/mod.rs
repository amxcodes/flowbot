// Browser automation module
#[cfg(feature = "browser")]
pub mod client;
#[cfg(feature = "browser")]
pub mod actions;
#[cfg(feature = "browser")]
pub mod tools;

#[cfg(feature = "browser")]
pub use client::BrowserClient;
#[cfg(feature = "browser")]
pub use actions::BrowserActions;
