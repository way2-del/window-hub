pub mod components;
pub mod resources;
pub mod systems;

use bevy_ecs::prelude::*;
use parking_lot::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::AppHandle;

use crate::ecs::resources::{
    AppEmitter, CaptureConfig, HubCommand, LayoutConfig, PendingCommands, SlotsDirty,
};
use crate::ecs::systems::{
    apply_commands_system, capture_system, emit_frames_system, emit_slots_system, force_unpark_all,
    input_forward_system, InputQueue,
};

pub struct EcsHandle {
    pub tx: Sender<HubCommand>,
    _join: JoinHandle<()>,
}

impl EcsHandle {
    pub fn send(&self, cmd: HubCommand) {
        let _ = self.tx.send(cmd);
    }
}

pub fn spawn_ecs_thread(app: AppHandle) -> EcsHandle {
    let (tx, rx) = mpsc::channel::<HubCommand>();
    let join = thread::spawn(move || ecs_loop(app, rx));
    EcsHandle { tx, _join: join }
}

fn ecs_loop(app: AppHandle, rx: Receiver<HubCommand>) {
    let mut world = World::new();
    world.insert_resource(PendingCommands::default());
    world.insert_resource(InputQueue::default());
    world.insert_resource(SlotsDirty::default());
    world.insert_resource(LayoutConfig { columns: 3 });
    world.insert_resource(CaptureConfig {
        // Mosaic UI is uncommon; 8fps is enough and halves PrintWindow+JPEG cost.
        fps: 8,
        last_capture: Instant::now() - Duration::from_secs(1),
    });
    world.insert_resource(AppEmitter { app });
    world.insert_resource(CommandIngress {
        rx: Arc::new(Mutex::new(rx)),
    });

    let mut schedule = Schedule::default();
    schedule.add_systems(
        (
            drain_channel_system,
            apply_commands_system,
            emit_slots_system,
            capture_system,
            emit_frames_system,
            input_forward_system,
        )
            .chain(),
    );

    loop {
        let start = Instant::now();
        schedule.run(&mut world);

        if world
            .get_resource::<PendingCommands>()
            .map(|p| p.shutdown)
            .unwrap_or(false)
        {
            force_unpark_all(&mut world);
            break;
        }

        // No attached mosaic windows → idle downclock (still drain cmds promptly).
        let idle = {
            let mut q = world.query::<&crate::ecs::components::LiveFrame>();
            q.iter(&world).next().is_none()
        };
        let frame = if idle {
            Duration::from_millis(80)
        } else {
            Duration::from_millis(16)
        };
        let elapsed = start.elapsed();
        if elapsed < frame {
            thread::sleep(frame - elapsed);
        }
    }
}

#[derive(Resource)]
struct CommandIngress {
    rx: Arc<Mutex<Receiver<HubCommand>>>,
}

fn drain_channel_system(ingress: Res<CommandIngress>, mut pending: ResMut<PendingCommands>) {
    let rx = ingress.rx.lock();
    while let Ok(cmd) = rx.try_recv() {
        if matches!(cmd, HubCommand::Shutdown) {
            pending.shutdown = true;
        }
        pending.queue.push(cmd);
    }
}
