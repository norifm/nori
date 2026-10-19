//! A custom Slint platform on winit with OpenGL on Linux and wgpu elsewhere. Skia renders the page, the top
//! pill and the player pill into separate layers; glass.wgsl composites them: the page, the edge fades, and
//! inside each pill the page blurred (the pill's own layer lays its tint and outline over that). Pointer events
//! go to the layer under the pointer (or the one pressed in), keys to the page. Frames are drawn only when a
//! layer is dirty or animating.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(not(target_os = "linux"))]
use slint::platform::skia_renderer::SkiaWGPU30Renderer;
use slint::platform::{Key, PointerEventButton, WindowAdapter, WindowEvent};
#[cfg(not(target_os = "linux"))]
use slint::wgpu_30::wgpu;

#[cfg(target_os = "linux")]
mod gl;
#[cfg(target_os = "linux")]
use gl::{Draw, Gpu, LayerTexture, Renderer};
#[cfg(not(target_os = "linux"))]
type Renderer = SkiaWGPU30Renderer;
#[cfg(not(target_os = "linux"))]
type LayerTexture = (wgpu::Texture, wgpu::TextureView);
use slint::{LogicalPosition, LogicalSize, PhysicalSize, PlatformError, SharedString};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::window::{Window as WinitWindow, WindowAttributes, WindowId};

use crate::menu::MenuBar;

/// Layer geometry in logical pixels; must match app.slint.
const TOP_H: f32 = 48.0;
const TOP_Y: f32 = 12.0;
const TOP_RADIUS: f32 = 24.0;
const PLAYER_H: f32 = 64.0;
const PLAYER_BOTTOM: f32 = 12.0;
const PLAYER_RADIUS: f32 = 26.0;
const PILL_MAX_W: f32 = 1040.0;
/// The window's side margin, and at the top left what the traffic lights take.
const SIDE: f32 = 16.0;
#[cfg(target_os = "macos")]
const TRAFFIC: f32 = 92.0;
#[cfg(not(target_os = "macos"))]
const TRAFFIC: f32 = SIDE;
/// The pills blur the page by this much, and the content fades into the page over this much, as Android's
/// chrome does (`BACKDROP_BLUR`, `EDGE_REACH`).
const BLUR_SIGMA: f32 = 24.0;
const EDGE_REACH: f32 = 24.0;
/// How far a fade's strip runs on past the pill's outer edge.
const FADE_PAD: f32 = 6.0;
/// What the pointer in a pill's margin is kept from the page for.
const KEEP: f32 = 8.0;

/// Format of every layer and the surface; Skia renders it on Metal and Vulkan.
#[cfg(not(target_os = "linux"))]
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

enum Wake {
    Call(Box<dyn FnOnce() + Send>),
    Quit,
}

#[cfg(not(target_os = "linux"))]
struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

#[cfg(not(target_os = "linux"))]
impl Gpu {
    /// Uploads decoded pixels once; Slint keeps the texture instead of CPU raster copies.
    fn picture(&self, pixels: &[u8], width: u32, height: u32) -> slint::Image {
        let texture = self.image_texture(width, height);
        self.queue.write_texture(
            texture.as_image_copy(), pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
            texture.size(),
        );
        slint::Image::try_from(texture).expect("RGBA texture with render and sampling usages")
    }

    fn image_texture(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cover"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Card covers are decoded to equal squares; copy their pixels into one seamless image.
    fn mosaic(&self, images: &[slint::Image]) -> Option<slint::Image> {
        if images.len() != 4 { return None; }
        let textures: Vec<_> = images.iter().map(slint::Image::to_wgpu_30_texture).collect::<Option<_>>()?;
        let size = textures[0].size();
        if size.width != size.height || textures.iter().any(|t| t.size() != size) { return None; }
        let target = self.image_texture(size.width * 2, size.height * 2);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pick covers") });
        for (i, texture) in textures.iter().enumerate() {
            let mut destination = target.as_image_copy();
            destination.origin = wgpu::Origin3d { x: (i as u32 % 2) * size.width, y: (i as u32 / 2) * size.height, z: 0 };
            encoder.copy_texture_to_texture(texture.as_image_copy(), destination, size);
        }
        self.queue.submit([encoder.finish()]);
        Some(slint::Image::try_from(target).expect("RGBA texture with render and sampling usages"))
    }

}

/// A Slint window rendered into its own texture.
struct Layer {
    window: slint::Window,
    renderer: Renderer,
    size: Cell<PhysicalSize>,
    dirty: Cell<bool>,
    texture: RefCell<Option<LayerTexture>>,
    /// Weak: `Shared` owns the layers.
    shared: Weak<Shared>,
}

impl WindowAdapter for Layer {
    fn window(&self) -> &slint::Window {
        &self.window
    }

    fn size(&self) -> PhysicalSize {
        self.size.get()
    }

    fn renderer(&self) -> &dyn slint::platform::Renderer {
        &self.renderer
    }

