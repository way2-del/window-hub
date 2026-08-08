use base64::{engine::general_purpose::STANDARD as B64, Engine};
use bevy_ecs::prelude::*;
use std::time::{Duration, Instant};
use tauri::Emitter;

use crate::ecs::components::*;
use crate::ecs::resources::*;
use crate::win32::capture::capture_window_jpeg;
use crate::win32::input::{
    client_size, forward_key, forward_pointer, map_preview_to_client, KeyEvent, KeyKind,
    PointerEvent, PointerKind,
};
use crate::win32::park::{park_window, unpark_window};

#[derive(Resource, Default)]
pub struct InputQueue {
    pub pointer: Vec<HubCommand>,
    pub keys: Vec<HubCommand>,
}

pub fn apply_commands_system(
    mut commands: Commands,
    mut pending: ResMut<PendingCommands>,
    mut input_q: ResMut<InputQueue>,
    q: Query<(Entity, &Hwnd, &LayoutSlot, Option<&Parked>)>,
    mut dirty: ResMut<SlotsDirty>,
) {
    let batch = std::mem::take(&mut pending.queue);
    let mut structural = Vec::new();

    for cmd in batch {
        match &cmd {
            HubCommand::Pointer { .. } => input_q.pointer.push(cmd),
            HubCommand::Key { .. } => input_q.keys.push(cmd),
            HubCommand::Shutdown => pending.shutdown = true,
            _ => structural.push(cmd),
        }
    }

    for cmd in structural {
        match cmd {
            HubCommand::Attach {
                hwnd,
                slot,
                title,
                class_name,
                pid,
            } => {
                let mut to_detach = Vec::new();
                for (e, h, s, parked) in q.iter() {
                    if s.0 == slot || h.0 == hwnd {
                        if let Some(p) = parked {
                            let _ = unpark_window(h.0, &p.placement);
                        }
                        to_detach.push(e);
                    }
                }
                for e in to_detach {
                    commands.entity(e).despawn();
                    dirty.0 = true;
                }
                match park_window(hwnd) {
                    Ok(placement) => {
                        commands.spawn((
                            Hwnd(hwnd),
                            WindowMeta {
                                title,
                                class_name,
                                pid,
                            },
                            CaptureRoi::default(),
                            LayoutSlot(slot),
                            Parked { placement },
                            LiveFrame::default(),
                            InputEnabled(true),
                        ));
                        dirty.0 = true;
                    }
                    Err(err) => eprintln!("park_window failed: {err}"),
                }
            }
            HubCommand::Detach { slot } => {
                let mut to_detach = Vec::new();
                for (e, h, s, parked) in q.iter() {
                    if s.0 == slot {
                        if let Some(p) = parked {
                            let _ = unpark_window(h.0, &p.placement);
                        }
                        to_detach.push(e);
                    }
                }
                for e in to_detach {
                    commands.entity(e).despawn();
                    dirty.0 = true;
                }
            }
            HubCommand::SetRoi { slot, roi } => {
                for (e, _, s, _) in q.iter() {
                    if s.0 == slot {
                        commands.entity(e).insert(roi);
                        dirty.0 = true;
                    }
                }
            }
            HubCommand::SwapSlots { a, b } => {
                let mut ent_a = None;
                let mut ent_b = None;
                for (e, _, s, _) in q.iter() {
                    if s.0 == a {
                        ent_a = Some(e);
                    }
                    if s.0 == b {
                        ent_b = Some(e);
                    }
                }
                if let Some(ea) = ent_a {
                    commands.entity(ea).insert(LayoutSlot(b));
                }
                if let Some(eb) = ent_b {
                    commands.entity(eb).insert(LayoutSlot(a));
                }
                dirty.0 = true;
            }
            _ => {}
        }
    }
}

pub fn emit_slots_system(
    mut dirty: ResMut<SlotsDirty>,
    emitter: Res<AppEmitter>,
    q: Query<(&Hwnd, &LayoutSlot, &WindowMeta, &CaptureRoi)>,
) {
    if !dirty.0 {
        return;
    }
    dirty.0 = false;

    let mut slots: Vec<SlotInfo> = (0..3)
        .map(|i| SlotInfo {
            slot: i,
            hwnd: None,
            title: None,
            class_name: None,
            pid: None,
            roi: None,
        })
        .collect();

    for (h, s, meta, roi) in q.iter() {
        if (s.0 as usize) < slots.len() {
            let info = &mut slots[s.0 as usize];
            info.hwnd = Some(h.0);
            info.title = Some(meta.title.clone());
            info.class_name = Some(meta.class_name.clone());
            info.pid = Some(meta.pid);
            info.roi = Some(*roi);
        }
    }

    let _ = emitter.app.emit("slots", SlotStateEvent { slots });
}

