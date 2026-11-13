package dev.nori.music.app.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Icon
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
import androidx.compose.ui.draw.alpha
import dev.nori.music.ffi.settings.ServerKind
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
import dev.nori.music.settings.label
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
 * `ProviderInfo.sign_in`), as Spotify's and Apple Music's account rows read: who is signed in, and those
 * the account may act as (its channels), the one it acts as checked; or a button for each way the
 * provider signs in elsewhere, its page or a code (SignInPage.kt), with pasting folded under them. The
 * sign-in itself, and what each answer means, are the core's (`Client::sign_in`); [signed] hands the
 * account's name to the form being edited.
 */
@Composable
private fun AccountPart(vm: SettingsViewModel, info: ProviderInfo, signed: (String) -> Unit) {
    val ui by vm.account.collectAsStateWithLifecycle()
    val name = vm.prefs.collectAsStateWithLifecycle().value.server?.user.orEmpty()
    LaunchedEffect(Unit) { vm.lookAtAccount() }
    LaunchedEffect(name) { signed(name) }
    GroupLabel(say.account)
    if (ui.account == AccountState.SIGNED_IN) {
        GroupCard {
            IdentityLine(vm, name, say.signedIn, ui.identities.firstOrNull()?.picture, chosen = false, divider = true, Modifier.testTag("signed-in"), null)
            GroupRow(say.signOut, vm::signOut, Modifier.testTag("sign-out"), divider = false)
        }
        val kind = info.identities
        if (kind != null && ui.identities.size > 1) {
            GroupLabel(say.identities(kind))
            GroupCard {
                ui.identities.forEachIndexed { i, id ->
                    IdentityLine(vm, id.name, id.detail, id.picture, id.chosen, i < ui.identities.lastIndex, Modifier.testTag("identity-$i")) { if (!id.chosen) vm.chooseIdentity(i) }
                }
            }
        }
        return
    }
    Column(Modifier.padding(horizontal = GROUP_MARGIN), Arrangement.spacedBy(12.dp)) { SignInWays(vm, info, ui) }
}