    fn request_redraw(&self) {
        self.dirty.set(true);
        if let Some(s) = self.shared.upgrade() {
            redraw(&s);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Page,
    Top,
    Player,
}

/// Layers in creation order, each with its role once assigned.
type Layers = RefCell<Vec<(Rc<Layer>, Cell<Option<Role>>)>>;

/// Halvings in the page's blur pyramid: its smallest level is 1/128 of the page.
const PYRAMID_LEVELS: u32 = 7;

/// Format of the blur pyramid: half floats, so a long smooth blur does not band.
#[cfg(not(target_os = "linux"))]
const PYRAMID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The page blurred at every halving: the texture, a view of each mip to draw into, and one of them all
/// to read.
#[cfg(not(target_os = "linux"))]
struct Pyramid {
    texture: wgpu::Texture,
    mips: Vec<wgpu::TextureView>,
    all: wgpu::TextureView,
}

/// State shared by the platform, the event loop and [`Compositor`] handles (main thread only).
struct Shared {
    gpu: Rc<Gpu>,
    layers: Layers,
    window: RefCell<Option<Arc<WinitWindow>>>,
    size: Cell<PhysicalSize>,
    frames: RefCell<FrameSchedule>,
    proxy: EventLoopProxy<Wake>,
    top_shown: Cell<bool>,
    player_shown: Cell<bool>,
    /// The page's colour as painted (RGB, 0 to 1): what the content fades into.
    page_colour: Cell<[f32; 3]>,
    /// The window is not on screen (minimised, covered, locked): nothing is drawn until it shows again.
    occluded: Cell<bool>,
    /// Width of the right panel (0 when closed); the pills centre in the remaining page.
    right: Cell<f32>,
    /// Queried each frame for the lyrics focus blur.
    focus: RefCell<Option<FocusSource>>,
    menu: MenuBar,
    /// Run as the event loop ends, however it ends: on macOS the app menu's and the Dock's Quit, and logging
    /// out, end the process from inside the loop, which never returns to the caller of `run`.
    on_exit: RefCell<Option<Box<dyn FnOnce()>>>,
    /// Run when the window is focused or the system's appearance changes: the app looks at the date and the
    /// theme again.
    on_wake: RefCell<Option<Rc<dyn Fn()>>>,
}

type FocusSource = Box<dyn Fn() -> Option<Focus>>;

struct FrameSchedule {
    next: Instant,
    interval: Duration,
    pending: bool,
}

impl FrameSchedule {
    fn request(&mut self, now: Instant) -> bool {
        self.pending = true;
        now >= self.next
    }

    fn begin(&mut self, now: Instant) -> bool {
        if !self.request(now) { return false; }
        self.pending = false;
        self.next = now + self.interval;
        true
    }

    fn deadline(&self) -> Option<Instant> { self.pending.then_some(self.next) }
}

/// A page region drawn blurred except for a sharp horizontal band (logical pixels).
#[derive(Clone, Copy)]
pub struct Focus {
    pub region: [f32; 4],
    pub band_top: f32,
    pub band_h: f32,
    /// A line and the gap after it: each step further from the band blurs a point more.
    pub pitch: f32,
}

impl Shared {
    fn shown(&self, role: Role) -> bool {
        match role {
            Role::Page => true,
            Role::Top => self.top_shown.get(),
            Role::Player => self.player_shown.get(),
        }
    }
}

/// The app's handle to the compositor, alongside the platform Slint owns.
#[derive(Clone)]
pub struct Compositor(Rc<Shared>);

struct Platform {
    shared: Rc<Shared>,
    event_loop: RefCell<Option<EventLoop<Wake>>>,
}

impl slint::platform::Platform for Platform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let g = &self.shared.gpu;
        #[cfg(target_os = "linux")]
        let renderer = g.renderer();
        #[cfg(not(target_os = "linux"))]
        let renderer = SkiaWGPU30Renderer::new(g.instance.clone(), g.adapter.clone(), g.device.clone(), g.queue.clone())?;
        #[cfg(not(target_os = "linux"))]
        renderer.set_resource_cache_limit(16 * 1024 * 1024);
        let layer = Rc::new_cyclic(|me: &Weak<Layer>| Layer {
            window: slint::Window::new(me.clone() as Weak<dyn WindowAdapter>),
            renderer,
            size: Cell::new(PhysicalSize::new(1, 1)),
            dirty: Cell::new(true),
            texture: RefCell::new(None),
            shared: Rc::downgrade(&self.shared),
        });
        self.shared.layers.borrow_mut().push((layer.clone(), Cell::new(None)));
        Ok(layer)
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        let event_loop = self.event_loop.borrow_mut().take().ok_or_else(|| PlatformError::from("the event loop ran already"))?;
        let mut runner = Runner { shared: self.shared.clone(), draw: None, pointer: LogicalPosition::new(0.0, 0.0), over: None, held: None, modifiers: Default::default() };
        event_loop.run_app(&mut runner).map_err(|e| PlatformError::from(e.to_string()))
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn slint::platform::EventLoopProxy>> {
        Some(Box::new(Proxy(self.shared.proxy.clone())))
    }

    fn set_clipboard_text(&self, text: &str, clipboard: slint::platform::Clipboard) {
        if clipboard == slint::platform::Clipboard::DefaultClipboard {
            if let Ok(mut c) = arboard::Clipboard::new() {
                let _ = c.set_text(text.to_string());
            }
        }
    }

    fn clipboard_text(&self, clipboard: slint::platform::Clipboard) -> Option<String> {
        if clipboard != slint::platform::Clipboard::DefaultClipboard {
            return None;
        }
        arboard::Clipboard::new().ok()?.get_text().ok()
    }
}

struct Proxy(EventLoopProxy<Wake>);

impl slint::platform::EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        self.0.send_event(Wake::Quit).map_err(|_| slint::EventLoopError::EventLoopTerminated)
    }

    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), slint::EventLoopError> {
        self.0.send_event(Wake::Call(event)).map_err(|_| slint::EventLoopError::EventLoopTerminated)
    }
}

/// Installs the platform. Must run before any Slint window is created.
pub fn install() -> Result<Compositor, String> {
    let event_loop = EventLoop::<Wake>::with_user_event().build().map_err(|e| format!("the event loop: {e}"))?;
    #[cfg(target_os = "linux")]
    let gpu = Gpu::new();
    #[cfg(not(target_os = "linux"))]
    let gpu = {
        let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        // Every draw uses a fixed vertex range; indirect-command pipelines are unused.
        instance_descriptor.flags.remove(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
        let instance = wgpu::Instance::new(instance_descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::LowPower, ..Default::default() }))
            .map_err(|e| format!("no GPU: {e}"))?;
        let descriptor = wgpu::DeviceDescriptor { memory_hints: wgpu::MemoryHints::MemoryUsage, ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).map_err(|e| format!("the GPU: {e}"))?;
        Gpu { instance, adapter, device, queue }
    };
    let shared = Rc::new(Shared {
        gpu: Rc::new(gpu),
        layers: RefCell::new(Vec::new()),
        window: RefCell::new(None),
        size: Cell::new(PhysicalSize::new(1280, 820)),
        frames: RefCell::new(FrameSchedule { next: Instant::now(), interval: Duration::ZERO, pending: false }),
        proxy: event_loop.create_proxy(),
        top_shown: Cell::new(false),
        player_shown: Cell::new(false),
        page_colour: Cell::new([0.0; 3]),
        occluded: Cell::new(false),
        right: Cell::new(0.0),
        focus: RefCell::new(None),
        menu: MenuBar::default(),
        on_exit: RefCell::new(None),
        on_wake: RefCell::new(None),
    });
    slint::platform::set_platform(Box::new(Platform { shared: shared.clone(), event_loop: RefCell::new(Some(event_loop)) })).map_err(|e| format!("the platform: {e}"))?;
    Ok(Compositor(shared))
}

impl Compositor {
    /// Skia owns each uploaded cover texture on the shared OpenGL context.
    #[cfg(target_os = "linux")]
    pub fn picture(&self, pixels: &[u8], width: u32, height: u32) -> slint::Image {
        self.0.gpu.picture(pixels, width, height)
    }

    #[cfg(target_os = "linux")]
    pub fn mosaic(&self, images: &[slint::Image]) -> Option<slint::Image> {
        self.0.gpu.mosaic(images)
    }

