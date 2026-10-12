package dev.nori.music.playback

import android.net.Uri
import androidx.media3.common.C
import androidx.media3.datasource.BaseDataSource
import androidx.media3.datasource.DataSpec
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

class RustBodyTest {
    @Test fun availableBytesReachTheDecoderBeforeTheNextPacket() {
        val source = Packets()
        val body = RustBody(source, 7) {}
        val first = body.read(body.buffer.size)
        assertEquals(3, first)
        assertArrayEquals(byteArrayOf(1, 2, 3), body.buffer.copyOf(first))
        source.nextReady = true
        val second = body.read(body.buffer.size)
        assertEquals(4, second)
        assertArrayEquals(byteArrayOf(4, 5, 6, 7), body.buffer.copyOf(second))
        assertEquals(-1, body.read(body.buffer.size))
    }

    private class Packets : BaseDataSource(false) {
        var nextReady = false
        private var reads = 0

        override fun open(dataSpec: DataSpec) = 7L
        override fun getUri(): Uri? = null
        override fun close() {}
        override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
            if (reads > 0 && !nextReady) throw AssertionError("the next network packet has not arrived")
            val packet = when (reads++) {
                0 -> byteArrayOf(1, 2, 3)
                1 -> byteArrayOf(4, 5, 6, 7)
                else -> return C.RESULT_END_OF_INPUT
            }
            require(length >= packet.size)
            packet.copyInto(buffer, offset)
            return packet.size
        }
    }
}
