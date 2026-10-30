package dev.nori.music.app.ui

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.nori.music.app.vm.PluginsViewModel
import dev.nori.music.ffi.plugins.ControlledDevice

/**
 * A controlled device's known controls, drawn the same for every brand: its name with each battery it
 * has, and its noise control as one row of chips (the mode on is raised). With [onOpen], a way on to the
 * device's page.
 */
@Composable
fun DeviceControls(vm: PluginsViewModel, d: ControlledDevice, modifier: Modifier = Modifier, onOpen: (() -> Unit)? = null) {
    GroupCard(modifier) {
        GroupRow(d.name, icon = Iconoir.Headset, value = say.battery(d.battery), divider = d.noiseModes.isNotEmpty() || onOpen != null)
        if (d.noiseModes.isNotEmpty()) {
            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = GROUP_INSET, vertical = 10.dp),
                Arrangement.spacedBy(6.dp),
            ) {
                d.noiseModes.forEach { m -> Chip(say.noise(m), m == d.noise) { vm.setNoise(d.id, m) } }
            }
            if (onOpen != null) Hairline(startIndent = GROUP_INSET)
        }
        if (onOpen != null) GroupRow(say.deviceSettings, onOpen, chevron = true, divider = false)
    }
}

/** Everything of a controlled device: its known controls, then its plugin's rows under the plugin's name. */
@Composable
fun ControlledDevicePage(vm: PluginsViewModel, d: ControlledDevice) {
    DeviceControls(vm, d)
    PluginRows(vm, d.card, d.card.name)
}
