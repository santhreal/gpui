use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    ffi::c_void,
    ptr::NonNull,
    rc::Rc,
    sync::Arc,
};

use blade_graphics as gpu;
use calloop::{
    LoopHandle, RegistrationToken,
    timer::{TimeoutAction, Timer},
};
use collections::HashMap;
use futures::channel::oneshot::Receiver;

use raw_window_handle as rwh;
use wayland_backend::client::ObjectId;
use wayland_client::WEnum;
use wayland_client::{Proxy, protocol::wl_surface};
use wayland_protocols::wp::viewporter::client::wp_viewport;
use wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1;
use wayland_protocols::xdg::shell::client::xdg_surface;
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self};
use wayland_protocols::{
    wp::fractional_scale::v1::client::wp_fractional_scale_v1,
    xdg::shell::client::xdg_toplevel::XdgToplevel,
};
use wayland_protocols_plasma::blur::client::org_kde_kwin_blur;

use crate::{
    AnyWindowHandle, Bounds, Decorations, Globals, GpuSpecs, Modifiers, Output, Pixels,
    PlatformDisplay, PlatformInput, Point, PromptButton, PromptLevel, RequestFrameOptions,
    ResizeEdge, Size, Tiling, WaylandClientStatePtr, WindowAppearance, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowControls, WindowDecorations, WindowParams, px, size,
};
use crate::{
    Capslock,
    platform::{
        PlatformAtlas, PlatformInputHandler, PlatformWindow,
        blade::{BladeContext, BladeRenderer, BladeSurfaceConfig},
        linux::wayland::{display::WaylandDisplay, serial::SerialKind},
    },
};
use crate::{WindowKind, scene::Scene};

#[derive(Default)]
pub(crate) struct Callbacks {
    request_frame: Option<Box<dyn FnMut(RequestFrameOptions)>>,
    input: Option<Box<dyn FnMut(crate::PlatformInput) -> crate::DispatchEventResult>>,
    active_status_change: Option<Box<dyn FnMut(bool)>>,
    hover_status_change: Option<Box<dyn FnMut(bool)>>,
    resize: Option<Box<dyn FnMut(Size<Pixels>, f32)>>,
    moved: Option<Box<dyn FnMut()>>,
    should_close: Option<Box<dyn FnMut() -> bool>>,
    close: Option<Box<dyn FnOnce()>>,
    appearance_changed: Option<Box<dyn FnMut()>>,
}

struct RawWindow {
    window: *mut c_void,
    display: *mut c_void,
}

impl rwh::HasWindowHandle for RawWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        let window = NonNull::new(self.window).unwrap();
        let handle = rwh::WaylandWindowHandle::new(window);
        Ok(unsafe { rwh::WindowHandle::borrow_raw(handle.into()) })
    }
}
impl rwh::HasDisplayHandle for RawWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        let display = NonNull::new(self.display).unwrap();
        let handle = rwh::WaylandDisplayHandle::new(display);
        Ok(unsafe { rwh::DisplayHandle::borrow_raw(handle.into()) })
    }
}

#[derive(Debug)]
struct InProgressConfigure {
    size: Option<Size<Pixels>>,
    fullscreen: bool,
    maximized: bool,
    resizing: bool,
    tiling: Tiling,
}

pub struct WaylandWindowState {
    xdg_surface: xdg_surface::XdgSurface,
    acknowledged_first_configure: bool,
    pub surface: wl_surface::WlSurface,
    decoration: Option<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
    app_id: Option<String>,
    appearance: WindowAppearance,
    blur: Option<org_kde_kwin_blur::OrgKdeKwinBlur>,
    toplevel: xdg_toplevel::XdgToplevel,
    viewport: Option<wp_viewport::WpViewport>,
    outputs: HashMap<ObjectId, Output>,
    display: Option<(ObjectId, Output)>,
    globals: Globals,
    renderer: BladeRenderer,
    bounds: Bounds<Pixels>,
    scale: f32,
    input_handler: Option<PlatformInputHandler>,
    decorations: WindowDecorations,
    background_appearance: WindowBackgroundAppearance,
    fullscreen: bool,
    maximized: bool,
    tiling: Tiling,
    window_bounds: Bounds<Pixels>,
    client: WaylandClientStatePtr,
    handle: AnyWindowHandle,
    active: bool,
    hovered: bool,
    in_progress_configure: Option<InProgressConfigure>,
    /// Interactive resizes apply once per frame: an `xdg_surface.configure`
    /// that arrives after one was applied this frame waits here with its
    /// serial, and the frame callback applies and acks the latest.
    throttled_configure: Option<(InProgressConfigure, u32)>,
    resize_throttle: bool,
    in_progress_window_controls: Option<WindowControls>,
    window_controls: WindowControls,
    client_inset: Option<Pixels>,
}

#[derive(Clone)]
pub struct WaylandWindowStatePtr {
    state: Rc<RefCell<WaylandWindowState>>,
    callbacks: Rc<RefCell<Callbacks>>,
    frame_loop: Rc<FrameLoop>,
}

/// Runs a window's frames on demand, paced by the compositor's frame
/// callbacks. A frame requests the next callback only when it presented or
/// GPUI still has frame demand for the window, so an idle window neither
/// wakes the client nor keeps the compositor repainting for it.
pub(crate) struct FrameLoop {
    surface_id: ObjectId,
    loop_handle: LoopHandle<'static, WaylandClientStatePtr>,
    phase: Cell<FramePhase>,
    timer: Cell<Option<RegistrationToken>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FramePhase {
    /// Before the first configure, which runs the first frame: attaching a
    /// buffer to an unconfigured xdg_surface is a protocol error.
    Unconfigured,
    /// No frame callback is outstanding and no frame is scheduled.
    Idle,
    /// A timer runs the next frame when the event loop next turns.
    Scheduled,
    /// A frame is running. `demand` arrived while it ran; `presented`
    /// records whether it presented a new buffer.
    Running { demand: bool, presented: bool },
    /// A committed frame callback runs the next frame.
    Waiting,
}

impl FramePhase {
    /// The phase after GPUI gains frame demand, and whether a frame must be
    /// scheduled for it.
    fn wake(self) -> (Self, bool) {
        match self {
            Self::Idle => (Self::Scheduled, true),
            Self::Running { presented, .. } => (
                Self::Running {
                    demand: true,
                    presented,
                },
                false,
            ),
            Self::Unconfigured | Self::Scheduled | Self::Waiting => (self, false),
        }
    }

