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

pub struct RendererService(RendererStateStore);

impl RendererService {
    pub fn new(state: RendererStateStore) -> Self {
        Self(state)
    }
}

#[zbus::interface(name = "com.sunaemon.KeymapOverlay.Renderer1")]
impl RendererService {
    fn get_state(&self) -> RendererState {
        self.0.get()
    }
}

#[zbus::proxy(
    interface = "com.sunaemon.KeymapOverlay.Renderer1",
    default_service = "com.sunaemon.KeymapOverlay",
    default_path = "/com/sunaemon/KeymapOverlay",
    gen_async = false
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

    #[test]
    fn contract_names_are_stable() {
        assert_eq!(BUS_NAME, "com.sunaemon.KeymapOverlay");
        assert_eq!(OBJECT_PATH, "/com/sunaemon/KeymapOverlay");
        assert_eq!(RENDERER_INTERFACE, "com.sunaemon.KeymapOverlay.Renderer1");
    }

    #[test]
    fn state_store_is_shared_with_the_service() {
        let store = RendererStateStore::new((1, false, String::new()));
        let _service = RendererService::new(store.clone());

        store.set((2, true, "{\"version\":1}".into()));

        assert_eq!(store.get(), (2, true, "{\"version\":1}".into()));
    }
}
