# Window Hub

Windows-only desktop tool that parks ordinary application windows off-screen, captures a configurable region of each into a **3-column mosaic**, and forwards mouse/keyboard input from the preview tiles back to the real windows.

## Stack

- **Tauri 2** + React/Vite UI
- **Rust** backend with standalone **`bevy_ecs`**
- Win32: `EnumWindows`, off-screen park, `PrintWindow(PW_RENDERFULLCONTENT)`, `PostMessage` input

## Run

```bash
npm install
npm run tauri dev
```

Requires Rust (MSVC), WebView2, and Windows 10+.

## Usage

1. Click **附着窗口** on an empty slot and pick a top-level window.
2. The target is moved off-screen (parked) and appears in the mosaic.
3. Click/type inside the preview to operate the real window.
4. Use **截取区域** to drag a ROI; **全窗** resets; **分离** restores the window.

## Limitations

- “Minimize” is implemented as **off-screen parking** (true `SW_MINIMIZE` usually blacks out capture and breaks input).
- Some DirectUI / elevated / UWP apps may ignore `PostMessage` input.
- Games and exclusive fullscreen are out of scope.
- Accelerated windows generally need `PW_RENDERFULLCONTENT`; rare black frames may need a future WGC path.