/** The ways [info]'s provider signs in, for an account that is not signed in, or whose session [ui] says ended. */
@Composable
private fun SignInWays(vm: SettingsViewModel, info: ProviderInfo, ui: dev.nori.music.app.vm.AccountUi) {
    val words = providerWords(vm, info.kind).account
    var answer by remember { mutableStateOf("") }
    val page = info.signIn.any { it is SignIn.WebPage }
    val code = SignIn.Code in info.signIn
    val pastes = info.signIn.any { it is SignIn.Paste }
    var pasting by remember { mutableStateOf(!page && !code) }
    // Words in line with the label above, the buttons as wide as a card.
    val inLine = Modifier.padding(horizontal = GROUP_MARGIN)
    if (ui.account == AccountState.ENDED) Text(words.signedOutAgain, inLine, color = MaterialTheme.colorScheme.error)
    Text(words.signInHint, inLine, color = MaterialTheme.colorScheme.onSurfaceVariant)
    // The ways in the provider's order, the first prominent.
    for (way in info.signIn) when (way) {
        SignIn.Code -> PillButton(words.code.button, null, vm::signInWithCode, Modifier.fillMaxWidth().testTag("sign-in-code"), prominent = way == info.signIn.first(), enabled = !ui.busy)
        is SignIn.WebPage -> PillButton(words.pageButton, null, vm::openSignInPage, Modifier.fillMaxWidth().testTag("sign-in-page"), prominent = way == info.signIn.first(), enabled = !ui.busy)
        SignIn.Paste -> {}
    }
    if (pastes && !pasting) {
        TextAction(words.pasteInstead, { pasting = true }, Modifier.padding(start = GROUP_MARGIN - 12.dp).testTag("paste-instead"))
    } else if (pastes) {
        Text(words.pasteHint, inLine, color = MaterialTheme.colorScheme.onSurfaceVariant)
        FormField(answer, { answer = it }, label = { Text(words.pasteLabel) }, minLines = 3, maxLines = 6, visualTransformation = PasswordVisualTransformation(), modifier = Modifier.fillMaxWidth().testTag("pasted"))
        // The sign-in page shows its own failures.
        ui.failed?.takeIf { ui.flow.shown !is dev.nori.music.ffi.SignInShown.Page && ui.flow.shown !is dev.nori.music.ffi.SignInShown.Code }?.let { Text(say.signIn(words, it), inLine.testTag("sign-in-failed"), color = MaterialTheme.colorScheme.error) }
        PillButton(
            if (ui.busy) say.connecting else say.signInAction, null, { vm.signIn(answer) },
            Modifier.fillMaxWidth().testTag("sign-in"), enabled = !ui.busy && answer.isNotBlank(),
        )
    }
    Text(words.risk, inLine, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** One the account is or may act as: its picture, its name and what tells it apart, a check in the accent on the one it acts as. */
@Composable
private fun IdentityLine(vm: SettingsViewModel, name: String, detail: String?, picture: String?, chosen: Boolean, divider: Boolean, modifier: Modifier, onClick: (() -> Unit)?) {
    Column(modifier.fillMaxWidth()) {
        Row(
            Modifier.fillMaxWidth().then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
                .padding(start = GROUP_INSET, end = 4.dp, top = 10.dp, bottom = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Cover(remember(picture) { vm.cover(picture, CoverSize.ROW) }, IDENTITY_PICTURE, radius = IDENTITY_PICTURE / 2)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                if (!detail.isNullOrEmpty()) Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
            }
            if (chosen) Icon(Iconoir.Check, say.inUse, Modifier.padding(horizontal = 10.dp).size(20.dp), tint = MaterialTheme.colorScheme.primary)
        }
        if (divider) Hairline(startIndent = GROUP_INSET + IDENTITY_PICTURE + 12.dp)
    }
}

private val IDENTITY_PICTURE = 40.dp

/** A kind as tests tag it: `subsonic`, or `plugin:<id>`. */
fun kindTag(kind: dev.nori.music.ffi.settings.ServerKind): String = when (kind) {
    dev.nori.music.ffi.settings.ServerKind.Subsonic -> "subsonic"
    is dev.nori.music.ffi.settings.ServerKind.Plugin -> "plugin:${kind.id}"
}

/** A switch of the form, as a row of a [GroupCard]. */
@Composable
private fun Check(title: String, detail: String, value: Boolean, divider: Boolean, onChange: (Boolean) -> Unit) {
    GroupRow(title, { onChange(!value) }, subtitle = detail, divider = divider) { Spacer(Modifier.width(12.dp)); NoriSwitch(value, onChange) }
}

/** The form's switches: drawn together on a card rather than one by one among the fields. */
private val SWITCHES = setOf(Field.LEGACY_AUTH, Field.SELF_SIGNED, Field.WIFI_ONLY)

/**
 * First run, and "add / edit server" from settings ([initial] and [onClose] set). A profile being added
 * picks its kind first (the core's `providers`); the form then has the fields that kind's provider asks,
 * its advanced ones folded away.
 */
@Composable
fun LoginScreen(vm: SettingsViewModel, initial: SavedServer? = null, onClose: (() -> Unit)? = null) {
    val add by vm.add.collectAsStateWithLifecycle()
    var p by remember { mutableStateOf(initial ?: vm.newProfile()) }
    val adding = remember { initial == null || vm.prefs.value.servers.none { it.id == initial.id } }
    val providers by vm.providers.collectAsStateWithLifecycle()
    val info = providers.firstOrNull { it.kind == p.kind }
    val words = providerWords(vm, p.kind)
    var advanced by remember { mutableStateOf(false) }
    var headers by remember { mutableStateOf(dev.nori.music.ffi.settings.serverHeadersText(p.headers)) }
    var certPassword by remember { mutableStateOf("") }
    val pickCert = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> if (uri != null) p = vm.importClientCert(p, uri, certPassword) }
    LaunchedEffect(vm) { vm.added.collect { onClose?.invoke() } }
    // Gone, the next form starts afresh (the core's `AddFlow`).
    androidx.compose.runtime.DisposableEffect(vm) { onDispose { vm.addOn(dev.nori.music.ffi.AddEvent.Start) } }
    val busy = add.send is dev.nori.music.ffi.AddSend.Sending

    // Reading the form - the headers typed, the addresses trimmed, whether it can be sent, the schemes
    // offered for an address typed without one - is the core's (settings.rs, providers).
    val ready by androidx.compose.runtime.produceState(false, p.kind, p.url, p.user, p.apiKey) { value = vm.serverReady(p) }
    val schemes = remember(p.url) { dev.nori.music.ffi.settings.serverUrlSchemes(p.url) }

    /** The switch for one of [SWITCHES]. */
    @Composable
    fun Switch(what: Field, divider: Boolean) {
        when (what) {
            Field.LEGACY_AUTH -> Check(say.legacyAuth, say.legacyAuthHint, p.legacyAuth, divider) { p = p.copy(legacyAuth = it) }
            Field.SELF_SIGNED -> Check(say.selfSigned, say.selfSignedHint, p.allowSelfSigned, divider) { p = p.copy(allowSelfSigned = it) }
            Field.WIFI_ONLY -> Check(say.wifiOnly, say.wifiOnlyHint, p.wifiOnly, divider) { p = p.copy(wifiOnly = it) }
            else -> {}
        }
    }

    /** The switches among [fields], on a card of their own. */
    @Composable
    fun Switches(fields: List<Field>) {
        val switches = fields.filter { it in SWITCHES }
        if (switches.isNotEmpty()) GroupCard(Modifier.padding(top = 12.dp)) { switches.forEachIndexed { i, f -> Switch(f, i < switches.lastIndex) } }
    }

    /** The text fields among [fields] the form asks now: user and password step aside for an API key, and a
     *  saved profile is renamed from its menu. */
    fun asked(fields: List<Field>) = fields.filter {
        it !in SWITCHES && !(p.apiKey.isNotEmpty() && (it == Field.USER || it == Field.PASSWORD)) && !(it == Field.NAME && !adding)
    }

    /** One field of the form, a row of its card. */
    @Composable
    fun Ask(what: Field, divider: Boolean) {
        when (what) {
            Field.NAME -> GroupField(p.name, { p = p.copy(name = it) }, label = say.nameOptional, stacked = true, divider = divider)
            Field.ADDRESS -> {
                GroupField(
                    p.url, { p = p.copy(url = it) }, Modifier.testTag("url"), label = say.serverUrl, stacked = true, divider = divider && schemes.isEmpty(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                )
                if (schemes.isNotEmpty()) {
                    Row(Modifier.padding(start = GROUP_INSET, end = GROUP_INSET, bottom = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        schemes.forEach { s -> Chip(s, false) { p = p.copy(url = s + p.url) } }
                    }
                    if (divider) Hairline(startIndent = GROUP_INSET)
                }
            }
            Field.USER -> GroupField(p.user, { p = p.copy(user = it) }, Modifier.testTag("user"), label = say.user, stacked = true, divider = divider)
            Field.PASSWORD -> GroupField(
                p.password, { p = p.copy(password = it) }, Modifier.testTag("password"), label = say.password, stacked = true, divider = divider,
                visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            )
            Field.SECOND_ADDRESS -> GroupField(p.altUrl, { p = p.copy(altUrl = it) }, label = say.secondAddress, stacked = true, supporting = say.secondAddressHint, divider = divider)
            Field.API_KEY -> GroupField(p.apiKey, { p = p.copy(apiKey = it) }, label = say.apiKey, stacked = true, divider = divider)
            Field.HEADERS -> GroupField(headers, { headers = it }, label = say.extraHeaders, stacked = true, supporting = say.extraHeadersHint, minLines = 2, divider = divider)
            Field.LEGACY_AUTH, Field.SELF_SIGNED, Field.WIFI_ONLY -> {}
            Field.CLIENT_CERT -> {
                GroupField(
                    certPassword, { certPassword = it }, label = say.clientCertPassword, stacked = true, divider = false,
                    visualTransformation = PasswordVisualTransformation(),
                )
                Row(Modifier.padding(start = GROUP_INSET - 12.dp), verticalAlignment = Alignment.CenterVertically) {
                    TextAction(if (p.clientCert.isEmpty()) say.importClientCert else say.replaceClientCert, { pickCert.launch(arrayOf("application/x-pkcs12", "application/octet-stream", "*/*")) })
                    if (p.clientCert.isNotEmpty()) TextAction(say.remove, { p = p.copy(clientCert = "", clientCertPassword = "") }, destructive = true)
                }
                if (divider) Hairline(startIndent = GROUP_INSET)
            }
        }
    }

    /** The text fields among [fields], on a card of their own. */
    @Composable
    fun Fields(fields: List<Field>) {
        val shown = asked(fields)
        if (shown.isNotEmpty()) GroupCard(Modifier.padding(top = 8.dp)) { shown.forEachIndexed { i, f -> Ask(f, i < shown.lastIndex) } }
    }

    // A new profile starts by choosing what it plays from; back from its form chooses again. One with
    // nothing to ask is added as soon as it's chosen, its page showing only a failure (the core's `AddFlow`).
    val page = add.page
    androidx.activity.compose.BackHandler(adding && page is dev.nori.music.ffi.AddPage.Provider) { vm.addOn(dev.nori.music.ffi.AddEvent.Back) }
    if (adding && page !is dev.nori.music.ffi.AddPage.Provider) {
        ProviderChoice(vm, onClose) { kind ->
            p = p.copy(kind = kind)
            vm.chooseKind(kind) { dev.nori.music.ffi.settings.serverFromForm(p, headers) }
        }
        return
    }
    // Nothing to ask: an account's provider offers signing in or going on as a guest.
    val step = (page as? dev.nori.music.ffi.AddPage.Provider)?.step ?: dev.nori.music.ffi.FirstStep.FORM
    val asksNothing = step != dev.nori.music.ffi.FirstStep.FORM
    val signsIn = step == dev.nori.music.ffi.FirstStep.ACCOUNT
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.systemBarsPadding().imePadding().verticalScroll(rememberScrollState())) {
            LargeTitle(
                if (adding) words.name else p.label.ifBlank { words.name }, Modifier.padding(top = 16.dp),
                trailing = onClose?.let { close -> { GlyphButton(Iconoir.Xmark, say.cancel, Modifier.testTag("close"), onClick = close) } },
            )
            if (adding && words.hint.isNotEmpty()) Text(words.hint, Modifier.padding(horizontal = Space.gutter, vertical = 4.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
            val main = info?.fields?.filter { !it.advanced }.orEmpty().map { it.what }
            val folded = info?.fields?.filter { it.advanced }.orEmpty().map { it.what }
            if (!asksNothing) {
                Fields(main)
                Switches(main)
            }
            if (folded.isNotEmpty()) {
                GroupCard(Modifier.padding(top = 8.dp)) {
                    GroupRow(say.advanced, { advanced = !advanced }, Modifier.testTag("advanced"), divider = false) {
                        Icon(if (advanced) Iconoir.NavArrowUp else Iconoir.NavArrowDown, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                if (advanced) {
                    Fields(folded)
                    Switches(folded)
                }
            }
            // A saved profile with nothing to edit here has no button: it is renamed from its menu.
            val saves = adding || (main + folded).any { it != Field.NAME }
            val failed = (add.send as? dev.nori.music.ffi.AddSend.Failed)?.why
            if (signsIn || saves || failed != null) Column(Modifier.padding(horizontal = GROUP_MARGIN, vertical = 12.dp), Arrangement.spacedBy(12.dp)) {
                failed?.let { Text(say.addFailure(it), color = MaterialTheme.colorScheme.error) }
                if (signsIn) {
                    PillButton(
                        if (busy) say.connecting else words.account.pageButton, null,
                        { vm.login(dev.nori.music.ffi.settings.serverFromForm(p, headers), dev.nori.music.ffi.AfterAdding.SIGN_IN) },
                        Modifier.fillMaxWidth().padding(top = 4.dp).testTag("connect-sign-in"), prominent = true, enabled = !busy && ready,
                    )
                    PillButton(
                        say.providerGuest, null, { vm.login(dev.nori.music.ffi.settings.serverFromForm(p, headers), dev.nori.music.ffi.AfterAdding.GUEST) },
                        Modifier.fillMaxWidth().testTag("connect-guest"), enabled = !busy && ready,
                    )
                } else if (saves) PillButton(
                    if (busy) say.connecting else if (adding) words.add else say.save, null,
                    { vm.login(dev.nori.music.ffi.settings.serverFromForm(p, headers)) },
                    Modifier.fillMaxWidth().padding(top = 4.dp).testTag("connect"), prominent = true,
                    enabled = !busy && ready,
                )
            }
            // Signing in is the profile in use's: its core asks the provider.
            if (info != null && info.signIn.isNotEmpty() && !adding && p.id == vm.prefs.value.activeServerId) AccountPart(vm, info) { name -> p = p.copy(user = name) }
            Spacer(Modifier.height(24.dp))
        }
    }
}

/**
 * What a new profile plays from: nori's own and the plugins' providers in ink, then, quieter, those a repo
 * offers that aren't on. Choosing one of those installs it (its sheet says what it may do) and turns it on.
 * The title's plus reaches every plugin and the repos.
 */
@Composable
private fun ProviderChoice(vm: SettingsViewModel, onClose: (() -> Unit)?, choose: (ServerKind) -> Unit) {
    val plugins: dev.nori.music.app.vm.PluginsViewModel = androidx.lifecycle.viewmodel.compose.viewModel()
    val providers by vm.providers.collectAsStateWithLifecycle()
    val offers by plugins.providerOffers.collectAsStateWithLifecycle()
    val cards by plugins.cards.collectAsStateWithLifecycle()
    var more by remember { mutableStateOf(false) }
    // The page of a plugin a repo offers, open to install it, then that plugin installing on this screen
    // (the core's `AddPage::Offered`, `AddPage::Installing`).
    val page = vm.add.collectAsStateWithLifecycle().value.page
    val offered = (page as? dev.nori.music.ffi.AddPage.Offered)?.plugin
    val awaited = (page as? dev.nori.music.ffi.AddPage.Installing)?.plugin
    val installing by plugins.installing.collectAsStateWithLifecycle()
    val refused by plugins.refused.collectAsStateWithLifecycle()
    val allOff = vm.prefs.collectAsStateWithLifecycle().value.pluginsOff
    LaunchedEffect(Unit) { plugins.loadOffers() }
    LaunchedEffect(installing, offered) { if (installing != null && installing == offered) vm.addOn(dev.nori.music.ffi.AddEvent.InstallBegan) }
    // Installed, it goes on and its form opens; refused, its page says why.
    LaunchedEffect(cards, awaited, installing, refused) {
        val id = awaited ?: return@LaunchedEffect
        when {
            cards.any { it.id == id } -> { plugins.turn(id, true); choose(ServerKind.Plugin(id)) }
            installing != id && refused != null -> vm.addOn(dev.nori.music.ffi.AddEvent.InstallRefused)
        }
    }
    NoriDialog(more, { more = false }, DialogStyle.Page) {
        Surface(Modifier.fillMaxSize()) { Column(Modifier.systemBarsPadding()) { PluginsPages(plugins, vm) } }
    }
    val back = { vm.addOn(dev.nori.music.ffi.AddEvent.Back); Unit }
    NoriDialog(offered != null, back, DialogStyle.Page) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.systemBarsPadding()) { offered?.let { PluginPage(plugins, vm, it, back) } }
        }
    }
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.systemBarsPadding().verticalScroll(rememberScrollState())) {
            LargeTitle(say.providerChoose, Modifier.padding(top = 16.dp), trailing = { GlyphButton(Iconoir.Plus, say.pluginsGet, Modifier.testTag("get-plugins")) { more = true } })
            Spacer(Modifier.height(12.dp))
            GroupCard {
                providers.forEachIndexed { i, k ->
                    val words = providerWords(vm, k.kind)
                    val divider = i < providers.lastIndex || offers.isNotEmpty()
                    val tag = Modifier.testTag("kind-${kindTag(k.kind)}")
                    when (val kind = k.kind) {
                        is ServerKind.Plugin -> ProviderLine(plugins, kind.id, null, words.name, null, divider, tag) { choose(kind) }
                        ServerKind.Subsonic -> ProviderLine(Iconoir.Server, words.name, say.providerSubsonicAbout, divider, tag) { choose(kind) }
                    }
                }
                offers.forEachIndexed { i, o ->
                    ProviderLine(plugins, o.id, o, o.name, null, i < offers.lastIndex, Modifier.alpha(0.5f).testTag("offered-${o.id}")) {
                        when {
                            // With every plugin off none could run: it leads to the switch.
                            allOff -> more = true
                            cards.any { it.id == o.id } -> { plugins.turn(o.id, true); choose(ServerKind.Plugin(o.id)) }
                            else -> vm.addOn(dev.nori.music.ffi.AddEvent.OfferOpened(o.id))
                        }
                    }
                }
            }
            onClose?.let { Row(Modifier.padding(horizontal = 8.dp, vertical = 12.dp)) { TextAction(say.cancel, it) } }
        }
    }
}

/**
 * The profile in use while its plugin doesn't run here ([wait], the core's `Kinds::waits`): what it plays
 * from and the one step that opens it, then the other profiles to switch to. Once the plugin runs, the
 * view model opens the profile.
 */
@Composable
fun WaitingProfile(vm: SettingsViewModel, wait: dev.nori.music.ffi.ProfileWait) {
    val plugins: dev.nori.music.app.vm.PluginsViewModel = androidx.lifecycle.viewmodel.compose.viewModel()
    val prefs by vm.prefs.collectAsStateWithLifecycle()
    val cards by plugins.cards.collectAsStateWithLifecycle()
    val offers by plugins.offers.collectAsStateWithLifecycle()
    val profile = prefs.server ?: return
    val id = (profile.kind as? ServerKind.Plugin)?.id ?: return
    LaunchedEffect(Unit) { plugins.loadOffers() }
    val name = remember(id, cards, offers) { dev.nori.music.ffi.plugins.pluginDisplayName(id, cards, offers.offers) }
    var installing by remember { mutableStateOf(false) }
    var adding by remember { mutableStateOf(false) }
    NoriDialog(installing, { installing = false }, DialogStyle.Page) {
        Surface(Modifier.fillMaxSize()) { Column(Modifier.systemBarsPadding()) { PluginPage(plugins, vm, id) { installing = false } } }
    }
    NoriDialog(adding, { adding = false }, DialogStyle.Page) { LoginScreen(vm) { adding = false } }
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.systemBarsPadding().verticalScroll(rememberScrollState())) {
            LargeTitle(profile.label.ifBlank { name }, Modifier.padding(top = 16.dp))
            Column(Modifier.padding(horizontal = GROUP_MARGIN, vertical = 12.dp), Arrangement.spacedBy(12.dp)) {
                Text(say.profileWaits(name, wait), Modifier.padding(horizontal = Space.gutter - GROUP_MARGIN), color = MaterialTheme.colorScheme.onSurfaceVariant)
                say.profileWaitAction(name, wait)?.let { action ->
                    PillButton(action, null, {
                        when (wait) {
                            dev.nori.music.ffi.ProfileWait.INSTALL -> installing = true
                            dev.nori.music.ffi.ProfileWait.TURN_ON -> plugins.turn(id, true)
                            dev.nori.music.ffi.ProfileWait.PLUGINS_ON -> plugins.allOff(vm, false)
                            dev.nori.music.ffi.ProfileWait.UPDATE, dev.nori.music.ffi.ProfileWait.NO_PLUGINS -> {}
                        }
                    }, Modifier.fillMaxWidth().padding(top = 4.dp).testTag("profile-wait"), prominent = true)
                }
            }
            val others = prefs.servers.filter { it.id != profile.id }
            if (others.isNotEmpty()) GroupLabel(say.profiles)
            GroupCard {
                others.forEach { other -> ProfileLine(vm, other, active = false, divider = true) { vm.switchServer(other) } }
                GroupRow(say.addProfile, { adding = true }, icon = Iconoir.Plus, divider = false)
            }
        }
    }
}
