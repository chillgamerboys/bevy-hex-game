//! Safe, store-neutral online-session boundary.
//!
//! The real EOS implementation will own identity, lobby callbacks, and P2P packet I/O
//! here. This foundation fixes the safe interface and default-off lifecycle first:
//! installing [`OnlinePlugin`] neither loads the EOS runtime nor opens a network session.

use std::{collections::VecDeque, fmt, path::PathBuf};

use bevy_app::{App, Plugin, Update};
use bevy_ecs::{
    message::{MessageReader, MessageWriter},
    prelude::Resource,
};
use hex_eos_ffi::{EosRuntimeLibrary, EosRuntimeLoadError, EosRuntimeVersion};
use hex_multiplayer::{
    OnlineServiceRefusal, OnlineSessionEvent, OnlineSessionOperation, OnlineSessionRequest,
};

/// Safe backend contract implemented by EOS and deterministic test doubles.
pub trait OnlineBackend: Send + Sync + 'static {
    /// Accepts one explicit application request.
    fn submit(&mut self, request: OnlineSessionRequest);
    /// Polls SDK callbacks and packet I/O once without blocking the Bevy frame.
    fn tick(&mut self);
    /// Drains typed, disclosure-safe outcomes in callback order.
    fn drain_events(&mut self) -> Vec<OnlineSessionEvent>;
}

/// Injected online backend. `None` is the socket-free/source-build default.
#[derive(Resource, Default)]
pub struct OnlineBackendSlot(Option<Box<dyn OnlineBackend>>);

impl fmt::Debug for OnlineBackendSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_some() {
            formatter.write_str("OnlineBackendSlot([INJECTED BACKEND])")
        } else {
            formatter.write_str("OnlineBackendSlot(None)")
        }
    }
}

impl OnlineBackendSlot {
    /// Installs an explicitly constructed safe backend.
    pub fn install(&mut self, backend: impl OnlineBackend) {
        self.0 = Some(Box::new(backend));
    }

    /// Removes the current backend after its owner has closed sessions and callbacks.
    pub fn clear(&mut self) {
        self.0 = None;
    }

    /// Whether an explicit online backend is installed.
    #[must_use]
    pub fn is_installed(&self) -> bool {
        self.0.is_some()
    }
}

/// Explicit release-staged EOS runtime configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EosRuntimeConfig {
    /// Absolute path to the checksum-verified official target runtime.
    pub library_path: PathBuf,
}

/// Safe proof that the explicitly staged runtime matches the pinned header baseline.
#[derive(Debug)]
pub struct EosRuntimeProbe {
    library: EosRuntimeLibrary,
    version: EosRuntimeVersion,
}

impl EosRuntimeProbe {
    /// Loads only the selected runtime and reads its version; no EOS platform or socket is
    /// created.
    pub fn load(config: &EosRuntimeConfig) -> Result<Self, EosRuntimeLoadError> {
        let library = EosRuntimeLibrary::load_explicit(&config.library_path)?;
        let version = library.version()?;
        Ok(Self { library, version })
    }

    /// Verified runtime version.
    #[must_use]
    pub const fn version(&self) -> &EosRuntimeVersion {
        &self.version
    }

    /// Explicit staged runtime path.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        self.library.path()
    }
}

/// Deterministic safe backend for headless composition tests.
///
/// Requests are retained in order and scripted events are emitted on the next tick. No
/// runtime, thread, socket, or external service is used.
#[derive(Debug, Default)]
pub struct ScriptedOnlineBackend {
    requests: Vec<OnlineSessionRequest>,
    queued: VecDeque<OnlineSessionEvent>,
    ready: Vec<OnlineSessionEvent>,
}

impl ScriptedOnlineBackend {
    /// Queues a typed event for the next backend tick.
    pub fn queue_event(&mut self, event: OnlineSessionEvent) {
        self.queued.push_back(event);
    }

    /// Submitted requests in stable order.
    #[must_use]
    pub fn requests(&self) -> &[OnlineSessionRequest] {
        &self.requests
    }
}

impl OnlineBackend for ScriptedOnlineBackend {
    fn submit(&mut self, request: OnlineSessionRequest) {
        self.requests.push(request);
    }

    fn tick(&mut self) {
        self.ready.extend(self.queued.drain(..));
    }

    fn drain_events(&mut self) -> Vec<OnlineSessionEvent> {
        std::mem::take(&mut self.ready)
    }
}

/// Installs default-off online request/event plumbing.
pub struct OnlinePlugin;

impl Plugin for OnlinePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OnlineBackendSlot>()
            .add_message::<OnlineSessionRequest>()
            .add_message::<OnlineSessionEvent>()
            .add_systems(Update, drive_online_backend);
    }
}

fn drive_online_backend(
    mut requests: MessageReader<OnlineSessionRequest>,
    mut events: MessageWriter<OnlineSessionEvent>,
    mut backend: bevy_ecs::prelude::ResMut<OnlineBackendSlot>,
) {
    let incoming = requests.read().cloned().collect::<Vec<_>>();
    let Some(backend) = backend.0.as_mut() else {
        for request in incoming {
            events.write(OnlineSessionEvent::Refused {
                operation: operation(&request),
                reason: OnlineServiceRefusal::Disabled,
            });
        }
        return;
    };
    for request in incoming {
        backend.submit(request);
    }
    backend.tick();
    for event in backend.drain_events() {
        events.write(event);
    }
}

const fn operation(request: &OnlineSessionRequest) -> OnlineSessionOperation {
    match request {
        OnlineSessionRequest::Host => OnlineSessionOperation::Host,
        OnlineSessionRequest::JoinCode(_) => OnlineSessionOperation::Join,
        OnlineSessionRequest::Reconnect => OnlineSessionOperation::Reconnect,
        OnlineSessionRequest::Leave => OnlineSessionOperation::Leave,
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::message::Messages;

    use super::*;

    #[test]
    fn plugin_is_default_off_and_refuses_without_loading_or_networking() {
        let mut app = App::new();
        app.add_plugins(OnlinePlugin);
        assert!(!app.world().resource::<OnlineBackendSlot>().is_installed());
        app.world_mut()
            .resource_mut::<Messages<OnlineSessionRequest>>()
            .write(OnlineSessionRequest::Host);
        app.update();
        let events = app
            .world()
            .resource::<Messages<OnlineSessionEvent>>()
            .iter_current_update_messages()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            events,
            [OnlineSessionEvent::Refused {
                operation: OnlineSessionOperation::Host,
                reason: OnlineServiceRefusal::Disabled,
            }]
        );
    }

    #[test]
    fn scripted_backend_preserves_request_and_callback_order() {
        let mut backend = ScriptedOnlineBackend::default();
        backend.queue_event(OnlineSessionEvent::Left);
        backend.submit(OnlineSessionRequest::Leave);
        backend.tick();
        assert_eq!(backend.requests(), [OnlineSessionRequest::Leave]);
        assert_eq!(backend.drain_events(), [OnlineSessionEvent::Left]);
        assert!(backend.drain_events().is_empty());
    }
}
