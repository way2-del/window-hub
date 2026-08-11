//! Battery / AC power status for island chips + flyouts.

#![cfg(windows)]

use serde::Serialize;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerSnapshot {
    /// AC line: online | offline | unknown
    pub ac_line: String,
    /// 0–100, or None when unknown / no battery
    pub percent: Option<u8>,
    pub charging: bool,
    /// Estimated seconds remaining; None if unknown / charging / AC
    pub life_sec: Option<u32>,
    pub has_battery: bool,
}

pub fn snapshot() -> PowerSnapshot {
    unsafe {
        let mut st = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut st).is_err() {
            return PowerSnapshot {
                ac_line: "unknown".into(),
                percent: None,
                charging: false,
                life_sec: None,
                has_battery: false,
            };
        }

        let ac_line = match st.ACLineStatus {
            0 => "offline",
            1 => "online",
            _ => "unknown",
        }
        .to_string();

        let percent = if st.BatteryLifePercent <= 100 {
            Some(st.BatteryLifePercent)
        } else {
            None
        };

        let flag = st.BatteryFlag;
        let no_battery = (flag & 128) != 0;
        let charging = (flag & 8) != 0 || (ac_line == "online" && percent.is_some() && !no_battery);
        let has_battery = !no_battery && percent.is_some();

        let life_sec = if st.BatteryLifeTime != u32::MAX && has_battery && !charging {
            Some(st.BatteryLifeTime)
        } else {
            None
        };

        PowerSnapshot {
            ac_line,
            percent,
            charging: charging && has_battery,
            life_sec,
            has_battery,
        }
    }
}

pub fn open_power_settings() -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    unsafe {
        let r = ShellExecuteW(
            HWND::default(),
            windows::core::w!("open"),
            &HSTRING::from("ms-settings:batterysaver"),
            None,
            None,
            SW_SHOWNORMAL,
        );
        if (r.0 as isize) <= 32 {
            return Err(format!("ShellExecute power settings failed ({})", r.0 as isize));
        }
    }
    Ok(())
}
