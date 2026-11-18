package dev.nori.music.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.nori.music.app.vm.SettingsViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import dev.nori.music.ffi.ScrobblerAsk
import dev.nori.music.ffi.ScrobblerField
import dev.nori.music.ffi.ScrobblerForm
import dev.nori.music.ffi.ScrobblerSignIn
import dev.nori.music.ffi.scrobblerFields
import dev.nori.music.ffi.scrobblerFormReady
import dev.nori.music.ffi.scrobblerFormValue
import dev.nori.music.ffi.scrobblerFormWith
import dev.nori.music.ffi.scrobblerHelpPage
import dev.nori.music.ffi.scrobblerKinds
import dev.nori.music.ffi.scrobblerSignIn
import dev.nori.music.ffi.settings.SavedServer
import dev.nori.music.ffi.settings.ScrobblerKind

/**
 * Where a saved profile's plays go: its own server or provider first, built in, where it takes them;
 * then its scrobbling accounts, whatever it plays from, each with its switch and a menu to remove it;
 * then a row adding one, which picks the service and then asks what it needs. Sending is the core's
 * (`Client::scrobble`). [changed] hands the profile saved to the form being edited.
 */
@Composable
fun ScrobblingPart(vm: SettingsViewModel, profile: SavedServer, changed: (SavedServer) -> Unit) {
    var adding by remember { mutableStateOf(false) }
    // The profile's rules, read off the UI thread: a plugin's provider is asked.
    val itself by produceState(false, profile.id, profile.kind) { value = withContext(Dispatchers.IO) { vm.scrobblesItself(profile) } }
    GroupLabel(say.scrobbling)
    GroupCard {
        if (itself) {
            GroupRow(
                providerWords(vm, profile.kind).name, { changed(vm.switchServerScrobble(profile, profile.scrobbleOff)) }, Modifier.testTag("scrobbler-server"),
                subtitle = say.scrobblerBuiltIn,
            ) {
                Spacer(Modifier.width(12.dp))
                NoriSwitch(!profile.scrobbleOff, { on -> changed(vm.switchServerScrobble(profile, on)) })
            }
        }
        profile.scrobblers.forEach { a ->
            val host = a.url.takeIf { it.isNotBlank() }?.let { android.net.Uri.parse(it.trim()).host ?: it.trim() }
            GroupRow(
                say.scrobbler(a.kind), { changed(vm.putScrobbler(profile, a.copy(on = !a.on))) }, Modifier.testTag("scrobbler-${a.id}"),
                subtitle = listOfNotNull(a.user.ifBlank { null }, host).joinToString(" · "),
            ) {
                Spacer(Modifier.width(12.dp))
                NoriSwitch(a.on, { on -> changed(vm.putScrobbler(profile, a.copy(on = on))) })
                MoreButton(listOf(say.remove to { changed(vm.removeScrobbler(profile, a.id)) }))
            }
        }
        GroupRow(
            say.addScrobblerAccount, { vm.scrobblerAddReset(); adding = true }, Modifier.testTag("add-scrobbler"),
            subtitle = say.scrobblingHint.takeIf { profile.scrobblers.isEmpty() && !itself }, icon = Iconoir.Plus, divider = false,
        )
    }
    NoriSheet(adding, { adding = false }) { AddScrobbler(vm, profile) { saved -> adding = false; changed(saved) } }
}

/** Adding an account: the services to pick from, then the picked one's form. */
@Composable
private fun AddScrobbler(vm: SettingsViewModel, profile: SavedServer, added: (SavedServer) -> Unit) {
    var kind by remember { mutableStateOf<ScrobblerKind?>(null) }
    val picked = kind
    if (picked != null) return ScrobblerFormPart(vm, profile, picked, added)
    SheetTitle(say.addScrobblerTitle)
    GroupCard(Modifier.padding(bottom = 16.dp)) {
        val kinds = remember { scrobblerKinds() }
        kinds.forEachIndexed { i, k ->
            GroupRow(
                say.scrobbler(k), { vm.scrobblerAddReset(); kind = k }, Modifier.testTag("add-scrobbler-${k.name.lowercase()}"),
                subtitle = say.scrobblerCost(k), chevron = true, divider = i < kinds.lastIndex,
            )
        }
    }
}

