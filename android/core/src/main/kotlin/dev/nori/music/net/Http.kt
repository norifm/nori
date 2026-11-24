package dev.nori.music.net

import android.annotation.SuppressLint
import android.content.Context
import android.net.ConnectivityManager
import dev.nori.music.ffi.model.CoreException
import dev.nori.music.ffi.net.FailureKind
import dev.nori.music.ffi.net.getFailed
import dev.nori.music.ffi.net.NetException
import dev.nori.music.ffi.net.Network
import dev.nori.music.ffi.net.Exchange
import dev.nori.music.ffi.net.HttpHeader
import dev.nori.music.ffi.net.HttpMethod
import dev.nori.music.ffi.net.RequestPolicy
import dev.nori.music.ffi.net.ServerHosts
import dev.nori.music.ffi.net.Transport
import dev.nori.music.ffi.net.TransportException
import dev.nori.music.ffi.net.TransportResponse
import dev.nori.music.ffi.net.netPolicy
import dev.nori.music.ffi.settings.SavedProxy
import dev.nori.music.ffi.settings.SavedServer
import dev.nori.music.ffi.settings.skipsProxy
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.suspendCancellableCoroutine
import okhttp3.Call
import okhttp3.Callback
import okhttp3.ConnectionPool
import okhttp3.Dispatcher
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import java.io.IOException
import java.security.KeyFactory
import java.security.KeyStore
import java.security.Principal
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.security.spec.PKCS8EncodedKeySpec
import java.util.concurrent.TimeUnit
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLEngine
import javax.net.ssl.SSLSession
import javax.net.ssl.SSLSocket
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509ExtendedKeyManager
import javax.net.ssl.X509ExtendedTrustManager
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import dev.nori.music.settings.server

private val JSON = "application/json".toMediaType()
private val FORM = "application/x-www-form-urlencoded".toMediaType()

/** The server is only allowed on unmetered networks and this is not one. Its message is for the log; [said] words it. */
class MeteredNetworkException : IOException("metered network, server is Wi-Fi only")

/** The server answered with an error status and nothing a Subsonic client can read. Its message is for the log; [said] words it. */
class HttpStatusException(val status: Int) : IOException("HTTP $status")

/** The profile's proxy failed: [kind] is one of the core's proxy kinds. Its message is for the log; [said] words it. */
class ProxyFailureException(val kind: FailureKind, message: String?, cause: Throwable? = null) : IOException(message, cause)

/**
 * One connection pool for everything: API calls, cover art and audio all ride
 * the same HTTP/2 connection to the server, so the radio wakes once, not three times.
 * The clients are rebuilt when the server profile changes (TLS, proxy); callers
 * go through [callFactory] / [streamFactory] so they always use the current ones.
 * How the clients are tuned (pool, request caps, timeouts) is the core's [netPolicy], with the reasons.
 */
