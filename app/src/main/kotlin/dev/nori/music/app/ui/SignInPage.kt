package dev.nori.music.app.ui

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebStorage
import android.webkit.WebView
import android.webkit.WebViewClient
import dev.nori.music.app.R
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.nori.music.app.vm.SettingsViewModel
import dev.nori.music.ffi.SignIn
import kotlinx.coroutines.launch
import dev.nori.music.ffi.settings.ServerKind

/**
 * Signing the account of a [kind] profile in on its provider's own page (the core's `SignIn::WebPage`):
 * the offer a just-added profile makes, and the page. Shown over everything while the view model says
 * so; nothing for a kind without such a page.
 */
@Composable
fun SignInFlow(vm: SettingsViewModel, kind: ServerKind) {
    val ui by vm.account.collectAsStateWithLifecycle()
    val providers by vm.providers.collectAsStateWithLifecycle()
    val words = providerWords(vm, kind).account
    val start = providers.firstOrNull { it.kind == kind }?.signIn?.firstNotNullOfOrNull { (it as? SignIn.WebPage)?.startUrl } ?: return
    NoriDialog(ui.offer, vm::skipSignIn) {
        AlertCard(
            confirmButton = { TextAction(words.pageButton, vm::openSignInPage, Modifier.testTag("offer-sign-in")) },
            dismissButton = {
                Row {
                    if (ui.ended) TextAction(say.word(R.string.dont_show_again), vm::stopReminding, Modifier.testTag("offer-quiet"))
                    TextAction(say.notNow, vm::skipSignIn, Modifier.testTag("offer-skip"))
                }
            },
            title = { Text(words.offerTitle) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(if (ui.ended) words.signedOutAgain else words.offerText)
                    Text(words.risk, style = MaterialTheme.typography.bodySmall)
                }
            },
        )
    }
    NoriDialog(ui.page, vm::closeSignInPage, DialogStyle.Page) { SignInSheet(vm, kind, ui.pageStart ?: start, words) }
}

/** The sign-in page as a sheet: Cancel and the title over the provider's page, then the core's answer. */
@Composable
private fun SignInSheet(vm: SettingsViewModel, kind: ServerKind, start: String, words: AccountWords) {
    val ui by vm.account.collectAsStateWithLifecycle()
    var loading by remember { mutableStateOf(true) }
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.systemBarsPadding()) {
            Box(Modifier.fillMaxWidth().height(52.dp)) {
                TextAction(say.cancel, vm::closeSignInPage, Modifier.align(Alignment.CenterStart).padding(start = 4.dp).testTag("sign-in-cancel"))
                Text(
                    words.pageTitle, Modifier.align(Alignment.Center),
                    style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold), maxLines = 1,
                )
                if (loading && !ui.busy && ui.failed == null) LoadingDots(Modifier.align(Alignment.CenterEnd).padding(end = Space.gutter), dot = 5.dp)
            }
            Hairline(0.dp)
            val failed = ui.failed
            when {
                ui.busy -> Centered {
                    LoadingDots()
                    Text(say.signingIn, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                failed != null -> Centered {
                    Text(say.signInPage(words, failed), textAlign = TextAlign.Center, modifier = Modifier.testTag("page-failed"))
                    PillButton(say.tryAgain, null, vm::retrySignIn, prominent = true)
                }
                else -> WebSignIn({ url, cookies -> vm.signInDone(kind, url, cookies) }, start, { loading = it }, vm::signIn, Modifier.weight(1f).fillMaxWidth())
            }
        }
    }
}

@Composable
private fun ColumnScope.Centered(content: @Composable ColumnScope.() -> Unit) {
    Column(
        Modifier.weight(1f).fillMaxWidth().padding(horizontal = Space.gutter * 2),
        Arrangement.spacedBy(16.dp, Alignment.CenterVertically), Alignment.CenterHorizontally, content,
    )
}

/**
 * A provider's sign-in page, opened at [start], in a WebView of its own: no JavaScript bridge, nothing
 * but https, no other WebView in the app to share its cookies with. Once the core says the page is done
 * (`sign_in_done`, from the address shown or refused and its cookies), its Cookie header goes to [signedIn]. Every
 * cookie and all storage are cleared before it opens, as soon as the cookies are taken, and when it
 * leaves (Cancel included): a WebView keeping the session could rotate it behind nori's back and end
 * nori's copy.
 */
@SuppressLint("SetJavaScriptEnabled")
@Composable
private fun WebSignIn(done: suspend (String, String) -> Boolean, start: String, loading: (Boolean) -> Unit, signedIn: (String) -> Unit, modifier: Modifier) {
    val onLoading by rememberUpdatedState(loading)
    val onSignedIn by rememberUpdatedState(signedIn)
    val isDone by rememberUpdatedState(done)
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    DisposableEffect(Unit) { onDispose { forgetWebSession() } }
    AndroidView(
        factory = { context ->
            WebView(context).apply {
                settings.javaScriptEnabled = true
                settings.domStorageEnabled = true
                settings.allowFileAccess = false
                settings.allowContentAccess = false
                settings.setSupportMultipleWindows(false)
                CookieManager.getInstance().setAcceptCookie(true)
                webViewClient = object : WebViewClient() {
                    var taken = false

                    // Web pages only: an intent:// or app link opens nothing. Its address is still looked at, as a
                    // sign-in may end there (an OAuth redirect to an app's own address carries its code).
                    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                        if (request.url.scheme == "https") return false
                        look(view, request.url.toString())
                        return true
                    }

                    override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) = onLoading(true)

                    override fun onPageFinished(view: WebView, url: String) {
                        onLoading(false)
                        look(view, url)
                    }

                    override fun doUpdateVisitedHistory(view: WebView, url: String, isReload: Boolean) = look(view, url)

                    // The provider answers off the main thread; the page goes on meanwhile.
                    fun look(view: WebView, url: String) {
                        if (taken) return
                        val cookies = CookieManager.getInstance().getCookie(url).orEmpty()
                        scope.launch {
                            if (taken || !isDone(url, cookies)) return@launch
                            taken = true
                            view.stopLoading()
                            forgetWebSession()
                            onSignedIn(cookies)
                        }
                    }
                }
                forgetWebSession { loadUrl(start) }
            }
        },
        modifier = modifier,
        onRelease = { view ->
            view.stopLoading()
            view.clearHistory()
            view.clearCache(true)
            view.clearFormData()
            view.destroy()
        },
    )
}

/** Clears the sign-in WebView's cookies and storage, then [then]. */
private fun forgetWebSession(then: () -> Unit = {}) {
    WebStorage.getInstance().deleteAllData()
    val cookies = CookieManager.getInstance()
    cookies.removeAllCookies {
        cookies.flush()
        then()
    }
}
