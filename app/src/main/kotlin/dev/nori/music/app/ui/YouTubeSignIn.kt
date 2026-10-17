package dev.nori.music.app.ui

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebStorage
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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

/**
 * Signing a YouTube Music profile in with Google: the offer a just-added profile makes, and Google's
 * sign-in page. Shown over everything while the view model says so.
 */
@Composable
fun YouTubeSignInFlow(vm: SettingsViewModel) {
    val yt by vm.youtube.collectAsStateWithLifecycle()
    NoriDialog(yt.offer, vm::skipYoutubeSignIn) {
        AlertCard(
            confirmButton = { TextButton(vm::openYoutubeSignIn, Modifier.testTag("youtube-offer-sign-in")) { Text(say.youtubeSignInGoogle) } },
            dismissButton = { TextButton(vm::skipYoutubeSignIn, Modifier.testTag("youtube-offer-skip")) { Text(say.youtubeNotNow) } },
            title = { Text(say.youtubeOfferTitle) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(say.youtubeOfferText)
                    Text(say.youtubeAccountRisk, style = MaterialTheme.typography.bodySmall)
                }
            },
        )
    }
    NoriDialog(yt.page, vm::closeYoutubeSignIn, DialogStyle.Page) { SignInPage(vm) }
}

/** The sign-in page as a sheet: Cancel and the title over Google's page, then the core's answer. */
@Composable
private fun SignInPage(vm: SettingsViewModel) {
    val yt by vm.youtube.collectAsStateWithLifecycle()
    var loading by remember { mutableStateOf(true) }
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.systemBarsPadding()) {
            Box(Modifier.fillMaxWidth().height(52.dp)) {
                TextButton(vm::closeYoutubeSignIn, Modifier.align(Alignment.CenterStart).padding(start = 4.dp).testTag("youtube-sign-in-cancel")) { Text(say.cancel) }
                Text(
                    say.youtubeSignInTitle, Modifier.align(Alignment.Center),
                    style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold), maxLines = 1,
                )
                if (loading && !yt.busy && yt.failed == null) LoadingDots(Modifier.align(Alignment.CenterEnd).padding(end = Space.gutter), dot = 5.dp)
            }
            Hairline(0.dp)
            val failed = yt.failed
            when {
                yt.busy -> Centered {
                    LoadingDots()
                    Text(say.youtubeSigningIn, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                failed != null -> Centered {
                    Text(say.youtubeSignInPage(failed), textAlign = TextAlign.Center, modifier = Modifier.testTag("youtube-page-failed"))
                    PillButton(say.youtubeTryAgain, null, vm::retryYoutubeSignIn, prominent = true)
                }
                else -> GoogleSignIn({ loading = it }, vm::youtubeSignIn, Modifier.weight(1f).fillMaxWidth())
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
 * Google's sign-in page in a WebView of its own: no JavaScript bridge, nothing but https, no other
 * WebView in the app to share its cookies with. Once the page is back on music.youtube.com signed in
 * (the core's `youtube_sign_in_done`), its Cookie header goes to [signedIn]. Every cookie and all
 * storage are cleared before it opens, as soon as the cookies are taken, and when it leaves (Cancel
 * included): a WebView keeping the session would rotate it behind nori's back and end nori's copy.
 */
@SuppressLint("SetJavaScriptEnabled")
@Composable
private fun GoogleSignIn(loading: (Boolean) -> Unit, signedIn: (String) -> Unit, modifier: Modifier) {
    val onLoading by rememberUpdatedState(loading)
    val onSignedIn by rememberUpdatedState(signedIn)
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

                    // Google's pages only: an intent:// or app link opens nothing.
                    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest) = request.url.scheme != "https"

                    override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) = onLoading(true)

                    override fun onPageFinished(view: WebView, url: String) {
                        onLoading(false)
                        look(view, url)
                    }

                    override fun doUpdateVisitedHistory(view: WebView, url: String, isReload: Boolean) = look(view, url)

                    fun look(view: WebView, url: String) {
                        if (taken) return
                        val cookies = CookieManager.getInstance().getCookie(url) ?: return
                        if (!dev.nori.music.ffi.youtubeSignInDone(url, cookies)) return
                        taken = true
                        view.stopLoading()
                        forgetWebSession()
                        onSignedIn(cookies)
                    }
                }
                forgetWebSession { loadUrl(dev.nori.music.ffi.youtubeSignInPage()) }
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
