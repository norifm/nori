package dev.nori.music.app.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.nori.music.app.vm.SettingsViewModel
import dev.nori.music.ffi.AccountState
import dev.nori.music.ffi.FormField as Field
import dev.nori.music.ffi.ProviderInfo
import dev.nori.music.ffi.SignIn
import dev.nori.music.ffi.settings.SavedServer
import dev.nori.music.settings.server

/** [kind]'s words, a plugin kind's name and about from its plugin as the view model has them. */
@Composable
fun providerWords(vm: SettingsViewModel, kind: dev.nori.music.ffi.settings.ServerKind): ProviderWords {
    val names by vm.pluginNames.collectAsStateWithLifecycle()
    val cards by vm.pluginCards.collectAsStateWithLifecycle()
    val id = (kind as? dev.nori.music.ffi.settings.ServerKind.Plugin)?.id
    return remember(kind, names, cards, say) { say.provider(kind, id?.let(names::get), cards.firstOrNull { it.id == id }?.about) }
}

/**
 * The account of a profile whose provider signs it in apart from the form (the core's
 * `ProviderInfo.sign_in`), as Spotify's and Apple Music's account rows read: who is signed in, or a
 * button to the provider's sign-in page (SignInPage.kt), with pasting folded under it. The sign-in
 * itself, and what each answer means, are the core's (`Client::sign_in`); [signed] hands the account's
 * name to the form being edited.
 */
@Composable
private fun AccountPart(vm: SettingsViewModel, info: ProviderInfo, signed: (String) -> Unit) {
    val ui by vm.account.collectAsStateWithLifecycle()
    val words = providerWords(vm, info.kind).account
    val name = vm.prefs.collectAsStateWithLifecycle().value.server?.user.orEmpty()
    var answer by remember { mutableStateOf("") }
    val page = info.signIn.any { it is SignIn.WebPage }
    val pastes = info.signIn.any { it is SignIn.Paste }
    var pasting by remember { mutableStateOf(!page) }
    LaunchedEffect(Unit) { vm.lookAtAccount() }
    LaunchedEffect(name) { signed(name) }
    Caption(say.account)
    when (ui.account) {
        AccountState.SIGNED_IN -> {
            Text(say.signedInAs(name), Modifier.testTag("signed-in"))
            PillButton(say.signOut, null, vm::signOut, Modifier.fillMaxWidth().testTag("sign-out"))
            return
        }
        AccountState.ENDED -> Text(words.signedOutAgain, color = MaterialTheme.colorScheme.error)
        else -> {}
    }
    Text(words.signInHint, color = MaterialTheme.colorScheme.onSurfaceVariant)
    if (page) PillButton(words.pageButton, null, vm::openSignInPage, Modifier.fillMaxWidth().testTag("sign-in-page"), prominent = true, enabled = !ui.busy)
    if (pastes && !pasting) {
        TextAction(words.pasteInstead, { pasting = true }, Modifier.testTag("paste-instead"))
    } else if (pastes) {
        Text(words.pasteHint, color = MaterialTheme.colorScheme.onSurfaceVariant)
        FormField(answer, { answer = it }, label = { Text(words.pasteLabel) }, minLines = 3, visualTransformation = PasswordVisualTransformation(), modifier = Modifier.fillMaxWidth().testTag("pasted"))
        // The sign-in page shows its own failures.
        ui.failed?.takeIf { !ui.page }?.let { Text(say.signIn(words, it), color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("sign-in-failed")) }
        PillButton(
            if (ui.busy) say.connecting else say.signInAction, null, { vm.signIn(answer) },
            Modifier.fillMaxWidth().testTag("sign-in"), enabled = !ui.busy && answer.isNotBlank(),
        )
    }
    Text(words.risk, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** A kind as tests tag it: `subsonic`, or `plugin:<id>`. */
fun kindTag(kind: dev.nori.music.ffi.settings.ServerKind): String = when (kind) {
    dev.nori.music.ffi.settings.ServerKind.Subsonic -> "subsonic"
    is dev.nori.music.ffi.settings.ServerKind.Plugin -> "plugin:${kind.id}"
}

@Composable
private fun Check(title: String, detail: String, value: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().clickable { onChange(!value) }, verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f).padding(end = 12.dp)) {
            Text(title)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        NoriSwitch(value, onChange)
    }
}

/**
 * First run, and "add / edit server" from settings ([initial] and [onClose] set). A profile being added
 * picks its kind first (the core's `providers`); the form then has the fields that kind's provider asks,
 * its advanced ones folded away.
 */