    #[cfg(target_os = "linux")]
    pub fn on_mosaics_ready(&self, callback: impl Fn() + 'static) { self.0.gpu.on_mosaics_ready(callback); }

    #[cfg(not(target_os = "linux"))]
    pub fn picture(&self, pixels: &[u8], width: u32, height: u32) -> slint::Image {
        self.0.gpu.picture(pixels, width, height)
    }

    #[cfg(not(target_os = "linux"))]
    pub fn mosaic(&self, images: &[slint::Image]) -> Option<slint::Image> {
        self.0.gpu.mosaic(images)
    }

    /// Assigns each Slint window its layer role.
    pub fn roles(&self, page: &slint::Window, top: &slint::Window, player: &slint::Window) {
        let s = &self.0;
        for (layer, role) in s.layers.borrow().iter() {
            let w = &layer.window as *const slint::Window;
            if std::ptr::eq(w, page) {
                role.set(Some(Role::Page));
            } else if std::ptr::eq(w, top) {
                role.set(Some(Role::Top));
            } else if std::ptr::eq(w, player) {
                role.set(Some(Role::Player));
            }
        }
        s.top_shown.set(true);
        s.player_shown.set(true);
        relayout(s);
    }

    /// The page's colour as it is painted now (ARGB), which the content under the pills fades into.
    pub fn set_page_colour(&self, argb: u32) {
        let [_, r, g, b] = argb.to_be_bytes();
        let colour = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
        if self.0.page_colour.replace(colour) != colour {
            redraw(&self.0);
        }
    }

    pub fn set_focus_source(&self, f: impl Fn() -> Option<Focus> + 'static) {
        *self.0.focus.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_right_panel(&self, w: f32) {
        if self.0.right.get() != w {
            self.0.right.set(w);
            relayout(&self.0);
        }
    }

    /// The pills' x and width (logical pixels), top then player, for the page's popovers.
    pub fn pills(&self) -> ((f32, f32), (f32, f32)) {
        let size = self.0.window.borrow().as_ref().map_or(LogicalSize::new(1280.0, 820.0), |w| {
            let l = w.inner_size().to_logical::<f32>(w.scale_factor());
            LogicalSize::new(l.width, l.height)
        });
        let right = self.0.right.get();
        let (top, player) = (pill_rect(Role::Top, size, right), pill_rect(Role::Player, size, right));
        ((top.x, top.w), (player.x, player.w))
    }

    /// Shows or hides the two pills over the page.
    pub fn show_pills(&self, top: bool, player: bool) {
        let s = &self.0;
        if s.top_shown.get() != top || s.player_shown.get() != player {
            s.top_shown.set(top);
            s.player_shown.set(player);
            redraw(s);
        }
    }

    /// Starts a window drag, as from a titlebar.
    pub fn drag_window(&self) {
        if let Some(w) = self.0.window.borrow().as_ref() {
            let _ = w.drag_window();
        }
    }

    /// Toggles maximized, as a titlebar double click does.
    pub fn zoom_window(&self) {
        if let Some(w) = self.0.window.borrow().as_ref() {
            w.set_maximized(!w.is_maximized());
        }
    }

    pub fn menu(&self) -> &MenuBar {
        &self.0.menu
    }

    /// Whether the system is dark (dark where it cannot say).
    pub fn system_dark(&self) -> bool {
        self.0.window.borrow().as_ref().and_then(|w| w.theme()).is_none_or(|t| t == winit::window::Theme::Dark)
    }

    /// Whether the system asks for less motion.
    pub fn reduce_motion(&self) -> bool {
        reduce_motion()
    }

    pub fn on_wake(&self, f: impl Fn() + 'static) {
        *self.0.on_wake.borrow_mut() = Some(Rc::new(f));
    }

    pub fn on_exit(&self, f: impl FnOnce() + 'static) {
        *self.0.on_exit.borrow_mut() = Some(Box::new(f));
    }
}

fn wake(s: &Shared) {
    let f = s.on_wake.borrow().clone();
    if let Some(f) = f {
        f();
    }
}

fn redraw(s: &Shared) {
    if s.occluded.get() {
        return;
    }
    if !s.frames.borrow_mut().request(Instant::now()) { return; }
    if let Some(w) = s.window.borrow().as_ref() {
        w.request_redraw();
    }
}

/// A rect in the window, logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn scaled(self, scale: f32) -> [f32; 4] {
        [self.x * scale, self.y * scale, self.w * scale, self.h * scale]
    }
}

/// A pill's rect: as wide as the page beside the panel allows up to [`PILL_MAX_W`], centred on that page.
/// The top pill keeps clear of the traffic lights on both sides.
fn pill_rect(role: Role, window: LogicalSize, right: f32) -> Rect {
    let page = (window.width - right).max(0.0);
    let (side, h, y) = match role {
        Role::Top => (TRAFFIC, TOP_H, TOP_Y),
        _ => (SIDE, PLAYER_H, window.height - PLAYER_H - PLAYER_BOTTOM),
    };
    let w = (page - 2.0 * side).clamp(200.0, PILL_MAX_W);
    Rect { x: (page - w) / 2.0, y, w, h }
}

fn pill_radius(role: Role) -> f32 {
    if role == Role::Top { TOP_RADIUS } else { PLAYER_RADIUS }
}

/// The fade under a pill: its strip across the window's width (from the window's edge to `reach` past the
/// pill's outer side), where in the ramp the pill begins, and whether the ramp is opaque at the top.
struct Fade {
    strip: Rect,
    at: f32,
    top: bool,
}

fn fade_of(role: Role, window: LogicalSize, right: f32) -> Fade {
    let pill = pill_rect(role, window, right);
    let top = role == Role::Top;
    let strip = if top { pill.y + pill.h + FADE_PAD } else { window.height - pill.y + FADE_PAD };
    let h = strip + EDGE_REACH;
    Fade { strip: Rect { x: 0.0, y: if top { 0.0 } else { window.height - h }, w: window.width, h }, at: EDGE_REACH / h, top }
}

/// The pyramid level that blurs by [`BLUR_SIGMA`] logical pixels: level k blurs about 0.6 * 2^k pixels.
fn blur_level(scale: f32) -> f32 {
    (BLUR_SIGMA * scale / 0.6).log2()
}

/// A layer's rect in the window, logical pixels.
fn place(s: &Shared, role: Role, window: LogicalSize) -> (LogicalPosition, LogicalSize) {
    if role == Role::Page {
        return (LogicalPosition::new(0.0, 0.0), window);
    }
    let r = pill_rect(role, window, s.right.get());
    (LogicalPosition::new(r.x, r.y), LogicalSize::new(r.w, r.h))
}

/// Resizes every layer to the current window size.
fn relayout(s: &Shared) {
    layout_layers(s);
    redraw(s);
}

fn layout_layers(s: &Shared) {
    let Some(win) = s.window.borrow().clone() else { return };
    let scale = win.scale_factor() as f32;
    let phys = s.size.get();
    let logical = LogicalSize::new(phys.width as f32 / scale, phys.height as f32 / scale);
    for (layer, role) in s.layers.borrow().iter() {
        let Some(role) = role.get() else { continue };
        let (_, size) = place(s, role, logical);
        let px = PhysicalSize::new((size.width * scale).round().max(1.0) as u32, (size.height * scale).round().max(1.0) as u32);
        if layer.window.scale_factor() != scale {
            layer.window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
        }
        if layer.size.get() != px {
            layer.size.set(px);
            layer.window.dispatch_event(WindowEvent::Resized { size });
            layer.dirty.set(true);
        }
    }
}

struct Runner {
    shared: Rc<Shared>,
    draw: Option<Draw>,
    pointer: LogicalPosition,
    /// Layer under the pointer, and the layer a button was pressed in (captures until release).
    over: Option<Role>,
    held: Option<Role>,
    modifiers: winit::keyboard::ModifiersState,
}

impl Runner {
    fn layer(&self, role: Role) -> Option<Rc<Layer>> {
        self.shared.layers.borrow().iter().find(|(_, r)| r.get() == Some(role)).map(|(l, _)| l.clone())
    }

