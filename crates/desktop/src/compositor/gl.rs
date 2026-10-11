//! Linux presentation and glass use one OpenGL context shared with all Slint layers.

use super::*;
use glow::HasContext;
use i_slint_renderer_skia::{
    opengl_surface::OpenGLSurface, SkiaRenderer, SkiaSharedContext, Surface,
};

pub(super) fn window_attributes(
    display: &impl winit::raw_window_handle::HasDisplayHandle,
    attributes: WindowAttributes,
) -> Result<WindowAttributes, String> {
    use glutin::{config::GlConfig, display::GlDisplay, platform::x11::X11GlConfigExt};
    use winit::{platform::x11::WindowAttributesExtX11, raw_window_handle::RawDisplayHandle};
    let handle = display.display_handle().map_err(|e| e.to_string())?;
    let attributes = attributes.with_transparent(false);
    if !matches!(
        handle.as_raw(),
        RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_)
    ) {
        return Ok(attributes);
    }
    // The X11 visual must come from EGL, rather than the first ARGB visual offered by X11.
    let display = unsafe {
        glutin::display::Display::new(handle.as_raw(), glutin::display::DisplayApiPreference::Egl)
    }
    .map_err(|e| e.to_string())?;
    let template = glutin::config::ConfigTemplateBuilder::new()
        .with_alpha_size(0)
        .with_transparency(false)
        .build();
    let config = unsafe { display.find_configs(template) }
        .map_err(|e| e.to_string())?
        .filter(|config| !config.supports_transparency().unwrap_or(false))
        .min_by_key(|config| config.num_samples())
        .ok_or("no EGL window visual")?;
    let visual = config.x11_visual().ok_or("no X11 visual for EGL")?;
    Ok(attributes.with_x11_visual(visual.visual_id() as u32))
}

pub(super) type Renderer = SkiaRenderer;
pub(super) type LayerTexture = Texture;

pub(super) struct Gpu {
    shared: SkiaSharedContext,
    native: RefCell<Option<Rc<Native>>>,
    mosaics: RefCell<std::collections::HashMap<[u32; 4], Mosaic>>,
    ready: RefCell<Option<Box<dyn Fn()>>>,
}

enum Mosaic {
    Pending(Vec<skia_safe::Image>),
    Ready(slint::Image),
}

struct Native {
    surface: OpenGLSurface,
    gl: glow::Context,
    // The drawable lives until the last layer texture releases its context.
    _window: Arc<WinitWindow>,
}

impl Gpu {
    pub(super) fn new() -> Self {
        Self {
            shared: SkiaSharedContext::default(),
            native: RefCell::new(None),
            mosaics: RefCell::new(Default::default()),
            ready: RefCell::new(None),
        }
    }
    pub(super) fn renderer(&self) -> Renderer {
        SkiaRenderer::retained_opengl(&self.shared)
    }

    pub(super) fn picture(&self, pixels: &[u8], width: u32, height: u32) -> slint::Image {
        let info = skia_safe::ImageInfo::new(
            (width as i32, height as i32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            skia_safe::ColorSpace::new_srgb(),
        );
        let raster = skia_safe::images::raster_from_data(
            &info,
            skia_safe::Data::new_copy(pixels),
            width as usize * 4,
        )
        .expect("decoded cover pixels");
        let mut image = None;
        if let Some(native) = self.native.borrow().as_ref() {
            let _ = native.surface.with_context(&mut |context| {
                context.reset(None);
                image = raster.new_texture_image(context, skia_safe::gpu::Mipmapped::No);
            });
        }
        i_slint_renderer_skia::image_from_skia(image.unwrap_or(raster))
    }

    pub(super) fn on_mosaics_ready(&self, callback: impl Fn() + 'static) {
        *self.ready.borrow_mut() = Some(Box::new(callback));
    }

    pub(super) fn mosaic(&self, images: &[slint::Image]) -> Option<slint::Image> {
        if images.len() != 4 {
            return None;
        }
        let binding = self.native.borrow();
        let Some(native) = binding.as_ref() else {
            return crate::app::mosaic(images);
        };
        let images: Vec<_> = images
            .iter()
            .map(i_slint_renderer_skia::image_to_skia)
            .collect::<Option<_>>()?;
        let keys = std::array::from_fn(|i| images[i].unique_id());
        let mut mosaics = self.mosaics.borrow_mut();
        match mosaics.entry(keys) {
            std::collections::hash_map::Entry::Occupied(entry) => match entry.get() {
                Mosaic::Ready(_) => match entry.remove() {
                    Mosaic::Ready(image) => Some(image),
                    _ => unreachable!(),
                },
                Mosaic::Pending(_) => None,
            },
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Mosaic::Pending(images));
                native._window.request_redraw();
                None
            }
        }
    }

    fn prepare_mosaics(&self) -> Result<(), String> {
        let binding = self.native.borrow();
        let Some(native) = binding.as_ref() else {
            return Ok(());
        };
        let mut mosaics = self.mosaics.borrow_mut();
        if mosaics.is_empty() {
            return Ok(());
        }
        let mut ready = false;
        let mut failed = false;
        native
            .surface
            .with_context(&mut |context| {
                context.reset(None);
                for mosaic in mosaics.values_mut() {
                    let Mosaic::Pending(images) = mosaic else {
                        continue;
                    };
                    let side = images[0].width();
                    let info = skia_safe::ImageInfo::new(
                        (side * 2, side * 2),
                        skia_safe::ColorType::RGBA8888,
                        skia_safe::AlphaType::Premul,
                        skia_safe::ColorSpace::new_srgb(),
                    );
                    let Some(mut surface) = skia_safe::gpu::surfaces::render_target(
                        context,
                        skia_safe::gpu::Budgeted::Yes,
                        &info,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ) else {
                        failed = true;
                        continue;
                    };
                    surface.canvas().clear(skia_safe::Color::TRANSPARENT);
                    for (i, image) in images.iter().enumerate() {
                        let crop = image.width().min(image.height()) as f32;
                        let source = skia_safe::Rect::from_xywh(
                            (image.width() as f32 - crop) / 2.0,
                            (image.height() as f32 - crop) / 2.0,
                            crop,
                            crop,
                        );
                        let target = skia_safe::Rect::from_xywh(
                            (i % 2) as f32 * side as f32,
                            (i / 2) as f32 * side as f32,
                            side as f32,
                            side as f32,
                        );
                        surface.canvas().draw_image_rect(
                            image,
                            Some((&source, skia_safe::canvas::SrcRectConstraint::Strict)),
                            target,
                            &skia_safe::Paint::default(),
                        );
                    }
                    context.flush_and_submit_surface(&mut surface, None);
                    *mosaic = Mosaic::Ready(i_slint_renderer_skia::image_from_skia(
                        surface.image_snapshot(),
                    ));
                    ready = true;
                }
            })
            .map_err(|e| e.to_string())?;
        drop(mosaics);
        if failed {
            return Err("the GPU mosaic cannot be allocated".into());
        }
        if ready {
            if let Some(callback) = self.ready.borrow().as_ref() {
                callback();
            }
        }
        Ok(())
    }
}

