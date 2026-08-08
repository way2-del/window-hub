use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tauri::AppHandle;

use crate::ecs::components::CaptureRoi;

#[derive(Resource, Default)]
pub struct PendingCommands {
    pub queue: Vec<HubCommand>,
    pub shutdown: bool,
}

#[derive(Resource)]
#[allow(dead_code)]
pub struct LayoutConfig {
    pub columns: u8,
}

#[derive(Resource)]
pub struct CaptureConfig {
    pub fps: u32,
    pub last_capture: Instant,
}

#[derive(Resource)]
pub struct AppEmitter {
    pub app: AppHandle,
}

#[derive(Resource, Default)]
pub struct SlotsDirty(pub bool);

#[derive(Debug, Clone)]
pub enum HubCommand {
    Attach {
        hwnd: isize,
        slot: u8,
        title: String,
        class_name: String,
        pid: u32,
    },
    Detach {
        slot: u8,
    },
    SetRoi {
        slot: u8,
        roi: CaptureRoi,
    },
    SwapSlots {
        a: u8,
        b: u8,
    },
    Pointer {
        slot: u8,
        kind: PointerKindDto,
        norm_x: f64,
        norm_y: f64,
        buttons: u32,
        delta_y: i32,
    },
    Key {
        slot: u8,
        kind: KeyKindDto,
        vk: u16,
        scan: u16,
        text: Option<String>,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerKindDto {
    Down,
    Move,
    Up,
    Wheel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKindDto {
    Down,
    Up,
    Char,
}

#[derive(Clone, Serialize)]
pub struct FrameEvent {
    pub slot: u8,
    pub width: u32,
    pub height: u32,
    pub seq: u64,
    pub jpeg_base64: String,
}

#[derive(Clone, Serialize)]
pub struct SlotStateEvent {
    pub slots: Vec<SlotInfo>,
}

#[derive(Clone, Serialize)]
pub struct SlotInfo {
    pub slot: u8,
    pub hwnd: Option<isize>,
    pub title: Option<String>,
    pub class_name: Option<String>,
    pub pid: Option<u32>,
    pub roi: Option<CaptureRoi>,
}