    fn present(self) -> Self {
        match self {
            Self::Running { demand, .. } => Self::Running {
                demand,
                presented: true,
            },
            phase => phase,
        }
    }

    /// The phase after a frame ends, and whether the frame requests a frame
    /// callback. A frame that presented waits for one before the next frame
    /// draws, so demand arriving right after a present is paced by the
    /// compositor instead of drawing frames the display never shows.
    fn end(self) -> (Self, bool) {
        match self {
            Self::Running { demand, presented } if demand || presented => (Self::Waiting, true),
            _ => (Self::Idle, false),
        }
    }
}

impl FrameLoop {
    fn new(surface_id: ObjectId, loop_handle: LoopHandle<'static, WaylandClientStatePtr>) -> Self {
        Self {
            surface_id,
            loop_handle,
            phase: Cell::new(FramePhase::Unconfigured),
            timer: Cell::new(None),
        }
    }

    /// Request a frame: at once after an idle stretch, else from the
    /// outstanding frame callback or the frame running now.
    fn wake(&self) {
        let (phase, schedule) = self.phase.get().wake();
        if schedule {
            let surface_id = self.surface_id.clone();
            // A timer, not an idle callback: calloop polls with no timeout
            // for idles inserted while idles dispatch, and GPUI's foreground
            // tasks run inside idle callbacks.
            match self
                .loop_handle
                .insert_source(Timer::immediate(), move |_, _, client| {
                    client.run_scheduled_frame(&surface_id);
                    TimeoutAction::Drop
                }) {
                Ok(token) => self.timer.set(Some(token)),
                Err(err) => {
                    log::error!("Wayland: failed to schedule a frame: {}", err.error);
                    return;
                }
            }
        }
        self.phase.set(phase);
    }

    /// The scheduled frame's timer is running; it drops itself on return.
    fn fired(&self) {
        self.timer.set(None);
    }

    fn begin_frame(&self) {
        self.cancel();
        self.phase.set(FramePhase::Running {
            demand: false,
            presented: false,
        });
    }

    fn presented(&self) {
        self.phase.set(self.phase.get().present());
    }

    /// Ends the running frame; returns whether it requests a frame callback.
    fn end_frame(&self) -> bool {
        let (phase, request) = self.phase.get().end();
        self.phase.set(phase);
        request
    }

    fn cancel(&self) {
        if let Some(token) = self.timer.take() {
            self.loop_handle.remove(token);
        }
    }
}

impl WaylandWindowState {
    pub(crate) fn new(
        handle: AnyWindowHandle,
        surface: wl_surface::WlSurface,
        xdg_surface: xdg_surface::XdgSurface,
        toplevel: xdg_toplevel::XdgToplevel,
        decoration: Option<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
        appearance: WindowAppearance,
        viewport: Option<wp_viewport::WpViewport>,
        client: WaylandClientStatePtr,
        globals: Globals,
        gpu_context: &BladeContext,
        options: WindowParams,
    ) -> anyhow::Result<Self> {
        let renderer = {
            let raw_window = RawWindow {
                window: surface.id().as_ptr().cast::<c_void>(),
                display: surface
                    .backend()
                    .upgrade()
                    .unwrap()
                    .display_ptr()
                    .cast::<c_void>(),
            };
            let config = BladeSurfaceConfig {
                size: gpu::Extent {
                    width: options.bounds.size.width.0 as u32,
                    height: options.bounds.size.height.0 as u32,
                    depth: 1,
                },
                transparent: true,
            };
            BladeRenderer::new(gpu_context, &raw_window, config)?
        };

        // Set app_id on the toplevel before the caller's first
        // surface.commit(): KWin applies window rules from the first
        // commit's app_id and does not re-check when it changes, so a
        // None here leaves the window unidentifiable to the compositor.
        if let Some(app_id) = options.app_id.as_ref() {
            toplevel.set_app_id(app_id.clone());
        }

        Ok(Self {
            xdg_surface,
            acknowledged_first_configure: false,
            surface,
            decoration,
            app_id: options.app_id.clone(),
            blur: None,
            toplevel,
            viewport,
            globals,
            outputs: HashMap::default(),
            display: None,
            renderer,
            bounds: options.bounds,
            scale: 1.0,
            input_handler: None,
            decorations: WindowDecorations::Client,
            background_appearance: WindowBackgroundAppearance::Opaque,
            fullscreen: false,
            maximized: false,
            tiling: Tiling::default(),
            window_bounds: options.bounds,
            in_progress_configure: None,
            throttled_configure: None,
            resize_throttle: false,
            client,
            appearance,
            handle,
            active: false,
            hovered: false,
            in_progress_window_controls: None,
            window_controls: WindowControls::default(),
            client_inset: None,
        })
    }

    pub fn is_transparent(&self) -> bool {
        self.decorations == WindowDecorations::Client
            || self.background_appearance != WindowBackgroundAppearance::Opaque
    }

    pub fn primary_output_scale(&mut self) -> i32 {
        let mut scale = 1;
        let mut current_output = self.display.take();
        for (id, output) in self.outputs.iter() {
            if let Some((_, output_data)) = &current_output {
                if output.scale > output_data.scale {
                    current_output = Some((id.clone(), output.clone()));
                }
            } else {
                current_output = Some((id.clone(), output.clone()));
            }
            scale = scale.max(output.scale);
        }
        self.display = current_output;
        scale
    }