pub(super) struct Texture {
    surface: Option<skia_safe::Surface>,
    framebuffer: glow::NativeFramebuffer,
    texture: glow::NativeTexture,
    stencil: glow::NativeRenderbuffer,
    size: PhysicalSize,
    native: Rc<Native>,
}

impl Texture {
    fn new(
        native: Rc<Native>,
        context: &mut skia_safe::gpu::DirectContext,
        size: PhysicalSize,
    ) -> Result<Self, String> {
        let gl = &native.gl;
        // The context is current throughout the native surface's rendering callback.
        let (texture, framebuffer, stencil) = unsafe {
            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_storage_2d(
                glow::TEXTURE_2D,
                1,
                glow::RGBA8,
                size.width as i32,
                size.height as i32,
            );
            sampling(gl);
            let framebuffer = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            let stencil = gl.create_renderbuffer()?;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(stencil));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::STENCIL_INDEX8,
                size.width as i32,
                size.height as i32,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::STENCIL_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(stencil),
            );
            gl.disable(glow::SCISSOR_TEST);
            gl.color_mask(true, true, true, true);
            gl.stencil_mask(0xff);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear_stencil(0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::STENCIL_BUFFER_BIT);
            (texture, framebuffer, stencil)
        };
        context.reset(None);
        let target = skia_safe::gpu::backend_render_targets::make_gl(
            (size.width as i32, size.height as i32),
            Some(0),
            8,
            skia_safe::gpu::gl::FramebufferInfo {
                fboid: framebuffer.0.get(),
                format: glow::RGBA8,
                ..Default::default()
            },
        );
        let surface = skia_safe::gpu::surfaces::wrap_backend_render_target(
            context,
            &target,
            skia_safe::gpu::SurfaceOrigin::TopLeft,
            skia_safe::ColorType::RGBA8888,
            skia_safe::ColorSpace::new_srgb(),
            None,
        )
        .ok_or("the OpenGL layer cannot be wrapped by Skia")?;
        Ok(Self {
            surface: Some(surface),
            framebuffer,
            texture,
            stencil,
            size,
            native,
        })
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        let _ = self.native.surface.with_active_surface(&mut || {
            self.surface.take();
            // These attachments belong to this layer and its retained current context.
            unsafe {
                self.native.gl.delete_framebuffer(self.framebuffer);
                self.native.gl.delete_renderbuffer(self.stencil);
                self.native.gl.delete_texture(self.texture);
            }
        });
    }
}

struct Program(glow::NativeProgram);

pub(super) struct Draw {
    native: Rc<Native>,
    size: PhysicalSize,
    copy: Program,
    opaque: Program,
    down: Program,
    glass: Program,
    focus: Program,
    buffer: glow::NativeBuffer,
    vertices: glow::NativeVertexArray,
    pyramid: Option<Pyramid>,
    page_blurred: bool,
}

struct Pyramid {
    texture: glow::NativeTexture,
    levels: Vec<glow::NativeTexture>,
    framebuffer: glow::NativeFramebuffer,
    size: PhysicalSize,
    native: Rc<Native>,
}

