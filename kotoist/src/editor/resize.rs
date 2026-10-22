//! Bridge native child-window resize gestures to the plugin host.

use super::geometry::EditorGeometry;
use baseview::{host::HostCallbacks, HandlerError, WindowSize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub(crate) struct ResizeBridge {
    geometry: Arc<Mutex<EditorGeometry>>,
    ready: Arc<AtomicBool>,
    request: Box<dyn Fn() -> bool>,
}

impl ResizeBridge {
    pub(crate) fn new(
        geometry: Arc<Mutex<EditorGeometry>>,
        ready: Arc<AtomicBool>,
        request: impl Fn() -> bool + 'static,
    ) -> Self {
        Self {
            geometry,
            ready,
            request: Box::new(request),
        }
    }
}

impl HostCallbacks for ResizeBridge {
    fn request_resize(&mut self, new_size: WindowSize) -> Result<(), HandlerError> {
        #[cfg(windows)]
        if super::windows_native_resize::is_parent_sync() {
            let mut geometry = self.geometry.lock().unwrap();
            let scale = geometry.scale;
            geometry.remember((new_size.physical.width, new_size.physical.height), scale);
            return Ok(());
        }

        // Do not re-enter the host while it is creating the embedded child.
        if !self.ready.load(Ordering::Acquire) {
            return Ok(());
        }

        let requested = (new_size.physical.width, new_size.physical.height);
        let previous = {
            let mut geometry = self.geometry.lock().unwrap();
            let scale = geometry.scale;
            if EditorGeometry::constrain(requested, scale) != requested {
                return Err(
                    std::io::Error::other("Editor size is outside its supported limits").into(),
                );
            }
            let previous = geometry.clone();
            geometry.remember(requested, scale);
            previous
        };

        // Never hold the geometry lock across a synchronous host callback.
        if (self.request)() {
            Ok(())
        } else {
            *self.geometry.lock().unwrap() = previous;
            Err(std::io::Error::other("The host rejected the editor resize request").into())
        }
    }

    fn destroyed(&mut self) {
        self.ready.store(false, Ordering::Release);
    }
}
