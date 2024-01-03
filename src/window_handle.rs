use std::error::Error;
use std::fmt;

use raw_window_handle::RawWindowHandle;
use windows::Win32::Foundation::HWND;

pub struct WindowHandle {
    handle: HWND,
}

#[derive(Debug)]
pub struct UnsupportedPlatformError();

impl fmt::Display for UnsupportedPlatformError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Tried to get a Win32 handle on a non-Windows platform")
    }
}

impl Error for UnsupportedPlatformError {
    fn description(&self) -> &str {
        "Tried to get a Win32 handle on a non-Windows platform"
    }
}

impl TryFrom<RawWindowHandle> for WindowHandle {
    type Error = UnsupportedPlatformError;

    fn try_from(handle: RawWindowHandle) -> Result<Self, Self::Error> {
        match handle {
            RawWindowHandle::Win32(handle) => Ok(WindowHandle {
                handle: HWND(handle.hwnd.into()),
            }),
            _ => Err(UnsupportedPlatformError()),
        }
    }
}

impl From<WindowHandle> for HWND {
    fn from(handle: WindowHandle) -> Self {
        handle.handle
    }
}
