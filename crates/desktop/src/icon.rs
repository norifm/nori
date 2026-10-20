//! nori's icon for the program when it does not run from its bundle (`cargo run`): the Dock's and the
//! About panel's on macOS, the window's elsewhere (Wayland takes it from the .desktop file only).

/// The icon at full size, the macOS one (rounded square on its padding).
pub const PNG: &[u8] = include_bytes!("../../../docs/brand/nori-macos.png");

/// The side of [`rgba`], pixels.
pub const SIDE: u32 = 256;

/// The icon as straight RGBA rows, [`SIDE`] across.
pub fn rgba() -> Option<Vec<u8>> {
    nori_covers::decode::Decoder::new().decode(PNG, SIDE as usize, SIDE as usize, nori_covers::Alpha::Straight).ok()
}

/// Shows the icon in the Dock and the app switcher, as the bundle's would. Main thread.
#[cfg(target_os = "macos")]
pub fn dock() {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    let Some(mtm) = MainThreadMarker::new() else { return };
    let data = NSData::with_bytes(PNG);
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        // SAFETY: on the main thread, with an image that lives as long as the app keeps it.
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_icon_decodes_at_its_size() {
        let px = super::rgba().unwrap();
        assert_eq!(px.len(), (super::SIDE * super::SIDE * 4) as usize);
        assert!(px.chunks(4).any(|p| p[3] == 0) && px.chunks(4).any(|p| p[3] == 255), "the padding stays clear around the square");
    }
}
