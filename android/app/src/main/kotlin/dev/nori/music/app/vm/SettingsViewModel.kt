package dev.nori.music.app.vm

import android.app.Application
import androidx.lifecycle.viewModelScope
import dev.nori.music.ffi.model.IngestStats
import dev.nori.music.net.said
import dev.nori.music.playback.DacState
import dev.nori.music.playback.DeviceSound
import dev.nori.music.playback.Outputs
import dev.nori.music.ffi.settings.StoredPrefs
import dev.nori.music.ffi.settings.SavedServer
import dev.nori.music.net.describeConnectionError
import dev.nori.music.ffi.model.MusicFolder
import dev.nori.music.ffi.settings.SoundBand
import dev.nori.music.ffi.devices.ChoiceKind
import dev.nori.music.ffi.devices.deviceRows
import dev.nori.music.ffi.settings.eqPresets
import dev.nori.music.ffi.settings.SoundTool
import dev.nori.music.ffi.settings.EqLevel
import dev.nori.music.ffi.settings.SettingChange
import dev.nori.music.ffi.settings.newServerId
import dev.nori.music.ffi.settings.soundFromJson
import dev.nori.music.ffi.settings.storageIndexFiles
import dev.nori.music.ffi.model.NamedPreset
import dev.nori.music.ffi.model.AutoEqEntry
import dev.nori.music.ffi.settings.SoundException
import dev.nori.music.ffi.model.SoundProfile
import dev.nori.music.settings.serverList
import dev.nori.music.settings.sound
import dev.nori.music.settings.withSound
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.merge
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.launch
import dev.nori.music.settings.server
import dev.nori.music.update.Updates

/** settings_store.rs `CACHE_LIMIT`: the stream cache size changed. */
private const val CACHE_LIMIT = 32u

/**
 * The account of the profile in use, for a provider that signs it in apart from its form
 * (`ProviderInfo.sign_in`): where it stands (null until asked), and what is shown for signing in with the
 * last sign-in (the core's `SignInFlow`, one dialog at a time).
 */
data class AccountUi(
    val account: dev.nori.music.ffi.AccountState? = null,
    val flow: dev.nori.music.ffi.SignInFlow = dev.nori.music.ffi.SignInFlow(dev.nori.music.ffi.SignInShown.Closed, dev.nori.music.ffi.SignInAttempt.Idle),
) {
    val busy: Boolean get() = flow.attempt == dev.nori.music.ffi.SignInAttempt.Busy
    val failed: dev.nori.music.ffi.SignInResult? get() = (flow.attempt as? dev.nori.music.ffi.SignInAttempt.Failed)?.result
}

/**
 * The AutoEQ list: how many headphones it holds ([countWords] and [searchWords] say so), the query and
 * its hits, and whether the query is still too short to search (the core's `autoeq_too_short`).
 */
data class AutoEqUi(
    val count: Int = 0,
    val query: String = "",
    val hits: List<AutoEqHit> = emptyList(),
    val tooShort: Boolean = true,
    val countWords: String = "",
    val searchWords: String = "",
    /** The list's download or a curve being applied: one at a time, with how the last one ended. */
    val task: AutoEqTask = AutoEqTask.Idle,
) {
    val busy: Boolean get() = task is AutoEqTask.Downloading || task is AutoEqTask.Applying
    val applied: String? get() = when (val t = task) {
        is AutoEqTask.Applied -> t.name
        is AutoEqTask.Applying -> t.applied
        is AutoEqTask.Downloading -> t.applied
        AutoEqTask.Idle, is AutoEqTask.Failed -> null
    }
    val error: String? get() = (task as? AutoEqTask.Failed)?.error
}

/** While one is under way, the curve applied before it is still said. */
sealed interface AutoEqTask {
    data object Idle : AutoEqTask
    data class Downloading(val applied: String?) : AutoEqTask
    data class Applying(val applied: String?) : AutoEqTask
    data class Applied(val name: String) : AutoEqTask
    data class Failed(val error: String) : AutoEqTask
}

/**
 * One output device in the equalizer's device list: [name] is what it calls itself, [kind] where it is
 * plugged in, [sound] what it gets (a profile's name, "Flat", "Automatic", "Leave as is").
 */
data class DeviceRow(val output: String, val name: String, val kind: String?, val current: Boolean, val sound: String, val choice: DeviceSound.Choice)

/** One AutoEQ curve with the lines under it, in the app's words: [caption] in the browser, [short] in a device's sheet. */
data class AutoEqHit(val entry: dev.nori.music.ffi.model.AutoEqEntry, val caption: String, val short: String) {
    companion object {
        fun of(e: dev.nori.music.ffi.model.AutoEqEntry) = AutoEqHit(e, dev.nori.music.app.ui.say.autoeqCaption(e), dev.nori.music.app.ui.say.autoeqShort(e))
    }
}

/** A line for the snackbar about the device that just connected, with the one thing it offers to do. */
data class EqNotice(val message: String, val action: String, val source: dev.nori.music.ffi.devices.CurveNotice)

data class SyncUi(val running: Boolean = false, val indexed: IngestStats = IngestStats(0u, 0u, 0u), val error: String? = null)

