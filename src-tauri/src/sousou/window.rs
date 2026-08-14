//! Sousou floating window — solid light chrome (no Mica).

use tauri::{AppHandle, DragDropEvent, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use super::config;
use super::drop_ingest;

const LABEL: &str = "sousou";

/// Visible and not minimized — taskbar/minimize still reports `is_visible() == true`.
fn is_front_ready(w: &WebviewWindow) -> bool {
    w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false)
}

fn show_front(w: &WebviewWindow) {
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
}

pub async fn toggle(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(LABEL) {
        // Minimized counts as "not shown" — restore instead of hide.
        if is_front_ready(&w) {
            let _ = w.hide();
            return Ok(());
        }
        show_front(&w);
        return Ok(());
    }
    open(app).await
}

pub async fn open(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(LABEL) {
        show_front(&w);
        return Ok(());
    }
    create_window(app, true).await
}

/// Create hidden window for faster first toggle (no focus).
/// Prefer not calling this — hidden create breaks OS file drag-drop on WebView2.
pub async fn warm(app: AppHandle) -> Result<(), String> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    create_window(app, false).await
}

async fn create_window(app: AppHandle, show: bool) -> Result<(), String> {
    let cfg = config::load();
    let w = cfg.window_width.max(720.0);
    let h = cfg.window_height.max(480.0);

    let init = r#"
      window.__WH_IS_SOUSOU__ = true;
      document.documentElement.style.background = '#f0f1f3';
      document.body.style.background = '#f0f1f3';
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape') {
          try { window.__TAURI__.core.invoke('sousou_hide'); } catch (_) {}
        }
      });
    "#;

    // IMPORTANT: create visible so WebView2 child HWNDs exist when wry
    // registers IDropTarget. Building with visible(false) → permanent no-drop cursor.
    let win = WebviewWindowBuilder::new(
        &app,
        LABEL,
        WebviewUrl::App("index.html?window=sousou".into()),
    )
    .title("搜搜")
    .inner_size(w, h)
    .min_inner_size(720.0, 480.0)
    .resizable(true)
    .maximizable(true)
    .minimizable(true)
    .closable(true)
    .decorations(true)
    .transparent(false)
    .always_on_top(false)
    .skip_taskbar(false)
    .center()
    .focused(show)
    .visible(true)
    .initialization_script(init)
    .build()
    .map_err(|e| format!("open sousou failed: {e}"))?;

    let app2 = app.clone();
    win.on_window_event(move |ev| match ev {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Some(w) = app2.get_webview_window(LABEL) {
                let _ = w.hide();
            }
        }
        WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
            let paths: Vec<String> = paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            let app3 = app2.clone();
            std::thread::spawn(move || {
                let result = drop_ingest::ingest_dropped_paths(paths);
                drop_ingest::emit_drop_result(&app3, result);
            });
        }
        WindowEvent::DragDrop(DragDropEvent::Enter { paths, .. }) => {
            // Early signal so UI can highlight; ingest still happens on Drop.
            let _ = paths;
        }
        _ => {}
    });

    if show {
        show_front(&win);
    } else {
        // Brief delay so drag-drop child registration can finish while visible.
        std::thread::sleep(std::time::Duration::from_millis(120));
        let _ = win.hide();
    }
    Ok(())
}

pub fn hide(app: &AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
    Ok(())
}

pub fn is_visible(app: &AppHandle) -> bool {
    app.get_webview_window(LABEL)
        .map(|w| is_front_ready(&w))
        .unwrap_or(false)
}

/// Destroy any existing sousou window so the next open recreates drag-drop targets.
pub fn recreate_next_open(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.destroy();
    }
}