pub fn capture_system(
    mut cfg: ResMut<CaptureConfig>,
    mut q: Query<(&Hwnd, &CaptureRoi, &mut LiveFrame)>,
) {
    let interval = Duration::from_secs_f64(1.0 / cfg.fps.max(1) as f64);
    if cfg.last_capture.elapsed() < interval {
        return;
    }
    cfg.last_capture = Instant::now();

    for (hwnd, roi, mut frame) in q.iter_mut() {
        match capture_window_jpeg(hwnd.0, roi.to_win32()) {
            Ok(cap) => {
                frame.jpeg = cap.jpeg;
                frame.width = cap.width;
                frame.height = cap.height;
                frame.seq = frame.seq.wrapping_add(1);
                frame.dirty = true;
            }
            Err(err) => {
                eprintln!("capture failed hwnd={}: {err}", hwnd.0);
            }
        }
    }
}

pub fn emit_frames_system(emitter: Res<AppEmitter>, mut q: Query<(&LayoutSlot, &mut LiveFrame)>) {
    for (slot, mut frame) in q.iter_mut() {
        if !frame.dirty || frame.jpeg.is_empty() {
            continue;
        }
        frame.dirty = false;
        let payload = FrameEvent {
            slot: slot.0,
            width: frame.width,
            height: frame.height,
            seq: frame.seq,
            jpeg_base64: B64.encode(&frame.jpeg),
        };
        let _ = emitter.app.emit("frame", payload);
    }
}

pub fn input_forward_system(
    mut input_q: ResMut<InputQueue>,
    q: Query<(&Hwnd, &LayoutSlot, &CaptureRoi, &InputEnabled)>,
) {
    let pointers = std::mem::take(&mut input_q.pointer);
    let keys = std::mem::take(&mut input_q.keys);

    for cmd in pointers {
        let HubCommand::Pointer {
            slot,
            kind,
            norm_x,
            norm_y,
            buttons,
            delta_y,
        } = cmd
        else {
            continue;
        };

        for (hwnd, s, roi, enabled) in q.iter() {
            if s.0 != slot || !enabled.0 {
                continue;
            }
            let Ok((cw, ch)) = client_size(hwnd.0) else {
                continue;
            };
            let (x, y) = map_preview_to_client(norm_x, norm_y, roi.to_win32(), cw, ch);
            let kind = match kind {
                PointerKindDto::Down => PointerKind::Down,
                PointerKindDto::Move => PointerKind::Move,
                PointerKindDto::Up => PointerKind::Up,
                PointerKindDto::Wheel => PointerKind::Wheel,
            };
            let _ = forward_pointer(
                hwnd.0,
                &PointerEvent {
                    kind,
                    x,
                    y,
                    buttons,
                    delta_y,
                },
            );
        }
    }

    for cmd in keys {
        let HubCommand::Key {
            slot,
            kind,
            vk,
            scan,
            text,
        } = cmd
        else {
            continue;
        };

        for (hwnd, s, _, enabled) in q.iter() {
            if s.0 != slot || !enabled.0 {
                continue;
            }
            let kind = match kind {
                KeyKindDto::Down => KeyKind::Down,
                KeyKindDto::Up => KeyKind::Up,
                KeyKindDto::Char => KeyKind::Char,
            };
            let _ = forward_key(
                hwnd.0,
                &KeyEvent {
                    kind,
                    vk,
                    scan,
                    text: text.clone(),
                },
            );
        }
    }
}

pub fn force_unpark_all(world: &mut World) {
    let mut to_clear = Vec::new();
    {
        let mut q = world.query::<(Entity, &Hwnd, &Parked)>();
        for (e, h, p) in q.iter(world) {
            let _ = unpark_window(h.0, &p.placement);
            to_clear.push(e);
        }
    }
    for e in to_clear {
        world.despawn(e);
    }
}