    pub fn inset(&self) -> Pixels {
        match self.decorations {
            WindowDecorations::Server => px(0.0),
            WindowDecorations::Client => self.client_inset.unwrap_or(px(0.0)),
        }
    }
}

pub(crate) struct WaylandWindow(pub WaylandWindowStatePtr);
pub enum ImeInput {
    InsertText(String),
    SetMarkedText(String),
    UnmarkText,
    DeleteText,
}

impl Drop for WaylandWindow {
    fn drop(&mut self) {
        self.0.frame_loop.cancel();
        let mut state = self.0.state.borrow_mut();
        let surface_id = state.surface.id();
        let client = state.client.clone();

        state.renderer.destroy();
        if let Some(decoration) = &state.decoration {
            decoration.destroy();
        }
        if let Some(blur) = &state.blur {
            blur.release();
        }
        state.toplevel.destroy();
        if let Some(viewport) = &state.viewport {
            viewport.destroy();
        }
        state.xdg_surface.destroy();
        state.surface.destroy();

        let state_ptr = self.0.clone();
        state
            .globals
            .executor
            .spawn(async move {
                state_ptr.close();
                client.drop_window(&surface_id)
            })
            .detach();
        drop(state);
    }
}

impl WaylandWindow {
    fn borrow(&self) -> Ref<'_, WaylandWindowState> {
        self.0.state.borrow()
    }

    fn borrow_mut(&self) -> RefMut<'_, WaylandWindowState> {
        self.0.state.borrow_mut()
    }

    pub fn new(
        handle: AnyWindowHandle,
        globals: Globals,
        gpu_context: &BladeContext,
        client: WaylandClientStatePtr,
        params: WindowParams,
        appearance: WindowAppearance,
        parent: Option<XdgToplevel>,
        loop_handle: LoopHandle<'static, WaylandClientStatePtr>,
    ) -> anyhow::Result<(Self, ObjectId)> {
        let surface = globals.compositor.create_surface(&globals.qh, ());
        let xdg_surface = globals
            .wm_base
            .get_xdg_surface(&surface, &globals.qh, surface.id());
        let toplevel = xdg_surface.get_toplevel(&globals.qh, surface.id());

        if params.kind == WindowKind::Floating {
            toplevel.set_parent(parent.as_ref());
        }

        if let Some(size) = params.window_min_size {
            toplevel.set_min_size(size.width.0 as i32, size.height.0 as i32);
        }

        if let Some(fractional_scale_manager) = globals.fractional_scale_manager.as_ref() {
            fractional_scale_manager.get_fractional_scale(&surface, &globals.qh, surface.id());
        }

        // Attempt to set up window decorations based on the requested configuration
        let decoration = globals
            .decoration_manager
            .as_ref()
            .map(|decoration_manager| {
                decoration_manager.get_toplevel_decoration(&toplevel, &globals.qh, surface.id())
            });

        let viewport = globals
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, &globals.qh, ()));

        let this = Self(WaylandWindowStatePtr {
            state: Rc::new(RefCell::new(WaylandWindowState::new(
                handle,
                surface.clone(),
                xdg_surface,
                toplevel,
                decoration,
                appearance,
                viewport,
                client,
                globals,
                gpu_context,
                params,
            )?)),
            callbacks: Rc::new(RefCell::new(Callbacks::default())),
            frame_loop: Rc::new(FrameLoop::new(surface.id(), loop_handle)),
        });

        // Kick things off
        surface.commit();

        Ok((this, surface.id()))
    }
}

impl WaylandWindowStatePtr {
    pub fn handle(&self) -> AnyWindowHandle {
        self.state.borrow().handle
    }

    pub fn surface(&self) -> wl_surface::WlSurface {
        self.state.borrow().surface.clone()
    }