impl Drop for Pyramid {
    fn drop(&mut self) {
        let _ = self.native.surface.with_active_surface(&mut || unsafe {
            let gl = &self.native.gl;
            gl.delete_framebuffer(self.framebuffer);
            for texture in &self.levels {
                gl.delete_texture(*texture);
            }
            gl.delete_texture(self.texture);
        });
    }
}

impl Pyramid {
    fn new(native: Rc<Native>, size: PhysicalSize) -> Result<Self, String> {
        let count = PYRAMID_LEVELS.min(32 - size.width.max(size.height).leading_zeros());
        let gl = &native.gl;
        let (texture, framebuffer, levels) = unsafe {
            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_storage_2d(
                glow::TEXTURE_2D,
                count as i32,
                glow::RGBA16F,
                size.width as i32,
                size.height as i32,
            );
            sampling(gl);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR_MIPMAP_NEAREST as i32,
            );
            let mut levels = Vec::new();
            for level in 0..count {
                let texture = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_storage_2d(
                    glow::TEXTURE_2D,
                    1,
                    glow::RGBA16F,
                    (size.width >> level).max(1) as i32,
                    (size.height >> level).max(1) as i32,
                );
                sampling(gl);
                levels.push(texture);
            }
            (texture, gl.create_framebuffer()?, levels)
        };
        Ok(Self {
            texture,
            framebuffer,
            levels,
            size,
            native,
        })
    }
}

impl Drop for Draw {
    fn drop(&mut self) {
        let _ = self.native.surface.with_active_surface(&mut || unsafe {
            let gl = &self.native.gl;
            for program in [
                &self.copy,
                &self.opaque,
                &self.down,
                &self.glass,
                &self.focus,
            ] {
                gl.delete_program(program.0);
            }
            gl.delete_buffer(self.buffer);
            gl.delete_vertex_array(self.vertices);
        });
    }
}

impl Draw {
    pub(super) fn new(gpu: &Gpu, window: Arc<WinitWindow>) -> Result<Self, String> {
        use winit::raw_window_handle::HasWindowHandle;
        let size = window.inner_size();
        let size = PhysicalSize::new(size.width.max(1), size.height.max(1));
        let surface = OpenGLSurface::new_with_config(
            window.clone(),
            window.clone(),
            size,
            Some(i_slint_core::graphics::RequestedOpenGLVersion::OpenGL(
                Some((4, 3)),
            )),
            glutin::config::ConfigTemplateBuilder::new()
                .with_alpha_size(0)
                .compatible_with_native_window(
                    window.window_handle().map_err(|e| e.to_string())?.as_raw(),
                ),
            None,
        )
        .map_err(|e| e.to_string())?;
        let immediate = composited_x11(&window);
        if immediate {
            surface
                .set_swap_interval(false)
                .map_err(|e| e.to_string())?;
        }
        let mut gl = None;
        surface.with_graphics_api(&mut |api| {
            if let slint::GraphicsAPI::NativeOpenGL { get_proc_address } = api {
                // The loader and context are supplied by the retained native surface.
                gl = Some(unsafe { glow::Context::from_loader_function_cstr(get_proc_address) });
            }
        });
        let native = Rc::new(Native {
            surface,
            gl: gl.ok_or("no OpenGL function loader")?,
            _window: window,
        });
        let gl = &native.gl;
        let shader = |desktop, embedded| {
            if gl.version().is_embedded {
                embedded
            } else {
                desktop
            }
        };
        let vertex = shader(
            include_str!(concat!(env!("OUT_DIR"), "/vs.glsl")),
            include_str!(concat!(env!("OUT_DIR"), "/vs_es.glsl")),
        );
        let copy = program(
            gl,
            vertex,
            shader(
                include_str!(concat!(env!("OUT_DIR"), "/fs_copy.glsl")),
                include_str!(concat!(env!("OUT_DIR"), "/fs_copy_es.glsl")),
            ),
            include!(concat!(env!("OUT_DIR"), "/fs_copy.bindings")),
        )?;
        let opaque = program(
            gl,
            vertex,
            shader(
                include_str!(concat!(env!("OUT_DIR"), "/fs_opaque.glsl")),
                include_str!(concat!(env!("OUT_DIR"), "/fs_opaque_es.glsl")),
            ),
            include!(concat!(env!("OUT_DIR"), "/fs_opaque.bindings")),
        )?;
        let down = program(
            gl,
            shader(
                include_str!(concat!(env!("OUT_DIR"), "/vs_texture.glsl")),
                include_str!(concat!(env!("OUT_DIR"), "/vs_texture_es.glsl")),
            ),
            shader(
                include_str!(concat!(env!("OUT_DIR"), "/fs_down.glsl")),
                include_str!(concat!(env!("OUT_DIR"), "/fs_down_es.glsl")),
            ),
            include!(concat!(env!("OUT_DIR"), "/fs_down.bindings")),
        )?;
        let glass = program(
            gl,
            vertex,
            shader(
                include_str!(concat!(env!("OUT_DIR"), "/fs_glass.glsl")),
                include_str!(concat!(env!("OUT_DIR"), "/fs_glass_es.glsl")),
            ),
            include!(concat!(env!("OUT_DIR"), "/fs_glass.bindings")),
        )?;
        let focus = program(
            gl,
            vertex,
            shader(
                include_str!(concat!(env!("OUT_DIR"), "/fs_focus_opaque.glsl")),
                include_str!(concat!(env!("OUT_DIR"), "/fs_focus_opaque_es.glsl")),
            ),
            include!(concat!(env!("OUT_DIR"), "/fs_focus_opaque.bindings")),
        )?;
        let (buffer, vertices) = unsafe {
            let buffer = gl.create_buffer()?;
            gl.bind_buffer(glow::UNIFORM_BUFFER, Some(buffer));
            (buffer, gl.create_vertex_array()?)
        };
        native
            .surface
            .with_context(&mut |context| {
                context.set_resource_cache_limit(16 * 1024 * 1024);
            })
            .map_err(|e| e.to_string())?;
        *gpu.native.borrow_mut() = Some(native.clone());
        Ok(Self {
            native,
            size,
            copy,
            opaque,
            down,
            glass,
            focus,
            buffer,
            vertices,
            pyramid: None,
            page_blurred: false,
        })
    }