/**
 * A [kind] account's form: the fields it asks (`scrobbler_fields`), a link to where they come from, the
 * address folded under Advanced, then Add; a kind signing in on its page asks for a token, opens the
 * page, and adds the account once the user is back.
 */
@Composable
private fun ScrobblerFormPart(vm: SettingsViewModel, profile: SavedServer, kind: ScrobblerKind, added: (SavedServer) -> Unit) {
    val ui by vm.scrobblerAdd.collectAsStateWithLifecycle()
    val uri = LocalUriHandler.current
    var form by remember(kind) { mutableStateOf(ScrobblerForm(kind, "", "", "", "", "", "")) }
    var advanced by remember(kind) { mutableStateOf(false) }
    val asks = remember(kind) { scrobblerFields(kind) }
    val browser = remember(kind) { scrobblerSignIn(kind) == ScrobblerSignIn.BROWSER }
    val approval = ui.approval
    LaunchedEffect(approval) { approval?.let { runCatching { uri.openUri(it.page) } } }
    SheetTitle(say.addScrobbler(kind))
    val help = remember(kind) { scrobblerHelpPage(kind) }
    GroupCard {
        val main = asks.filter { !it.advanced }
        Fields(main, kind, form, last = help == null) { form = it }
        help?.let { page ->
            GroupRow(say.scrobblerHelp(kind), { runCatching { uri.openUri(page) } }, Modifier.testTag("scrobbler-help"), divider = false) {
                Icon(Iconoir.ShareIos, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
    val folded = asks.filter { it.advanced }
    if (folded.isNotEmpty()) {
        GroupCard {
            GroupRow(say.advanced, { advanced = !advanced }, Modifier.testTag("scrobbler-advanced"), divider = advanced) {
                Icon(if (advanced) Iconoir.NavArrowUp else Iconoir.NavArrowDown, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (advanced) Fields(folded, kind, form, last = true) { form = it }
        }
    }
    Column(Modifier.padding(horizontal = GROUP_MARGIN, vertical = 16.dp), Arrangement.spacedBy(12.dp)) {
        if (browser && approval != null) Text(say.scrobblerAllow(kind), Modifier.testTag("scrobbler-allow"), color = MaterialTheme.colorScheme.onSurfaceVariant)
        ui.failed?.let { Text(it, Modifier.testTag("scrobbler-failed"), color = MaterialTheme.colorScheme.error) }
        val ready = scrobblerFormReady(form) && !ui.busy
        when {
            ui.busy -> PillButton(say.connecting, null, {}, Modifier.fillMaxWidth().testTag("scrobbler-add"), prominent = true, enabled = false)
            browser && approval == null -> PillButton(
                say.scrobblerContinue(kind), Iconoir.ShareIos, { vm.askScrobblerApproval(form) },
                Modifier.fillMaxWidth().testTag("scrobbler-continue"), prominent = true, enabled = ready,
            )
            else -> PillButton(
                if (browser) say.done else say.add, null, { vm.addScrobbler(profile, form, added) },
                Modifier.fillMaxWidth().testTag("scrobbler-add"), prominent = true, enabled = ready,
            )
        }
    }
}

/** [asks]' fields, a hairline under each but the [last] one's. */
@Composable
private fun Fields(asks: List<ScrobblerAsk>, kind: ScrobblerKind, form: ScrobblerForm, last: Boolean, changed: (ScrobblerForm) -> Unit) {
    val hidden = remember { PasswordVisualTransformation() }
    asks.forEachIndexed { i, a ->
        val f = a.field
        GroupField(
            scrobblerFormValue(form, f), { changed(scrobblerFormWith(form, f, it)) }, Modifier.testTag("scrobbler-${f.name.lowercase()}"),
            label = say.scrobblerField(f), stacked = true, supporting = say.scrobblerFieldHint(kind, f), divider = !last || i < asks.lastIndex,
            visualTransformation = if (f in listOf(ScrobblerField.TOKEN, ScrobblerField.SECRET, ScrobblerField.PASSWORD)) hidden else VisualTransformation.None,
            keyboardOptions = when (f) {
                ScrobblerField.PASSWORD -> KeyboardOptions(keyboardType = KeyboardType.Password)
                ScrobblerField.ADDRESS -> KeyboardOptions(keyboardType = KeyboardType.Uri)
                else -> KeyboardOptions.Default
            },
        )
    }
}