    pub fn toplevel(&self) -> xdg_toplevel::XdgToplevel {
        self.state.borrow().toplevel.clone()
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.state, &other.state)
    }

    /// Runs a frame: on the first configure, a frame callback, or the
    /// timer [`FrameLoop::wake`] scheduled.
    pub fn frame(&self) {
        self.frame_loop.begin_frame();
        let throttled = {
            let mut state = self.state.borrow_mut();
            state.resize_throttle = false;
            state.throttled_configure.take()
        };

        // Before the frame renders: the frame then draws the latest size
        // and its commit carries the configure's ack.
        if let Some((configure, serial)) = throttled {
            self.apply_configure(Some(configure), serial);
        }

        let mut cb = self.callbacks.borrow_mut();
        if let Some(fun) = cb.request_frame.as_mut() {
            fun(Default::default());
        }
        drop(cb);

        // The commit also applies state set outside a draw, such as a
        // configure's ack when the configure left the size unchanged.
        let state = self.state.borrow();
        if self.frame_loop.end_frame() {
            state.surface.frame(&state.globals.qh, state.surface.id());
        }
        state.surface.commit();
    }

    pub fn run_scheduled_frame(&self) {
        self.frame_loop.fired();
        self.frame();
    }

    pub fn handle_xdg_surface_event(&self, event: xdg_surface::Event) {
        if let xdg_surface::Event::Configure { serial } = event {
            {
                let mut state = self.state.borrow_mut();
                if let Some(window_controls) = state.in_progress_window_controls.take() {
                    state.window_controls = window_controls;

                    drop(state);
                    let mut callbacks = self.callbacks.borrow_mut();
                    if let Some(appearance_changed) = callbacks.appearance_changed.as_mut() {
                        appearance_changed();
                    }
                }
            }
            let mut state = self.state.borrow_mut();
            // The latest toplevel state: this configure's, or else a
            // throttled one's, which acking this newer serial acks too.
            let configure = state
                .in_progress_configure
                .take()
                .or_else(|| state.throttled_configure.take().map(|(c, _)| c));
            // Limit interactive resizes to once per vblank. A configure
            // dropped here would leave the window at an older size than
            // the compositor set, and the compositor takes the size the
            // window commits: the resize would end short of the pointer.
            match configure {
                Some(configure) if configure.resizing && state.resize_throttle => {
                    state.throttled_configure = Some((configure, serial));
                    drop(state);
                    // The next frame applies it, and a frame must come.
                    self.frame_loop.wake();
                }
                configure => {
                    // Newer than a throttled configure, which must not be
                    // acked after it: acking an older serial than the last
                    // acked one is a protocol error.
                    state.throttled_configure = None;
                    drop(state);
                    self.apply_configure(configure, serial);
                }
            }
        }
    }

    /// Apply the toplevel state of the configure with `serial`, if any
    /// came with it, and ack it.
    fn apply_configure(&self, configure: Option<InProgressConfigure>, serial: u32) {
        if let Some(mut configure) = configure {
            let mut state = self.state.borrow_mut();
            let got_unmaximized = state.maximized && !configure.maximized;
            state.fullscreen = configure.fullscreen;
            state.maximized = configure.maximized;
            state.tiling = configure.tiling;
            if configure.resizing {
                state.resize_throttle = true;
            }
            if !configure.fullscreen && !configure.maximized {
                configure.size = if got_unmaximized {
                    Some(state.window_bounds.size)
                } else {
                    compute_outer_size(state.inset(), configure.size, state.tiling)
                };
                if let Some(size) = configure.size {
                    state.window_bounds = Bounds {
                        origin: Point::default(),
                        size,
                    };
                }
            }
            drop(state);
            if let Some(size) = configure.size {
                self.resize(size);
            }
        }
        let mut state = self.state.borrow_mut();
        state.xdg_surface.ack_configure(serial);

        let window_geometry = inset_by_tiling(
            state.bounds.map_origin(|_| px(0.0)),
            state.inset(),
            state.tiling,
        )
        .map(|v| v.0 as i32)
        .map_size(|v| if v <= 0 { 1 } else { v });

        state.xdg_surface.set_window_geometry(
            window_geometry.origin.x,
            window_geometry.origin.y,
            window_geometry.size.width,
            window_geometry.size.height,
        );

        let first_configure = !state.acknowledged_first_configure;
        if first_configure {
            state.acknowledged_first_configure = true;
            drop(state);
            self.frame();
        } else {
            drop(state);
            // The ack takes effect on the next commit, which a frame makes
            // even when the configure changed nothing that redraws.
            self.frame_loop.wake();
        }
    }

    pub fn handle_toplevel_decoration_event(&self, event: zxdg_toplevel_decoration_v1::Event) {
        if let zxdg_toplevel_decoration_v1::Event::Configure { mode } = event {
            match mode {
                WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ServerSide) => {
                    self.state.borrow_mut().decorations = WindowDecorations::Server;
                    if let Some(mut appearance_changed) =
                        self.callbacks.borrow_mut().appearance_changed.as_mut()
                    {
                        appearance_changed();
                    }
                }
                WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ClientSide) => {
                    self.state.borrow_mut().decorations = WindowDecorations::Client;
                    // Update background to be transparent
                    if let Some(mut appearance_changed) =
                        self.callbacks.borrow_mut().appearance_changed.as_mut()
                    {
                        appearance_changed();
                    }
                }
                WEnum::Value(_) => {
                    log::warn!("Unknown decoration mode");
                }
                WEnum::Unknown(v) => {
                    log::warn!("Unknown decoration mode: {}", v);
                }
            }
        }
    }

    pub fn handle_fractional_scale_event(&self, event: wp_fractional_scale_v1::Event) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            self.rescale(scale as f32 / 120.0);
        }
    }

    pub fn handle_toplevel_event(&self, event: xdg_toplevel::Event) -> bool {
        match event {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } => {
                let mut size = if width == 0 || height == 0 {
                    None
                } else {
                    Some(size(px(width as f32), px(height as f32)))
                };

                let states = extract_states::<xdg_toplevel::State>(&states);

                let mut tiling = Tiling::default();
                let mut fullscreen = false;
                let mut maximized = false;
                let mut resizing = false;

                for state in states {
                    match state {
                        xdg_toplevel::State::Maximized => {
                            maximized = true;
                        }
                        xdg_toplevel::State::Fullscreen => {
                            fullscreen = true;
                        }
                        xdg_toplevel::State::Resizing => resizing = true,
                        xdg_toplevel::State::TiledTop => {
                            tiling.top = true;
                        }
                        xdg_toplevel::State::TiledLeft => {
                            tiling.left = true;
                        }
                        xdg_toplevel::State::TiledRight => {
                            tiling.right = true;
                        }
                        xdg_toplevel::State::TiledBottom => {
                            tiling.bottom = true;
                        }
                        _ => {
                            // noop
                        }
                    }
                }

                if fullscreen || maximized {
                    tiling = Tiling::tiled();
                }

                let mut state = self.state.borrow_mut();
                state.in_progress_configure = Some(InProgressConfigure {
                    size,
                    fullscreen,
                    maximized,
                    resizing,
                    tiling,
                });

                false
            }
            xdg_toplevel::Event::Close => {
                let mut cb = self.callbacks.borrow_mut();
                if let Some(mut should_close) = cb.should_close.take() {
                    let result = (should_close)();
                    cb.should_close = Some(should_close);
                    if result {
                        drop(cb);
                        self.close();
                    }
                    result
                } else {
                    true
                }
            }
            xdg_toplevel::Event::WmCapabilities { capabilities } => {
                let mut window_controls = WindowControls::default();

                let states = extract_states::<xdg_toplevel::WmCapabilities>(&capabilities);

                for state in states {
                    match state {
                        xdg_toplevel::WmCapabilities::Maximize => {
                            window_controls.maximize = true;
                        }
                        xdg_toplevel::WmCapabilities::Minimize => {
                            window_controls.minimize = true;
                        }
                        xdg_toplevel::WmCapabilities::Fullscreen => {
                            window_controls.fullscreen = true;
                        }
                        xdg_toplevel::WmCapabilities::WindowMenu => {
                            window_controls.window_menu = true;
                        }
                        _ => {}
                    }
                }

                let mut state = self.state.borrow_mut();
                state.in_progress_window_controls = Some(window_controls);
                false
            }
            _ => false,
        }
    }

    #[allow(clippy::mutable_key_type)]
    pub fn handle_surface_event(
        &self,
        event: wl_surface::Event,
        outputs: HashMap<ObjectId, Output>,
    ) {
        let mut state = self.state.borrow_mut();

        match event {
            wl_surface::Event::Enter { output } => {
                let id = output.id();

                let Some(output) = outputs.get(&id) else {
                    return;
                };

                state.outputs.insert(id, output.clone());

                let scale = state.primary_output_scale();

                // We use `PreferredBufferScale` instead to set the scale if it's available
                if state.surface.version() < wl_surface::EVT_PREFERRED_BUFFER_SCALE_SINCE {
                    state.surface.set_buffer_scale(scale);
                    drop(state);
                    self.rescale(scale as f32);
                }
            }
            wl_surface::Event::Leave { output } => {
                state.outputs.remove(&output.id());

                let scale = state.primary_output_scale();

                // We use `PreferredBufferScale` instead to set the scale if it's available
                if state.surface.version() < wl_surface::EVT_PREFERRED_BUFFER_SCALE_SINCE {
                    state.surface.set_buffer_scale(scale);
                    drop(state);
                    self.rescale(scale as f32);
                }
            }
            wl_surface::Event::PreferredBufferScale { factor } => {
                // We use `WpFractionalScale` instead to set the scale if it's available
                if state.globals.fractional_scale_manager.is_none() {
                    state.surface.set_buffer_scale(factor);
                    drop(state);
                    self.rescale(factor as f32);
                }
            }
            _ => {}
        }
    }

    pub fn handle_ime(&self, ime: ImeInput) {
        let mut state = self.state.borrow_mut();
        if let Some(mut input_handler) = state.input_handler.take() {
            drop(state);
            match ime {
                ImeInput::InsertText(text) => {
                    input_handler.replace_text_in_range(None, &text);
                }
                ImeInput::SetMarkedText(text) => {
                    input_handler.replace_and_mark_text_in_range(None, &text, None);
                }
                ImeInput::UnmarkText => {
                    input_handler.unmark_text();
                }
                ImeInput::DeleteText => {
                    if let Some(marked) = input_handler.marked_text_range() {
                        input_handler.replace_text_in_range(Some(marked), "");
                    }
                }
            }
            self.state.borrow_mut().input_handler = Some(input_handler);
        }
    }

    pub fn get_ime_area(&self) -> Option<Bounds<Pixels>> {
        let mut state = self.state.borrow_mut();
        let mut bounds: Option<Bounds<Pixels>> = None;
        if let Some(mut input_handler) = state.input_handler.take() {
            drop(state);
            if let Some(selection) = input_handler.marked_text_range() {
                bounds = input_handler.bounds_for_range(selection.start..selection.start);
            }
            self.state.borrow_mut().input_handler = Some(input_handler);
        }
        bounds
    }

    pub fn set_size_and_scale(&self, size: Option<Size<Pixels>>, scale: Option<f32>) {
        let (size, scale) = {
            let mut state = self.state.borrow_mut();
            if size.is_none_or(|size| size == state.bounds.size)
                && scale.is_none_or(|scale| scale == state.scale)
            {
                return;
            }
            if let Some(size) = size {
                state.bounds.size = size;
            }
            if let Some(scale) = scale {
                state.scale = scale;
            }
            let device_bounds = state.bounds.to_device_pixels(state.scale);
            state.renderer.update_drawable_size(device_bounds.size);
            (state.bounds.size, state.scale)
        };

        if let Some(ref mut fun) = self.callbacks.borrow_mut().resize {
            fun(size, scale);
        }

        {
            let state = self.state.borrow();
            if let Some(viewport) = &state.viewport {
                viewport.set_destination(size.width.0 as i32, size.height.0 as i32);
            }
        }
    }

    pub fn resize(&self, size: Size<Pixels>) {
        self.set_size_and_scale(Some(size), None);
    }

    pub fn rescale(&self, scale: f32) {
        self.set_size_and_scale(None, Some(scale));
    }

    pub fn close(&self) {
        let mut callbacks = self.callbacks.borrow_mut();
        if let Some(fun) = callbacks.close.take() {
            fun()
        }
    }

    pub fn handle_input(&self, input: PlatformInput) {
        if let Some(ref mut fun) = self.callbacks.borrow_mut().input
            && !fun(input.clone()).propagate
        {
            return;
        }
        if let PlatformInput::KeyDown(event) = input
            && event.keystroke.modifiers.is_subset_of(&Modifiers::shift())
            && let Some(key_char) = &event.keystroke.key_char
        {
            let mut state = self.state.borrow_mut();
            if let Some(mut input_handler) = state.input_handler.take() {
                drop(state);
                input_handler.replace_text_in_range(None, key_char);
                self.state.borrow_mut().input_handler = Some(input_handler);
            }
        }
    }

    pub fn set_focused(&self, focus: bool) {
        self.state.borrow_mut().active = focus;
        if let Some(ref mut fun) = self.callbacks.borrow_mut().active_status_change {
            fun(focus);
        }
    }

    pub fn set_hovered(&self, focus: bool) {
        if let Some(ref mut fun) = self.callbacks.borrow_mut().hover_status_change {
            fun(focus);
        }
    }

    pub fn set_appearance(&mut self, appearance: WindowAppearance) {
        self.state.borrow_mut().appearance = appearance;

        let mut callbacks = self.callbacks.borrow_mut();
        if let Some(ref mut fun) = callbacks.appearance_changed {
            (fun)()
        }
    }

    pub fn primary_output_scale(&self) -> i32 {
        self.state.borrow_mut().primary_output_scale()
    }
}

