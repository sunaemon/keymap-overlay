//! Typed D-Bus contract shared by Linux overlay renderers.

use std::sync::{Arc, Mutex};

pub const BUS_NAME: &str = "com.sunaemon.KeymapOverlay";
pub const OBJECT_PATH: &str = "/com/sunaemon/KeymapOverlay";
pub const RENDERER_INTERFACE: &str = "com.sunaemon.KeymapOverlay.Renderer1";

pub type RendererState = (u64, bool, String);

#[derive(Clone)]
pub struct RendererStateStore(Arc<Mutex<RendererState>>);

impl RendererStateStore {
    pub fn new(initial: RendererState) -> Self {
        Self(Arc::new(Mutex::new(initial)))
    }

    pub fn set(&self, state: RendererState) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = state;
    }

    pub fn get(&self) -> RendererState {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

pub struct RendererService {
    state: RendererStateStore,
    reload_keyboards: Box<dyn Fn() -> bool + Send + Sync>,
}

impl RendererService {
    pub fn new(
        state: RendererStateStore,
        reload_keyboards: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            state,
            reload_keyboards: Box::new(reload_keyboards),
        }
    }
}

#[zbus::interface(name = "com.sunaemon.KeymapOverlay.Renderer1")]
impl RendererService {
    fn get_state(&self) -> RendererState {
        self.state.get()
    }

    fn reload_keyboards(&self) -> bool {
        (self.reload_keyboards)()
    }
}

#[zbus::proxy(
    interface = "com.sunaemon.KeymapOverlay.Renderer1",
    default_service = "com.sunaemon.KeymapOverlay",
    default_path = "/com/sunaemon/KeymapOverlay",
    gen_async = false,
    blocking_name = "RendererProxyBlocking"
)]
pub trait Renderer {
    fn get_state(&self) -> zbus::Result<RendererState>;

    #[zbus(signal)]
    fn state_changed(&self, generation: u64, visible: bool, model_json: &str) -> zbus::Result<()>;
}

pub fn decode_state(signal: &StateChanged) -> zbus::Result<RendererState> {
    let arguments = signal.args()?;
    Ok((
        arguments.generation,
        arguments.visible,
        arguments.model_json.to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn contract_names_are_stable() {
        let _: Option<RendererProxyBlocking<'static>> = None;
        assert_eq!(BUS_NAME, "com.sunaemon.KeymapOverlay");
        assert_eq!(OBJECT_PATH, "/com/sunaemon/KeymapOverlay");
        assert_eq!(RENDERER_INTERFACE, "com.sunaemon.KeymapOverlay.Renderer1");
    }

    #[test]
    fn state_store_is_shared_with_the_service() {
        let store = RendererStateStore::new((1, false, String::new()));
        let service = RendererService::new(store.clone(), || true);

        store.set((2, true, "{\"version\":1}".into()));

        assert_eq!(store.get(), (2, true, "{\"version\":1}".into()));
        assert!(service.reload_keyboards());
    }

    #[test]
    fn reload_method_calls_the_daemon_requester() {
        let called = Arc::new(AtomicBool::new(false));
        let callback_called = Arc::clone(&called);
        let service = RendererService::new(
            RendererStateStore::new((1, false, String::new())),
            move || {
                callback_called.store(true, Ordering::Release);
                true
            },
        );

        assert!(service.reload_keyboards());
        assert!(called.load(Ordering::Acquire));
    }
}