    pub(super) fn size(&self) -> PhysicalSize {
        self.size
    }
    pub(super) fn resize(&mut self, width: u32, height: u32) {
        self.size = PhysicalSize::new(width.max(1), height.max(1));
        self.pyramid = None;
        self.page_blurred = false;
    }

    pub(super) fn frame(&mut self, shared: &Shared, mut win: LogicalSize) -> Result<(), String> {
        shared.gpu.prepare_mosaics()?;
        let native = self.native.clone();
        let actual = native._window.inner_size();
        let actual = PhysicalSize::new(actual.width.max(1), actual.height.max(1));
        if self.size != actual {
            shared.size.set(actual);
            self.resize(actual.width, actual.height);
            layout_layers(shared);
            let scale = native._window.scale_factor() as f32;
            win = LogicalSize::new(actual.width as f32 / scale, actual.height as f32 / scale);
        }
        native
            .surface
            .resize_event(self.size)
            .map_err(|e| e.to_string())?;
        native
            .surface
            .present(&mut |context| {
                self.paint(shared, win, context)
                    .map_err(slint::PlatformError::from)?;
                native._window.pre_present_notify();
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        shared
            .gpu
            .mosaics
            .borrow_mut()
            .retain(|_, mosaic| matches!(mosaic, Mosaic::Pending(_)));
        Ok(())
    }

    fn paint(
        &mut self,
        shared: &Shared,
        win: LogicalSize,
        context: &mut skia_safe::gpu::DirectContext,
    ) -> Result<(), String> {
        let native = self.native.clone();
        let scale = native._window.scale_factor() as f32;
        context.reset(None);
        for (layer, role) in shared.layers.borrow().iter() {
            let Some(role) = role.get().filter(|r| shared.shown(*r)) else {
                continue;
            };
            let size = layer.size.get();
            let fresh = layer
                .texture
                .borrow()
                .as_ref()
                .is_none_or(|t| t.size != size);
            if fresh {
                *layer.texture.borrow_mut() = Some(Texture::new(native.clone(), context, size)?);
            }
            if fresh || layer.dirty.get() {
                layer.dirty.set(false);
                if role == Role::Page {
                    self.page_blurred = false;
                }
                let mut texture = layer.texture.borrow_mut();
                if let Some(texture) = texture.as_mut() {
                    context.reset(None);
                    layer
                        .renderer
                        .render_to_surface(texture.surface.as_mut().unwrap(), context, !fresh)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        let layers = shared.layers.borrow();
        let find = |role: Role| {
            layers
                .iter()
                .find(|(_, r)| r.get() == Some(role))
                .and_then(|(l, _)| l.texture.borrow().as_ref().map(|t| t.texture))
        };
        let Some(page) = find(Role::Page) else {
            return Ok(());
        };
        let (w, h) = (self.size.width as f32, self.size.height as f32);
        let focus = shared.focus.borrow().as_ref().and_then(|f| f());
        if shared.sidebar_shown.get() || shared.player_shown.get() || focus.is_some() {
            self.blur_page(page)?;
        }
        let pyramid = self.pyramid.as_ref().map_or(page, |p| p.texture);
        let dense = scale.max(1.0).log2();
        let full = uniforms(
            [0.0, 0.0, w, h],
            [w, h, w, h],
            [0.0; 4],
            [0.0; 4],
            [0.0; 4],
            [0.0; 4],
        );
        self.pass(
            (None, [w as i32, h as i32]),
            &self.opaque,
            &full,
            [page, pyramid],
            false,
            false,
        );
        if let Some(f) = focus {
            let rect = [
                f.region[0] * scale,
                f.region[1] * scale,
                f.region[2] * scale,
                f.region[3] * scale,
            ];
            let u = uniforms(
                rect,
                [w, h, w, h],
                [0.0; 4],
                [0.0; 4],
                [f.band_top * scale, f.band_h * scale, f.pitch * scale, scale],
                [0.0; 4],
            );
            self.pass(
                (None, [w as i32, h as i32]),
                &self.focus,
                &u,
                [page, pyramid],
                false,
                false,
            );
        }
        let glass = |role: Role, rect: [f32; 4], look: Glass| {
            let Some(texture) = find(role) else { return };
            let u = uniforms(
                rect,
                [w, h, w, h],
                look.shape,
                look.face,
                look.light,
                look.gather,
            );
            self.pass(
                (None, [w as i32, h as i32]),
                &self.glass,
                &u,
                [page, pyramid],
                false,
                true,
            );
            let (origin, size) = place(shared, role, win);
            let rect = [
                origin.x * scale,
                origin.y * scale,
                size.width * scale,
                size.height * scale,
            ];
            let u = uniforms(rect, [w, h, w, h], [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]);
            self.pass(
                (None, [w as i32, h as i32]),
                &self.copy,
                &u,
                [texture, texture],
                false,
                true,
            );
        };
        if shared.sidebar_shown.get() {
            let rect = [
                -8000.0 * scale,
                -8000.0 * scale,
                (SIDEBAR_W + 8000.0) * scale,
                h + 16000.0 * scale,
            ];
            glass(
                Role::Sidebar,
                rect,
                Glass {
                    shape: [0.0, 28.0 * scale, 8.0 * scale, 0.04],
                    face: [0.06, 0.22, 1.2, 4.5 + dense],
                    light: [0.05, 0.34, 45.0 * scale, 6.0 + dense],
                    gather: [180.0 * scale, 1.0, 0.0, 0.0],
                },
            );
        }
        if shared.player_shown.get() {
            let (origin, size) = place(shared, Role::Player, win);
            glass(
                Role::Player,
                [
                    origin.x * scale,
                    origin.y * scale,
                    size.width * scale,
                    size.height * scale,
                ],
                Glass {
                    shape: [PLAYER_H * 0.5 * scale, 14.0 * scale, 6.0 * scale, 0.05],
                    face: [0.05, 0.24, 1.5, 3.0 + dense],
                    light: [0.14, 0.0, 0.0, 0.0],
                    gather: [0.0; 4],
                },
            );
        }
        Ok(())
    }

    fn pass(
        &self,
        target: (Option<glow::NativeFramebuffer>, [i32; 2]),
        program: &Program,
        uniforms: &[u8],
        textures: [glow::NativeTexture; 2],
        clear: bool,
        blend: bool,
    ) {
        let gl = &self.native.gl;
        // Skia shares this context; establish every state the compositor consumes.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, target.0);
            gl.viewport(0, 0, target.1[0], target.1[1]);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::STENCIL_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            if !gl.version().is_embedded
                || gl
                    .supported_extensions()
                    .contains("GL_EXT_sRGB_write_control")
            {
                gl.disable(glow::FRAMEBUFFER_SRGB);
            }
            gl.color_mask(true, true, true, true);
            if clear {
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            if blend {
                gl.enable(glow::BLEND);
                gl.blend_equation(glow::FUNC_ADD);
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            } else {
                gl.disable(glow::BLEND);
            }
            gl.use_program(Some(program.0));
            gl.bind_vertex_array(Some(self.vertices));
            gl.bind_buffer(glow::UNIFORM_BUFFER, Some(self.buffer));
            // Rename storage so uploads cannot wait for a preceding pass to consume it.
            gl.buffer_data_u8_slice(glow::UNIFORM_BUFFER, uniforms, glow::STREAM_DRAW);
            gl.bind_buffer_base(glow::UNIFORM_BUFFER, 0, Some(self.buffer));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_sampler(0, None);
            gl.bind_texture(glow::TEXTURE_2D, Some(textures[0]));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_sampler(1, None);
            gl.bind_texture(glow::TEXTURE_2D, Some(textures[1]));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }

    fn blur_page(&mut self, page: glow::NativeTexture) -> Result<(), String> {
        let size = PhysicalSize::new((self.size.width / 2).max(1), (self.size.height / 2).max(1));
        if self.pyramid.as_ref().is_none_or(|p| p.size != size) {
            self.pyramid = Some(Pyramid::new(self.native.clone(), size)?);
        }
        if self.page_blurred {
            return Ok(());
        }
        let pyramid = self.pyramid.as_ref().unwrap();
        for (level, texture) in pyramid.levels.iter().enumerate() {
            let source = if level == 0 {
                page
            } else {
                pyramid.levels[level - 1]
            };
            let (w, h) = ((size.width >> level).max(1), (size.height >> level).max(1));
            let u = uniforms(
                [0.0, 0.0, w as f32, h as f32],
                [w as f32, h as f32, w as f32, h as f32],
                [0.0; 4],
                [0.0; 4],
                [0.0; 4],
                [0.0; 4],
            );
            unsafe {
                self.native
                    .gl
                    .bind_framebuffer(glow::FRAMEBUFFER, Some(pyramid.framebuffer));
                self.native.gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(*texture),
                    0,
                );
            }
            self.pass(
                (Some(pyramid.framebuffer), [w as i32, h as i32]),
                &self.down,
                &u,
                [source, source],
                false,
                false,
            );
            unsafe {
                self.native
                    .gl
                    .bind_texture(glow::TEXTURE_2D, Some(pyramid.texture));
                self.native.gl.copy_tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    level as i32,
                    0,
                    0,
                    0,
                    0,
                    w as i32,
                    h as i32,
                );
            }
        }
        self.page_blurred = true;
        Ok(())
    }
}

fn composited_x11(window: &WinitWindow) -> bool {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::protocol::xproto::ConnectionExt;
    if !window.window_handle().is_ok_and(|h| {
        matches!(
            h.as_raw(),
            RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)
        )
    }) {
        return false;
    }
    (|| {
        let (connection, screen) = x11rb::connect(None).ok()?;
        let atom = connection
            .intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes())
            .ok()?
            .reply()
            .ok()?
            .atom;
        let owner = connection
            .get_selection_owner(atom)
            .ok()?
            .reply()
            .ok()?
            .owner;
        Some(owner != 0)
    })()
    .unwrap_or(false)
}

fn program(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
    bindings: &[(&str, i32)],
) -> Result<Program, String> {
    // Shader objects and programs belong to the current native context.
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, vertex),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                gl.delete_program(program);
                return Err(log);
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(log);
        }
        gl.use_program(Some(program));
        for block in 0..gl.get_program_parameter_i32(program, glow::ACTIVE_UNIFORM_BLOCKS) as u32 {
            gl.uniform_block_binding(program, block, 0);
        }
        for (name, unit) in bindings {
            gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), *unit);
        }
        Ok(Program(program))
    }
}