fn extract_states<'a, S: TryFrom<u32> + 'a>(states: &'a [u8]) -> impl Iterator<Item = S> + 'a
where
    <S as TryFrom<u32>>::Error: 'a,
{
    states
        .chunks_exact(4)
        .flat_map(TryInto::<[u8; 4]>::try_into)
        .map(u32::from_ne_bytes)
        .flat_map(S::try_from)
}

impl rwh::HasWindowHandle for WaylandWindow {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        let surface = self.0.surface().id().as_ptr() as *mut libc::c_void;
        let c_ptr = NonNull::new(surface).ok_or(rwh::HandleError::Unavailable)?;
        let handle = rwh::WaylandWindowHandle::new(c_ptr);
        let raw_handle = rwh::RawWindowHandle::Wayland(handle);
        Ok(unsafe { rwh::WindowHandle::borrow_raw(raw_handle) })
    }
}

impl rwh::HasDisplayHandle for WaylandWindow {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        let display = self
            .0
            .surface()
            .backend()
            .upgrade()
            .ok_or(rwh::HandleError::Unavailable)?
            .display_ptr() as *mut libc::c_void;

        let c_ptr = NonNull::new(display).ok_or(rwh::HandleError::Unavailable)?;
        let handle = rwh::WaylandDisplayHandle::new(c_ptr);
        let raw_handle = rwh::RawDisplayHandle::Wayland(handle);
        Ok(unsafe { rwh::DisplayHandle::borrow_raw(raw_handle) })
    }
}