    fn logical_window(&self) -> Option<LogicalSize> {
        let w = self.shared.window.borrow().clone()?;
        let scale = w.scale_factor() as f32;
        let p = self.shared.size.get();
        Some(LogicalSize::new(p.width as f32 / scale, p.height as f32 / scale))
    }

    fn under(&self, at: LogicalPosition) -> Role {
        let Some(win) = self.logical_window() else { return Role::Page };
        // A pill takes the pointer over its margin too, so a click beside it opens nothing under it.
        for role in [Role::Player, Role::Top] {
            if !self.shared.shown(role) || self.layer(role).is_none() {
                continue;
            }
            let r = pill_rect(role, win, self.shared.right.get());
            let (top, bottom) = if role == Role::Top { (0.0, r.y + r.h + KEEP) } else { (r.y - KEEP, win.height) };
            if at.x >= r.x - KEEP && at.x < r.x + r.w + KEEP && at.y >= top && at.y < bottom {
                return role;
            }
        }
        Role::Page
    }

    fn local(&self, role: Role, at: LogicalPosition) -> LogicalPosition {
        let Some(win) = self.logical_window() else { return at };
        let (o, _) = place(&self.shared, role, win);
        LogicalPosition::new(at.x - o.x, at.y - o.y)
    }

    fn pointer(&mut self, event: impl Fn(LogicalPosition) -> WindowEvent) {
        let to = self.held.unwrap_or_else(|| self.under(self.pointer));
        if self.over != Some(to) {
            if let Some(old) = self.over.and_then(|r| self.layer(r)) {
                old.window.dispatch_event(WindowEvent::PointerExited);
            }
            self.over = Some(to);
        }
        if let Some(l) = self.layer(to) {
            l.window.dispatch_event(event(self.local(to, self.pointer)));
        }
    }

    fn key(&self, event: &winit::event::KeyEvent) {
        let Some(page) = self.layer(Role::Page) else { return };
        let Some(text) = key_text(event) else { return };
        let e = match (event.state, event.repeat) {
            (ElementState::Pressed, false) => WindowEvent::KeyPressed { text },
            (ElementState::Pressed, true) => WindowEvent::KeyPressRepeated { text },
            (ElementState::Released, _) => WindowEvent::KeyReleased { text },
        };
        page.window.dispatch_event(e);
    }

    fn animating(&self) -> bool {
        self.shared.layers.borrow().iter().any(|(l, r)| r.get().is_some_and(|r| self.shared.shown(r)) && (l.window.has_active_animations() || l.dirty.get()))
    }
}

fn wheel_event(position: LogicalPosition, dx: f32, dy: f32, phase: winit::event::TouchPhase) -> WindowEvent {
    use i_slint_core::input::{BackendMouseEvent, TouchPhase};
    let phase = match phase {
        winit::event::TouchPhase::Started => TouchPhase::Started,
        winit::event::TouchPhase::Moved => TouchPhase::Moved,
        winit::event::TouchPhase::Ended => TouchPhase::Ended,
        winit::event::TouchPhase::Cancelled => TouchPhase::Cancelled,
    };
    WindowEvent::internal(BackendMouseEvent::Wheel {
        position: i_slint_core::lengths::logical_point_from_api(position), delta_x: dx, delta_y: dy, phase,
    })
}

/// Maps a winit key to Slint's key text. On macOS, Command maps to Control and Control to Meta, as in
/// Slint's own macOS backend.
fn key_text(event: &winit::event::KeyEvent) -> Option<SharedString> {
    let mac = cfg!(target_os = "macos");
    let named = |k: Key| Some(SharedString::from(k));
    match &event.logical_key {
        WKey::Named(n) => match n {
            NamedKey::Enter => named(Key::Return),
            NamedKey::Tab => named(Key::Tab),
            NamedKey::Backspace => named(Key::Backspace),
            NamedKey::Delete => named(Key::Delete),
            NamedKey::Escape => named(Key::Escape),
            NamedKey::ArrowLeft => named(Key::LeftArrow),
            NamedKey::ArrowRight => named(Key::RightArrow),
            NamedKey::ArrowUp => named(Key::UpArrow),
            NamedKey::ArrowDown => named(Key::DownArrow),
            NamedKey::Home => named(Key::Home),
            NamedKey::End => named(Key::End),
            NamedKey::PageUp => named(Key::PageUp),
            NamedKey::PageDown => named(Key::PageDown),
            NamedKey::Space => Some(" ".into()),
            NamedKey::Shift => named(Key::Shift),
            NamedKey::Alt => named(Key::Alt),
            NamedKey::Control => named(if mac { Key::Meta } else { Key::Control }),
            NamedKey::Super | NamedKey::Meta => named(if mac { Key::Control } else { Key::Meta }),
            _ => None,
        },
        WKey::Character(c) => Some(event.text.as_ref().filter(|t| !t.is_empty() && !t.chars().any(char::is_control)).map_or_else(|| c.as_str().into(), |t| t.as_str().into())),
        _ => None,
    }
}

