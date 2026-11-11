//! The pairing scanner's door: a camera frame's luma plane, read for a QR code by nori-remote.

use jni::objects::{JByteBuffer, JClass};
use jni::sys::{jint, jstring};
use jni::JNIEnv;

use crate::{java_string, native, Class};

pub(crate) static CLASS: Class = Class {
    name: c"dev/nori/music/remote/QrScan",
    methods: &[native!(c"read", c"(Ljava/nio/ByteBuffer;III)Ljava/lang/String;", read)],
};

/// The text of the QR code in direct buffer `luma` (`width` × `height`, rows `stride` bytes apart); null
/// when it shows none.
extern "system" fn read(env: JNIEnv, _: JClass, luma: JByteBuffer, width: jint, height: jint, stride: jint) -> jstring {
    let (Ok(address), Ok(capacity)) = (env.get_direct_buffer_address(&luma), env.get_direct_buffer_capacity(&luma)) else { return std::ptr::null_mut() };
    if address.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: a direct buffer's memory stays valid for `capacity` bytes while the call holds the buffer.
    let bytes = unsafe { std::slice::from_raw_parts(address, capacity) };
    match nori_core::remote::qr_read(bytes, width.max(0) as usize, height.max(0) as usize, stride.max(0) as usize) {
        Some(text) => java_string(&env, &text),
        None => std::ptr::null_mut(),
    }
}
