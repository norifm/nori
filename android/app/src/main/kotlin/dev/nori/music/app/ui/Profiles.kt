package dev.nori.music.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.nori.music.app.vm.PluginsViewModel
import dev.nori.music.app.vm.SettingsViewModel
import dev.nori.music.ffi.settings.SavedServer
import dev.nori.music.ffi.settings.ServerKind
import dev.nori.music.settings.label

/** What a profile is called: its name, else its provider's. */
@Composable
fun profileLabel(vm: SettingsViewModel, profile: SavedServer): String {
    val words = providerWords(vm, profile.kind)
    return profile.label.ifBlank { words.name }
}

/**
 * A profile's mark on a circle: its plugin's icon, or for a server the first letter of who signs in
 * (of its name without a user).
 */
@Composable
fun ProfileMark(profile: SavedServer, label: String, side: Dp, modifier: Modifier = Modifier) {
    when (val kind = profile.kind) {
        is ServerKind.Plugin -> {
            val plugins: PluginsViewModel = viewModel()
            Box(modifier) { PluginTile(plugins, kind.id, null, label, side, CircleShape) }
        }
        ServerKind.Subsonic -> Box(
            modifier.size(side).background(MaterialTheme.colorScheme.surfaceContainerHighest, CircleShape),
            Alignment.Center,
        ) {
            Text(
                remember(profile.user, label) { profile.user.ifBlank { label }.take(1).uppercase() },
                style = MaterialTheme.typography.titleMedium.copy(fontSize = (side.value * 0.42f).sp, fontWeight = FontWeight.SemiBold),
            )
        }
    }
}

/** Home's corner: the profile in use's mark, opening the switcher. */
@Composable
fun ProfileButton(vm: SettingsViewModel, onManage: () -> Unit) {
    val prefs by vm.prefs.collectAsStateWithLifecycle()
    val profile = prefs.servers.firstOrNull { it.id == prefs.activeServerId } ?: return
    var open by remember { mutableStateOf(false) }
    val label = profileLabel(vm, profile)
    Box(
        Modifier.padding(end = 8.dp).size(40.dp).clip(CircleShape).clickable { open = true }.testTag("profile-button")
            .clearAndSetSemantics { contentDescription = say.profiles; role = androidx.compose.ui.semantics.Role.Button },
        Alignment.Center,
    ) {
        ProfileMark(profile, label, 32.dp, Modifier.border(hairlineBorder(), CircleShape))
    }
    ProfileSwitcher(vm, open, { open = false }, onManage)
}

/** The profiles to switch to, in a sheet: each with its mark, the one in use checked in the accent. */
@Composable
private fun ProfileSwitcher(vm: SettingsViewModel, open: Boolean, onDismiss: () -> Unit, onManage: () -> Unit) {
    var adding by remember { mutableStateOf(false) }
    NoriDialog(adding, { adding = false }, DialogStyle.Page) { LoginScreen(vm) { adding = false } }
    NoriSheet(open, onDismiss) {
        val prefs by vm.prefs.collectAsStateWithLifecycle()
        val options = dev.nori.music.ffi.settings.SettingsPart.SERVER_OPTIONS in LocalRules.current.settings
        Column(Modifier.windowInsetsPadding(gestureBarRoom)) {
            SheetTitle(say.profiles)
            GroupCard {
                prefs.servers.forEachIndexed { i, profile ->
                    val active = profile.id == prefs.activeServerId
                    ProfileLine(vm, profile, active, divider = i < prefs.servers.lastIndex, Modifier.testTag("profile-${profile.id}")) {
                        if (!active) vm.switchServer(profile)
                        onDismiss()
                    }
                }
            }
            if (options) GroupCard {
                GroupRow(say.addProfile, { onDismiss(); adding = true }, icon = Iconoir.Plus)
                GroupRow(say.manageProfiles, { onDismiss(); onManage() }, icon = Iconoir.Settings, divider = false)
            }
        }
    }
}

/** A profile in a list: its mark, what it is called and who signs in, a check in the accent while it is in use. */
@Composable
fun ProfileLine(
    vm: SettingsViewModel, profile: SavedServer, active: Boolean, divider: Boolean, modifier: Modifier = Modifier,
    detail: String? = null, trailing: @Composable () -> Unit = {}, onClick: (() -> Unit)?,
) {
    val label = profileLabel(vm, profile)
    val who = detail ?: profile.user.ifBlank { providerWords(vm, profile.kind).noUser }
    Column(modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
                .padding(start = GROUP_INSET, end = 4.dp, top = 10.dp, bottom = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ProfileMark(profile, label, PROFILE_MARK)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(label, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (who.isNotEmpty()) Text(who, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (active) Icon(Iconoir.Check, say.inUse, Modifier.padding(horizontal = 10.dp).size(20.dp), tint = MaterialTheme.colorScheme.primary)
            trailing()
        }
        if (divider) Hairline(startIndent = GROUP_INSET + PROFILE_MARK + 12.dp)
    }
}

private val PROFILE_MARK = 40.dp
