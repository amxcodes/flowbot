// Browser automation module
#[cfg(feature = "browser")]
pub mod actions;
#[cfg(feature = "browser")]
pub mod client;
#[cfg(feature = "browser")]
pub mod tools;

#[cfg(feature = "browser")]
pub use actions::BrowserActions;
#[cfg(feature = "browser")]
pub use client::BrowserClient;
