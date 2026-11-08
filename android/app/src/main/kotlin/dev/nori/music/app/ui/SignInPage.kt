package dev.nori.music.app.ui

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.os.Message
import android.webkit.WebChromeClient
import android.widget.FrameLayout
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
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.nori.music.app.vm.SettingsViewModel
import dev.nori.music.ffi.SignIn
import kotlinx.coroutines.launch
import dev.nori.music.ffi.settings.ServerKind

/**
 * Signing the account of a [kind] profile in elsewhere: on its provider's own page (the core's
 * `SignIn::WebPage`) or with a code allowed in a browser (`SignIn::Code`). The offer a just-added profile
 * makes, the page and the code, shown over everything while the view model says so; nothing for a kind
 * signing in neither way.
 */
@Composable
fun SignInFlow(vm: SettingsViewModel, kind: ServerKind) {
    val ui by vm.account.collectAsStateWithLifecycle()
    val providers by vm.providers.collectAsStateWithLifecycle()
    val words = providerWords(vm, kind).account
    val ways = providers.firstOrNull { it.kind == kind }?.signIn.orEmpty()
    val start = ways.firstNotNullOfOrNull { (it as? SignIn.WebPage)?.startUrl }
    if (start == null && SignIn.Code !in ways) return
    NoriDialog(ui.flow.shown as? dev.nori.music.ffi.SignInShown.Offered, vm::skipSignIn) { offered ->
        AlertCard(
            confirmButton = { TextAction(words.pageButton, { vm.openSignIn() }, Modifier.testTag("offer-sign-in")) },
            dismissButton = {
                Row {
                    if (offered.ended) TextAction(say.word(R.string.dont_show_again), vm::stopReminding, Modifier.testTag("offer-quiet"))
                    TextAction(say.notNow, vm::skipSignIn, Modifier.testTag("offer-skip"))
                }
            },
            title = { Text(words.offerTitle) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(if (offered.ended) words.signedOutAgain else words.offerText)
                    Text(words.risk, style = MaterialTheme.typography.bodySmall)
                }
            },
        )
    }
    NoriDialog(ui.flow.shown as? dev.nori.music.ffi.SignInShown.Page, vm::closeSignInPage, DialogStyle.Page) { page ->
        (page.start ?: start)?.let { SignInSheet(vm, kind, it, words) }
    }
    NoriDialog(ui.flow.shown as? dev.nori.music.ffi.SignInShown.Code, vm::closeSignInPage) { shown -> CodeCard(vm, shown, ui.failed, words) }
}

/**
 * The code step: the code large, a button opening its page in the system's browser and one copying it,
 * and whether it waits, expired or failed (a new code then).
 */
