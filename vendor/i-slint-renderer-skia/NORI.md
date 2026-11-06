Based on i-slint-renderer-skia 1.18.1 from crates.io. Upstream licenses are in LICENSES.

Changes marked NORI expose owned Skia image conversion, the wgpu 30 resource-cache budget,
and rendering offscreen layers through a shared OpenGL context. OpenGL context access supports
owned cover uploads and GPU mosaics. Custom presentation skips unused native buffer-history queries; swap interval control lets the desktop pace composited X11
frames without a second vblank wait. These avoid duplicate raster buffers and bound retained GPU
resources without borrowed Vulkan cover images.

A drawn text reads its text, font, wrap, overflow, alignment and markdown flag itself, so partial
rendering repaints it when they change even after the text layout cache replaced its entry (an item
measured in another wrap mode): without that, a retained layer kept an old title on screen.

A wgpu 30 renderer can be made retained, as the OpenGL layers are: drawing into a texture that still
holds its last frame repaints only what changed (`SkiaWGPU30Renderer::retained`,
`render_to_texture_kept`), so a seek bar step repaints the bar, not the whole player.

This crate is a workspace member so Bazel resolves its local source without absolute path repositories.
Two style lints allow the existing upstream rendering callback signatures.
