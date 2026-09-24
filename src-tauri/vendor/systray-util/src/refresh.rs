//! Synthetic TaskbarCreated requests must never target the hosting UI process.
use windows::{
  core::w,
  Win32::{
    Foundation::{BOOL, HWND, LPARAM},
    UI::WindowsAndMessaging::{
      EnumWindows, GetWindowThreadProcessId, PostMessageW,
      RegisterWindowMessageW,
    },
  },
};

fn is_external(owner: u32, current: u32) -> bool {
  owner != 0 && owner != current
}

unsafe extern "system" fn notify(hwnd: HWND, message: LPARAM) -> BOOL {
  let mut owner = 0;
  GetWindowThreadProcessId(hwnd, Some(&mut owner));
  if is_external(owner, std::process::id()) {
    // Posted, not sent: a slow receiver must not hold up tray initialization.
    let _ = PostMessageW(hwnd, message.0 as u32, None, None);
  }
  BOOL(1)
}

/// Ask other applications to republish their tray icons without notifying self.
/// Tao's TaskbarCreated handler holds WindowState while calling ITaskbarList;
/// Explorer can reenter that HWND and deadlock the same mutex during startup.
pub fn refresh_taskbar_icons() -> crate::Result<()> {
  unsafe {
    let message = RegisterWindowMessageW(w!("TaskbarCreated"));
    if message == 0 {
      return Err(windows::core::Error::from_win32().into());
    }
    EnumWindows(Some(notify), LPARAM(message as isize))?;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  #[test]
  fn synthetic_refresh_excludes_self_and_invalid_owners() {
    assert!(!super::is_external(42, 42));
    assert!(!super::is_external(0, 42));
    assert!(super::is_external(43, 42));
  }
}
