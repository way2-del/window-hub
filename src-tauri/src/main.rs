#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--autostart-svc") {
        #[cfg(windows)]
        window_hub_lib::run_autostart_service();
        return;
    }
    // One-shot elevated helper: install SCM autostart then exit (no GUI stay-elevated).
    if args.iter().any(|a| a == "--install-autostart-service") {
        #[cfg(windows)]
        {
            let code = match window_hub_lib::install_autostart_service_once() {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("[autostart] install service failed: {e}");
                    1
                }
            };
            std::process::exit(code);
        }
        #[cfg(not(windows))]
        std::process::exit(1);
    }
    if args.iter().any(|a| a == "--uninstall-autostart-service") {
        #[cfg(windows)]
        {
            let code = match window_hub_lib::uninstall_autostart_service_once() {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("[autostart] uninstall service failed: {e}");
                    1
                }
            };
            std::process::exit(code);
        }
        #[cfg(not(windows))]
        std::process::exit(1);
    }
    #[cfg(windows)]
    window_hub_lib::ensure_single_instance();
    window_hub_lib::run()
}