impl PlatformWindow for WaylandWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.borrow().bounds
    }

    fn is_maximized(&self) -> bool {
        self.borrow().maximized
    }

    fn window_bounds(&self) -> WindowBounds {
        let state = self.borrow();
        if state.fullscreen {
            WindowBounds::Fullscreen(state.window_bounds)
        } else if state.maximized {
            WindowBounds::Maximized(state.window_bounds)
        } else {
            drop(state);
            WindowBounds::Windowed(self.bounds())
        }
    }

    fn inner_window_bounds(&self) -> WindowBounds {
        let state = self.borrow();
        if state.fullscreen {
            WindowBounds::Fullscreen(state.window_bounds)
        } else if state.maximized {
            WindowBounds::Maximized(state.window_bounds)
        } else {
            let inset = state.inset();
            drop(state);
            WindowBounds::Windowed(self.bounds().inset(inset))
        }
    }

    fn content_size(&self) -> Size<Pixels> {
        self.borrow().bounds.size
    }

    fn resize(&mut self, size: Size<Pixels>) {
        let state = self.borrow();
        let state_ptr = self.0.clone();
        let dp_size = size.to_device_pixels(self.scale_factor());

        state.xdg_surface.set_window_geometry(
            state.bounds.origin.x.0 as i32,
            state.bounds.origin.y.0 as i32,
            dp_size.width.0,
            dp_size.height.0,
        );

        state
            .globals
            .executor
            .spawn(async move { state_ptr.resize(size) })
            .detach();
        drop(state);
        self.0.frame_loop.wake();
    }

    fn scale_factor(&self) -> f32 {
        self.borrow().scale
    }

    fn appearance(&self) -> WindowAppearance {
        self.borrow().appearance
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        let state = self.borrow();
        state.display.as_ref().map(|(id, display)| {
            Rc::new(WaylandDisplay {
                id: id.clone(),
                name: display.name.clone(),
                bounds: display.bounds.to_pixels(state.scale),
            }) as Rc<dyn PlatformDisplay>
        })
    }

    fn mouse_position(&self) -> Point<Pixels> {
        self.borrow()
            .client
            .get_client()
            .borrow()
            .mouse_location
            .unwrap_or_default()
    }

    fn modifiers(&self) -> Modifiers {
        self.borrow().client.get_client().borrow().modifiers
    }

    fn capslock(&self) -> Capslock {
        self.borrow().client.get_client().borrow().capslock
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.borrow_mut().input_handler = Some(input_handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.borrow_mut().input_handler.take()
    }

    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[PromptButton],
    ) -> Option<Receiver<usize>> {
        None
    }

    fn activate(&self) {
        // Try to request an activation token. Even though the activation is likely going to be rejected,
        // KWin and Mutter can use the app_id to visually indicate we're requesting attention.
        let state = self.borrow();
        if let (Some(activation), Some(app_id)) = (&state.globals.activation, state.app_id.clone())
        {
            state.client.set_pending_activation(state.surface.id());
            let token = activation.get_activation_token(&state.globals.qh, ());
            // The serial isn't exactly important here, since the activation is probably going to be rejected anyway.
            let serial = state.client.get_serial(SerialKind::MousePress);
            token.set_app_id(app_id);
            token.set_serial(serial, &state.globals.seat);
            token.set_surface(&state.surface);
            token.commit();
        }
    }

    fn is_active(&self) -> bool {
        self.borrow().active
    }

    fn is_hovered(&self) -> bool {
        self.borrow().hovered
    }

    fn set_title(&mut self, title: &str) {
        self.borrow().toplevel.set_title(title.to_string());
    }

    fn set_app_id(&mut self, app_id: &str) {
        let mut state = self.borrow_mut();
        state.toplevel.set_app_id(app_id.to_owned());
        state.app_id = Some(app_id.to_owned());
    }

    fn set_background_appearance(&self, background_appearance: WindowBackgroundAppearance) {
        let mut state = self.borrow_mut();
        state.background_appearance = background_appearance;
        update_window(state);
        self.0.frame_loop.wake();
    }

    fn minimize(&self) {
        self.borrow().toplevel.set_minimized();
    }

    fn zoom(&self) {
        let state = self.borrow();
        if !state.maximized {
            state.toplevel.set_maximized();
        } else {
            state.toplevel.unset_maximized();
        }
    }

    fn toggle_fullscreen(&self) {
        let mut state = self.borrow_mut();
        if !state.fullscreen {
            state.toplevel.set_fullscreen(None);
        } else {
            state.toplevel.unset_fullscreen();
        }
    }

    fn is_fullscreen(&self) -> bool {
        self.borrow().fullscreen
    }

    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        let frame_loop = Rc::downgrade(&self.0.frame_loop);
        Some(Rc::new(move || {
            if let Some(frame_loop) = frame_loop.upgrade() {
                frame_loop.wake();
            }
        }))
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.0.callbacks.borrow_mut().request_frame = Some(callback);
    }

    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> crate::DispatchEventResult>) {
        self.0.callbacks.borrow_mut().input = Some(callback);
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.callbacks.borrow_mut().active_status_change = Some(callback);
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.0.callbacks.borrow_mut().hover_status_change = Some(callback);
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.0.callbacks.borrow_mut().resize = Some(callback);
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.0.callbacks.borrow_mut().moved = Some(callback);
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.0.callbacks.borrow_mut().should_close = Some(callback);
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.0.callbacks.borrow_mut().close = Some(callback);
    }

    fn on_hit_test_window_control(&self, _callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.0.callbacks.borrow_mut().appearance_changed = Some(callback);
    }

    fn draw(&self, scene: &Scene) {
        let mut state = self.borrow_mut();
        state.renderer.draw(scene);
        self.0.frame_loop.presented();
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        let state = self.borrow();
        state.renderer.sprite_atlas().clone()
    }

    fn show_window_menu(&self, position: Point<Pixels>) {
        let state = self.borrow();
        let serial = state.client.get_serial(SerialKind::MousePress);
        state.toplevel.show_window_menu(
            &state.globals.seat,
            serial,
            position.x.0 as i32,
            position.y.0 as i32,
        );
    }

    fn start_window_move(&self) {
        let state = self.borrow();
        let serial = state.client.get_serial(SerialKind::MousePress);
        state.toplevel._move(&state.globals.seat, serial);
    }

    fn start_window_resize(&self, edge: crate::ResizeEdge) {
        let state = self.borrow();
        state.toplevel.resize(
            &state.globals.seat,
            state.client.get_serial(SerialKind::MousePress),
            edge.to_xdg(),
        )
    }

    fn window_decorations(&self) -> Decorations {
        let state = self.borrow();
        match state.decorations {
            WindowDecorations::Server => Decorations::Server,
            WindowDecorations::Client => Decorations::Client {
                tiling: state.tiling,
            },
        }
    }

    fn request_decorations(&self, decorations: WindowDecorations) {
        let mut state = self.borrow_mut();
        state.decorations = decorations;
        if let Some(decoration) = state.decoration.as_ref() {
            decoration.set_mode(decorations.to_xdg());
            update_window(state);
            self.0.frame_loop.wake();
        }
    }

    fn window_controls(&self) -> WindowControls {
        self.borrow().window_controls
    }

    fn set_client_inset(&self, inset: Pixels) {
        let mut state = self.borrow_mut();
        if Some(inset) != state.client_inset {
            state.client_inset = Some(inset);
            update_window(state);
            self.0.frame_loop.wake();
        }
    }

    fn update_ime_position(&self, bounds: Bounds<Pixels>) {
        let state = self.borrow();
        state.client.update_ime_position(bounds);
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        self.borrow().renderer.gpu_specs().into()
    }
}