@Composable
private fun CodeCard(vm: SettingsViewModel, shown: dev.nori.music.ffi.SignInShown.Code, failed: dev.nori.music.ffi.SignInResult?, words: AccountWords) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val code = shown.code
    val renew = shown.expired || failed != null
    AlertCard(
        confirmButton = { TextAction(say.cancel, vm::closeSignInPage, Modifier.testTag("code-cancel")) },
        title = { Text(words.pageButton) },
        text = {
            Column(Modifier.fillMaxWidth(), Arrangement.spacedBy(16.dp), Alignment.CenterHorizontally) {
                Text(words.code.how, textAlign = TextAlign.Center)
                if (code == null) {
                    LoadingDots(Modifier.testTag("code-asking"))
                    Text(words.code.asking, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    return@Column
                }
                Text(
                    code.code, Modifier.testTag("sign-in-code"),
                    style = MaterialTheme.typography.headlineMedium.copy(fontWeight = FontWeight.SemiBold, letterSpacing = 4.sp),
                    color = if (renew) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                )
                when {
                    failed != null -> Text(say.signInCode(words, failed), color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center, modifier = Modifier.testTag("code-failed"))
                    shown.expired -> Text(words.code.expired, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("code-expired"))
                }
                if (renew) {
                    PillButton(words.code.renew, Iconoir.Refresh, vm::signInWithCode, Modifier.fillMaxWidth().testTag("code-renew"), prominent = true)
                    return@Column
                }
                PillButton(words.code.open, Iconoir.ShareIos, {
                    runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(code.url))) }
                }, Modifier.fillMaxWidth().testTag("code-open"), prominent = true)
                PillButton(words.code.copy, Iconoir.Copy, {
                    val clip = context.getSystemService(android.content.ClipboardManager::class.java)
                    clip?.setPrimaryClip(android.content.ClipData.newPlainText(code.code, code.code))
                    // Android 13 and later show the copy themselves.
                    if (android.os.Build.VERSION.SDK_INT < 33) android.widget.Toast.makeText(context, words.code.copied, android.widget.Toast.LENGTH_SHORT).show()
                }, Modifier.fillMaxWidth().testTag("code-copy"))
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    LoadingDots(dot = 5.dp)
                    Text(words.code.waiting, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("code-waiting"))
                }
            }
        },
    )
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
@Composable
private fun WebSignIn(done: suspend (String, String) -> Boolean, start: String, loading: (Boolean) -> Unit, signedIn: (String) -> Unit, modifier: Modifier) {
    val onLoading by rememberUpdatedState(loading)
    val onSignedIn by rememberUpdatedState(signedIn)
    val isDone by rememberUpdatedState(done)
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    DisposableEffect(Unit) { onDispose { forgetWebSession() } }
    // Windows the page opened, the newest last: Back goes back in the newest, then closes it.
    val popups = remember { androidx.compose.runtime.mutableStateListOf<WebView>() }
    fun close(window: WebView) {
        popups.remove(window)
        (window.parent as? FrameLayout)?.removeView(window)
        window.destroy()
    }
    androidx.activity.compose.BackHandler(popups.isNotEmpty()) {
        val top = popups.last()
        if (top.canGoBack()) top.goBack() else close(top)
    }
    AndroidView(
        factory = { context ->
            CookieManager.getInstance().setAcceptCookie(true)
            val page = WebView(context)
            val client = object : WebViewClient() {
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
            FrameLayout(context).apply {
                // Signing in with another account (Apple, Google) opens it in a window of its own, which
                // hands the page its answer through `window.opener`: drawn over the page until it closes.
                val windows = object : WebChromeClient() {
                    override fun onCreateWindow(view: WebView, isDialog: Boolean, isUserGesture: Boolean, resultMsg: Message): Boolean {
                        val popup = WebView(context).also { signInSettings(it, client, this) }
                        addView(popup, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
                        popups.add(popup)
                        (resultMsg.obj as WebView.WebViewTransport).webView = popup
                        resultMsg.sendToTarget()
                        return true
                    }

                    override fun onCloseWindow(window: WebView) = close(window)
                }
                signInSettings(page, client, windows)
                addView(page, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
                forgetWebSession { page.loadUrl(start) }
            }
        },
        modifier = modifier,
        onRelease = { frame ->
            popups.clear()
            for (i in frame.childCount - 1 downTo 0) {
                val view = frame.getChildAt(i) as WebView
                view.stopLoading()
                view.clearHistory()
                view.clearCache(true)
                view.clearFormData()
                view.destroy()
            }
            frame.removeAllViews()
        },
    )
}

/** A sign-in WebView's settings: JavaScript and storage on, files and content off, windows through [windows]. */
@SuppressLint("SetJavaScriptEnabled")
private fun signInSettings(view: WebView, client: WebViewClient, windows: WebChromeClient) {
    view.settings.javaScriptEnabled = true
    view.settings.domStorageEnabled = true
    view.settings.allowFileAccess = false
    view.settings.allowContentAccess = false
    view.settings.setSupportMultipleWindows(true)
    view.settings.javaScriptCanOpenWindowsAutomatically = true
    view.webViewClient = client
    view.webChromeClient = windows
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
