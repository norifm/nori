package dev.nori.music.plugins

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothSocket
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat
import dev.nori.music.ffi.plugins.Bluetooth
import dev.nori.music.ffi.plugins.BtDevice
import dev.nori.music.ffi.plugins.BtFailure
import dev.nori.music.ffi.plugins.Plugins
import java.io.IOException
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

/**
 * Android's Bluetooth for plugins (the core's `Bluetooth`): devices connecting and going while the core
 * watches (a receiver registered only then), RFCOMM sockets and GATT connections numbered by the core's
 * links. Calls come on a plugin's own thread and may block; what a device sends goes to [plugins] from
 * the socket's reading thread or the GATT callback's. Without `BLUETOOTH_CONNECT` it watches nothing and
 * refuses every link.
 */
@SuppressLint("MissingPermission") // Every call that needs it checks granted() first.
class PluginBluetooth(private val context: Context, private val plugins: () -> Plugins) : Bluetooth {
    private val manager = context.getSystemService(BluetoothManager::class.java)
    private val links = ConcurrentHashMap<ULong, Link>()
    private var receiver: BroadcastReceiver? = null
    private val proxies = ConcurrentHashMap<Int, BluetoothProfile>()
    private var proxied = CountDownLatch(0)
    // Made at the first device event: a receiver's work leaves the main thread here.
    private val events: ExecutorService by lazy { Executors.newSingleThreadExecutor { r -> Thread(r, "nori-bluetooth") } }

    private sealed interface Link
    private class Rfcomm(val socket: BluetoothSocket) : Link

    fun granted(): Boolean = Build.VERSION.SDK_INT < 31 ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED

    private fun device(d: BluetoothDevice) = BtDevice(
        d.address,
        (if (Build.VERSION.SDK_INT >= 30) d.alias else null) ?: d.name.orEmpty(),
        d.uuids.orEmpty().map { it.uuid.toString().lowercase() },
        null, null,
    )

