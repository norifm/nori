//! What cpal does not say on macOS: how an output is connected.

use std::ffi::c_void;
use std::ptr::{self, NonNull};

use nori_engine::OutputKind;
use objc2_core_audio::{
    kAudioDevicePropertyDeviceNameCFString, kAudioDevicePropertyTransportType, kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, AudioObjectGetPropertyData,
    AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
};
use objc2_core_foundation::{CFRetained, CFString};

const fn code(c: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*c)
}

/// CoreAudio's `kAudioDeviceTransportType` values.
const BLUETOOTH: u32 = code(b"blue");
const BLUETOOTH_LE: u32 = code(b"blea");
const USB: u32 = code(b"usb ");

fn read<T>(object: AudioObjectID, selector: u32, value: *mut T) -> bool {
    let address = AudioObjectPropertyAddress { mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain };
    let mut size = size_of::<T>() as u32;
    // SAFETY: `value` holds a `T`, the size of the property read.
    unsafe { AudioObjectGetPropertyData(object, NonNull::from(&address), 0, ptr::null(), NonNull::from(&mut size), NonNull::new_unchecked(value.cast::<c_void>())) == 0 }
}

fn devices() -> Vec<AudioObjectID> {
    let address = AudioObjectPropertyAddress { mSelector: kAudioHardwarePropertyDevices, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain };
    let mut size = 0u32;
    // SAFETY: asks the size of the system object's list of devices.
    if unsafe { AudioObjectGetPropertyDataSize(kAudioObjectSystemObject as AudioObjectID, NonNull::from(&address), 0, ptr::null(), NonNull::from(&mut size)) } != 0 {
        return Vec::new();
    }
    let mut ids = vec![0 as AudioObjectID; size as usize / size_of::<AudioObjectID>()];
    let Some(at) = NonNull::new(ids.as_mut_ptr().cast::<c_void>()) else { return Vec::new() };
    // SAFETY: `ids` holds the `size` bytes asked for.
    if unsafe { AudioObjectGetPropertyData(kAudioObjectSystemObject as AudioObjectID, NonNull::from(&address), 0, ptr::null(), NonNull::from(&mut size), at) } != 0 {
        return Vec::new();
    }
    ids.truncate(size as usize / size_of::<AudioObjectID>());
    ids
}

fn name_of(id: AudioObjectID) -> Option<String> {
    let mut name: *mut CFString = ptr::null_mut();
    if !read(id, kAudioDevicePropertyDeviceNameCFString, &mut name) {
        return None;
    }
    // SAFETY: the property hands over a +1 reference, or none.
    let name = unsafe { CFRetained::from_raw(NonNull::new(name)?) };
    Some(name.to_string())
}

/// How the output called `name` is connected, when that makes it a USB or Bluetooth one.
pub fn kind_of(name: &str) -> Option<OutputKind> {
    let id = devices().into_iter().find(|id| name_of(*id).as_deref() == Some(name))?;
    let mut transport = 0u32;
    if !read(id, kAudioDevicePropertyTransportType, &mut transport) {
        return None;
    }
    match transport {
        BLUETOOTH | BLUETOOTH_LE => Some(OutputKind::Bluetooth),
        USB => Some(OutputKind::Usb),
        _ => None,
    }
}
