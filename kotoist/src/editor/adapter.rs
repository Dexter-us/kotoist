//! Host adapter for the current egui-baseview and egui editor stack.

use super::{
    draw_editor_ui, geometry::EditorGeometry, piano_roll::PianoRoll, resize::ResizeBridge, GuiState,
};
use crate::parameters::Parameters;
use crate::pipe::PipeOut;
use baseview::dpi::PhysicalSize;
use egui_baseview::{App, EguiWindow, EguiWindowSettings, Frame};
use nih_plug::prelude::{Editor, GuiContext, ParentWindowHandle};
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use std::any::Any;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub(crate) struct KotoistEditor {
    params: Arc<Parameters>,
    pipe_out: Arc<Mutex<PipeOut>>,
    piano_roll: Arc<Mutex<PianoRoll>>,
    geometry: Arc<Mutex<EditorGeometry>>,
    context: Arc<Mutex<Option<egui::Context>>>,
}

impl KotoistEditor {
    pub(crate) fn new(
        params: Arc<Parameters>,
        pipe_out: Arc<Mutex<PipeOut>>,
        piano_roll: Arc<Mutex<PianoRoll>>,
        geometry: Arc<Mutex<EditorGeometry>>,
    ) -> Self {
        Self {
            params,
            pipe_out,
            piano_roll,
            geometry,
            context: Arc::new(Mutex::new(None)),
        }
    }

    fn request_repaint(&self) {
        if let Some(context) = self.context.lock().unwrap().as_ref() {
            context.request_repaint();
        }
    }
}

impl Editor for KotoistEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn Any + Send> {
        let (scale, size) = {
            let geometry = self.geometry.lock().unwrap();
            (geometry.scale, geometry.physical_size(geometry.scale))
        };
        let parent = NativeParent(parent);
        let settings = EguiWindowSettings::new()
            .with_title(format!(
                "Dexter U.S. Kotoist v{}",
                env!("CARGO_PKG_VERSION")
            ))
            .with_size(PhysicalSize::new(size.0, size.1))
            // Native host DPI is already applied to the parent; use only the
            // host-reported scale as egui's logical zoom factor.
            .with_zoom_factor((scale / parent_scale(parent.0)) as f32)
            .with_resizable(true)
            .with_parent(&parent);

        let ready = Arc::new(AtomicBool::new(false));
        let host = baseview::host::Host::new().with_callbacks(ResizeBridge::new(
            self.geometry.clone(),
            ready.clone(),
            move || context.request_resize(),
        ));

        let window = EguiWindow::create_with_host(
            settings,
            EditorApp {
                params: self.params.clone(),
                pipe_out: self.pipe_out.clone(),
                piano_roll: self.piano_roll.clone(),
                context: self.context.clone(),
                scale,
                state: GuiState::default(),
            },
            Some(host),
        )
        .expect("Could not create the Kotoist editor window");
        window
            .show()
            .expect("Could not show the Kotoist editor window");
        ready.store(true, Ordering::Release);
        Box::new(KotoistEditorHandle {
            window: Some(window),
        })
    }

    fn size(&self) -> (u32, u32) {
        self.geometry.lock().unwrap().logical_size()
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        if !factor.is_finite() || factor <= 0.0 {
            return false;
        }
        self.geometry.lock().unwrap().scale = factor as f64;
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        self.request_repaint();
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {
        self.request_repaint();
    }

    fn param_values_changed(&self) {
        self.request_repaint();
    }
}

struct EditorApp {
    params: Arc<Parameters>,
    pipe_out: Arc<Mutex<PipeOut>>,
    piano_roll: Arc<Mutex<PianoRoll>>,
    context: Arc<Mutex<Option<egui::Context>>>,
    scale: f64,
    state: GuiState,
}

impl App for EditorApp {
    fn build(
        &mut self,
        ctx: egui::Context,
        frame: &mut Frame,
    ) -> Result<(), baseview::HandlerError> {
        ctx.options_mut(|options| {
            options.zoom_factor = (self.scale / frame.baseview_window().size().scale_factor) as f32;
            options.zoom_with_keyboard = false;
        });
        *self.context.lock().unwrap() = Some(ctx);

        #[cfg(windows)]
        super::windows_focus::install(frame.baseview_window())
            .map_err(|error| baseview::HandlerError::from(std::io::Error::other(error)))?;
        Ok(())
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        #[cfg(windows)]
        {
            // Mirror Glicol's workaround for embedded windows that do not gain
            // native keyboard focus automatically on the first mouse click.
            let clicked = ui.ctx().input(|input| input.pointer.any_pressed());
            let window = frame.baseview_window();
            if clicked && !window.has_focus() {
                let _ = window.focus();
            }
        }

        #[cfg(windows)]
        super::windows_native_resize::before_draw(frame.baseview_window());
        draw_editor_ui(
            ui,
            &mut self.state,
            &self.params,
            &self.pipe_out,
            &self.piano_roll,
        );
    }
}

/// NIH-plug requires a `Send` editor handle even though Baseview's native
/// window owns thread-affine OS handles. Like NIH's egui adapter, the host only
/// stores and drops this opaque handle; Baseview's `close()` dispatches window
/// teardown through its own platform event machinery.
struct KotoistEditorHandle {
    window: Option<baseview::Window>,
}

unsafe impl Send for KotoistEditorHandle {}

impl Drop for KotoistEditorHandle {
    fn drop(&mut self) {
        if let Some(window) = self.window.take() {
            window.close();
        }
    }
}

fn parent_scale(parent: ParentWindowHandle) -> f64 {
    #[cfg(windows)]
    if let ParentWindowHandle::Win32Hwnd(hwnd) = parent {
        use winapi::um::libloaderapi::{GetModuleHandleW, GetProcAddress};
        unsafe {
            let module =
                GetModuleHandleW("user32.dll\0".encode_utf16().collect::<Vec<_>>().as_ptr());
            if !module.is_null() {
                let proc = GetProcAddress(module, b"GetDpiForWindow\0".as_ptr().cast());
                if !proc.is_null() {
                    let get_dpi: unsafe extern "system" fn(winapi::shared::windef::HWND) -> u32 =
                        std::mem::transmute(proc);
                    let dpi = get_dpi(hwnd.cast());
                    if dpi != 0 {
                        return dpi as f64 / 96.0;
                    }
                }
            }
        }
    }
    let _ = parent;
    1.0
}

// NIH-plug exposes raw-window-handle 0.5; egui-baseview 0.8.1 uses 0.6.
struct NativeParent(ParentWindowHandle);

impl HasWindowHandle for NativeParent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = match self.0 {
            #[cfg(windows)]
            ParentWindowHandle::Win32Hwnd(hwnd) => {
                RawWindowHandle::Win32(raw_window_handle::Win32WindowHandle::new(
                    std::num::NonZeroIsize::new(hwnd as isize).ok_or(HandleError::Unavailable)?,
                ))
            }
            #[cfg(target_os = "macos")]
            ParentWindowHandle::AppKitNsView(view) => {
                RawWindowHandle::AppKit(raw_window_handle::AppKitWindowHandle::new(
                    std::ptr::NonNull::new(view).ok_or(HandleError::Unavailable)?,
                ))
            }
            #[cfg(target_os = "linux")]
            ParentWindowHandle::X11Window(window) => {
                RawWindowHandle::Xcb(raw_window_handle::XcbWindowHandle::new(
                    std::num::NonZeroU32::new(window).ok_or(HandleError::Unavailable)?,
                ))
            }
            _ => return Err(HandleError::NotSupported),
        };
        // The plugin host owns the parent for the embedded editor's lifetime.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}
