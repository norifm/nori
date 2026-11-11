package dev.nori.music.remote

/** Reads a camera frame for a QR code: nori-remote's `qr_read`, through crates/android's scan.rs. */
object QrScan {
    /** The text of the QR code in [luma], a direct buffer of [width] × [height] with rows [stride] bytes apart; null when it shows none. */
    @JvmStatic external fun read(luma: java.nio.ByteBuffer, width: Int, height: Int, stride: Int): String?
}