@Composable
fun LoginScreen(vm: SettingsViewModel, initial: SavedServer? = null, onClose: (() -> Unit)? = null) {
    val ui by vm.login.collectAsStateWithLifecycle()
    var p by remember { mutableStateOf(initial ?: vm.newProfile()) }
    val adding = remember { initial == null || vm.prefs.value.servers.none { it.id == initial.id } }
    val providers by vm.providers.collectAsStateWithLifecycle()
    val info = providers.firstOrNull { it.kind == p.kind }
    val words = providerWords(vm, p.kind)
    var advanced by remember { mutableStateOf(initial != null) }
    var headers by remember { mutableStateOf(dev.nori.music.ffi.settings.serverHeadersText(p.headers)) }
    var certPassword by remember { mutableStateOf("") }
    val pickCert = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> if (uri != null) p = vm.importClientCert(p, uri, certPassword) }
    LaunchedEffect(ui.done) { if (ui.done) { vm.clearLoginResult(); onClose?.invoke() } }

    // Reading the form - the headers typed, the addresses trimmed, whether it can be sent, the schemes
    // offered for an address typed without one - is the core's (settings.rs, providers).
    val ready by androidx.compose.runtime.produceState(false, p.kind, p.url, p.user, p.apiKey) { value = vm.serverReady(p) }
    val schemes = remember(p.url) { dev.nori.music.ffi.settings.serverUrlSchemes(p.url) }

    /** The box for one field of the form. */
    @Composable
    fun Ask(what: Field) {
        when (what) {
            Field.NAME -> FormField(p.name, { p = p.copy(name = it) }, label = { Text(say.nameOptional) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Field.ADDRESS -> {
                FormField(p.url, { p = p.copy(url = it) }, label = { Text(say.serverUrl) }, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri), modifier = Modifier.fillMaxWidth().testTag("url"))
                if (schemes.isNotEmpty()) Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    schemes.forEach { s -> Chip(s, false) { p = p.copy(url = s + p.url) } }
                }
            }
            // An API key stands in for the user and password.
            Field.USER -> if (p.apiKey.isEmpty()) FormField(p.user, { p = p.copy(user = it) }, label = { Text(say.user) }, singleLine = true, modifier = Modifier.fillMaxWidth().testTag("user"))
            Field.PASSWORD -> if (p.apiKey.isEmpty()) FormField(p.password, { p = p.copy(password = it) }, label = { Text(say.password) }, singleLine = true, visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), modifier = Modifier.fillMaxWidth().testTag("password"))
            Field.SECOND_ADDRESS -> FormField(p.altUrl, { p = p.copy(altUrl = it) }, label = { Text(say.secondAddress) }, supportingText = { Text(say.secondAddressHint) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Field.API_KEY -> FormField(p.apiKey, { p = p.copy(apiKey = it) }, label = { Text(say.apiKey) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Field.HEADERS -> FormField(headers, { headers = it }, label = { Text(say.extraHeaders) }, supportingText = { Text(say.extraHeadersHint) }, minLines = 2, modifier = Modifier.fillMaxWidth())
            Field.LEGACY_AUTH -> Check(say.legacyAuth, say.legacyAuthHint, p.legacyAuth) { p = p.copy(legacyAuth = it) }
            Field.SELF_SIGNED -> Check(say.selfSigned, say.selfSignedHint, p.allowSelfSigned) { p = p.copy(allowSelfSigned = it) }
            Field.WIFI_ONLY -> Check(say.wifiOnly, say.wifiOnlyHint, p.wifiOnly) { p = p.copy(wifiOnly = it) }
            Field.CLIENT_CERT -> {
                FormField(certPassword, { certPassword = it }, label = { Text(say.clientCertPassword) }, singleLine = true, visualTransformation = PasswordVisualTransformation(), modifier = Modifier.fillMaxWidth())
                Row(verticalAlignment = Alignment.CenterVertically) {
                    TextAction(if (p.clientCert.isEmpty()) say.importClientCert else say.replaceClientCert, { pickCert.launch(arrayOf("application/x-pkcs12", "application/octet-stream", "*/*")) })
                    if (p.clientCert.isNotEmpty()) TextAction(say.remove, { p = p.copy(clientCert = "", clientCertPassword = "") }, destructive = true)
                }
            }
        }
    }

    var plugins by remember { mutableStateOf(false) }
    NoriDialog(plugins, { plugins = false }, DialogStyle.Page) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.systemBarsPadding()) { PluginsPages(androidx.lifecycle.viewmodel.compose.viewModel(), vm) { plugins = false } }
        }
    }
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.systemBarsPadding().imePadding().verticalScroll(rememberScrollState()).padding(24.dp), Arrangement.spacedBy(12.dp)) {
            Text(
                if (initial == null) "nori" else say.server,
                style = titled(MaterialTheme.typography.displaySmall), modifier = Modifier.padding(top = 16.dp, bottom = 2.dp),
            )
            if (adding && providers.size > 1) androidx.compose.foundation.layout.FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                providers.forEach { k -> Chip(providerWords(vm, k.kind).name, p.kind == k.kind, Modifier.testTag("kind-${kindTag(k.kind)}")) { p = p.copy(kind = k.kind) } }
            }
            // A kind a plugin brings can be the first profile: its plugin is got here.
            if (adding) TextAction(say.pluginsGetMore, { plugins = true }, Modifier.testTag("get-plugins"))
            if (adding && words.hint.isNotEmpty()) Text(words.hint, color = MaterialTheme.colorScheme.onSurfaceVariant)
            info?.fields?.filter { !it.advanced }?.forEach { Ask(it.what) }
            ui.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            PillButton(
                if (ui.busy) say.connecting else if (adding) words.add else say.save, null,
                { vm.login(dev.nori.music.ffi.settings.serverFromForm(p, headers)) },
                Modifier.fillMaxWidth().padding(top = 4.dp).testTag("connect"), prominent = true,
                enabled = !ui.busy && ready,
            )
            // Signing in is the profile in use's: its core asks the provider.
            if (info != null && info.signIn.isNotEmpty() && !adding && p.id == vm.prefs.value.activeServerId) AccountPart(vm, info) { name -> p = p.copy(user = name) }
            val folded = info?.fields?.filter { it.advanced }.orEmpty()
            Row {
                if (folded.isNotEmpty()) TextAction(if (advanced) say.hideAdvanced else say.advanced, { advanced = !advanced })
                onClose?.let { TextAction(say.cancel, it) }
            }
            if (advanced) folded.forEach { Ask(it.what) }
        }
    }
}