/** What lives on the phone: streamed music, covers, lyrics found online, finished downloads and the library index. */
data class StorageUi(
    val streamBytes: Long = 0L,
    val coverBytes: Long = 0L,
    val lyricsBytes: Long = 0L,
    /** Every profile's downloads: they share the phone's download cache. */
    val downloadBytes: Long = 0L,
    val allDownloadSongs: Int = 0,
    /** Each profile with downloads (the id its rows are kept under) and what they hold. */
    val profileDownloads: List<Pair<String, Long>> = emptyList(),
    /** This profile's finished downloads, which its actions work on. */
    val downloadSongs: Int = 0,
    val indexBytes: Long = 0L,
    val busy: Boolean = false,
    /** Changes waiting for the server, and dropped unsent. */
    val unsentWaiting: Int = 0,
    val unsentDropped: Int = 0,
)

class SettingsViewModel(app: Application) : NoriViewModel(app) {
    val prefs: StateFlow<StoredPrefs> = nori.settings.prefs

    /** The palette worn where the season's is [palette] (at night when [dark]): a plugin's season of its own, or null for the season's. */
    suspend fun look(palette: dev.nori.music.ffi.look.Palette, dark: Boolean): dev.nori.music.ffi.look.Palette? =
        kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { nori.session.look(palette, null, dark) }.takeIf { it != palette }

    /** What the limiter is pulling back now, in dB, read on the meter's beat while collected. */
    val limiterDb = meter { dev.nori.music.playback.Equalizer.meterDb }

    /** What the compressor is taking off now, in dB, read on the meter's beat while collected. */
    val compressionDb = meter { dev.nori.music.playback.Equalizer.compressionDb }

    private fun meter(read: () -> Float) = kotlinx.coroutines.flow.flow {
        val beat = dev.nori.music.ffi.stage().meterMs
        while (true) {
            emit(read())
            kotlinx.coroutines.delay(beat)
        }
    }
    val dac: StateFlow<DacState> = nori.dac.state
    /** Adding or editing a profile: the page shown and the profile on its way (the core's `AddFlow`). */
    private val _add = MutableStateFlow(dev.nori.music.ffi.AddFlow(dev.nori.music.ffi.AddPage.Choosing, dev.nori.music.ffi.AddSend.Idle))
    val add: StateFlow<dev.nori.music.ffi.AddFlow> = _add
    private val _added = kotlinx.coroutines.flow.MutableSharedFlow<Unit>(extraBufferCapacity = 1)
    /** A profile sent was saved: the form closes. */
    val added: kotlinx.coroutines.flow.SharedFlow<Unit> = _added
    private val _sync = MutableStateFlow(SyncUi())
    val sync: StateFlow<SyncUi> = _sync

    init { viewModelScope.launch { runCatching { nori.library.indexSize() }.onSuccess { n -> _sync.update { it.copy(indexed = n) } } } }

    fun update(change: (StoredPrefs) -> StoredPrefs) = nori.settings.update(change)

    // ---- the settings screen (SettingsPages.kt), on the core's settings model (nori-settings, settings_model.rs) ----

    /** The groups the root of Settings lists, in [res]'s language. */
    fun settingsGroups(res: android.content.res.Resources): List<SettingsGroup> = dev.nori.music.app.vm.settingsGroups(res, nori.rules.settings)

    private var search: Pair<android.content.res.Resources, SettingsSearch>? = null

    /** The rows whose title (first) or words (after) contain [query]; the index is built once per [res]. */
    fun searchSettings(query: String, res: android.content.res.Resources): List<SettingsHit> {
        val s = search?.takeIf { it.first === res }?.second
            ?: SettingsSearch(res, prefs.value, nori.settings.core.settingsState(false, false), settingsFacts.value, nori.rules.settings).also { search = res to it }
        return s.find(query)
    }

    /**
     * One group's page for settings [p] and [facts], in [res]'s language: one call into the core for what
     * its rules make of the settings, asked only when they or [facts] change.
     */
    fun settingsPage(id: String, p: StoredPrefs, facts: SettingsFacts, res: android.content.res.Resources): SettingsPage? =
        settingsPage(res, id, p, facts, nori.settings.core.settingsState(facts.dac.bitPerfect, facts.dac.device != null), nori.rules.settings)

    /** Whether the settings action [action] asks first, and what it says; null for one done at once. */
    fun actionAsks(action: String, res: android.content.res.Resources): ActionAsk? = settingsActionAsks(res, action, settingsFacts.value)

    /** What a settings page depends on besides the settings, from this platform. */
    val settingsFacts: StateFlow<SettingsFacts> by lazy {
        combine(combine(dac, _sync, _storage, _analysed, _folders, ::factsOf), update, nori.plugins.names, _backup) { f, u, names, b -> f.copy(update = u, installsUpdates = nori.updates.installs, pluginNames = names, backup = b) }
            .stateIn(
                viewModelScope, SharingStarted.WhileSubscribed(5_000),
                factsOf(dac.value, _sync.value, _storage.value, _analysed.value, _folders.value).copy(update = update.value, installsUpdates = nori.updates.installs, pluginNames = nori.plugins.names.value, backup = _backup.value),
            )
    }

    private fun factsOf(d: DacState, s: SyncUi, st: StorageUi, analysed: Int, folders: List<MusicFolder>) = SettingsFacts(
        dac = d,
        analysed = analysed,
        sync = s,
        storage = st,
        folders = folders,
    )

    /** A row's setting changed: its name and the value picked, which the core reads and applies; false for a name or value it does not take. */
    fun set(name: String, value: String): Boolean {
        apply(nori.settings.core.settingSet(name, value) ?: return false)
        return true
    }