class Http(private val context: Context) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val policy = netPolicy()
    private val pool = ConnectionPool(policy.poolMaxIdle.toInt(), policy.poolKeepAliveMs.toLong(), TimeUnit.MILLISECONDS)
    private val dispatcher = Dispatcher().apply { maxRequestsPerHost = policy.maxRequestsPerHost.toInt(); maxRequests = policy.maxRequests.toInt() }
    private val streamDispatcher = Dispatcher().apply { maxRequestsPerHost = policy.streamMaxRequestsPerHost.toInt(); maxRequests = policy.streamMaxRequests.toInt() }

    /**
     * [profile]'s requests: which are its server's, and its Wi-Fi-only rule, the core's (transport.rs
     * `ServerHosts`), with its answer per host kept, since every request (each cover, each range of a
     * stream) asks it and the answer only depends on the host and port. Replaced whole when the profile
     * changes, so an answer from the old one is never read.
     */
    private class Hosts(val profile: SavedServer?) {
        val server = ServerHosts(profile?.url, profile?.altUrl, profile?.wifiOnly == true)
        val policies = java.util.concurrent.ConcurrentHashMap<String, Pair<Int, RequestPolicy>>()

        /** The policy for [url], asked of the core once per host and port. */
        fun policyOf(url: okhttp3.HttpUrl): RequestPolicy {
            policies[url.host]?.let { (port, rule) -> if (port == url.port) return rule }
            return server.policy(url.toString()).also { policies[url.host] = url.port to it }
        }
    }

    /** The profile in use's. */
    @Volatile private var hosts = Hosts(null)

    /**
     * What failed connecting through the profile's proxy, as OkHttp told [ProxyConnects]: [proxyFailure] words
     * those as the proxy's, and no other failure. Weak, so one nobody asks about goes.
     */
    private val proxyConnectFailures: MutableSet<IOException> = java.util.Collections.synchronizedSet(java.util.Collections.newSetFromMap(java.util.WeakHashMap()))

    /** The audio's stall timeout (see [Stalls]), shared by every stream client. Starts no thread until a song streams. */
    private val stalls = Stalls(policy.streamReadTimeoutMs.toLong())

    @Volatile var api: OkHttpClient = build(null) { hosts }
        private set

    @Volatile var stream: OkHttpClient = streamClient(api)
        private set

    val callFactory = Call.Factory { api.newCall(it) }
    val streamFactory = Call.Factory { stream.newCall(it) }

    /**
     * Asked once per track, when its quality is chosen. A registered network callback would be woken
     * for every signal-strength change for as long as the process lives; this costs one binder call.
     */
    val metered: Boolean get() = connectivity.isActiveNetworkMetered

    fun configure(next: SavedServer?) {
        val old = hosts.profile
        hosts = Hosts(next)
        // Only TLS settings and the proxy need new clients; headers and the Wi-Fi rule are read per request.
        if (old?.allowSelfSigned != next?.allowSelfSigned || old?.clientCert != next?.clientCert || old?.clientCertPassword != next?.clientCertPassword || old?.proxy != next?.proxy) {
            api = build(next) { hosts }
            stream = streamClient(api)
        }
    }

    /**
     * The profile's SOCKS proxy for every connection but one to a local destination, which the core names
     * ([skipsProxy]): a proxy elsewhere cannot reach it. Unresolved, so the proxy looks the names up; its
     * login is [SocksLogin]'s to give.
     */
    private class ProfileProxy(proxy: SavedProxy) : java.net.ProxySelector() {
        private val through = listOf(java.net.Proxy(java.net.Proxy.Type.SOCKS, java.net.InetSocketAddress.createUnresolved(proxy.host, proxy.port.toInt())))
        private val direct = listOf(java.net.Proxy.NO_PROXY)

        override fun select(uri: java.net.URI): List<java.net.Proxy> = if (skipsProxy(uri.host)) direct else through

        override fun connectFailed(uri: java.net.URI, address: java.net.SocketAddress, e: IOException) {}
    }

    /**
     * A client's connections through its SOCKS proxy [own]: while one connects, its thread offers [own]'s login
     * ([SocksLogin]); each failure is kept in [failed].
     */
    private class ProxyConnects(private val failed: MutableSet<IOException>, private val own: SavedProxy) : okhttp3.EventListener() {
        override fun connectStart(call: Call, inetSocketAddress: java.net.InetSocketAddress, proxy: java.net.Proxy) {
            if (proxy.type() == java.net.Proxy.Type.SOCKS) SocksLogin.connecting.set(own)
        }

        override fun connectEnd(call: Call, inetSocketAddress: java.net.InetSocketAddress, proxy: java.net.Proxy, protocol: okhttp3.Protocol?) {
            SocksLogin.connecting.remove()
        }

        override fun connectFailed(call: Call, inetSocketAddress: java.net.InetSocketAddress, proxy: java.net.Proxy, protocol: okhttp3.Protocol?, ioe: IOException) {
            SocksLogin.connecting.remove()
            if (proxy.type() == java.net.Proxy.Type.SOCKS) failed.add(ioe)
        }
    }

    /**
     * [e] as the profile's proxy's failure, when it failed connecting through the proxy. Java's SOCKS client
     * (SocksSocketImpl) throws every failure of its step as a plain SocketException, told apart only by its
     * text: the login's, the proxy's reply ("SOCKS: Host unreachable"), or else reaching the proxy at all.
     * The server's TLS, past the proxy, stays its own.
     */
    private fun proxyFailure(e: IOException): ProxyFailureException? {
        if (!proxyConnectFailures.remove(e) || e is javax.net.ssl.SSLException) return null
        val said = e.message.orEmpty()
        val kind = when {
            said in SOCKS_LOGIN -> FailureKind.PROXY_LOGIN
            said.startsWith("SOCKS") -> FailureKind.PROXY_NO_ROUTE
            else -> FailureKind.PROXY_UNREACHABLE
        }
        return ProxyFailureException(kind, e.message, e)
    }

    /**
     * The login of the SOCKS proxy a client is connecting through. Java signs in to a SOCKS proxy only through
     * the process's one Authenticator, asked on the connecting thread: this is it once a client with such a
     * proxy is built, and it answers with the login of the client connecting on that thread ([ProxyConnects]),
     * for its proxy alone.
     */
    private object SocksLogin : java.net.Authenticator() {
        val connecting = ThreadLocal<SavedProxy?>()
        private var installed = false

        @Synchronized
        fun install() {
            if (!installed) java.net.Authenticator.setDefault(this)
            installed = true
        }

        override fun getPasswordAuthentication(): java.net.PasswordAuthentication? {
            val proxy = connecting.get() ?: return null
            val login = proxy.login ?: return null
            if (requestingProtocol != "SOCKS5" || requestingHost != proxy.host || requestingPort != proxy.port.toInt()) return null
            return java.net.PasswordAuthentication(login.user, login.password.toCharArray())
        }
    }

    /**
     * The audio's client: no read timeout, since OkHttp would arm its watchdog around every read of a song's
     * body, a network packet at a time; a stalled connection is found by [Stalls] instead.
     */
    private fun streamClient(api: OkHttpClient) =
        api.newBuilder().dispatcher(streamDispatcher).readTimeout(0, TimeUnit.MILLISECONDS).addInterceptor(stalls).build()

    /** A client for [p], whose requests read their rules from [hosts]. */
    private fun build(p: SavedServer?, hosts: () -> Hosts): OkHttpClient {
        val proxy = p?.proxy
        val b = OkHttpClient.Builder()
            .dispatcher(dispatcher)
            .connectionPool(pool)
            .connectTimeout(policy.connectTimeoutMs.toLong(), TimeUnit.MILLISECONDS)
            .readTimeout(policy.readTimeoutMs.toLong(), TimeUnit.MILLISECONDS)
            .addInterceptor { chain ->
                val route = hosts()
                val now = route.profile
                val request = chain.request()
                // Third parties (LRCLIB, AutoEQ) must not receive a reverse-proxy token, and are not subject to the
                // server's Wi-Fi-only setting: the core says which this is. The network is only looked at when
                // the answer depends on it.
                val rule = if (now != null) route.policyOf(request.url) else null
                if (rule?.unmeteredOnly == true && metered) throw MeteredNetworkException()
                // Public services ask clients to identify themselves; some reject OkHttp's default outright.
                // One the core's exchange names (a provider's) is its own.
                val sent = if (request.header("User-Agent") == null) request.newBuilder().header("User-Agent", policy.userAgent).build() else request
                try {
                    chain.proceed(sent)
                } catch (e: IOException) {
                    throw proxyFailure(e) ?: e
                }
            }
            // The server's headers, decided at each hop of a redirect: one leaving the server carries none.
            .addNetworkInterceptor { chain ->
                val route = hosts()
                val request = chain.request()
                val headers = route.profile?.headers.orEmpty()
                if (headers.isEmpty() || !route.policyOf(request.url).server) chain.proceed(request)
                else chain.proceed(request.newBuilder().apply { headers.forEach { (k, v) -> header(k, v) } }.build())
            }
        if (p != null && (p.allowSelfSigned || p.clientCert.isNotEmpty())) tls(b, p, hosts)
        if (proxy != null) {
            if (proxy.login != null) SocksLogin.install()
            b.proxySelector(ProfileProxy(proxy)).eventListener(ProxyConnects(proxyConnectFailures, proxy))
        }
        return b.build()
    }

    /**
     * [p]'s TLS exceptions, for its server alone ([Hosts.policyOf] at each connection, a redirect's too): any
     * certificate accepted where it says so, and its client certificate (read by the core,
     * `cert::client_identity`) sent; every other host is checked as usual and gets none.
     */
    private fun tls(b: OkHttpClient.Builder, p: SavedServer, hosts: () -> Hosts) {
        val toServer = { host: String?, port: Int ->
            host != null && port > 0 && hosts().policyOf(okhttp3.HttpUrl.Builder().scheme("https").host(host).port(port).build()).server
        }
        val ofServer = { session: SSLSession? -> session != null && toServer(session.peerHost, session.peerPort) }
        val system = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm()).apply { init(null as KeyStore?) }
            .trustManagers.filterIsInstance<X509ExtendedTrustManager>().first()
        val trust = if (p.allowSelfSigned) ServerTrust(system, ofServer) else system
        val keys = clientKeys(p)?.let { arrayOf(ServerKeys(it, ofServer)) }
        val ssl = SSLContext.getInstance("TLS").apply { init(keys, arrayOf(trust), null) }
        b.sslSocketFactory(ssl.socketFactory, trust)
        if (p.allowSelfSigned) {
            val checked = HttpsURLConnection.getDefaultHostnameVerifier()
            b.hostnameVerifier { host, session -> toServer(host, session.peerPort) || checked.verify(host, session) }
        }
    }

    /** [p]'s client certificate as Java's TLS sends it; null without one, or for one the core can't read. */
    private fun clientKeys(p: SavedServer): X509ExtendedKeyManager? {
        val identity = try {
            dev.nori.music.ffi.cert.clientIdentity(context.filesDir.path, p)
        } catch (e: dev.nori.music.ffi.cert.CertException) {
            dev.nori.music.NoriLog.w("http: the client certificate", e)
            null
        } ?: return null
        val factory = CertificateFactory.getInstance("X.509")
        val chain = identity.chain.map { factory.generateCertificate(it.inputStream()) }.toTypedArray()
        val spec = PKCS8EncodedKeySpec(identity.key)
        val key = listOf("RSA", "EC", "Ed25519").firstNotNullOfOrNull { runCatching { KeyFactory.getInstance(it).generatePrivate(spec) }.getOrNull() } ?: return null
        val store = KeyStore.getInstance(KeyStore.getDefaultType()).apply { load(null); setKeyEntry("client", key, CharArray(0), chain) }
        return KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply { init(store, CharArray(0)) }
            .keyManagers.filterIsInstance<X509ExtendedKeyManager>().first()
    }

    /** The user ticked "accept any certificate" for their server ([ofServer]); every other host is [system]'s to check. */
    @SuppressLint("CustomX509TrustManager")
    private class ServerTrust(private val system: X509ExtendedTrustManager, private val ofServer: (SSLSession?) -> Boolean) : X509ExtendedTrustManager() {
        override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String, socket: java.net.Socket?) {
            if (!ofServer((socket as? SSLSocket)?.handshakeSession)) system.checkServerTrusted(chain, authType, socket)
        }

        override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String, engine: SSLEngine?) {
            if (!ofServer(engine?.handshakeSession)) system.checkServerTrusted(chain, authType, engine)
        }

        override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String) = system.checkServerTrusted(chain, authType)
        override fun checkClientTrusted(chain: Array<out X509Certificate>, authType: String, socket: java.net.Socket?) = system.checkClientTrusted(chain, authType, socket)
        override fun checkClientTrusted(chain: Array<out X509Certificate>, authType: String, engine: SSLEngine?) = system.checkClientTrusted(chain, authType, engine)
        override fun checkClientTrusted(chain: Array<out X509Certificate>, authType: String) = system.checkClientTrusted(chain, authType)
        override fun getAcceptedIssuers(): Array<X509Certificate> = system.acceptedIssuers
    }

    /** The client certificate in [keys], offered to the profile's server alone ([ofServer]). */
    private class ServerKeys(private val keys: X509ExtendedKeyManager, private val ofServer: (SSLSession?) -> Boolean) : X509ExtendedKeyManager() {
        override fun chooseClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, socket: java.net.Socket?): String? =
            if (ofServer((socket as? SSLSocket)?.handshakeSession)) keys.chooseClientAlias(keyType, issuers, socket) else null

        override fun chooseEngineClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, engine: SSLEngine?): String? =
            if (ofServer(engine?.handshakeSession)) keys.chooseEngineClientAlias(keyType, issuers, engine) else null

        override fun getClientAliases(keyType: String?, issuers: Array<out Principal>?): Array<String>? = keys.getClientAliases(keyType, issuers)
        override fun getServerAliases(keyType: String?, issuers: Array<out Principal>?): Array<String>? = null
        override fun chooseServerAlias(keyType: String?, issuers: Array<out Principal>?, socket: java.net.Socket?): String? = null
        override fun getCertificateChain(alias: String?): Array<X509Certificate>? = keys.getCertificateChain(alias)
        override fun getPrivateKey(alias: String?): java.security.PrivateKey? = keys.getPrivateKey(alias)
    }

    /** The status and the whole body. Cancelling the coroutine cancels the call, which is what makes live search cheap. */
    suspend fun exchange(url: String, timeoutMs: Long = 0, over: OkHttpClient = api): TransportResponse = exchange(Request.Builder().url(url).build(), timeoutMs, over)

    /**
     * [exchange] with a third party's own method, headers and body: the core's [Exchange], which says what to
     * send; this only carries the bytes.
     */
    suspend fun exchange(e: Exchange, over: OkHttpClient): TransportResponse {
        val request = Request.Builder().url(e.url)
        for ((name, value) in e.headers) request.header(name, value)
        val type = e.headers.entries.firstOrNull { it.key.equals("Content-Type", ignoreCase = true) }?.value?.toMediaTypeOrNull()
        val body = e.json?.toRequestBody(JSON) ?: e.form?.toRequestBody(FORM) ?: e.body?.toRequestBody(type)
        val method = e.method ?: if (body != null) HttpMethod.POST else HttpMethod.GET
        // OkHttp wants a body, if empty, for the methods that carry one.
        val carries = method == HttpMethod.POST || method == HttpMethod.PUT || method == HttpMethod.PATCH
        request.method(method.name, body ?: if (carries) ByteArray(0).toRequestBody(type) else null)
        return exchange(request.build(), e.timeoutMs.toLong(), over)
    }

    private suspend fun exchange(request: Request, timeoutMs: Long, over: OkHttpClient): TransportResponse = suspendCancellableCoroutine { cont ->
        // A request's own timeout is the whole wait, reading too: a held poll answers only when there is news.
        val client = if (timeoutMs > 0) over.newBuilder().callTimeout(timeoutMs, TimeUnit.MILLISECONDS).readTimeout(timeoutMs, TimeUnit.MILLISECONDS).build() else over
        val call = client.newCall(request)
        cont.invokeOnCancellation { call.cancel() }
        call.enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) {
                if (cont.isActive) cont.resumeWithException(e)
            }

            override fun onResponse(call: Call, response: Response) {
                try {
                    val body = response.use { it.body.bytes() }
                    val headers = response.headers.map { (name, value) -> HttpHeader(name.lowercase(), value) }
                    cont.resume(TransportResponse(response.code.toUShort(), body, headers))
                } catch (e: IOException) {
                    if (cont.isActive) cont.resumeWithException(e)
                }
            }
        })
    }

    /**
     * A GET for callers outside the core's client (AutoEQ). octo-fiesta reports auth failures as 401 with a
     * normal Subsonic error body, so the body is read either way; only an empty error answer is a failure.
     */
    suspend fun get(url: String, timeoutMs: Long = 0): ByteArray {
        val r = exchange(url, timeoutMs)
        if (getFailed(r.status, r.body.isEmpty())) throw HttpStatusException(r.status.toInt())
        return r.body
    }

    /** The core's door to the network for the profile in use: one GET, with the platform's exceptions sorted into kinds. */
    fun transport(onAddressChanged: () -> Unit): Transport = transportOver({ api }, onAddressChanged)

    /**
     * The core's door to the network for [p] alone, apart from the profile in use (an account edited): a client
     * of its own, with its proxy, TLS and headers, over the same pool.
     */
    fun transportOf(p: SavedServer): Transport {
        val route = Hosts(p)
        val client = build(p) { route }
        return transportOver({ client }) {}
    }

    private fun transportOver(client: () -> OkHttpClient, onAddressChanged: () -> Unit): Transport = object : Transport {
        override suspend fun get(url: String, timeoutMs: UInt): TransportResponse = try {
            exchange(url, timeoutMs.toLong(), client())
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            throw TransportException.Failed(failureKind(e), e.message)
        }

        override suspend fun send(request: Exchange): TransportResponse = try {
            exchange(request, client())
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            throw TransportException.Failed(failureKind(e), e.message)
        }

        override fun addressChanged() = onAddressChanged()

        /** Asked when the core chooses a quality or whether something may use mobile data, never per request. */
        override fun network() = if (metered) Network.METERED else Network.UNMETERED
    }
}

