package dev.nori.music.app.vm

import android.app.Application
import androidx.lifecycle.viewModelScope
import dev.nori.music.ffi.model.HistoryEntry
import dev.nori.music.ffi.library.HistoryAfter
import dev.nori.music.ffi.library.SmartPage
import dev.nori.music.ffi.library.ServerHistory
import dev.nori.music.ffi.library.StatsPage
import dev.nori.music.ffi.library.StatsSource
import dev.nori.music.ffi.library.SmartEdit
import dev.nori.music.ffi.model.SmartPlaylist
import dev.nori.music.ffi.model.SmartProblem
import dev.nori.music.app.R
import dev.nori.music.ffi.library.smartEditPrepare
import dev.nori.music.app.ui.Note
import dev.nori.music.app.ui.say
import dev.nori.music.net.said
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// The editor's form is the core's `SmartEdit`: what a new one holds, how a rule changes and how it is
// written and read back are all smart/draft.rs.

class SmartViewModel(app: Application) : NoriViewModel(app) {
    private val _saved = MutableStateFlow<List<SmartPlaylist>>(emptyList())
    val saved: StateFlow<List<SmartPlaylist>> = _saved
    val defaults: List<SmartPlaylist> = nori.library.smartDefaults()
    private val _songs = MutableStateFlow<Load<SmartPage>>(Load.Loading)
    /** The playlist's songs and their caption, both the core's. */
    val songs: StateFlow<Load<SmartPage>> = _songs

    init { refresh() }
    private fun refresh() = viewModelScope.launch { _saved.value = runCatching { nori.library.smartPlaylists() }.getOrDefault(emptyList()) }

    fun find(id: String): SmartPlaylist? = (_saved.value + defaults).firstOrNull { it.id == id }

    fun open(json: String) = viewModelScope.launch {
        _songs.value = Load.Loading
        _songs.value = runCatching { Load.Ready(nori.library.smartPage(json)) }.getOrElse { Load.Failed(it.said ?: say.note(Note.COULD_NOT_EVALUATE)) }
    }

    /** Null when saved; otherwise what is wrong with the definition. */
    fun save(draft: SmartEdit, onSaved: (String) -> Unit): String? {
        val ready = smartEditPrepare(draft)
        ready.error?.let { return problem(it) }
        viewModelScope.launch { val id = nori.library.smartSave(ready.id, ready.name.ifEmpty { say.smartPlaylist }, ready.json); refresh(); onSaved(id) }
        return null
    }

    fun delete(id: String) = viewModelScope.launch { nori.library.smartDelete(id); refresh() }

    /** What is wrong with the rules, in words. */
    private fun problem(p: SmartProblem): String = getApplication<Application>().getString(
        when (p) {
            SmartProblem.NO_VALUE -> R.string.smart_problem_no_value
            SmartProblem.NOT_NUMBER -> R.string.smart_problem_not_number
            SmartProblem.NOT_DATE -> R.string.smart_problem_not_date
            SmartProblem.DAYS_OUT_OF_RANGE -> R.string.smart_problem_days
            SmartProblem.BACKWARDS -> R.string.smart_problem_backwards
            SmartProblem.NEGATIVE -> R.string.smart_problem_negative
            else -> R.string.smart_problem_other
        },
    )
}

class HistoryViewModel(app: Application) : NoriViewModel(app) {
    private val _entries = MutableStateFlow(Grown.empty<HistoryEntry>())
    val entries: StateFlow<List<HistoryEntry>> = _entries
    private val _stats = MutableStateFlow<StatsPage?>(null)
    /** The stats with their words and tiles, all the core's. */
    val stats: StateFlow<StatsPage?> = _stats
    private var exhausted = false
    private var job: Job? = null
    /** Where the next page starts. */
    private var after: HistoryAfter? = null

    init { loadMore() }

    fun loadMore() {
        if (exhausted || job?.isActive == true) return
        job = viewModelScope.launch {
            // A page that could not be read ends the list, as an empty one would.
            val next = runCatching { withContext(Dispatchers.IO) { nori.core.historyPage(after) } }.getOrNull()
            // runCatching also caught a cancellation: a clear while the page was being read drops it.
            ensureActive()
            after = next?.next
            exhausted = after == null
            _entries.value = _entries.value.plus(next?.entries.orEmpty())
        }
    }

    private val _server = MutableStateFlow<ServerHistory?>(null)
    /** How the server account's history stood when its new plays were asked for; null before. */
    val server: StateFlow<ServerHistory?> = _server

    /** [days] 0 means everything. The server's new plays are asked for once, when they are first shown. */
    fun loadStats(days: Int, source: StatsSource) = viewModelScope.launch {
        if (source == StatsSource.SERVER_ACCOUNT && _server.value == null) _server.value = nori.client.serverHistorySync()
        val tzOffset = java.util.TimeZone.getDefault().getOffset(System.currentTimeMillis())
        _stats.value = runCatching { withContext(Dispatchers.IO) { nori.core.statsPage(days.toUInt(), source, tzOffset) } }.getOrNull()
    }

    fun clear() = viewModelScope.launch { job?.cancel(); nori.library.clearHistory(); _entries.value = Grown.empty(); exhausted = true }
}
