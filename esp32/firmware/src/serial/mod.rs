//! The console's serial line (log, `printf`, screenshots): the UART bridge or
//! the P4's own USB port, depending on the board.

#[cfg(not(feature = "guition"))]
mod uart;
#[cfg(not(feature = "guition"))]
pub use uart::*;

#[cfg(feature = "guition")]
mod usb;
#[cfg(feature = "guition")]
pub use usb::*;