/** Which kind of failure the platform's exception is; the core decides what each kind means. */
fun failureKind(e: Throwable): FailureKind = when (e) {
    is ProxyFailureException -> e.kind
    is MeteredNetworkException -> FailureKind.METERED
    is java.net.UnknownHostException -> FailureKind.UNKNOWN_HOST
    is java.net.ConnectException -> FailureKind.CONNECT
    is java.net.NoRouteToHostException -> FailureKind.NO_ROUTE
    is java.net.SocketTimeoutException -> FailureKind.TIMEOUT
    is java.io.InterruptedIOException -> FailureKind.INTERRUPTED
    is javax.net.ssl.SSLPeerUnverifiedException, is javax.net.ssl.SSLHandshakeException -> FailureKind.TLS
    is java.net.UnknownServiceException -> FailureKind.CLEARTEXT
    is IOException -> FailureKind.IO
    else -> FailureKind.OTHER
}

/** What Java's SOCKS client says when the proxy refuses its login, or asks for one it cannot give. */
private val SOCKS_LOGIN = setOf("SOCKS : authentication failed", "SOCKS : No acceptable methods")

/**
 * A failure from the core's client as the exception the platform would have thrown itself, so callers keep
 * telling network trouble ([IOException]) from a refusal ([CoreException]) the way they always did.
 */
