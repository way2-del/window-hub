use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::win32::park::PlacementSnapshot;

#[derive(Component, Clone, Copy, Debug)]
pub struct Hwnd(pub isize);

#[derive(Component, Clone, Debug)]
pub struct WindowMeta {
    pub title: String,
    pub class_name: String,
    pub pid: u32,
}

#[derive(Component, Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CaptureRoi {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub use_full: bool,
}

impl Default for CaptureRoi {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            use_full: true,
        }
    }
}

impl CaptureRoi {
    pub fn to_win32(self) -> crate::win32::capture::Roi {
        crate::win32::capture::Roi {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
            use_full: self.use_full,
        }
    }
}

#[derive(Component, Clone, Copy, Debug)]
pub struct LayoutSlot(pub u8);

#[derive(Component)]
pub struct Parked {
    pub placement: PlacementSnapshot,
}

#[derive(Component, Default)]
pub struct LiveFrame {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub seq: u64,
    pub dirty: bool,
}

#[derive(Component, Clone, Copy)]
pub struct InputEnabled(pub bool);