fn update_window(mut state: RefMut<WaylandWindowState>) {
    let opaque = !state.is_transparent();

    state.renderer.update_transparency(!opaque);
    let mut opaque_area = state.window_bounds.map(|v| v.0 as i32);
    opaque_area.inset(state.inset().0 as i32);

    let region = state
        .globals
        .compositor
        .create_region(&state.globals.qh, ());
    region.add(
        opaque_area.origin.x,
        opaque_area.origin.y,
        opaque_area.size.width,
        opaque_area.size.height,
    );

    // Note that rounded corners make this rectangle API hard to work with.
    // As this is common when using CSD, let's just disable this API.
    if state.background_appearance == WindowBackgroundAppearance::Opaque
        && state.decorations == WindowDecorations::Server
    {
        // Promise the compositor that this region of the window surface
        // contains no transparent pixels. This allows the compositor to skip
        // updating whatever is behind the surface for better performance.
        state.surface.set_opaque_region(Some(&region));
    } else {
        state.surface.set_opaque_region(None);
    }

    if let Some(ref blur_manager) = state.globals.blur_manager {
        if state.background_appearance == WindowBackgroundAppearance::Blurred {
            if state.blur.is_none() {
                let blur = blur_manager.create(&state.surface, &state.globals.qh, ());
                state.blur = Some(blur);
            }
            state.blur.as_ref().unwrap().commit();
        } else {
            // It probably doesn't hurt to clear the blur for opaque windows
            blur_manager.unset(&state.surface);
            if let Some(b) = state.blur.take() {
                b.release()
            }
        }
    }

    region.destroy();
}

impl WindowDecorations {
    fn to_xdg(self) -> zxdg_toplevel_decoration_v1::Mode {
        match self {
            WindowDecorations::Client => zxdg_toplevel_decoration_v1::Mode::ClientSide,
            WindowDecorations::Server => zxdg_toplevel_decoration_v1::Mode::ServerSide,
        }
    }
}

impl ResizeEdge {
    fn to_xdg(self) -> xdg_toplevel::ResizeEdge {
        match self {
            ResizeEdge::Top => xdg_toplevel::ResizeEdge::Top,
            ResizeEdge::TopRight => xdg_toplevel::ResizeEdge::TopRight,
            ResizeEdge::Right => xdg_toplevel::ResizeEdge::Right,
            ResizeEdge::BottomRight => xdg_toplevel::ResizeEdge::BottomRight,
            ResizeEdge::Bottom => xdg_toplevel::ResizeEdge::Bottom,
            ResizeEdge::BottomLeft => xdg_toplevel::ResizeEdge::BottomLeft,
            ResizeEdge::Left => xdg_toplevel::ResizeEdge::Left,
            ResizeEdge::TopLeft => xdg_toplevel::ResizeEdge::TopLeft,
        }
    }
}