fun NetException.lift(): Exception = when (this) {
    is NetException.Transport -> when (kind) {
        FailureKind.METERED -> MeteredNetworkException()
        FailureKind.UNKNOWN_HOST -> java.net.UnknownHostException(detail)
        FailureKind.CONNECT -> java.net.ConnectException(detail)
        FailureKind.NO_ROUTE -> java.net.NoRouteToHostException(detail)
        FailureKind.TIMEOUT -> java.net.SocketTimeoutException(detail)
        FailureKind.INTERRUPTED -> java.io.InterruptedIOException(detail)
        FailureKind.TLS -> javax.net.ssl.SSLHandshakeException(detail.orEmpty())
        FailureKind.CLEARTEXT -> java.net.UnknownServiceException(detail)
        FailureKind.PROXY_UNREACHABLE, FailureKind.PROXY_LOGIN, FailureKind.PROXY_NO_ROUTE -> ProxyFailureException(kind, detail)
        FailureKind.IO -> IOException(detail)
        FailureKind.OTHER -> IllegalStateException(detail)
    }
    is NetException.Http -> HttpStatusException(status.toInt())
    is NetException.Api -> CoreException.Api(code, reason)
    is NetException.Parse -> CoreException.Parse(reason)
    is NetException.Db -> CoreException.Db(reason)
}

/** Runs a call into the core's client, turning its failures into the platform's exceptions. */
inline fun <T> lifted(block: () -> T): T = try {
    block()
} catch (e: NetException) {
    throw e.lift()
}

/**
 * What a failure says. The core's exceptions carry no message of their own (uniffi's JNI bindings give
 * them none) and hand over a kind and its facts; they are worded here ([Failures]), as are the app's own
 * network exceptions. Anything else says its own message.
 */
val Throwable.said: String? get() = Failures.said(this)

/** What went wrong, in words a person can act on. */
fun describeConnectionError(e: Throwable): String = Failures.describe(e)
