#[cfg(feature = "code-mode")]
#[path = "code_mode_impl.rs"]
mod code_mode_impl;
#[cfg(not(feature = "code-mode"))]
#[path = "code_mode_stub.rs"]
mod code_mode_stub;

#[cfg(feature = "code-mode")]
pub use code_mode_impl::*;
#[cfg(not(feature = "code-mode"))]
pub use code_mode_stub::*;