    /**
     * A ranked row (a profile, a lyrics service) picked up and dropped at place [to] of its list; whether the
     * core took it. The order itself is the core's (`profilePlace`, `lyricsPlace`).
     */
    fun placeRanked(row: SettingRow.Placed, to: Int): Boolean {
        val name = when (row) { is SettingRow.Server -> "profilePlace"; is SettingRow.Ranked -> "lyricsPlace" }
        val change = nori.settings.core.settingSet(name, "${row.id}:$to") ?: return false
        apply(change)
        return true
    }

    private fun apply(change: SettingChange) {
        // The active server's own settings go through the server's update, which connects it again.
        if (change.server) {
            change.prefs.server?.let(nori::updateServer)
            return
        }
        nori.settings.took(change)
        if ((change.effect and CACHE_LIMIT) != 0u) applyCacheLimit()
    }

    private val _backup = kotlinx.coroutines.flow.MutableStateFlow<BackupUi>(BackupUi.Idle)

    /** Saves a backup sealed with [password] into the file the user picked at [uri]. */
    fun makeBackup(uri: android.net.Uri, password: String) = backupStep(false) {
        val file = nori.plugins.backup(password)
        kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            getApplication<Application>().contentResolver.openOutputStream(uri, "wt")?.use { it.write(file) } ?: throw java.io.IOException("no stream for $uri")
        }
        BackupUi.Saved
    }

    /** Restores the backup at [uri] with [password], then takes in the settings it brought back. */
    fun restoreBackup(uri: android.net.Uri, password: String) = backupStep(true) {
        val file = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            getApplication<Application>().contentResolver.openInputStream(uri)?.use { it.readBytes() } ?: throw java.io.IOException("no stream for $uri")
        }
        val r = nori.plugins.restore(file, password)
        apply(r.settings)
        BackupUi.Restored(r.plugins)
    }

    private fun backupStep(restore: Boolean, step: suspend () -> BackupUi) = viewModelScope.launch {
        _backup.value = BackupUi.Working(restore)
        _backup.value = try {
            step()
        } catch (e: dev.nori.music.ffi.backup.BackupException) {
            BackupUi.Failed(restore, e)
        } catch (e: java.io.IOException) {
            dev.nori.music.NoriLog.w("backup: ${e.message}")
            BackupUi.Failed(restore, null)
        }
    }

    /** A button on a settings row. */
    fun act(action: String) {
        when (action) {
            "measure-again" -> clearAnalyses()
            "sync-library" -> syncLibrary()
            "download-library" -> downloadLibrary()
            "clear-stream" -> clearStreamCache()
            "clear-covers" -> clearCovers()
            "clear-lyrics" -> clearLyrics()
            "update-check" -> checkForUpdates()
            "update-go" -> updateNow()
            "update-cancel" -> cancelUpdate()
        }
    }

    // ---- the app's own updates (core update/Updates.kt over nori-core update.rs) ----

    /** Where an update stands: About's row and the banner both draw it. */
    val update: StateFlow<Updates.State> = nori.updates.state

    /** Whether this build installs updates; a debug or perf build only points at the release. */
    val installsUpdates: Boolean get() = nori.updates.installs

    fun checkForUpdates() = nori.updates.checkNow()

    /**
     * The row's or the banner's main button, whatever it says now: download and install (a release build), the
     * release's page (any other build, or a release with no APK for this phone), Android's "install unknown
     * apps" page while that is what is missing, or another try after a failure.
     */
    fun updateNow() {
        val u = nori.updates
        when (val s = update.value) {
            is Updates.State.Available -> if (u.installs) u.update(s.update) else u.openPage(s.update.page)
            is Updates.State.Failed -> if ((s.why as? Updates.Failure.Install)?.conflict == true) u.openPage(s.update.page) else u.update(s.update)
            is Updates.State.NeedsPermission -> u.allowInstalls()
            is Updates.State.NoApk -> u.openPage(s.page)
            else -> u.checkNow()
        }
    }

    /** "Later": this version is not brought up again by itself. */
    fun updateLater() = nori.updates.later()

    fun cancelUpdate() = nori.updates.cancel()

    private val _changelog = MutableStateFlow<String?>(null)
    /** The notes of the update just installed, the first time the app is opened after it. */
    val changelog: StateFlow<String?> = _changelog

    init { viewModelScope.launch(Dispatchers.IO) { _changelog.value = runCatching { nori.updates.changelog() }.getOrNull() } }

    fun changelogSeen() { _changelog.value = null }

    /** Applies "Space for streamed music" at once instead of at the next track. */
    fun applyCacheLimit() = viewModelScope.launch(Dispatchers.IO) { nori.applyCacheLimit() }

    // ---- storage ----

    private val _storage = MutableStateFlow(StorageUi())
    val storage: StateFlow<StorageUi> = _storage

    /** Measures what is on the phone; the caches answer from their index, the rest is weighed. */
    fun refreshStorage() = viewModelScope.launch(Dispatchers.IO) {
        val app = getApplication<Application>()
        // Which files are the index is the core's rule; weighing them is the file system's.
        val files = app.filesDir.listFiles().orEmpty()
        val index = storageIndexFiles(files.map { it.name })
        val unsent = runCatching { nori.client.unsent() }.getOrNull()
        val profiles = runCatching { nori.core.downloadsByProfile() }.getOrDefault(emptyList())
        _storage.value = StorageUi(
            streamBytes = nori.sources.streamBytes(),
            coverBytes = dirBytes(java.io.File(app.cacheDir, dev.nori.music.data.CoverLoader.DIR)),
            lyricsBytes = runCatching { nori.core.lyricsCacheBytes() }.getOrDefault(0L),
            downloadBytes = nori.sources.downloadBytes(),
            allDownloadSongs = profiles.sumOf { it.ids.size },
            profileDownloads = profiles.map { it.profile to nori.sources.downloadBytes(it.ids) },
            downloadSongs = nori.downloads.state.value.done.size,
            indexBytes = index.sumOf { dirBytes(files[it.toInt()]) },
            unsentWaiting = unsent?.waiting?.toInt() ?: 0,
            unsentDropped = unsent?.dropped?.toInt() ?: 0,
        )
    }

    /** Empties the streamed-music cache; downloads, covers and the index stay. */
    fun clearStreamCache() = viewModelScope.launch(Dispatchers.IO) {
        _storage.update { it.copy(busy = true) }
        nori.sources.clearStream()
        refreshStorage()
    }

    /** Empties the cover cache; pictures are fetched again as they are shown. */
    fun clearCovers() = viewModelScope.launch(Dispatchers.IO) {
        _storage.update { it.copy(busy = true) }
        dev.nori.music.data.CoverLoader.get(getApplication()).clearDisk()
        refreshStorage()
    }

    /** Forgets the lyrics found online (the core's `lyrics_cache_clear`); they are looked up again when opened. */
    fun clearLyrics() = viewModelScope.launch(Dispatchers.IO) {
        _storage.update { it.copy(busy = true) }
        runCatching { nori.core.lyricsCacheClear() }
        refreshStorage()
    }

    private fun dirBytes(f: java.io.File): Long {
        if (f.isFile) return f.length()
        return f.listFiles()?.sumOf(::dirBytes) ?: 0L
    }

    /** A blank profile for the "add server" form, with a fresh id from the core. */
    fun newProfile() = dev.nori.music.settings.newServer(newServerId())

    /** The kinds of profile that can be added here, and how each is added and signed in (the core's `Kinds::providers`). */
    val providers: StateFlow<List<dev.nori.music.ffi.ProviderInfo>> = nori.plugins.providers

    fun provider(kind: dev.nori.music.ffi.settings.ServerKind): dev.nori.music.ffi.ProviderInfo? = providers.value.firstOrNull { it.kind == kind }

    /** Installed plugins' names by id, which name their kinds of profile. */
    val pluginNames: StateFlow<Map<String, String>> = nori.plugins.names

    /** The plugins whose rows show on the devices sheet too: with any, the output button opens that sheet. */
    val pluginDevices = nori.plugins.deviceCards

    /** A plugin's cast while one runs: the devices sheet lists its device. */
    val pluginCast = nori.plugins.cast

    /** The devices plugins can cast to now, and whether one looks for them while the sheet is open. */
    val pluginTargets = nori.plugins.castTargets
    val pluginsFindOutputs = nori.plugins.findsOutputs

    /** The music goes to [target], from whatever played it. */
    fun castTo(target: dev.nori.music.ffi.plugins.CastTarget) {
        viewModelScope.launch { nori.plugins.castTo(target) }
    }

    /** The music comes back to this phone: the plugin casting ends its cast. */
    fun endCast() {
        viewModelScope.launch { nori.plugins.castButton(dev.nori.music.ffi.plugins.CastButton.STOP) }
    }

    /** The installed plugins, for a plugin kind's about. */
    val pluginCards = nori.plugins.cards

    init {
        viewModelScope.launch { nori.plugins.refresh() }
    }

    /**
     * What the profile in use waits for before it opens: its plugin (the core's `Kinds::waits`); null when it
     * opens. Asked again when a plugin is installed, turned on, or starts running (the providers offered:
     * a safe start ending runs plugins whose switches were on all along).
     */
    val profileWait: StateFlow<dev.nori.music.ffi.ProfileWait?> =
        combine(prefs.map { it.server to it.pluginsOff }.distinctUntilChanged(), nori.plugins.cards, nori.plugins.providers) { (server, _), _, _ -> server }
            .map { server -> server?.let { withContext(Dispatchers.IO) { nori.plugins.kinds.waits(it) } } }
            .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    init {
        // Its plugin running now, the profile opens again, with it.
        viewModelScope.launch {
            var waited = false
            profileWait.collect { wait ->
                if (waited && wait == null) prefs.value.server?.let(nori::activate)
                waited = wait != null
            }
        }
    }

    /** Whether the add form [p] can be sent (its provider's say). */
    suspend fun serverReady(p: SavedServer): Boolean = withContext(Dispatchers.IO) { nori.plugins.kinds.serverReady(p) }

    /** Whether a [kind] sign-in page showing [address], holding [cookies], is done (its provider's say). */
    suspend fun signInDone(kind: dev.nori.music.ffi.settings.ServerKind, address: String, cookies: String): Boolean =
        withContext(Dispatchers.IO) { nori.plugins.kinds.signInDone(kind, address, cookies) }

    private val _account = MutableStateFlow(AccountUi())
    val account: StateFlow<AccountUi> = _account

    /** The open profile's rules as its core reports them; [rules] reads them. */
    val reportedRules = nori.reportedRules

    fun rules() = nori.rules

    /**
     * Takes [event] on the account's sign-in; false for an answer not to send (one is on its way). A code
     * asked for starts a code sign-in, which runs only while the flow checks its code.
     */
    private fun signInOn(event: dev.nori.music.ffi.SignInEvent): Boolean {
        val step = dev.nori.music.ffi.signInFlowOn(_account.value.flow, event)
        _account.update { it.copy(flow = step.flow) }
        if (!step.checksCode) stopCode()
        if (event is dev.nori.music.ffi.SignInEvent.Code && step.send) startCode()
        return step.send
    }

    /** The code sign-in running while the code step shows. */
    private var code: dev.nori.music.ffi.CodeSignIn? = null

    private fun startCode() {
        stopCode()
        var mine: dev.nori.music.ffi.CodeSignIn? = null
        mine = nori.signInWithCode { event ->
            viewModelScope.launch {
                if (code !== mine) return@launch
                if (event is dev.nori.music.ffi.SignInEvent.Answered) {
                    nori.signedIn(event.result)
                    val account = nori.account()
                    _account.update { it.copy(account = account) }
                }
                signInOn(event)
            }
        }
        code = mine
    }

    private fun stopCode() {
        code?.let { it.stop(); it.close() }
        code = null
    }

    override fun onCleared() {
        stopCode()
        super.onCleared()
    }

    // A provider's plugin asks for its sign-in page (its `sign-in.open`): the page opens at that address.
    init {
        viewModelScope.launch {
            nori.plugins.signIns.collect { (_, url) -> signInOn(dev.nori.music.ffi.SignInEvent.Open(url)) }
        }
    }

    // Each time the open profile's rules change (it opened, or its account changed under it), a session the
    // provider ended offers to sign in again, until told not to (the core's `sign_in_due`).
    init {
        viewModelScope.launch {
            nori.reportedRules.collect { reported ->
                if (reported != null) nori.signInDue(null)?.let(::signInOn)
            }
        }
    }

    /** The ended session's offer no longer comes back at start. */
    fun stopReminding() {
        signInOn(dev.nori.music.ffi.SignInEvent.Close)
        viewModelScope.launch { nori.stopReminding() }
    }

    fun lookAtAccount() {
        viewModelScope.launch { _account.update { it.copy(account = runCatching { nori.account() }.getOrNull()) } }
    }

    /** Signs the profile in use in with [answer]: a sign-in page's cookies, or what was pasted. */
    fun signIn(answer: String) {
        if (!signInOn(dev.nori.music.ffi.SignInEvent.Sent)) return
        viewModelScope.launch {
            val r = nori.signIn(answer)
            val account = nori.account()
            _account.update { it.copy(account = account, flow = dev.nori.music.ffi.signInFlowOn(it.flow, dev.nori.music.ffi.SignInEvent.Answered(r)).flow) }
        }
    }

    fun signOut() {
        nori.signOut()
        _account.value = AccountUi(account = dev.nori.music.ffi.AccountState.SIGNED_OUT)
    }

    /** Opens the provider's sign-in page, from the account's row. */
    fun openSignInPage() { signInOn(dev.nori.music.ffi.SignInEvent.Open(null)) }

    /** Opens the provider's first way to sign in elsewhere (its code or its page), from the offer. */
    fun openSignIn() = viewModelScope.launch { nori.signInOpen()?.let(::signInOn) }

    /** The code step, or a new code there. */
    fun signInWithCode() { signInOn(dev.nori.music.ffi.SignInEvent.Code) }

    fun closeSignInPage() { signInOn(dev.nori.music.ffi.SignInEvent.Close) }

    /** Leaves the sign-in page's failure for a fresh page. */
    fun retrySignIn() { signInOn(dev.nori.music.ffi.SignInEvent.Retry) }

    fun skipSignIn() { signInOn(dev.nori.music.ffi.SignInEvent.Close) }

    /** Offers signing in, as a seal tapped while signed out does. */
    fun offerSignIn() = viewModelScope.launch { signInOn(nori.signInOffer()) }

    /** Takes [event] on the pages of adding a profile; says what to do next. */
    fun addOn(event: dev.nori.music.ffi.AddEvent): dev.nori.music.ffi.AddDo {
        val step = dev.nori.music.ffi.addFlowOn(_add.value, event)
        _add.value = step.flow
        return step.then
    }

    /** The form opens: a new profile chooses its kind first; [editing] a saved one shows its form. */
    fun openLogin(editing: SavedServer?) {
        addOn(if (editing == null) dev.nori.music.ffi.AddEvent.Start else dev.nori.music.ffi.AddEvent.Edit(editing.kind))
    }

    /**
     * [kind] chosen on the first screen, once its provider is listed (a plugin just turned on comes a
     * moment later). One that asks nothing is sent at once, as [profile].
     */
    fun chooseKind(kind: dev.nori.music.ffi.settings.ServerKind, profile: () -> SavedServer) {
        viewModelScope.launch {
            val info = providers.map { all -> all.firstOrNull { it.kind == kind } }.filterNotNull().first()
            if (addOn(dev.nori.music.ffi.AddEvent.Chose(info)) == dev.nori.music.ffi.AddDo.Connect) login(profile())
        }
    }

    fun login(profile: SavedServer, afterAdding: dev.nori.music.ffi.AfterAdding = dev.nori.music.ffi.AfterAdding.REMIND) {
        if (_add.value.send is dev.nori.music.ffi.AddSend.Sending) return
        viewModelScope.launch {
            val (ready, adds) = withContext(Dispatchers.IO) {
                nori.plugins.kinds.serverReady(profile) to nori.plugins.kinds.addsProfile(prefs.value.serverList(), profile)
            }
            if (addOn(dev.nori.music.ffi.AddEvent.Send(ready, afterAdding.takeIf { adds })) !is dev.nori.music.ffi.AddDo.Send) return@launch
            try {
                nori.login(profile)
                val opened = addOn(dev.nori.music.ffi.AddEvent.Accepted) as? dev.nori.music.ffi.AddDo.Open
                val added = opened?.added
                // A new profile, in use now: the account shown so far was the last one's.
                if (added != null) _account.value = AccountUi()
                nori.signInDue(added)?.let(::signInOn)
                _added.tryEmit(Unit)
            } catch (e: Exception) {
                addOn(dev.nori.music.ffi.AddEvent.Refused(describeConnectionError(e)))
            }
        }
    }

    fun switchServer(profile: SavedServer) = nori.activate(profile)
    fun updateServer(profile: SavedServer) = nori.updateServer(profile)
    fun setProfileIcon(profile: SavedServer, icon: dev.nori.music.ffi.settings.ProfileIcon) = nori.setProfileIcon(profile, icon)
    fun removeServer(id: String) = nori.removeServer(id)

    /** Copies a picked PKCS#12 file into the app and returns the profile that uses it. */
    fun importClientCert(profile: SavedServer, uri: android.net.Uri, password: String): SavedServer {
        val name = "${profile.id}.p12"
        val dir = java.io.File(getApplication<Application>().filesDir, "certs").apply { mkdirs() }
        getApplication<Application>().contentResolver.openInputStream(uri)?.use { input -> java.io.File(dir, name).outputStream().use { input.copyTo(it) } }
        return profile.copy(clientCert = name, clientCertPassword = password)
    }

    private val _folders = MutableStateFlow<List<MusicFolder>>(emptyList())
    val musicFolders: StateFlow<List<MusicFolder>> = _folders
    fun loadMusicFolders() = viewModelScope.launch { runCatching { nori.library.musicFolders() }.onSuccess { _folders.value = it } }

    fun logout() = nori.logout()

    // ---- equalizer ----

    /** Moves one home shelf up or down the page (the core's `home_rows_moved`). */
    fun moveHomeRow(from: Int, to: Int) = update { p ->
        p.copy(homeRows = dev.nori.music.ffi.library.homeRowsMoved(p.homeRows, from.toUInt(), to.toUInt()))
    }

    /**
     * A report of a problem (core `Report`) under [header], written to a file of its own - a day of log is a
     * megabyte or two, more than a share's text or the clipboard carries - and handed to [share] as the
     * address others may read it at; null when it could not be written. Older reports go.
     */
    fun reportProblem(header: String, share: (android.net.Uri?) -> Unit) {
        val app = getApplication<Application>()
        val position = nori.player.positionMs
        viewModelScope.launch {
            val uri = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
                runCatching {
                    val dir = app.cacheDir.resolve("reports").apply { deleteRecursively(); mkdirs() }
                    val stamp = java.time.LocalDateTime.now().format(java.time.format.DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss"))
                    val file = dir.resolve("nori-report-$stamp.txt")
                    file.writeText(dev.nori.music.Report.write(app, header, position))
                    androidx.core.content.FileProvider.getUriForFile(app, "${app.packageName}.reports", file)
                }.getOrNull()
            }
            share(uri)
        }
    }

    /** The built-in curves, straight from the core so the numbers live in one place. */
    val presets: List<NamedPreset> by lazy { eqPresets() }

    // The edits themselves (a preset's pre-amp of 0 meaning automatic, the new band's defaults, the
    // graphic bands coming back when the last one goes) are the core's (settings.rs), made where it keeps
    // the settings (settings_store::settings_sound_tool).
    fun applyPreset(p: NamedPreset) { nori.settings.soundTool(SoundTool.Preset(p)) }

    /** One band changed, held in the equalizer's ranges by the core; on every step of a drag, so edited in place there. */
    fun setBand(index: Int, band: SoundBand) = nori.settings.setBand(index, band)

    /** Pre-amp, balance, limiter ceiling or volume boost moved; the core holds it in range and snaps it, in place like a band. */
    fun setLevel(level: EqLevel, value: Float) = nori.settings.setLevel(level, value)

    /** One graphic equalizer slider moved; in place in the core, like a band. */
    fun setGraphic(index: Int, value: Float) = nori.settings.setGraphic(index, value)

    /** The graphic or the parametric equalizer plays; each keeps its own settings. */
    fun setEqMode(mode: dev.nori.music.ffi.settings.EqMode) { set("eqMode", mode.name) }

    /** How many graphic bands (5, 10, 15 or 31): the core draws the same curve on the new layout. */
    fun setEqLayout(count: Int) { set("eqLayout", count.toString()) }

    /** The automatic pre-amp on or off; off starts from the level it was at. */
    fun setAutoPreamp(automatic: Boolean) { nori.settings.soundTool(SoundTool.AutoPreamp(automatic)) }
    fun addBand(freq: Float, gainDb: Float) { nori.settings.soundTool(SoundTool.AddBand(freq, gainDb)) }
    fun removeBand(index: Int) { nori.settings.soundTool(SoundTool.RemoveBand(index.toUInt())) }
    fun resetBands() { nori.settings.soundTool(SoundTool.ResetBands) }

    /** The sound pages in sight (`nori_core::tuning`): one coming in sight can start before the one it covers stops. */
    private val tuning = dev.nori.music.ffi.queue.SoundTuning()
    private var touchWatch: Job? = null

    /**
     * Sound page [page] (the equalizer, AutoEQ, the sound settings, a plugin's) came in sight ([on]) or left
     * it. The core says when the deep buffer is traded for one heard at once and back, and when to watch
     * for a setting or a plugin's layer changing.
     */
    fun soundPageShown(page: dev.nori.music.ffi.queue.SoundPage, on: Boolean) = tuned(if (on) tuning.shown(page) else tuning.left())

    private fun tuned(step: dev.nori.music.ffi.queue.TuningStep) {
        step.set?.let { nori.player.tuning(it == dev.nori.music.ffi.queue.Buffer.SHALLOW) }
        if (!step.watch) {
            touchWatch?.cancel(); touchWatch = null
        } else if (touchWatch == null) {
            touchWatch = viewModelScope.launch {
                merge(prefs.drop(1).map { }, nori.settings.effects.map { }).first()
                touchWatch = null
                tuned(tuning.edited())
            }
        }
    }

    /** AutoEQ "ParametricEQ.txt" / Equalizer APO text. Returns how many filters were found. */
    fun importPreset(text: String): Int = try { import(text) } catch (e: SoundException) { 0 }

    /** Switches the preset in [text] on; throws, saying why, when it has no filters in it. */
    private fun import(text: String): Int = nori.settings.soundTool(SoundTool.Import(text))

    // ---- saved profiles and the AutoEQ database ----

    private val devices = nori.deviceSound
    val profiles: StateFlow<List<SoundProfile>> = devices.profiles
    val currentOutput: StateFlow<String> = nori.outputs.current

    init { refreshProfiles() }
    private fun refreshProfiles() = viewModelScope.launch { devices.refresh() }

    /** Every output seen, the one playing now first, each with the sound it gets. */
    val deviceRows: StateFlow<List<DeviceRow>> = combine(nori.outputs.known, currentOutput, profiles, devices.quiet, ::deviceRowsOf)
        // Filled from the start, so the list is there on the screen's first frame rather than popping in.
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), deviceRowsOf(nori.outputs.known.value, currentOutput.value, profiles.value, devices.quiet.value))

    /** The rows are the core's (nori_player::device::rows); their words are Say's, made once per change of the list. */
    private fun deviceRowsOf(known: List<String>, current: String, profiles: List<SoundProfile>, quiet: List<String>): List<DeviceRow> =
        deviceRows(known, current, profiles, quiet).map { r ->
            val choice = when (r.choice) {
                ChoiceKind.AUTOMATIC -> DeviceSound.Choice.Automatic
                ChoiceKind.QUIET -> DeviceSound.Choice.Quiet
                ChoiceKind.FLAT -> DeviceSound.Choice.Flat
                ChoiceKind.BYPASS -> DeviceSound.Choice.Bypass
                ChoiceKind.PROFILE -> DeviceSound.Choice.Profile(r.profile.orEmpty())
            }
            val say = dev.nori.music.app.ui.say
            val sound = when (r.choice) {
                ChoiceKind.AUTOMATIC -> say.automatic
                ChoiceKind.QUIET -> say.leaveAsIs
                ChoiceKind.FLAT -> say.flat
                ChoiceKind.BYPASS -> say.noProcessing
                ChoiceKind.PROFILE -> r.profile.orEmpty()
            }
            DeviceRow(r.output, say.outputName(r.port, r.name), say.outputKind(r.port), r.current, sound, choice)
        }

    private val _assigning = MutableStateFlow<String?>(null)
    /** The device whose sound is being fetched and saved right now (an AutoEQ curve is a download). */
    val assigning: StateFlow<String?> = _assigning
    private val _assignError = MutableStateFlow<String?>(null)
    val assignError: StateFlow<String?> = _assignError

    /** Gives [output] its own sound; [onDone] runs once it is saved (and loaded, if that device is playing). */
    fun assignDevice(output: String, choice: DeviceSound.Choice, onDone: () -> Unit = {}) = viewModelScope.launch {
        _assigning.value = output
        _assignError.value = null
        try {
            devices.assign(output, choice, currentOutput.value)
            onDone()
        } catch (e: Exception) {
            _assignError.value = describeConnectionError(e)
        } finally {
            _assigning.value = null
        }
    }

    fun clearAssignError() { _assignError.value = null }

    /** Takes a device out of the list, with whatever was chosen for it. */
    fun forgetDevice(output: String) = viewModelScope.launch {
        devices.forget(output)
        nori.outputs.forget(output)
    }

    /** What to say about the device that just connected; only while it is still the one playing. */
    val eqNotice: StateFlow<EqNotice?> = combine(devices.notice, currentOutput) { n, current ->
        // Only about the device still playing; the words are Say's.
        val (output, text) = when (n) {
            is dev.nori.music.ffi.devices.CurveNotice.Offer -> n.output to dev.nori.music.app.ui.say.deviceNotice(true, n.entry.name)
            is dev.nori.music.ffi.devices.CurveNotice.Applied -> n.output to dev.nori.music.app.ui.say.deviceNotice(false, n.curve)
            null -> null to null
        }
        if (n == null || text == null || output != current) null else EqNotice(text.first, text.second, n)
    }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /** The notice is on screen now, so it is not shown again. */
    fun eqNoticeShown(n: EqNotice) = devices.consume(n.source)

    /** "Apply" on an offer, "Undo" on a curve applied without asking. */
    fun eqNoticeAction(n: EqNotice) = viewModelScope.launch {
        when (val src = n.source) {
            is dev.nori.music.ffi.devices.CurveNotice.Offer -> try {
                devices.accept(src)
            } catch (e: Exception) {
                _autoEq.update { it.copy(task = AutoEqTask.Failed(describeConnectionError(e))) }
            }
            is dev.nori.music.ffi.devices.CurveNotice.Applied -> devices.undo(src)
        }
    }

    /** Saves the sound settings as they are now under [name]. */
    fun saveProfile(name: String) = viewModelScope.launch {
        val sound = prefs.value.sound()
        // Saving over a profile keeps the devices it is chosen for (the core's profile_save_sound).
        withContext(Dispatchers.IO) { runCatching { nori.core.profileSaveSound(name, sound) } }
        refreshProfiles()
    }

    fun applyProfile(p: SoundProfile) = soundFromJson(p.json)?.let { s -> update { it.withSound(s) } }
    fun deleteProfile(name: String) = viewModelScope.launch {
        withContext(Dispatchers.IO) { runCatching { nori.core.profileDelete(name) } }
        refreshProfiles()
    }

    private val _autoEq = MutableStateFlow(AutoEqUi())
    val autoEq: StateFlow<AutoEqUi> = _autoEq

    init { viewModelScope.launch(Dispatchers.IO) { val n = runCatching { nori.core.autoeqCount() }.getOrDefault(0u); _autoEq.update { it.counted(n) } } }

    private fun AutoEqUi.counted(n: UInt) = copy(count = n.toInt(), countWords = dev.nori.music.app.ui.say.autoeqCount(n.toInt()), searchWords = dev.nori.music.app.ui.say.autoeqSearch(n.toInt()))

    /**
     * Downloads the AutoEQ index (850 kB) now so searching is local afterwards; the core otherwise keeps
     * it by itself on Wi-Fi (Nori.keepAutoEqList).
     */
    fun downloadAutoEqIndex() = viewModelScope.launch {
        // Asked for by name, with a button: that is the consent, on any network. The lookups switch and
        // "Keep the AutoEQ list" are for what the app fetches on its own, not for a download the user started.
        _autoEq.update { it.copy(task = AutoEqTask.Downloading(it.applied)) }
        try {
            val n = withContext(Dispatchers.IO) { nori.client.autoeqUpdate(true, nori.http.metered) }
            _autoEq.value = AutoEqUi().counted(n ?: withContext(Dispatchers.IO) { nori.core.autoeqCount() })
        } catch (e: Exception) {
            // The list held before stays.
            _autoEq.update { it.copy(task = AutoEqTask.Failed(describeConnectionError(e))) }
        }
    }

    fun searchAutoEq(query: String) = viewModelScope.launch {
        _autoEq.update { it.copy(query = query) }
        // Too short a query, the limit and the lines under each hit are the core's (autoeq_browse).
        val found = withContext(Dispatchers.IO) { runCatching { nori.core.autoeqBrowse(query) }.getOrNull() }
        _autoEq.update { if (it.query == query) it.copy(hits = found?.hits.orEmpty().map(AutoEqHit::of), tooShort = found?.tooShort ?: true) else it }
    }

    /**
     * What an output device might be in the AutoEQ database, best first: "Bluetooth: LE_WH-1000XM5" finds
     * "Sony WH-1000XM5". Empty when the index is not downloaded or the name says nothing (the speaker, a
     * generic "USB Audio").
     */
    suspend fun autoEqFor(output: String): List<AutoEqHit> = devices.curvesFor(output).map(AutoEqHit::of)

    /** Fetches one headphone's parametric preset and makes it the current curve. */
    fun applyAutoEq(entry: AutoEqEntry) = viewModelScope.launch {
        _autoEq.update { it.copy(task = AutoEqTask.Applying(it.applied)) }
        try {
            // The parametric preset, or the graphic curve fitted with parametric filters when that is all
            // AutoEQ has (the core's autoeq_curve and parse_eq_preset); null when it has neither.
            val text = withContext(Dispatchers.IO) { nori.client.autoeqCurve(entry) }
            if (text == null) {
                // The core has taken it out of the list: the search and the count say so at once.
                val n = withContext(Dispatchers.IO) { runCatching { nori.core.autoeqCount() }.getOrDefault(0u) }
                _autoEq.update { it.counted(n).copy(task = AutoEqTask.Failed(dev.nori.music.app.ui.say.autoeqNoCurve)) }
                searchAutoEq(_autoEq.value.query)
                return@launch
            }
            withContext(Dispatchers.IO) { import(text) }
            _autoEq.update { it.copy(task = AutoEqTask.Applied(entry.name)) }
        } catch (e: Exception) {
            _autoEq.update { it.copy(task = AutoEqTask.Failed(describeConnectionError(e))) }
        }
    }

    private val _analysed = MutableStateFlow(0)
    /** How many tracks AutoMix has measured. */
    val analysed: StateFlow<Int> = _analysed

    fun refreshAnalysed() = viewModelScope.launch {
        _analysed.value = withContext(Dispatchers.IO) { runCatching { nori.core.analysisCount() }.getOrDefault(0u).toInt() }
    }

    fun clearAnalyses() = viewModelScope.launch {
        withContext(Dispatchers.IO) { runCatching { nori.core.measureAgain() } }.getOrNull()?.let { nori.settings.asked(it.effect) }
        refreshAnalysed()
    }

    // ---- downloads ----

    /** Queues every song of the offline index for download; run [syncLibrary] first so the index is complete. */
    fun downloadLibrary() = nori.downloads.downloadLibrary()

    /** Fills the offline search index with the whole library. Optional: the app works without it. */
    fun syncLibrary() {
        if (_sync.value.running) return
        _sync.update { it.copy(running = true, error = null) }
        viewModelScope.launch {
            try {
                nori.library.sync()
                _sync.value = SyncUi(indexed = nori.library.indexSize())
            } catch (e: Exception) {
                _sync.update { it.copy(running = false, error = e.said) }
            }
        }
    }
}