/// The configuration event is in terms of the window geometry, which we are constantly
/// updating to account for the client decorations. But that's not the area we want to render
/// to, due to our intrusize CSD. So, here we calculate the 'actual' size, by adding back in the insets
fn compute_outer_size(
    inset: Pixels,
    new_size: Option<Size<Pixels>>,
    tiling: Tiling,
) -> Option<Size<Pixels>> {
    new_size.map(|mut new_size| {
        if !tiling.top {
            new_size.height += inset;
        }
        if !tiling.bottom {
            new_size.height += inset;
        }
        if !tiling.left {
            new_size.width += inset;
        }
        if !tiling.right {
            new_size.width += inset;
        }

        new_size
    })
}

fn inset_by_tiling(mut bounds: Bounds<Pixels>, inset: Pixels, tiling: Tiling) -> Bounds<Pixels> {
    if !tiling.top {
        bounds.origin.y += inset;
        bounds.size.height -= inset;
    }
    if !tiling.bottom {
        bounds.size.height -= inset;
    }
    if !tiling.left {
        bounds.origin.x += inset;
        bounds.size.width -= inset;
    }
    if !tiling.right {
        bounds.size.width -= inset;
    }

    bounds
}

#[cfg(test)]
mod tests {
    use super::FramePhase;

    /// A [`FramePhase`] with what surrounds it: GPUI's frame demand, the
    /// event loop's scheduled-frame timer, and the compositor's frame
    /// callbacks. The tests explore every state these can reach.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct World {
        phase: FramePhase,
        configured: bool,
        /// Demand the next frame must serve.
        dirty: bool,
        timer: bool,
        callbacks: u32,
        /// A frame presented since the compositor last delivered a callback.
        unpaced: bool,
    }

    #[derive(Clone, Copy, Debug)]
    enum Step {
        /// GPUI gains demand between frames.
        Demand,
        /// The first configure arrives and runs the first frame.
        Configure,
        /// The scheduled timer runs a frame, which leaves demand behind
        /// when `true`, as an animation's next-frame callback does.
        Timer(bool),
        /// The compositor delivers the frame callback, which runs a frame.
        Callback(bool),
    }

    const STEPS: [Step; 6] = [
        Step::Demand,
        Step::Configure,
        Step::Timer(false),
        Step::Timer(true),
        Step::Callback(false),
        Step::Callback(true),
    ];

    impl World {
        fn new() -> Self {
            Self {
                phase: FramePhase::Unconfigured,
                configured: false,
                dirty: false,
                timer: false,
                callbacks: 0,
                unpaced: false,
            }
        }

        fn wake(&mut self) {
            let (phase, schedule) = self.phase.wake();
            self.phase = phase;
            if schedule {
                assert!(!self.timer, "a second timer for one frame: {self:?}");
                self.timer = true;
            }
        }

        /// A frame as `WaylandWindowStatePtr::frame` runs one: GPUI draws
        /// when dirty, and `demand` arrives while the frame runs.
        fn frame(&mut self, demand: bool) {
            self.phase = FramePhase::Running {
                demand: false,
                presented: false,
            };
            if std::mem::take(&mut self.dirty) {
                assert!(
                    !self.unpaced,
                    "presented twice with no frame callback between: {self:?}"
                );
                self.phase = self.phase.present();
                self.unpaced = true;
            }
            if demand {
                self.dirty = true;
                self.wake();
            }
            let (phase, request) = self.phase.end();
            self.phase = phase;
            self.callbacks += u32::from(request);
        }

        /// The world after `step`, or `None` when `step` cannot happen now.
        fn step(mut self, step: Step) -> Option<Self> {
            match step {
                Step::Demand => {
                    self.dirty = true;
                    self.wake();
                }
                Step::Configure if !self.configured => {
                    self.configured = true;
                    self.frame(false);
                }
                Step::Timer(demand) if self.timer => {
                    self.timer = false;
                    self.frame(demand);
                }
                Step::Callback(demand) if self.callbacks > 0 => {
                    self.callbacks -= 1;
                    self.unpaced = false;
                    self.frame(demand);
                }
                _ => return None,
            }
            Some(self)
        }

        fn check(&self) {
            assert!(
                self.configured || self.phase == FramePhase::Unconfigured,
                "a frame scheduled before the first configure: {self:?}"
            );
            assert_eq!(
                self.timer,
                self.phase == FramePhase::Scheduled,
                "timer and phase disagree: {self:?}"
            );
            assert_eq!(
                self.callbacks,
                u32::from(self.phase == FramePhase::Waiting),
                "frame callbacks and phase disagree: {self:?}"
            );
            if self.dirty {
                assert!(
                    matches!(
                        self.phase,
                        FramePhase::Unconfigured | FramePhase::Scheduled | FramePhase::Waiting
                    ),
                    "demand with no frame coming: {self:?}"
                );
            }
        }
    }

    fn reachable() -> Vec<World> {
        let mut seen = vec![World::new()];
        let mut next = 0;
        while let Some(&world) = seen.get(next) {
            next += 1;
            for step in STEPS {
                if let Some(after) = world.step(step) {
                    after.check();
                    if !seen.contains(&after) {
                        seen.push(after);
                    }
                }
            }
        }
        seen
    }

    #[test]
    fn every_reachable_state_keeps_the_frame_loop_invariants() {
        let states = reachable();
        for phase in [
            FramePhase::Unconfigured,
            FramePhase::Idle,
            FramePhase::Scheduled,
            FramePhase::Waiting,
        ] {
            assert!(
                states.iter().any(|world| world.phase == phase),
                "{phase:?} is unreachable"
            );
        }
    }

    #[test]
    fn the_loop_parks_within_two_frames_once_demand_stops() {
        for mut world in reachable() {
            let mut frames = 0;
            while world.timer || world.callbacks > 0 {
                let step = if world.timer {
                    Step::Timer(false)
                } else {
                    Step::Callback(false)
                };
                world = world.step(step).expect("a pending frame can run");
                world.check();
                frames += 1;
                assert!(frames <= 2, "frames keep running with no demand: {world:?}");
            }
            assert!(
                matches!(world.phase, FramePhase::Idle | FramePhase::Unconfigured),
                "{world:?}"
            );
        }
    }
}
