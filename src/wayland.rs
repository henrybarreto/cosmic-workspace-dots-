//! Workspace state via ext-workspace. One thread owns the Wayland connection, reconnects
//! when it drops, pushes snapshots out and takes activation requests in.
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cosmic::iced::futures::channel::mpsc::UnboundedSender;
use cosmic_client_toolkit::{
    sctk::{
        output::{OutputHandler, OutputState},
        reexports::{calloop, calloop_wayland_source::WaylandSource},
        registry::{ProvidesRegistryState, RegistryState},
    },
    wayland_client::{
        Connection, Proxy, QueueHandle, globals::registry_queue_init, protocol::wl_output,
    },
    wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1::State,
    workspace::{WorkspaceHandler, WorkspaceState},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dot {
    /// Wayland object id of the workspace; unique across groups/outputs.
    pub id: String,
    pub active: bool,
}

/// Workspace groups (one per output, usually), each sorted by coordinates.
pub type Snapshot = Vec<Vec<Dot>>;

/// Where the current session's command sender lives, so `Watcher` stays valid across reconnects.
#[derive(Debug, Default)]
struct CmdSlot(Mutex<Option<calloop::channel::Sender<String>>>);

impl CmdSlot {
    fn set(&self, sender: Option<calloop::channel::Sender<String>>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = sender;
    }

    fn send(&self, id: String) {
        if let Some(sender) = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            let _ = sender.send(id);
        }
    }
}

type CmdSender = Arc<CmdSlot>;

/// Handle for sending activation requests to the watcher thread.
#[derive(Debug, Clone, Default)]
pub struct Watcher {
    cmd: CmdSender,
}

impl Watcher {
    /// Start the thread; it exits once `tx` is closed.
    pub fn spawn(tx: UnboundedSender<Snapshot>) -> Self {
        let watcher = Self::default();
        let cmd = watcher.cmd.clone();
        std::thread::spawn(move || {
            while !tx.is_closed() {
                Session::run(&tx, &cmd);
                std::thread::sleep(Duration::from_secs(1)); // reconnect backoff
            }
        });
        watcher
    }

    pub fn activate(&self, id: String) {
        self.cmd.send(id);
    }
}

struct Session {
    conn: Connection,
    out: OutputState,
    reg: RegistryState,
    ws: WorkspaceState,
    tx: UnboundedSender<Snapshot>,
}

impl Session {
    /// Runs until the connection breaks or the applet goes away.
    fn run(tx: &UnboundedSender<Snapshot>, cmd: &CmdSender) {
        let Ok(conn) = Connection::connect_to_env() else {
            return;
        };
        let Ok((globals, queue)) = registry_queue_init(&conn) else {
            return;
        };
        let Ok(mut event_loop) = calloop::EventLoop::<Self>::try_new() else {
            return;
        };
        let qh = queue.handle();
        let reg = RegistryState::new(&globals);
        let mut session = Self {
            conn: conn.clone(),
            out: OutputState::new(&globals, &qh),
            ws: WorkspaceState::new(&reg, &qh),
            reg,
            tx: tx.clone(),
        };

        let handle = event_loop.handle();
        if WaylandSource::new(conn, queue)
            .insert(handle.clone())
            .is_err()
        {
            return;
        }
        let (sender, receiver) = calloop::channel::channel::<String>();
        let inserted = handle.insert_source(receiver, |event, (), session| {
            if let calloop::channel::Event::Msg(id) = event {
                session.activate(&id);
            }
        });
        if inserted.is_err() {
            return;
        }
        cmd.set(Some(sender));

        // Wake periodically to notice that the applet has gone away.
        while !tx.is_closed() {
            if event_loop
                .dispatch(Some(Duration::from_secs(1)), &mut session)
                .is_err()
            {
                break;
            }
        }
        cmd.set(None);
    }

    fn activate(&self, id: &str) {
        let Some(workspace) = self
            .ws
            .workspaces()
            .find(|w| w.handle.id().to_string() == id)
        else {
            return;
        };
        workspace.handle.activate();
        if let Ok(manager) = self.ws.workspace_manager().get() {
            manager.commit();
        }
        let _ = self.conn.flush();
    }

    fn snapshot(&self) -> Snapshot {
        let mut groups = Vec::new();
        for group in self.ws.workspace_groups() {
            let mut workspaces: Vec<_> = self
                .ws
                .workspaces()
                .filter(|w| group.workspaces.contains(&w.handle))
                .filter(|w| !w.state.contains(State::Hidden))
                .collect();
            workspaces.sort_by(|a, b| a.coordinates.cmp(&b.coordinates));
            let dots: Vec<Dot> = workspaces
                .iter()
                .map(|w| Dot {
                    id: w.handle.id().to_string(),
                    active: w.state.contains(State::Active),
                })
                .collect();
            if !dots.is_empty() {
                // Order groups like the monitors: by their output's position, left to right.
                let pos = group
                    .outputs
                    .first()
                    .and_then(|o| self.out.info(o))
                    .and_then(|i| i.logical_position);
                groups.push((pos, dots));
            }
        }
        groups.sort_by_key(|(pos, _)| *pos);
        groups.into_iter().map(|(_, dots)| dots).collect()
    }
}

impl ProvidesRegistryState for Session {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.reg
    }
    cosmic_client_toolkit::sctk::registry_handlers!(OutputState);
}

// Outputs must be bound so the workspace groups receive their output events.
impl OutputHandler for Session {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.out
    }
    // Output positions decide group order, and may arrive after the workspace `done`.
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.done();
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.done();
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl WorkspaceHandler for Session {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.ws
    }

    fn done(&mut self) {
        let _ = self.tx.unbounded_send(self.snapshot()); // unbounded: never drops the latest
    }
}

cosmic_client_toolkit::sctk::delegate_output!(Session);
cosmic_client_toolkit::sctk::delegate_registry!(Session);
cosmic_client_toolkit::delegate_workspace!(Session);