impl ApplicationHandler<Wake> for Runner {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.shared.window.borrow().is_some() {
            return;
        }
        let attrs = WindowAttributes::default().with_title("nori").with_inner_size(winit::dpi::LogicalSize::new(1280.0, 820.0)).with_min_inner_size(winit::dpi::LogicalSize::new(860.0, 560.0));
        #[cfg(all(feature = "test-control", target_os = "linux"))]
        let attrs = if std::env::var_os("NORI_TEST_OFFSCREEN").is_some() {
            use winit::platform::x11::WindowAttributesExtX11;
            attrs.with_override_redirect(true).with_position(winit::dpi::PhysicalPosition::new(-4096, -4096)).with_active(false)
        } else {
            attrs
        };
        #[cfg(target_os = "linux")]
        let attrs = match gl::window_attributes(el, attrs) {
            Ok(attrs) => attrs,
            Err(e) => { eprintln!("nori: no window visual: {e}"); el.exit(); return; }
        };
        #[cfg(target_os = "macos")]
        let attrs = {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs.with_titlebar_transparent(true).with_fullsize_content_view(true).with_title_hidden(true)
        };
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("nori: no window: {e}");
                el.exit();
                return;
            }
        };
        match Draw::new(&self.shared.gpu, window.clone()) {
            Ok(d) => self.draw = Some(d),
            Err(e) => {
                eprintln!("nori: the window cannot be drawn: {e}");
                el.exit();
                return;
            }
        }
        #[cfg(target_os = "macos")]
        unified_toolbar(&window);
        #[cfg(target_os = "linux")]
        {
            let refresh = window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).filter(|rate| *rate > 0).unwrap_or(60_000);
            self.shared.frames.borrow_mut().interval = Duration::from_secs_f64(1000.0 / refresh as f64);
        }
        let size = window.inner_size();
        self.shared.size.set(PhysicalSize::new(size.width, size.height));
        *self.shared.window.borrow_mut() = Some(window);
        relayout(&self.shared);
        self.shared.menu.install();
        wake(&self.shared);
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        let f = self.shared.on_exit.borrow_mut().take();
        if let Some(f) = f {
            f();
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: Wake) {
        match event {
            Wake::Call(f) => f(),
            Wake::Quit => el.exit(),
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: winit::event::WindowEvent) {
        use winit::event::WindowEvent as E;
        match event {
            E::CloseRequested => el.exit(),
            E::Occluded(o) => {
                self.shared.occluded.set(o);
                redraw(&self.shared);
            }
            E::Resized(size) => {
                self.shared.size.set(PhysicalSize::new(size.width, size.height));
                redraw(&self.shared);
            }
            E::ScaleFactorChanged { .. } => relayout(&self.shared),
            E::Focused(on) => {
                if let Some(l) = self.layer(Role::Page) {
                    l.window.dispatch_event(WindowEvent::WindowActiveChanged(on));
                }
                if on {
                    wake(&self.shared);
                }
            }
            E::ThemeChanged(_) => wake(&self.shared),
            E::CursorMoved { position, .. } => {
                let scale = self.shared.window.borrow().as_ref().map_or(1.0, |w| w.scale_factor());
                let p = position.to_logical::<f32>(scale);
                self.pointer = LogicalPosition::new(p.x, p.y);
                self.pointer(|position| WindowEvent::PointerMoved { position });
            }
            E::CursorLeft { .. } => {
                if let Some(old) = self.over.take().and_then(|r| self.layer(r)) {
                    old.window.dispatch_event(WindowEvent::PointerExited);
                }
            }
            E::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => PointerEventButton::Left,
                    MouseButton::Right => PointerEventButton::Right,
                    MouseButton::Middle => PointerEventButton::Middle,
                    _ => PointerEventButton::Other,
                };
                match state {
                    ElementState::Pressed => {
                        self.held = Some(self.under(self.pointer));
                        self.pointer(|position| WindowEvent::PointerPressed { position, button });
                    }
                    ElementState::Released => {
                        self.pointer(|position| WindowEvent::PointerReleased { position, button });
                        self.held = None;
                    }
                }
            }
            E::MouseWheel { delta, phase, .. } => {
                let scale = self.shared.window.borrow().as_ref().map_or(1.0, |w| w.scale_factor()) as f32;
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * 60.0, y * 60.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32 / scale, p.y as f32 / scale),
                };
                self.pointer(|position| wheel_event(position, dx, dy, phase));
            }
            E::ModifiersChanged(m) => self.modifiers = m.state(),
            E::KeyboardInput { event, .. } => self.key(&event),
            E::Ime(winit::event::Ime::Commit(text)) => {
                if let Some(page) = self.layer(Role::Page) {
                    page.window.dispatch_event(WindowEvent::KeyPressed { text: text.as_str().into() });
                    page.window.dispatch_event(WindowEvent::KeyReleased { text: text.as_str().into() });
                }
            }
            E::RedrawRequested => {
                if !self.shared.frames.borrow_mut().begin(Instant::now()) { return; }
                slint::platform::update_timers_and_animations();
                if self.shared.occluded.get() {
                    return;
                }
                let size = self.shared.size.get();
                if let Some(d) = &mut self.draw {
                    if d.size() != PhysicalSize::new(size.width.max(1), size.height.max(1)) {
                        d.resize(size.width, size.height);
                        layout_layers(&self.shared);
                    }
                }
                let win = self.logical_window();
                if let (Some(d), Some(win)) = (&mut self.draw, win) {
                    #[cfg(target_os = "linux")]
                    if let Err(e) = d.frame(&self.shared, win) { eprintln!("nori: the window was not drawn: {e}"); }
                    #[cfg(not(target_os = "linux"))]
                    d.frame(&self.shared, win);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        slint::platform::update_timers_and_animations();
        self.shared.menu.poll();
        if self.animating() {
            redraw(&self.shared);
        }
        let now = Instant::now();
        let frame = (!self.shared.occluded.get()).then(|| self.shared.frames.borrow().deadline()).flatten();
        if frame.is_some_and(|at| at <= now) { redraw(&self.shared); }
        let timer = slint::platform::duration_until_next_timer_update().map(|d| now + d.min(Duration::from_secs(60)));
        let next = timer.into_iter().chain(frame.filter(|at| *at > now)).min();
        el.set_control_flow(match next {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}

#[cfg(not(target_os = "linux"))]
struct Draw {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    copy: wgpu::RenderPipeline,
    over: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    pill: wgpu::RenderPipeline,
    fade: wgpu::RenderPipeline,
    focus: wgpu::RenderPipeline,
    bind: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    groups: RefCell<Vec<Group>>,
    next_group: Cell<usize>,
    pyramid: Option<Pyramid>,
    /// The pyramid holds the page as last rendered.
    page_blurred: bool,
    /// Surface must be reconfigured before the next frame.
    stale: bool,
}

#[cfg(not(target_os = "linux"))]
struct Group {
    buffer: wgpu::Buffer,
    a: wgpu::TextureView,
    b: wgpu::TextureView,
    binding: wgpu::BindGroup,
}

#[cfg(not(target_os = "linux"))]
impl Draw {
    fn size(&self) -> PhysicalSize { PhysicalSize::new(self.config.width, self.config.height) }

    fn new(gpu: &Gpu, window: Arc<WinitWindow>) -> Result<Draw, String> {
        let d = &gpu.device;
        let size = window.inner_size();
        #[cfg(target_os = "linux")]
        let composited_x11 = {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let native = window.window_handle().is_ok_and(|h| matches!(h.as_raw(), RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)));
            native && (|| {
                use x11rb::protocol::xproto::ConnectionExt;
                let (connection, screen) = x11rb::connect(None).ok()?;
                let selection = connection.intern_atom(true, format!("_NET_WM_CM_S{screen}").as_bytes()).ok()?.reply().ok()?.atom;
                let owner = connection.get_selection_owner(selection).ok()?.reply().ok()?.owner;
                Some(owner != 0)
            })().unwrap_or(false)
        };
        let surface = gpu.instance.create_surface(window).map_err(|e| e.to_string())?;
        let mut config = surface.get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1)).ok_or("the surface has no configuration")?;
        config.format = FORMAT;
        config.alpha_mode = wgpu::CompositeAlphaMode::Auto;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        #[cfg(target_os = "linux")]
        if composited_x11 && surface.get_capabilities(&gpu.adapter).present_modes.contains(&wgpu::PresentMode::Immediate) {
            // The X11 compositor synchronizes display; pace drawing without blocking input on vblank.
            config.present_mode = wgpu::PresentMode::Immediate;
        }
        surface.configure(d, &config);
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("glass"), source: wgpu::ShaderSource::Wgsl(include_str!("glass.wgsl").into()) });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let bind = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex(1),
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                tex(3),
            ],
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("glass"), bind_group_layouts: &[Some(&bind)], immediate_size: 0 });
        let premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        };
        let pipeline = |entry: &str, blend: Option<wgpu::BlendState>, format: wgpu::TextureFormat| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glass"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Draw {
            surface,
            config,
            copy: pipeline("fs_copy", None, FORMAT),
            over: pipeline("fs_copy", Some(premultiplied), FORMAT),
            down: pipeline("fs_down", None, PYRAMID_FORMAT),
            pill: pipeline("fs_pill", Some(premultiplied), FORMAT),
            fade: pipeline("fs_fade", Some(premultiplied), FORMAT),
            focus: pipeline("fs_focus", None, FORMAT),
            bind,
            sampler,
            groups: RefCell::new(Vec::new()),
            next_group: Cell::new(0),
            pyramid: None,
            page_blurred: false,
            stale: false,
        })
    }

    fn resize(&mut self, w: u32, h: u32) {
        self.config.width = w.max(1);
        self.config.height = h.max(1);
        self.pyramid = None;
        self.groups.borrow_mut().clear();
        self.stale = true;
    }

    /// Renders dirty layers, then composites the page, the fades, and the pills on top.
    fn frame(&mut self, s: &Shared, mut win: LogicalSize) {
        self.next_group.set(0);
        let gpu = s.gpu.clone();
        let d = &gpu.device;
        let scale = s.window.borrow().as_ref().map_or(1.0, |w| w.scale_factor() as f32);
        if std::mem::take(&mut self.stale) {
            self.surface.configure(d, &self.config);
        }
        // Acquire before Skia submits this frame so the fence wait only covers earlier work.
        let mut acquired = self.surface.get_current_texture();
        if matches!(acquired, wgpu::CurrentSurfaceTexture::Outdated) {
            // X11 can resize again while configuration waits for outstanding presentation.
            let size = s.window.borrow().as_ref().unwrap().inner_size();
            s.size.set(PhysicalSize::new(size.width, size.height));
            self.resize(size.width, size.height);
            layout_layers(s);
            win = LogicalSize::new(size.width as f32 / scale, size.height as f32 / scale);
            self.surface.configure(d, &self.config);
            self.stale = false;
            acquired = self.surface.get_current_texture();
        }
        let frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            // Also when the window was hidden before it was ever shown, so winit sends no Occluded.
            wgpu::CurrentSurfaceTexture::Occluded => {
                s.occluded.set(true);
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.stale = true;
                redraw(s);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                redraw(s);
                return;
            }
            _ => {
                self.surface.configure(d, &self.config);
                return;
            }
        };
        for (layer, role) in s.layers.borrow().iter() {
            let Some(role) = role.get().filter(|r| s.shown(*r)) else { continue };
            let size = layer.size.get();
            let fresh = layer.texture.borrow().as_ref().is_none_or(|(t, _)| t.width() != size.width || t.height() != size.height);
            if fresh {
                let t = texture(d, size.width, size.height, "layer");
                let v = t.create_view(&Default::default());
                *layer.texture.borrow_mut() = Some((t, v));
            }
            if fresh || layer.dirty.get() {
                layer.dirty.set(false);
                if role == Role::Page {
                    self.page_blurred = false;
                }
                if let Some((t, _)) = layer.texture.borrow().as_ref() {
                    prepare_layer(&gpu, t, fresh);
                    if let Err(e) = layer.renderer.render_to_texture(t) {
                        eprintln!("nori: a layer was not drawn: {e}");
                    }
                }
            }
        }
        let target = frame.texture.create_view(&Default::default());
        let layers = s.layers.borrow();
        let find = |r: Role| layers.iter().find(|(_, x)| x.get() == Some(r)).and_then(|(l, _)| l.texture.borrow().as_ref().map(|(_, v)| v.clone()));
        let Some(page) = find(Role::Page) else { return };
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let mut enc = d.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let focus = s.focus.borrow().as_ref().and_then(|f| f());
        let glass_needed = s.top_shown.get() || s.player_shown.get() || focus.is_some();
        if glass_needed {
            self.blur_page(d, &gpu.queue, &mut enc, &page);
        }
        let pyramid = self.pyramid.as_ref().map(|p| p.all.clone());
        let full = uniforms([0.0, 0.0, w, h], [w, h, w, h], [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]);
        let g = self.group(d, &gpu.queue, &full, &page, pyramid.as_ref().unwrap_or(&page));
        pass(&mut enc, &target, &self.copy, &g, true);
        if let (Some(f), Some(py)) = (focus, &pyramid) {
            let r = [f.region[0] * scale, f.region[1] * scale, f.region[2] * scale, f.region[3] * scale];
            let u = uniforms(r, [w, h, w, h], [0.0; 4], [0.0; 4], [f.band_top * scale, f.band_h * scale, f.pitch * scale, scale], [0.0; 4]);
            let g = self.group(d, &gpu.queue, &u, &page, py);
            pass(&mut enc, &target, &self.focus, &g, false);
        }
        let [red, green, blue] = s.page_colour.get();
        let right = s.right.get();
        for role in [Role::Top, Role::Player] {
            let (Some(view), Some(py), true) = (find(role), &pyramid, s.shown(role)) else { continue };
            let f = fade_of(role, win, right);
            let u = uniforms(f.strip.scaled(scale), [w, h, w, h], [0.0; 4], [red, green, blue, 0.0], [f.at, f.top as u8 as f32, 0.0, 0.0], [0.0; 4]);
            let g = self.group(d, &gpu.queue, &u, &page, py);
            pass(&mut enc, &target, &self.fade, &g, false);
            let rect = pill_rect(role, win, right).scaled(scale);
            let u = uniforms(rect, [w, h, w, h], [pill_radius(role) * scale, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, blur_level(scale)], [0.0; 4], [0.0; 4]);
            let g = self.group(d, &gpu.queue, &u, &page, py);
            pass(&mut enc, &target, &self.pill, &g, false);
            let u = uniforms(rect, [w, h, w, h], [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]);
            let g = self.group(d, &gpu.queue, &u, &view, &view);
            pass(&mut enc, &target, &self.over, &g, false);
        }
        drop(layers);
        gpu.queue.submit([enc.finish()]);
        gpu.queue.present(frame);
        self.groups.borrow_mut().truncate(self.next_group.get());
    }

    /// Draws the page's blur pyramid, each level a 13-tap downsample of the one above it, unless it
    /// already holds the page as last rendered.
    fn blur_page(&mut self, d: &wgpu::Device, q: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, page: &wgpu::TextureView) {
        let (w, h) = ((self.config.width / 2).max(1), (self.config.height / 2).max(1));
        let sized = self.pyramid.as_ref().is_some_and(|p| p.texture.width() == w && p.texture.height() == h);
        if sized && self.page_blurred {
            return;
        }
        if !sized {
            self.pyramid = Some(pyramid(d, w, h));
        }
        self.page_blurred = true;
        let Some(p) = &self.pyramid else { return };
        for (k, target) in p.mips.iter().enumerate() {
            let from = if k == 0 { page } else { &p.mips[k - 1] };
            let (tw, th) = ((w >> k).max(1) as f32, (h >> k).max(1) as f32);
            let u = uniforms([0.0, 0.0, tw, th], [tw, th, tw, th], [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]);
            let g = self.group(d, q, &u, from, from);
            pass(enc, target, &self.down, &g, false);
        }
    }

    fn group(&self, d: &wgpu::Device, q: &wgpu::Queue, u: &[u8], a: &wgpu::TextureView, b: &wgpu::TextureView) -> wgpu::BindGroup {
        let slot = self.next_group.get();
        self.next_group.set(slot + 1);
        let mut groups = self.groups.borrow_mut();
        if let Some(group) = groups.get(slot) {
            if &group.a == a && &group.b == b {
                q.write_buffer(&group.buffer, 0, u);
                return group.binding.clone();
            }
        }
        let buf = groups.get(slot).map(|group| group.buffer.clone()).unwrap_or_else(|| d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glass"), size: u.len() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        }));
        q.write_buffer(&buf, 0, u);
        let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glass"),
            layout: &self.bind,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(a) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(b) },
            ],
        });
        let entry = Group { buffer: buf, a: a.clone(), b: b.clone(), binding: group.clone() };
        if slot == groups.len() {
            groups.push(entry);
        } else {
            groups[slot] = entry;
        }
        group
    }
}

