package dev.nori.music.plugins

import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import dev.nori.music.Nori
import dev.nori.music.ffi.plugins.Command
import dev.nori.music.ffi.plugins.WakeEvent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * The receivers that start nori for its plugins while it is closed, and the one other apps run plugins'
 * commands through. Each is declared disabled in the manifest; the plugin host (crates/plugins wake.rs)
 * says which some enabled plugin wants, and only those are enabled, so Android never starts nori for
 * one that is off. They only hand the event to the host.
 */
internal object PluginReceivers {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val wakes = mapOf(WakeEvent.BOOT to PluginBootReceiver::class.java, WakeEvent.BLUETOOTH_CONNECTED to PluginBluetoothReceiver::class.java)

    private fun enable(context: Context, receiver: Class<*>, on: Boolean) = context.packageManager.setComponentEnabledSetting(
        ComponentName(context, receiver),
        if (on) PackageManager.COMPONENT_ENABLED_STATE_ENABLED else PackageManager.COMPONENT_ENABLED_STATE_DEFAULT,
        PackageManager.DONT_KILL_APP,
    )

    /** Only [events]' receivers on. */
    fun wake(context: Context, events: List<WakeEvent>) = wakes.forEach { (event, receiver) -> enable(context, receiver, event in events) }

    /** The commands' receiver on while any plugin has commands. */
    fun commands(context: Context, any: Boolean) = enable(context, PluginCommandReceiver::class.java, any)

    /** Hands [then] the plugin host, off the main thread, keeping the broadcast alive until it is done. */
    fun handle(receiver: BroadcastReceiver, context: Context, then: suspend (PluginHost) -> Unit) {
        val pending = receiver.goAsync()
        scope.launch {
            try { then(Nori.get(context).plugins) } finally { pending.finish() }
        }
    }
}

/** The device started: the plugins that want it hear `woke("boot")`. */
class PluginBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        PluginReceivers.handle(this, context) { it.woke(WakeEvent.BOOT) }
    }
}

/** A Bluetooth audio device connected: the plugins that want it hear `woke("bluetooth-connected")`. */
class PluginBluetoothReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val state = intent.getIntExtra(android.bluetooth.BluetoothProfile.EXTRA_STATE, -1)
        if (state != android.bluetooth.BluetoothProfile.STATE_CONNECTED) return
        PluginReceivers.handle(this, context) { it.woke(WakeEvent.BLUETOOTH_CONNECTED) }
    }
}

/**
 * Another app runs a plugin's command: `dev.nori.music.RUN_COMMAND` with the extras `plugin` and
 * `command`, sent by an app holding nori's own permission `<applicationId>.permission.RUN_COMMAND`.
 */
class PluginCommandReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val plugin = intent.getStringExtra("plugin") ?: return
        val command = intent.getStringExtra("command") ?: return
        PluginReceivers.handle(this, context) { it.command(Command(plugin, command)) }
    }
}