unsafe fn sampling(gl: &glow::Context) {
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_WRAP_S,
        glow::CLAMP_TO_EDGE as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_WRAP_T,
        glow::CLAMP_TO_EDGE as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_MIN_FILTER,
        glow::LINEAR as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_MAG_FILTER,
        glow::LINEAR as i32,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestLayer {
        window: slint::Window,
        renderer: Renderer,
        size: Cell<PhysicalSize>,
    }

    impl WindowAdapter for TestLayer {
        fn window(&self) -> &slint::Window {
            &self.window
        }
        fn renderer(&self) -> &dyn slint::platform::Renderer {
            &self.renderer
        }
        fn size(&self) -> PhysicalSize {
            self.size.get()
        }
    }

    struct TestPlatform(Rc<TestLayer>);
    impl slint::platform::Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    #[ignore = "requires X11 and OpenGL; run with --ignored"]
    #[allow(deprecated)]
    fn layers_resize_without_losing_pixels_or_blur() {
        use winit::platform::x11::{EventLoopBuilderExtX11, WindowAttributesExtX11};
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt};
        let event_loop = EventLoop::builder()
            .with_x11()
            .with_any_thread(true)
            .build()
            .unwrap();
        let window = Arc::new(
            event_loop
                .create_window(
                    window_attributes(
                        &event_loop,
                        WindowAttributes::default()
                            .with_override_redirect(true)
                            .with_position(winit::dpi::PhysicalPosition::new(-4096, -4096))
                            .with_inner_size(winit::dpi::PhysicalSize::new(64, 64)),
                    )
                    .unwrap(),
                )
                .unwrap(),
        );
        let gpu = Gpu::new();
        let mut draw = Draw::new(&gpu, window.clone()).unwrap();
        let layer = Rc::new_cyclic(|me: &Weak<TestLayer>| TestLayer {
            window: slint::Window::new(me.clone() as Weak<dyn WindowAdapter>),
            renderer: gpu.renderer(),
            size: Cell::new(PhysicalSize::new(64, 64)),
        });
        slint::platform::set_platform(Box::new(TestPlatform(layer.clone()))).unwrap();
        slint::slint! {
            export component TestWindow inherits Window {
                in property <color> colour;
                in property <image> picture;
                in property <color> accent;
                Rectangle { background: root.colour; Image { width: parent.width; height: parent.height; source: root.picture; image-fit: fill; } }
                Rectangle { x: 4px; y: 4px; width: 8px; height: 8px; background: root.accent; }
            }
        }
        let ui = TestWindow::new().unwrap();
        ui.show().unwrap();
        let native = draw.native.clone();
        let (connection, _) = x11rb::connect(None).unwrap();
        let handle = match window.window_handle().unwrap().as_raw() {
            RawWindowHandle::Xlib(h) => h.window as u32,
            RawWindowHandle::Xcb(h) => h.window.get(),
            _ => unreachable!(),
        };
        for (width, height, colour) in [
            (64, 64, [18, 52, 86]),
            (96, 80, [171, 205, 239]),
            (72, 64, [86, 52, 18]),
        ] {
            connection
                .configure_window(
                    handle,
                    &ConfigureWindowAux::new().width(width).height(height),
                )
                .unwrap()
                .check()
                .unwrap();
            let size = PhysicalSize::new(width, height);
            layer.size.set(size);
            layer.window.dispatch_event(WindowEvent::Resized {
                size: LogicalSize::new(width as f32, height as f32),
            });
            ui.set_colour(slint::Color::from_rgb_u8(255, 0, 255));
            let mut source = vec![0; 8 * 8 * 4];
            for pixel in source.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&[colour[0], colour[1], colour[2], 255]);
            }
            let picture = gpu.picture(&source, 8, 8);
            assert!(
                i_slint_renderer_skia::image_to_skia(&picture)
                    .unwrap()
                    .is_texture_backed(),
                "covers must release their CPU raster"
            );
            source.fill(0);
            ui.set_picture(picture);
            native.surface.resize_event(size).unwrap();
            native
                .surface
                .present(&mut |context| {
                    let mut texture = Texture::new(native.clone(), context, size).unwrap();
                    context.reset(None);
                    layer
                        .renderer
                        .render_to_surface(texture.surface.as_mut().unwrap(), context, false)
                        .unwrap();
                    let u = uniforms(
                        [0.0, 0.0, width as f32, height as f32],
                        [width as f32, height as f32, 0.0, 0.0],
                        [0.0; 4],
                        [0.0; 4],
                        [0.0; 4],
                        [0.0; 4],
                    );
                    draw.pass(
                        (None, [width as i32, height as i32]),
                        &draw.opaque,
                        &u,
                        [texture.texture, texture.texture],
                        false,
                        false,
                    );
                    let mut pixels = vec![0; width as usize * height as usize * 4];
                    unsafe {
                        native.gl.read_pixels(
                            0,
                            0,
                            width as i32,
                            height as i32,
                            glow::RGBA,
                            glow::UNSIGNED_BYTE,
                            glow::PixelPackData::Slice(Some(&mut pixels)),
                        );
                    }
                    for pixel in pixels.as_chunks::<4>().0 {
                        assert_eq!(
                            *pixel,
                            [colour[0], colour[1], colour[2], 255],
                            "every pixel of the {width}x{height} resized layer must be current"
                        );
                    }
                    assert_eq!(unsafe { native.gl.get_error() }, glow::NO_ERROR);
                    Ok(())
                })
                .unwrap();
        }
        ui.set_picture(slint::Image::default());
        ui.set_colour(slint::Color::from_rgb_u8(18, 52, 86));
        ui.set_accent(slint::Color::from_rgb_u8(255, 0, 0));
        native
            .surface
            .with_context(&mut |context| {
                let mut texture = Texture::new(native.clone(), context, layer.size.get()).unwrap();
                layer
                    .renderer
                    .render_to_surface(texture.surface.as_mut().unwrap(), context, false)
                    .unwrap();
                ui.set_accent(slint::Color::from_rgb_u8(0, 0, 255));
                let damage = layer
                    .renderer
                    .render_to_surface(texture.surface.as_mut().unwrap(), context, true)
                    .unwrap();
                let area = damage
                    .expect("retained layers track repaint regions")
                    .iter()
                    .map(|rect| rect.area())
                    .sum::<f32>();
                assert!(
                    area < 72.0 * 64.0 / 4.0,
                    "changing an 8x8 control repainted {area} pixels"
                );
                let info = skia_safe::ImageInfo::new(
                    (72, 64),
                    skia_safe::ColorType::RGBA8888,
                    skia_safe::AlphaType::Unpremul,
                    None,
                );
                let mut pixels = vec![0; 72 * 64 * 4];
                assert!(texture.surface.as_mut().unwrap().read_pixels(
                    &info,
                    &mut pixels,
                    72 * 4,
                    (0, 0)
                ));
                for y in 0..64 {
                    for x in 0..72 {
                        let expected = if (4..12).contains(&x) && (4..12).contains(&y) {
                            [0, 0, 255, 255]
                        } else {
                            [18, 52, 86, 255]
                        };
                        assert_eq!(&pixels[(y * 72 + x) * 4..(y * 72 + x + 1) * 4], &expected);
                    }
                }
            })
            .unwrap();
        ui.set_accent(slint::Color::from_argb_u8(0, 0, 0, 0));
        let colours = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        let images: Vec<_> = colours
            .iter()
            .map(|colour| gpu.picture(&colour.repeat(8 * 8), 8, 8))
            .collect();
        assert!(gpu.mosaic(&images).is_none());
        gpu.prepare_mosaics().unwrap();
        let mosaic = gpu.mosaic(&images).unwrap();
        assert!(i_slint_renderer_skia::image_to_skia(&mosaic)
            .unwrap()
            .is_texture_backed());
        native
            .surface
            .with_context(&mut |context| {
                let image = i_slint_renderer_skia::image_to_skia(&mosaic).unwrap();
                let info = skia_safe::ImageInfo::new(
                    (16, 16),
                    skia_safe::ColorType::RGBA8888,
                    skia_safe::AlphaType::Unpremul,
                    skia_safe::ColorSpace::new_srgb(),
                );
                let mut pixels = vec![0; 16 * 16 * 4];
                assert!(image.read_pixels_with_context(
                    context,
                    &info,
                    &mut pixels,
                    16 * 4,
                    (0, 0),
                    skia_safe::image::CachingHint::Disallow
                ));
                for y in 0..16 {
                    for x in 0..16 {
                        assert_eq!(
                            &pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4],
                            colours[y / 8 * 2 + x / 8],
                            "GPU mosaic owns its quadrant pixels"
                        );
                    }
                }
            })
            .unwrap();
        drop(images);
        ui.set_picture(mosaic);
        let size = PhysicalSize::new(72, 64);
        native
            .surface
            .present(&mut |context| {
                let mut texture = Texture::new(native.clone(), context, size).unwrap();
                context.reset(None);
                layer
                    .renderer
                    .render_to_surface(texture.surface.as_mut().unwrap(), context, false)
                    .unwrap();
                let u = uniforms(
                    [0.0, 0.0, 72.0, 64.0],
                    [72.0, 64.0, 0.0, 0.0],
                    [0.0; 4],
                    [0.0; 4],
                    [0.0; 4],
                    [0.0; 4],
                );
                draw.pass(
                    (None, [72, 64]),
                    &draw.copy,
                    &u,
                    [texture.texture; 2],
                    true,
                    false,
                );
                let mut pixels = vec![0; 72 * 64 * 4];
                unsafe {
                    native.gl.read_pixels(
                        0,
                        0,
                        72,
                        64,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelPackData::Slice(Some(&mut pixels)),
                    );
                }
                for y in 0..64 {
                    for x in 0..72 {
                        let index = ((63 - y) / 32) * 2 + x / 36;
                        let pixel = &pixels[(y * 72 + x) * 4..(y * 72 + x + 1) * 4];
                        assert_eq!(pixel[3], 255, "the mosaic must have no transparent seams");
                        if x.abs_diff(36usize) > 4 && y.abs_diff(32usize) > 4 {
                            assert_eq!(
                                pixel, colours[index],
                                "each mosaic quadrant keeps its colour"
                            );
                        }
                    }
                }
                Ok(())
            })
            .unwrap();
        // Alternating pixels must blur into a continuous backdrop beneath the glass.
        let size = PhysicalSize::new(72, 64);
        for checker in [true, false] {
            let mut pixels =
                slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(size.width, size.height);
            for (i, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
                let value = if (i % size.width as usize + i / size.width as usize).is_multiple_of(2)
                {
                    0
                } else {
                    255
                };
                *pixel = if checker {
                    slint::Rgba8Pixel {
                        r: value,
                        g: value,
                        b: value,
                        a: 255,
                    }
                } else if i / (size.width as usize) < 32 {
                    slint::Rgba8Pixel {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    }
                } else {
                    slint::Rgba8Pixel {
                        r: 0,
                        g: 0,
                        b: 255,
                        a: 255,
                    }
                };
            }
            ui.set_picture(slint::Image::from_rgba8(pixels));
            let draw = RefCell::new(&mut draw);
            native
                .surface
                .present(&mut |context| {
                    let mut texture = Texture::new(native.clone(), context, size).unwrap();
                    context.reset(None);
                    layer
                        .renderer
                        .render_to_surface(texture.surface.as_mut().unwrap(), context, false)
                        .unwrap();
                    let mut draw = draw.borrow_mut();
                    draw.resize(size.width, size.height);
                    draw.blur_page(texture.texture).unwrap();
                    let u = uniforms(
                        [0.0, 0.0, 72.0, 64.0],
                        [72.0, 64.0, 72.0, 64.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 1.0, 1.0, if checker { 4.5 } else { 2.0 }],
                        [0.0; 4],
                        [0.0; 4],
                    );
                    draw.pass(
                        (None, [72, 64]),
                        &draw.glass,
                        &u,
                        [texture.texture, draw.pyramid.as_ref().unwrap().texture],
                        true,
                        false,
                    );
                    let mut result = [0; 8];
                    unsafe {
                        native.gl.read_pixels(
                            30,
                            30,
                            2,
                            1,
                            glow::RGBA,
                            glow::UNSIGNED_BYTE,
                            glow::PixelPackData::Slice(Some(&mut result)),
                        );
                    }
                    if checker {
                        assert!(
                            (100..155).contains(&result[0]),
                            "the glass must sample the blurred page: {result:?}"
                        );
                        assert!(
                            (result[0] as i32 - result[4] as i32).abs() <= 2,
                            "the checker must be smooth under glass: {result:?}"
                        );
                    } else {
                        for (y, red) in [(8, false), (56, true)] {
                            unsafe {
                                native.gl.read_pixels(
                                    30,
                                    y,
                                    1,
                                    1,
                                    glow::RGBA,
                                    glow::UNSIGNED_BYTE,
                                    glow::PixelPackData::Slice(Some(&mut result[..4])),
                                );
                            }
                            assert!(
                                if red {
                                    result[0] > 200 && result[2] < 50
                                } else {
                                    result[2] > 200 && result[0] < 50
                                },
                                "blur keeps the page orientation: {result:?}"
                            );
                        }
                    }
                    assert_eq!(unsafe { native.gl.get_error() }, glow::NO_ERROR);
                    Ok(())
                })
                .unwrap();
        }
    }
}
