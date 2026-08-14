fn main() {
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, PROPERTYKEY, SHGetPropertyStoreForWindow,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };

    const PKEY_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
        pid: 5,
    };
    const PKEY_RELAUNCH: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
        pid: 2,
    };
    const PKEY_ICON: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
        pid: 3,
    };

    unsafe extern "system" fn cb(hwnd: HWND, _: LPARAM) -> BOOL {
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let mut cls = [0u16; 128];
            let cn = GetClassNameW(hwnd, &mut cls);
            let class_name = String::from_utf16_lossy(&cls[..cn as usize]);
            if class_name != "Chrome_WidgetWin_1" {
                return BOOL(1);
            }
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(hwnd, &mut buf);
            let title = if n == 0 {
                String::new()
            } else {
                String::from_utf16_lossy(&buf[..n as usize])
            };
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let read = |key: &PROPERTYKEY| -> String {
                let Ok(store) = SHGetPropertyStoreForWindow(hwnd) else {
                    return "no-store".into();
                };
                let store: IPropertyStore = store;
                let Ok(pv) = store.GetValue(key) else {
                    return "get-fail".into();
                };
                let raw = pv.as_raw();
                let vt = raw.Anonymous.Anonymous.vt;
                if vt == VT_LPWSTR.0 {
                    let p = raw.Anonymous.Anonymous.Anonymous.pwszVal;
                    if p.is_null() {
                        return format!("vt={vt} null");
                    }
                    return PCWSTR(p).to_string().unwrap_or_default();
                }
                format!("vt={vt}")
            };
            println!("hwnd={hwnd:?} pid={pid} class={class_name}");
            println!("  title={title}");
            println!("  AUMID={}", read(&PKEY_ID));
            println!("  RELAUNCH={}", read(&PKEY_RELAUNCH));
            println!("  ICON={}", read(&PKEY_ICON));
            BOOL(1)
        }
    }

    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(0));
    }
}