    @Synchronized
    override fun watch(on: Boolean) {
        if (!on) {
            receiver?.let { runCatching { context.unregisterReceiver(it) } }
            receiver = null
            proxies.forEach { (profile, proxy) -> manager?.adapter?.closeProfileProxy(profile, proxy) }
            proxies.clear()
            return
        }
        val adapter = manager?.adapter ?: return
        if (receiver != null || !granted()) return
        val r = object : BroadcastReceiver() {
            override fun onReceive(c: Context, intent: Intent) {
                val d: BluetoothDevice = (if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE, BluetoothDevice::class.java)
                else @Suppress("DEPRECATION") intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE)) ?: return
                val action = intent.action
                events.execute {
                    when (action) {
                        BluetoothDevice.ACTION_ACL_CONNECTED, BluetoothDevice.ACTION_UUID -> plugins().bluetoothConnected(device(d))
                        BluetoothDevice.ACTION_ACL_DISCONNECTED -> plugins().bluetoothDisconnected(d.address)
                    }
                }
            }
        }
        val filter = IntentFilter().apply {
            addAction(BluetoothDevice.ACTION_ACL_CONNECTED)
            addAction(BluetoothDevice.ACTION_ACL_DISCONNECTED)
            addAction(BluetoothDevice.ACTION_UUID)
        }
        ContextCompat.registerReceiver(context, r, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        receiver = r
        // Classic headphones are listed by the audio profiles' proxies, which come asynchronously.
        val wanted = listOf(BluetoothProfile.A2DP, BluetoothProfile.HEADSET)
        val latch = CountDownLatch(wanted.size)
        proxied = latch
        wanted.forEach { profile ->
            val asked = adapter.getProfileProxy(context, object : BluetoothProfile.ServiceListener {
                override fun onServiceConnected(p: Int, proxy: BluetoothProfile) { proxies[p] = proxy; latch.countDown() }
                override fun onServiceDisconnected(p: Int) { proxies.remove(p) }
            }, profile)
            if (!asked) latch.countDown()
        }
    }

    override fun connected(): List<BtDevice> {
        if (!granted()) return emptyList()
        proxied.await(2, TimeUnit.SECONDS)
        val seen = LinkedHashMap<String, BluetoothDevice>()
        runCatching { manager?.getConnectedDevices(BluetoothProfile.GATT).orEmpty() }.getOrDefault(emptyList()).forEach { seen[it.address] = it }
        proxies.values.forEach { p -> runCatching { p.connectedDevices }.getOrDefault(emptyList()).forEach { seen[it.address] = it } }
        return seen.values.map(::device)
    }

    private fun remote(address: String): BluetoothDevice {
        if (!granted()) throw BtFailure.Refused()
        val adapter = manager?.adapter ?: throw BtFailure.Unsupported()
        return runCatching { adapter.getRemoteDevice(address) }.getOrElse { throw BtFailure.NotFound() }
    }

    override fun rfcomm(link: ULong, device: String, service: String) {
        val socket = try {
            remote(device).createRfcommSocketToServiceRecord(UUID.fromString(service)).also { it.connect() }
        } catch (e: IOException) {
            throw BtFailure.Failed(e.message.orEmpty())
        } catch (e: SecurityException) {
            throw BtFailure.Refused()
        }
        links[link] = Rfcomm(socket)
        Thread({
            val buf = ByteArray(4096)
            while (true) {
                val n = try { socket.inputStream.read(buf) } catch (e: IOException) { -1 }
                if (n < 0) { plugins().bluetoothLost(link); break }
                plugins().bluetoothHeard(link, "", buf.copyOf(n))
            }
        }, "nori-rfcomm-$link").start()
    }

    override fun rfcommWrite(link: ULong, bytes: ByteArray) {
        val s = (links[link] as? Rfcomm)?.socket ?: throw BtFailure.NotFound()
        try { s.outputStream.write(bytes); s.outputStream.flush() } catch (e: IOException) { throw BtFailure.Failed(e.message.orEmpty()) }
    }

    override fun gatt(link: ULong, device: String) {
        val g = Gatt(link)
        links[link] = g
        try {
            g.open(remote(device))
        } catch (e: BtFailure) {
            links.remove(link)?.let { close(it) }
            throw e
        }
    }

    private fun gattOf(link: ULong) = links[link] as? Gatt ?: throw BtFailure.NotFound()

    override fun gattMtu(link: ULong): UShort = ((links[link] as? Gatt)?.mtu ?: 23).toUShort()

    override fun gattServices(link: ULong): List<String> = (links[link] as? Gatt)?.gatt?.services.orEmpty().map { it.uuid.toString().lowercase() }

    override fun gattRead(link: ULong, service: String, characteristic: String): ByteArray = gattOf(link).read(service, characteristic)

    override fun gattWrite(link: ULong, service: String, characteristic: String, bytes: ByteArray, withResponse: Boolean) =
        gattOf(link).write(service, characteristic, bytes, withResponse)

    override fun gattNotify(link: ULong, service: String, characteristic: String, on: Boolean) = gattOf(link).notify(service, characteristic, on)

    override fun close(link: ULong) { links.remove(link)?.let { close(it) } }

    private fun close(l: Link) {
        when (l) {
            is Rfcomm -> runCatching { l.socket.close() }
            is Gatt -> l.gatt?.let { runCatching { it.disconnect(); it.close() } }
        }
    }

    /** One GATT connection: one call waits on the device at a time, its answer handed over by the callback. */
    private inner class Gatt(private val link: ULong) : BluetoothGattCallback(), Link {
        var gatt: BluetoothGatt? = null
        @Volatile var mtu = 23
        private val ready = CountDownLatch(1)
        @Volatile private var failed = false
        private val answers = LinkedBlockingQueue<Pair<Int, ByteArray>>()
        private val turn = Any()

        fun open(d: BluetoothDevice) {
            gatt = try { d.connectGatt(context, false, this, BluetoothDevice.TRANSPORT_LE) } catch (e: SecurityException) { throw BtFailure.Refused() }
            if (!ready.await(SLOWEST_S, TimeUnit.SECONDS)) throw BtFailure.Timeout()
            if (failed) throw BtFailure.Failed("no connection")
        }

        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, state: Int) {
            when {
                state == BluetoothProfile.STATE_CONNECTED -> g.discoverServices()
                ready.count > 0 -> { failed = true; ready.countDown() }
                else -> plugins().bluetoothLost(link)
            }
        }

        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
            if (status != BluetoothGatt.GATT_SUCCESS || !g.requestMtu(517)) ready.countDown()
        }

        override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) {
            if (status == BluetoothGatt.GATT_SUCCESS) this.mtu = mtu
            ready.countDown()
        }

        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray, status: Int) { answers.offer(status to value) }

        @Deprecated("Before Android 13")
        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            @Suppress("DEPRECATION") answers.offer(status to (c.value ?: ByteArray(0)))
        }

        override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) { answers.offer(status to ByteArray(0)) }

        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) { answers.offer(status to ByteArray(0)) }

        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) {
            plugins().bluetoothHeard(link, c.uuid.toString().lowercase(), value)
        }

        @Deprecated("Before Android 13")
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
            @Suppress("DEPRECATION") plugins().bluetoothHeard(link, c.uuid.toString().lowercase(), c.value ?: ByteArray(0))
        }

        private fun characteristic(service: String, characteristic: String): BluetoothGattCharacteristic =
            gatt?.getService(UUID.fromString(service))?.getCharacteristic(UUID.fromString(characteristic)) ?: throw BtFailure.NotFound()

        /** Starts one operation with [start] and waits for the callback's answer. */
        private fun waited(start: (BluetoothGatt) -> Boolean): ByteArray = synchronized(turn) {
            val g = gatt ?: throw BtFailure.NotFound()
            answers.clear()
            if (!start(g)) throw BtFailure.Failed("the device is busy")
            val (status, value) = answers.poll(SLOWEST_S, TimeUnit.SECONDS) ?: throw BtFailure.Timeout()
            if (status != BluetoothGatt.GATT_SUCCESS) throw BtFailure.Failed("GATT status $status")
            value
        }

        fun read(service: String, characteristic: String): ByteArray {
            val c = characteristic(service, characteristic)
            return waited { it.readCharacteristic(c) }
        }

        fun write(service: String, characteristic: String, bytes: ByteArray, withResponse: Boolean) {
            val c = characteristic(service, characteristic)
            val type = if (withResponse) BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT else BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
            waited { g ->
                if (Build.VERSION.SDK_INT >= 33) g.writeCharacteristic(c, bytes, type) == BluetoothGatt.GATT_SUCCESS
                else @Suppress("DEPRECATION") run { c.writeType = type; c.value = bytes; g.writeCharacteristic(c) }
            }
        }

        fun notify(service: String, characteristic: String, on: Boolean) {
            val c = characteristic(service, characteristic)
            val g = gatt ?: throw BtFailure.NotFound()
            if (!g.setCharacteristicNotification(c, on)) throw BtFailure.Failed("notifications refused")
            val d = c.getDescriptor(CLIENT_CONFIG) ?: return
            val value = when {
                !on -> BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE
                c.properties and BluetoothGattCharacteristic.PROPERTY_NOTIFY != 0 -> BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
                else -> BluetoothGattDescriptor.ENABLE_INDICATION_VALUE
            }
            waited { gg ->
                if (Build.VERSION.SDK_INT >= 33) gg.writeDescriptor(d, value) == BluetoothGatt.GATT_SUCCESS
                else @Suppress("DEPRECATION") run { d.value = value; gg.writeDescriptor(d) }
            }
        }
    }

    private companion object {
        val CLIENT_CONFIG: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
        /** The longest a connection or one GATT call takes. */
        const val SLOWEST_S = 10L
    }
}