/// Skia writes outside wgpu's tracker; initialize first and restore its expected attachment state.
#[cfg(not(target_os = "linux"))]
fn prepare_layer(gpu: &Gpu, texture: &wgpu::Texture, fresh: bool) {
    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Slint layer attachment") });
    if fresh {
        let view = texture.create_view(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Slint layer init"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view, depth_slice: None, resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
    } else {
        encoder.transition_resources(std::iter::empty(), [wgpu::TextureTransition { texture, selector: None, state: wgpu::TextureUses::COLOR_TARGET }].into_iter());
    }
    gpu.queue.submit([encoder.finish()]);
}

/// A pyramid `w` x `h` at its largest, halving [`PYRAMID_LEVELS`] times or until a side is one pixel.
#[cfg(not(target_os = "linux"))]
fn pyramid(d: &wgpu::Device, w: u32, h: u32) -> Pyramid {
    let levels = PYRAMID_LEVELS.min(32 - w.max(h).leading_zeros());
    let texture = d.create_texture(&wgpu::TextureDescriptor {
        label: Some("pyramid"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PYRAMID_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let mips = (0..levels).map(|k| texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: k, mip_level_count: Some(1), ..Default::default() })).collect();
    let all = texture.create_view(&Default::default());
    Pyramid { texture, mips, all }
}

#[cfg(not(target_os = "linux"))]
fn texture(d: &wgpu::Device, w: u32, h: u32, label: &str) -> wgpu::Texture {
    d.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

#[cfg(not(target_os = "linux"))]
fn pass(enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup, clear: bool) {
    let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: if clear { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    p.set_pipeline(pipeline);
    p.set_bind_group(0, group, &[]);
    p.draw(0..4, 0..1);
}

/// Uniforms as glass.wgsl lays them out (six vec4s).
fn uniforms(rect: [f32; 4], view: [f32; 4], shape: [f32; 4], face: [f32; 4], light: [f32; 4], mut gather: [f32; 4]) -> [u8; 96] {
    gather[2] = PYRAMID_LEVELS as f32;
    let mut bytes = [0; 96];
    for (out, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip([rect, view, shape, face, light, gather].iter().flatten()) {
        out.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

#[cfg(target_os = "macos")]
fn reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(not(target_os = "macos"))]
fn reduce_motion() -> bool {
    false
}

/// Adds an empty unified toolbar so the traffic lights sit lower, level with the page's toolbar.
#[cfg(target_os = "macos")]
fn unified_toolbar(window: &WinitWindow) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSToolbar, NSView, NSWindowToolbarStyle};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: winit's AppKit handle is the content view, alive as long as the window.
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    let Some(ns) = view.window() else { return };
    let toolbar = NSToolbar::new(mtm);
    ns.setToolbar(Some(&toolbar));
    ns.setToolbarStyle(NSWindowToolbarStyle::Unified);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pills_centre_on_the_page_beside_the_panel_and_stop_at_their_width() {
        let wide = pill_rect(Role::Player, LogicalSize::new(1600.0, 900.0), 0.0);
        assert_eq!((wide.x, wide.w, wide.h), ((1600.0 - PILL_MAX_W) / 2.0, PILL_MAX_W, PLAYER_H));
        assert_eq!(wide.y + wide.h + PLAYER_BOTTOM, 900.0);
        let narrow = pill_rect(Role::Player, LogicalSize::new(900.0, 600.0), 280.0);
        assert_eq!((narrow.x, narrow.w), (SIDE, 620.0 - 2.0 * SIDE));
        let top = pill_rect(Role::Top, LogicalSize::new(1280.0, 800.0), 0.0);
        assert_eq!((top.x + top.w / 2.0, top.y, top.h), (640.0, TOP_Y, TOP_H));
        assert!(top.x >= TRAFFIC, "clear of the traffic lights");
        let tight = pill_rect(Role::Top, LogicalSize::new(860.0, 600.0), 0.0);
        assert!(tight.x >= TRAFFIC && tight.x + tight.w <= 860.0 - TRAFFIC);
    }

    #[test]
    fn a_fade_runs_from_the_windows_edge_to_a_reach_past_the_pill() {
        let window = LogicalSize::new(1280.0, 800.0);
        let bottom = fade_of(Role::Player, window, 0.0);
        let pill = pill_rect(Role::Player, window, 0.0);
        assert_eq!(bottom.strip.y + bottom.strip.h, 800.0);
        assert!(!bottom.top);
        // The pill's outer side is `at` of the way up the strip's height from where the ramp begins.
        let from_top_of_strip = pill.y - bottom.strip.y;
        assert!((from_top_of_strip - (EDGE_REACH + FADE_PAD)).abs() < 1e-3, "{from_top_of_strip}");
        assert!((bottom.at - EDGE_REACH / bottom.strip.h).abs() < 1e-6);
        let top = fade_of(Role::Top, window, 0.0);
        assert!(top.top && top.strip.y == 0.0);
    }

    #[test]
    fn the_blur_level_follows_the_screens_density() {
        let (one, two) = (blur_level(1.0), blur_level(2.0));
        assert!((two - one - 1.0).abs() < 1e-5, "twice the pixels, one level more");
        assert!(two < PYRAMID_LEVELS as f32, "the pyramid is deep enough");
    }

    #[test]
    fn redraw_bursts_wait_for_the_next_frame() {
        let now = Instant::now();
        let interval = Duration::from_millis(16);
        let mut frames = FrameSchedule { next: now, interval, pending: false };
        assert!(frames.begin(now));
        assert_eq!(frames.deadline(), None);
        for ms in [1, 4, 8, 15] {
            assert!(!frames.request(now + Duration::from_millis(ms)));
            assert!(!frames.begin(now + Duration::from_millis(ms)));
            assert_eq!(frames.deadline(), Some(now + interval));
        }
        assert!(frames.begin(now + interval));
        assert_eq!(frames.deadline(), None);
        assert!(frames.request(now + interval * 10));
        assert!(frames.begin(now + interval * 10));
        assert_eq!(frames.deadline(), None);
    }

    #[test]
    fn wheel_scroll_moves_over_time() {
        slint::slint! {
            export component ScrollWindow inherits Window {
                width: 100px; height: 100px;
                out property <length> scroll-y: flick.content-y;
                flick := Flickable { content-height: 1000px; Rectangle { height: 1000px; } }
            }
        }
        i_slint_backend_testing::init_integration_test_with_mock_time();
        use slint::ComponentHandle;
        let ui = ScrollWindow::new().unwrap();
        ui.show().unwrap();
        let event = wheel_event(LogicalPosition::new(50.0, 50.0), 0.0, -60.0, winit::event::TouchPhase::Moved);
        ui.window().dispatch_event(event);
        assert!(ui.get_scroll_y() > -60.0, "a wheel notch starts an animation instead of jumping");
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(60));
        slint::platform::update_timers_and_animations();
        assert!(ui.get_scroll_y() < 0.0 && ui.get_scroll_y() > -60.0);
        ui.window().dispatch_event(wheel_event(LogicalPosition::new(50.0, 50.0), 0.0, -60.0, winit::event::TouchPhase::Moved));
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        assert!((ui.get_scroll_y() + 120.0).abs() < 0.1, "successive notches keep their full distance");
    }

    slint::slint! {
        export component CoverWindow inherits Window {
            width: 256px; height: 256px;
            in-out property <int> revision;
            pure callback cover(int, int) -> image;
            for tile in 16: Rectangle {
                x: mod(tile, 4) * 64px; y: floor(tile / 4) * 64px;
                width: 64px; height: 64px;
                border-radius: 6px; clip: true;
                Image {
                    width: 64px; height: 64px;
                    source: root.cover(tile, root.revision);
                }
                Text { x: 0px; y: 48px; height: 16px; text: "Cover"; color: white; }
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn cover_pixels_survive_async_layers_and_mosaics() {
        struct TestPlatform { gpu: Rc<Gpu>, layers: Rc<RefCell<Vec<Rc<Layer>>>> }
        impl slint::platform::Platform for TestPlatform {
            fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
                let gpu = &self.gpu;
                let renderer = SkiaWGPU30Renderer::new(gpu.instance.clone(), gpu.adapter.clone(), gpu.device.clone(), gpu.queue.clone())?;
                renderer.set_resource_cache_limit(16 * 1024 * 1024);
                let layer = Rc::new_cyclic(|me: &Weak<Layer>| Layer {
                    window: slint::Window::new(me.clone() as Weak<dyn WindowAdapter>), renderer,
                    size: Cell::new(PhysicalSize::new(256, 256)), dirty: Cell::new(true),
                    texture: RefCell::new(None), shared: Weak::new(),
                });
                self.layers.borrow_mut().push(layer.clone());
                Ok(layer)
            }
        }
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.flags.remove(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::LowPower, ..Default::default() })).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { memory_hints: wgpu::MemoryHints::MemoryUsage, ..Default::default() })).unwrap();
        let gpu = Rc::new(Gpu { instance, adapter, device, queue });
        let layers = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(TestPlatform { gpu: gpu.clone(), layers: layers.clone() })).unwrap();
        use slint::ComponentHandle;
        let cover = gpu.picture(&[27, 91, 173, 255].repeat(256 * 256), 256, 256);
        let windows: Vec<_> = (0..3).map(|_| {
            let ui = CoverWindow::new().unwrap();
            let (gpu, cover) = (gpu.clone(), cover.clone());
            ui.on_cover(move |tile, _| if tile % 2 == 0 { cover.clone() } else { gpu.mosaic(&vec![cover.clone(); 4]).unwrap() });
            ui.show().unwrap();
            ui
        }).collect();
        let targets: Vec<_> = (0..3).map(|_| gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cover layer"), size: wgpu::Extent3d { width: 256, height: 256, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
        })).collect();
        let frame_bytes = 256 * 1024;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("async cover frames"), size: frame_bytes * 3 * 32,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
        });
        for frame in 0..32 {
            for (index, ui) in windows.iter().enumerate() {
                ui.set_revision(frame);
                let target = &targets[index];
                prepare_layer(&gpu, target, frame == 0);
                layers.borrow()[index].renderer.render_to_texture(target).unwrap();
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(target.as_image_copy(), wgpu::TexelCopyBufferInfo {
                    buffer: &buffer, layout: wgpu::TexelCopyBufferLayout {
                        offset: (frame as u64 * 3 + index as u64) * frame_bytes,
                        bytes_per_row: Some(1024), rows_per_image: Some(256),
                    },
                }, target.size());
                gpu.queue.submit([encoder.finish()]);
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let pixels = buffer.slice(..).get_mapped_range().unwrap();
        let pixels = pixels.as_chunks::<4>().0;
        for frame in 0..32 {
            for layer in 0..3 {
                for tile in 0..16 {
                    for y in 8..40 {
                        for x in 8..56 {
                            let index = (frame * 3 + layer) * 256 * 256 + (tile / 4 * 64 + y) * 256 + tile % 4 * 64 + x;
                            assert_eq!(pixels[index], [173, 91, 27, 255], "frame {frame}, layer {layer}, cover {tile} at {x},{y}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn layer_pixels_survive_first_sampling_and_redraw() {
        slint::slint! {
            export component TestWindow inherits Window {
                in-out property <color> colour;
                in-out property <image> picture;
                background: colour;
                Image { x: 0px; y: 0px; width: 32px; height: 32px; source: root.picture; }
            }
        }
        struct TestPlatform(Rc<Layer>);
        impl slint::platform::Platform for TestPlatform {
            fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
                Ok(self.0.clone())
            }
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let gpu = Gpu { instance, adapter, device, queue };
        let renderer = SkiaWGPU30Renderer::new(gpu.instance.clone(), gpu.adapter.clone(), gpu.device.clone(), gpu.queue.clone()).unwrap();
        let layer = Rc::new_cyclic(|me: &Weak<Layer>| Layer {
            window: slint::Window::new(me.clone() as Weak<dyn WindowAdapter>), renderer,
            size: Cell::new(PhysicalSize::new(64, 64)), dirty: Cell::new(true),
            texture: RefCell::new(None), shared: Weak::new(),
        });
        slint::platform::set_platform(Box::new(TestPlatform(layer.clone()))).unwrap();
        use slint::ComponentHandle;
        let ui = TestWindow::new().unwrap();
        ui.show().unwrap();
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test layer"), size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
        });
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer pixels"), size: 64 * 256, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
        });
        let cover = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test cover"), size: wgpu::Extent3d { width: 32, height: 32, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        gpu.queue.write_texture(cover.as_image_copy(), &[27, 91, 173, 255].repeat(32 * 32),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(128), rows_per_image: Some(32) }, cover.size());
        ui.set_picture(slint::Image::try_from(cover).unwrap());
        for frame in 0..64 {
            let fresh = frame == 0;
            let (colour, expected) = if frame % 2 == 0 {
                (slint::Color::from_rgb_u8(18, 52, 86), [86, 52, 18, 255])
            } else {
                (slint::Color::from_rgb_u8(171, 205, 239), [239, 205, 171, 255])
            };
            ui.set_colour(colour);
            prepare_layer(&gpu, &target, fresh);
            layer.renderer.render_to_texture(&target).unwrap();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(target.as_image_copy(), wgpu::TexelCopyBufferInfo {
                buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(64) },
            }, target.size());
            gpu.queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            {
                let pixels = buffer.slice(..).get_mapped_range().unwrap();
                for (index, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
                    let expected = if index % 64 < 32 && index / 64 < 32 { [173, 91, 27, 255] } else { expected };
                    assert_eq!(*pixel, expected, "cover and layer pixels on frame {frame}");
                }
            }
            buffer.unmap();
        }
    }
}
