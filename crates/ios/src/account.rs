//! Saved servers. Login checks the address, then writes the profile; it does not open playback.

use std::ffi::{c_char, CStr, CString};
use std::path::Path;

use nori_core::providers::flow::{WelcomeDo, WelcomeEvent, WelcomeFlow, WelcomeSource};
use nori_core::providers::{form_fields, form_taken, log_in, FormField, Kinds, Platform};
use nori_core::proxy::{proxy_address, proxy_from_form, ProxyError};
use nori_core::settings::{profile_icons, profile_letter, servers_removed, ProfileIcon, SavedServer, ServerKind, ServerList, StoredPrefs};
use nori_core::settings_store::Settings;
use nori_core::transport::{block_on, FailureKind, NetError};
use nori_http::Http;

pub const LOGIN_OK: i32 = 0;
pub const LOGIN_INCOMPLETE: i32 = 1;
pub const LOGIN_NOT_FOUND: i32 = 2;
pub const LOGIN_UNREACHABLE: i32 = 3;
pub const LOGIN_TIMEOUT: i32 = 4;
pub const LOGIN_CERTIFICATE: i32 = 5;
pub const LOGIN_HTTP: i32 = 6;
pub const LOGIN_PASSWORD: i32 = 7;
pub const LOGIN_FORBIDDEN: i32 = 8;
pub const LOGIN_NOT_SUBSONIC: i32 = 9;
pub const LOGIN_DATABASE: i32 = 10;
pub const LOGIN_OTHER: i32 = 11;
pub const LOGIN_CLEARTEXT: i32 = 12;
pub const LOGIN_METERED: i32 = 13;
/// The proxy's fields are not a proxy (the page checks them first with [`nori_ios_proxy`]).
pub const LOGIN_PROXY: i32 = 14;
pub const LOGIN_PROXY_UNREACHABLE: i32 = 15;
pub const LOGIN_PROXY_LOGIN: i32 = 16;
pub const LOGIN_PROXY_NO_ROUTE: i32 = 17;

fn owned(s: &str) -> CString {
    CString::new(s.replace('\0', " ")).unwrap_or_else(|_| CString::new("").unwrap())
}

fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .unwrap_or("")
        .to_string()
}

/// `https://` when the typed address has no scheme.
fn address(raw: &str) -> String {
    let t = raw.trim().trim_end_matches('/');
    if t.contains("://") {
        t.to_string()
    } else if t.is_empty() {
        String::new()
    } else {
        format!("https://{t}")
    }
}

pub(crate) fn opened(dir: &Path) -> Result<(std::sync::Arc<Settings>, StoredPrefs), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let settings = Settings::new();
    let prefs = settings
        .open(&nori_host::db_path(dir))
        .map_err(|e| e.to_string())?;
    Ok((settings, prefs))
}

/// A profile's name, otherwise its user, otherwise its address; empty for none of them (the app then
/// calls it by its kind).
fn label(s: &SavedServer) -> String {
    [s.name.trim(), s.user.trim(), s.url.as_str()]
        .into_iter()
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// The active server's label and kind; None when none are saved.
pub(crate) fn active(dir: &Path) -> Result<Option<(String, ServerKind)>, String> {
    let (_, prefs) = opened(dir)?;
    let server = prefs.active_server().or_else(|| prefs.servers.first());
    Ok(server.map(|s| (label(s), s.kind.clone())))
}

fn list_of(prefs: &mut StoredPrefs) -> ServerList {
    ServerList {
        servers: std::mem::take(&mut prefs.servers),
        active_server_id: prefs.active_server_id.clone(),
    }
}

/// The settings to change the saved list through: the open session's when one is open, as the session
/// writes the list back from its own copy (an account's name).
fn live_settings(dir: &Path) -> Result<std::sync::Arc<Settings>, String> {
    match crate::session::with_session(|s| s.core.session.settings.clone()).filter(|s| s.current().is_some()) {
        Some(live) => Ok(live),
        None => opened(dir).map(|(settings, _)| settings),
    }
}

/// The saved list changed by `change` ([`live_settings`]).
fn edit_servers(dir: &Path, change: impl FnOnce(ServerList) -> ServerList) -> Result<(), String> {
    let settings = live_settings(dir)?;
    let mut prefs = settings.current().unwrap_or_default();
    let list = change(list_of(&mut prefs));
    (prefs.servers, prefs.active_server_id) = (list.servers, list.active_server_id);
    settings.put(prefs);
    settings.writer.flush();
    Ok(())
}

/// The saved servers as `{id, label, name, url, active, kind, waits, icon, letter}`, in the order they were
/// saved; `name` as it was given (empty for none), `icon` as the settings store it, `letter` what its circle
/// shows for the letter.
pub(crate) fn servers(dir: &Path) -> Result<Vec<serde_json::Value>, String> {
    let (_, prefs) = opened(dir)?;
    Ok(prefs
        .servers
        .iter()
        .map(|s| {
            let active = s.id == prefs.active_server_id;
            // A plugin's profile never opens here: the iPod runs none.
            let waits = Kinds::builtin().waits(s.clone()).is_some();
            let letter = profile_letter(s.clone(), nori_core::settings::label(&s.name, &s.url));
            serde_json::json!({ "id": s.id, "label": label(s), "name": s.name, "url": s.url, "active": active, "kind": s.kind.id(), "waits": waits,
                "icon": s.icon.id(), "letter": letter })
        })
        .collect())
}

/// Saved server `id`'s circle shows `icon`; an unknown id changes nothing.
pub(crate) fn set_icon(dir: &Path, id: &str, icon: ProfileIcon) -> Result<(), String> {
    let settings = live_settings(dir)?;
    nori_host::profile_look(&settings, id, |p| p.icon = icon);
    settings.writer.flush();
    Ok(())
}

/// Saved server `id` renamed; blank, it is called after its provider again. An unknown id changes nothing.
pub(crate) fn rename(dir: &Path, id: &str, name: &str) -> Result<(), String> {
    let settings = live_settings(dir)?;
    nori_host::profile_renamed(&settings, id, name);
    settings.writer.flush();
    Ok(())
}

/// Makes saved server `id` the active one. False when no server has that id.
pub(crate) fn pick(dir: &Path, id: &str) -> Result<bool, String> {
    let (settings, _) = opened(dir)?;
    let picked = nori_host::profile_picked(&settings, id).is_some();
    settings.writer.flush();
    Ok(picked)
}

/// Forgets saved server `id`; the first one left becomes active if it was.
pub(crate) fn remove(dir: &Path, id: &str) -> Result<(), String> {
    edit_servers(dir, |list| servers_removed(list, id.to_string()))
}

/// A failure as a login code, with the detail a code words itself.
pub(crate) fn fail(e: NetError) -> (i32, Option<String>) {
    match e {
        NetError::Transport { kind, detail } => match kind {
            FailureKind::UnknownHost => (LOGIN_NOT_FOUND, None),
            FailureKind::Connect | FailureKind::NoRoute => (LOGIN_UNREACHABLE, None),
            FailureKind::Timeout | FailureKind::Interrupted => (LOGIN_TIMEOUT, None),
            FailureKind::Tls => (LOGIN_CERTIFICATE, None),
            FailureKind::Cleartext => (LOGIN_CLEARTEXT, None),
            FailureKind::Metered => (LOGIN_METERED, None),
            FailureKind::ProxyUnreachable => (LOGIN_PROXY_UNREACHABLE, None),
            FailureKind::ProxyLogin => (LOGIN_PROXY_LOGIN, None),
            FailureKind::ProxyNoRoute => (LOGIN_PROXY_NO_ROUTE, None),
            FailureKind::Io | FailureKind::Other => (LOGIN_OTHER, detail),
        },
        NetError::Http { status } => (LOGIN_HTTP, Some(status.to_string())),
        NetError::Api { code: 40, .. } => (LOGIN_PASSWORD, None),
        NetError::Api { code: 50, .. } => (LOGIN_FORBIDDEN, None),
        NetError::Api { reason, .. } => (LOGIN_OTHER, Some(reason)),
        NetError::Parse { .. } => (LOGIN_NOT_SUBSONIC, None),
        NetError::Db { reason } => (LOGIN_DATABASE, Some(reason)),
    }
}

/// The login form as the app sends it: the fields its kind shows (`nori_ios_form_fields`); any may be
/// empty. `id` is the profile's from the page's opening (a client certificate is kept by it), `alt` the
/// second address, `key` an API key, `headers` one per line; `proxy`, `proxy_user` and `proxy_password`
/// the proxy's sheet's, as [`proxy_from_form`] reads them; `client_cert` the file
/// `nori_ios_import_cert` kept.
#[derive(Debug, Default, Clone)]
pub(crate) struct Form {
    pub id: String,
    pub kind: ServerKind,
    pub url: String,
    pub user: String,
    pub password: String,
    pub name: String,
    pub alt: String,
    pub key: String,
    pub headers: String,
    pub legacy_auth: bool,
    pub self_signed: bool,
    pub client_cert: String,
    pub client_cert_password: String,
    pub proxy: String,
    pub proxy_user: String,
    pub proxy_password: String,
}

impl Form {
    /// The app's JSON `{id, kind, url, user, password, name, alt, key, headers, legacyAuth, selfSigned,
    /// clientCert, clientCertPassword, proxy, proxyUser, proxyPassword}`, the switches booleans; a missing
    /// field is empty or off, and an unknown `kind` a server's.
    fn parse(json: &str) -> Form {
        let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
        let field = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        Form {
            id: field("id"),
            kind: ServerKind::from_id(&field("kind")).unwrap_or_default(),
            headers: field("headers"),
            legacy_auth: v["legacyAuth"].as_bool().unwrap_or_default(),
            self_signed: v["selfSigned"].as_bool().unwrap_or_default(),
            client_cert: field("clientCert"),
            client_cert_password: field("clientCertPassword"),
            url: field("url"),
            user: field("user"),
            password: field("password"),
            name: field("name"),
            alt: field("alt"),
            key: field("key"),
            proxy: field("proxy"),
            proxy_user: field("proxyUser"),
            proxy_password: field("proxyPassword"),
        }
    }
}

/// The form's kind's provider.
fn info(form: &Form) -> Option<nori_core::providers::ProviderInfo> {
    Kinds::builtin().providers().into_iter().find(|p| p.kind == form.kind)
}

/// The profile as `form` has it typed, its proxy as the core reads it.
fn typed(form: &Form) -> Result<SavedServer, ProxyError> {
    let typed = SavedServer {
        id: form.id.clone(),
        kind: form.kind.clone(),
        name: form.name.trim().to_string(),
        url: address(&form.url),
        alt_url: address(&form.alt),
        user: form.user.trim().to_string(),
        password: form.password.clone(),
        api_key: form.key.trim().to_string(),
        legacy_auth: form.legacy_auth,
        allow_self_signed: form.self_signed,
        client_cert: form.client_cert.clone(),
        client_cert_password: form.client_cert_password.clone(),
        proxy: proxy_from_form(form.proxy.trim().to_string(), form.proxy_user.trim().to_string(), form.proxy_password.clone())?,
        ..SavedServer::default()
    };
    Ok(nori_core::settings::server_from_form(typed, form.headers.clone()))
}

/// The profile `form` asks for, of the fields its kind shows on the iPod; else the `LOGIN_*` code for why
/// the form cannot be sent yet (a server's lacks an address, or both a user and an API key; its proxy is
/// not one).
fn draft(form: &Form) -> Result<SavedServer, i32> {
    let info = info(form).ok_or(LOGIN_INCOMPLETE)?;
    let typed = typed(form).map_err(|_| LOGIN_PROXY)?;
    let id = if form.id.is_empty() { nori_core::settings::new_server_id() } else { form.id.clone() };
    let base = SavedServer { id, kind: form.kind.clone(), ..SavedServer::default() };
    let profile = form_taken(base, &typed, &form_fields(&info, Platform::Ipod, &typed, true));
    Kinds::builtin().server_ready(profile.clone()).then_some(profile).ok_or(LOGIN_INCOMPLETE)
}

/// A field as the login JSON names it.
fn field_key(what: FormField) -> &'static str {
    match what {
        FormField::Name => "name",
        FormField::Address => "url",
        FormField::User => "user",
        FormField::Password => "password",
        FormField::SecondAddress => "alt",
        FormField::ApiKey => "key",
        FormField::Headers => "headers",
        FormField::LegacyAuth => "legacyAuth",
        FormField::SelfSigned => "selfSigned",
        FormField::WifiOnly => "wifiOnly",
        FormField::ClientCert => "clientCert",
        FormField::Proxy => "proxy",
    }
}

/// The kinds of profile the login page offers, as JSON to free: `[{kind}]`, `kind` as the settings store
/// it.
#[no_mangle]
pub extern "C" fn nori_ios_providers() -> *mut c_char {
    let rows: Vec<serde_json::Value> = Kinds::builtin().providers().into_iter().map(|p| serde_json::json!({ "kind": p.kind.id() })).collect();
    owned(&serde_json::Value::from(rows).to_string()).into_raw()
}

/// The fields the login form shows for `form` as typed (the login JSON), in order, as JSON to free:
/// `[{what, advanced}]`, `what` the login JSON's key it fills (`proxy`: the proxy's entry, whose sheet
/// fills `proxy`, `proxyUser` and `proxyPassword`; `clientCert`: the certificate's, which
/// `nori_ios_import_cert` fills). An API key typed takes the user and password away.
///
/// # Safety
/// `form` is NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_form_fields(form: *const c_char) -> *mut c_char {
    let form = Form::parse(&text(form));
    let typed = typed(&form).unwrap_or_else(|_| SavedServer { kind: form.kind.clone(), api_key: form.key.trim().to_string(), ..SavedServer::default() });
    let fields = info(&form).map(|i| form_fields(&i, Platform::Ipod, &typed, true)).unwrap_or_default();
    let rows: Vec<serde_json::Value> = fields.iter().map(|f| serde_json::json!({ "what": field_key(f.what), "advanced": f.advanced })).collect();
    owned(&serde_json::Value::from(rows).to_string()).into_raw()
}

/// A fresh profile id, for a login page's form, to free.
#[no_mangle]
pub extern "C" fn nori_ios_new_id() -> *mut c_char {
    owned(&nori_core::settings::new_server_id()).into_raw()
}

/// Why `nori_ios_import_cert` took no certificate.
pub const CERT_FILE: i32 = 1;
pub const CERT_PASSWORD: i32 = 2;
pub const CERT_NOT_PKCS12: i32 = 3;
pub const CERT_UNSUPPORTED: i32 = 4;

/// The .p12 file at `path`, opened with `password`, kept under `data_dir` as profile `id`'s client
/// certificate: the name the form's `clientCert` takes, to free; NULL when it isn't taken, with a `CERT_*`
/// code in `error`.
///
/// # Safety
/// The strings are NUL-terminated UTF-8, or NULL. `error` is NULL or a pointer this function writes.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_import_cert(data_dir: *const c_char, id: *const c_char, path: *const c_char, password: *const c_char, error: *mut i32) -> *mut c_char {
    let profile = SavedServer { id: text(id), ..SavedServer::default() };
    match nori_core::cert::import_client_cert_file(Path::new(&text(data_dir)), profile, Path::new(&text(path)), text(password)) {
        Ok(p) => owned(&p.client_cert).into_raw(),
        Err(e) => {
            if !error.is_null() {
                use nori_core::cert::CertError;
                unsafe {
                    *error = match e {
                        CertError::File => CERT_FILE,
                        CertError::Password => CERT_PASSWORD,
                        CertError::NotPkcs12 => CERT_NOT_PKCS12,
                        CertError::Unsupported => CERT_UNSUPPORTED,
                    }
                };
            }
            std::ptr::null_mut()
        }
    }
}

/// [`nori_ios_welcome_on`]'s events: a kind picked or dropped, Continue, a profile of a kind added, and back
/// from a kind's form.
pub const WELCOME_TOGGLED: i32 = 0;
pub const WELCOME_CONTINUE: i32 = 1;
pub const WELCOME_ADDED: i32 = 2;
pub const WELCOME_BACK: i32 = 3;
/// What [`nori_ios_welcome_on`] asks next: nothing, or the first run's questions.
pub const WELCOME_NOTHING: i32 = 0;
pub const WELCOME_QUESTIONS: i32 = 1;

/// A new first-run welcome (`WelcomeFlow`), the app's to hold and give to [`nori_ios_welcome_free`].
#[no_mangle]
pub extern "C" fn nori_ios_welcome_new() -> *mut WelcomeFlow {
    Box::into_raw(Box::default())
}

/// # Safety
/// `flow` came from [`nori_ios_welcome_new`] and is not used after.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_welcome_free(flow: *mut WelcomeFlow) {
    // SAFETY: the caller's.
    drop(unsafe { Box::from_raw(flow) });
}

/// The welcome takes `event` (`WELCOME_*`; `kind` the kind toggled or added, as the settings store it) and
/// answers what the app does next. The iPod offers no plugins, so nothing is ever to install.
///
/// # Safety
/// `flow` came from [`nori_ios_welcome_new`]; `kind` is NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_welcome_on(flow: *mut WelcomeFlow, event: i32, kind: *const c_char) -> i32 {
    // SAFETY: the caller's.
    let flow = unsafe { &mut *flow };
    let kind = || ServerKind::from_id(&text(kind));
    let event = match event {
        WELCOME_TOGGLED => kind().map(|kind| WelcomeEvent::Toggled { source: WelcomeSource::Kind { kind } }),
        WELCOME_CONTINUE => Some(WelcomeEvent::Continue),
        WELCOME_ADDED => kind().map(|kind| WelcomeEvent::Added { kind }),
        WELCOME_BACK => Some(WelcomeEvent::Back),
        _ => None,
    };
    match event.map(|e| flow.on(e)) {
        Some(WelcomeDo::Questions) => WELCOME_QUESTIONS,
        _ => WELCOME_NOTHING,
    }
}

/// The welcome as JSON to free: `{picked: [kind], one}`, the kinds picked as the settings store them, and
/// `one` the kind whose form shows now, or null.
///
/// # Safety
/// `flow` came from [`nori_ios_welcome_new`].
#[no_mangle]
pub unsafe extern "C" fn nori_ios_welcome(flow: *const WelcomeFlow) -> *mut c_char {
    // SAFETY: the caller's.
    let flow = unsafe { &*flow };
    let picked: Vec<String> = flow.picked.iter().filter_map(|s| if let WelcomeSource::Kind { kind } = s { Some(kind.id().to_string()) } else { None }).collect();
    owned(&serde_json::json!({ "picked": picked, "one": flow.one().map(|k| k.id().to_string()) }).to_string()).into_raw()
}

/// Why the proxy's sheet's fields are not a proxy, for `nori_ios_proxy`.
pub const PROXY_NOT_SOCKS5: i32 = 1;
pub const PROXY_HOST: i32 = 2;
pub const PROXY_PORT: i32 = 3;
pub const PROXY_LOGIN: i32 = 4;

/// The proxy the sheet's `address`, `user` and `password` give, as the page shows it: `host:port`, or
/// empty for none (an empty address). NULL when they are not a proxy, with a `PROXY_*` code in `error`.
/// Free with `nori_ios_free`.
///
/// # Safety
/// The strings are NUL-terminated UTF-8, or NULL. `error` is NULL or a pointer this function writes.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_proxy(address: *const c_char, user: *const c_char, password: *const c_char, error: *mut i32) -> *mut c_char {
    match proxy_from_form(text(address), text(user), text(password)) {
        Ok(proxy) => owned(&proxy_address(proxy)).into_raw(),
        Err(e) => {
            if !error.is_null() {
                unsafe { *error = proxy_error(e) };
            }
            std::ptr::null_mut()
        }
    }
}

fn proxy_error(e: ProxyError) -> i32 {
    match e {
        ProxyError::NotSocks5 => PROXY_NOT_SOCKS5,
        ProxyError::Host => PROXY_HOST,
        ProxyError::Port => PROXY_PORT,
        ProxyError::Login => PROXY_LOGIN,
    }
}

/// Checks the profile with its provider (a server is pinged) and, when it passes, saves it as the
/// active one.
pub(crate) fn login(dir: &Path, form: &Form) -> (i32, Option<String>) {
    let draft = match draft(form) {
        Ok(draft) => draft,
        Err(code) => return (code, None),
    };
    let (settings, _) = match opened(dir) {
        Ok(v) => v,
        Err(e) => return (LOGIN_DATABASE, Some(e)),
    };
    match block_on(log_in(&Kinds::builtin(), Http::for_profile(&draft, dir), &settings, draft)) {
        Ok(_) => {
            settings.writer.flush();
            (LOGIN_OK, None)
        }
        Err(e) => fail(e),
    }
}

/// The active server as JSON to free, `{label, kind}` (`{}` when none are saved); NULL for an
/// unreadable path.
///
/// # Safety
/// `data_dir` is NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_active(data_dir: *const c_char) -> *mut c_char {
    let dir = text(data_dir);
    if dir.is_empty() {
        return std::ptr::null_mut();
    }
    match active(Path::new(&dir)) {
        Ok(found) => {
            let row = found.map_or(serde_json::json!({}), |(label, kind)| serde_json::json!({ "label": label, "kind": kind.id() }));
            owned(&row.to_string()).into_raw()
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Checks the address and saves the profile as the active server. Does not open playback.
/// Returns a [`LOGIN_OK`] code. `detail`, when not NULL, receives a string to free, or NULL.
///
/// # Safety
/// The strings are NUL-terminated UTF-8, or NULL. `detail` is NULL or a pointer this function writes.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_login(
    data_dir: *const c_char,
    form: *const c_char,
    detail: *mut *mut c_char,
) -> i32 {
    if !detail.is_null() {
        unsafe { *detail = std::ptr::null_mut() };
    }
    let dir = text(data_dir);
    if dir.is_empty() {
        return LOGIN_DATABASE;
    }
    let form = Form::parse(&text(form));
    let (code, extra) = login(Path::new(&dir), &form);
    if !detail.is_null() {
        if let Some(extra) = extra.filter(|s| !s.is_empty()) {
            unsafe { *detail = owned(&extra).into_raw() };
        }
    }
    code
}

/// The saved servers as JSON to free: `[{id, label, name, url, active, kind, waits, icon, letter}]`. NULL for an
/// unreadable path.
///
/// # Safety
/// `data_dir` is NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_servers(data_dir: *const c_char) -> *mut c_char {
    let dir = text(data_dir);
    let Ok(rows) = servers(Path::new(&dir)) else {
        return std::ptr::null_mut();
    };
    owned(&serde_json::Value::from(rows).to_string()).into_raw()
}

/// Makes saved server `id` active (1) or finds none with that id (0). The open session keeps playing
/// the old one until `nori_ios_reopen`.
///
/// # Safety
/// Both are NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_pick_server(data_dir: *const c_char, id: *const c_char) -> i32 {
    i32::from(pick(Path::new(&text(data_dir)), &text(id)).unwrap_or(false))
}

/// Saved server `id`'s circle shows `icon`, one of `nori_ios_profile_icons`' ids; an unknown icon changes
/// nothing.
///
/// # Safety
/// The strings are NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_set_server_icon(data_dir: *const c_char, id: *const c_char, icon: *const c_char) {
    if let Some(icon) = ProfileIcon::from_id(&text(icon)) {
        let _ = set_icon(Path::new(&text(data_dir)), &text(id), icon);
    }
}

/// Renames saved server `id`; blank, it is called after its provider again.
///
/// # Safety
/// The strings are NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_rename_server(data_dir: *const c_char, id: *const c_char, name: *const c_char) {
    let _ = rename(Path::new(&text(data_dir)), &text(id), &text(name));
}

/// The icons a profile's circle can show, in the picker's order, as JSON to free: `["letter", ...]`.
#[no_mangle]
pub extern "C" fn nori_ios_profile_icons() -> *mut c_char {
    let ids: Vec<&str> = profile_icons().into_iter().map(ProfileIcon::id).collect();
    owned(&serde_json::Value::from(ids).to_string()).into_raw()
}

/// Forgets saved server `id`. Its downloads and index stay in the database.
///
/// # Safety
/// Both are NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_remove_server(data_dir: *const c_char, id: *const c_char) {
    let _ = remove(Path::new(&text(data_dir)), &text(id));
}

/// Saved server `id`'s scrobbling accounts as JSON to free: `[{id, kind, user, host, on}]`, `kind` as the
/// settings store it, `host` the address's, empty for the kind's own service. NULL for an unreadable path.
///
/// # Safety
/// Both are NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_scrobblers(data_dir: *const c_char, id: *const c_char) -> *mut c_char {
    let Ok(settings) = live_settings(Path::new(&text(data_dir))) else { return std::ptr::null_mut() };
    let id = text(id);
    let accounts = settings.prefs(|p| p.servers.iter().find(|s| s.id == id).map(|s| s.scrobblers.clone()).unwrap_or_default());
    let rows: Vec<serde_json::Value> = accounts
        .iter()
        .map(|a| serde_json::json!({ "id": a.id, "kind": a.kind.id(), "user": a.user, "host": nori_core::settings::label("", &a.url), "on": a.on }))
        .collect();
    owned(&serde_json::Value::from(rows).to_string()).into_raw()
}

/// Whether saved server `id` is told what plays itself, first in its scrobbling list: 1 on, 0 off, -1
/// where its rules have it take none (the open profile's as its session has them).
///
/// # Safety
/// Both are NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_scrobble_itself(data_dir: *const c_char, id: *const c_char) -> i32 {
    let Ok(settings) = live_settings(Path::new(&text(data_dir))) else { return -1 };
    let (prefs, id) = (settings.current().unwrap_or_default(), text(id));
    let Some(profile) = prefs.servers.iter().find(|s| s.id == id) else { return -1 };
    let open = (prefs.active_server_id == id).then(|| crate::session::with_session(|s| s.rules().scrobbles)).flatten();
    match open.unwrap_or_else(|| Kinds::builtin().rules_of(profile.clone(), false).scrobbles) {
        true => i32::from(!profile.scrobble_off),
        false => -1,
    }
}

/// Saved server `id` told what plays itself (non-zero), or not.
///
/// # Safety
/// Both are NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_switch_scrobble_itself(data_dir: *const c_char, id: *const c_char, on: i32) {
    if let Ok(settings) = live_settings(Path::new(&text(data_dir))) {
        nori_host::server_scrobble_switched(&settings, &text(id), on != 0);
        settings.writer.flush();
    }
}

/// Saved server `id`'s scrobbling account `account` turned on (non-zero) or off.
///
/// # Safety
/// The strings are NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_switch_scrobbler(data_dir: *const c_char, id: *const c_char, account: *const c_char, on: i32) {
    if let Ok(settings) = live_settings(Path::new(&text(data_dir))) {
        nori_host::scrobbler_switched(&settings, &text(id), &text(account), on != 0);
        settings.writer.flush();
    }
}

/// Saved server `id` without its scrobbling account `account`.
///
/// # Safety
/// The strings are NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_remove_scrobbler(data_dir: *const c_char, id: *const c_char, account: *const c_char) {
    if let Ok(settings) = live_settings(Path::new(&text(data_dir))) {
        nori_host::scrobbler_removed(&settings, &text(id), &text(account));
        settings.writer.flush();
    }
}

/// The kinds of scrobbling account a profile can add, with the fields each asks, as JSON to free
/// (`nori_core::scrobblers::scrobbler_kinds_json`).
#[no_mangle]
pub extern "C" fn nori_ios_scrobbler_kinds() -> *mut c_char {
    owned(&nori_core::scrobblers::scrobbler_kinds_json()).into_raw()
}

/// A token to allow on the scrobbling service's page for the account `form` describes (JSON as
/// [`nori_ios_add_scrobbler`] takes it), as JSON to free: `{token, page}`. Blocks on the network. NULL
/// when there is none, with a [`LOGIN_OK`] code's failure in `code` and `detail` as [`nori_ios_login`]'s.
///
/// # Safety
/// `form` is NUL-terminated UTF-8, or NULL. `code` and `detail` are NULL or pointers this function writes.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_scrobbler_approval(form: *const c_char, code: *mut i32, detail: *mut *mut c_char) -> *mut c_char {
    if !detail.is_null() {
        unsafe { *detail = std::ptr::null_mut() };
    }
    let v: serde_json::Value = serde_json::from_str(&text(form)).unwrap_or_default();
    let found = nori_core::scrobblers::ScrobblerForm::from_json(v["kind"].as_str().unwrap_or_default(), &v["fields"], "")
        .ok_or((LOGIN_INCOMPLETE, None))
        .and_then(|form| crate::session::server().map(|(client, _)| (client, form)).ok_or((LOGIN_DATABASE, None)))
        .and_then(|(client, form)| block_on(client.scrobbler_approval(form)).map_err(fail));
    let (failed, extra) = match found {
        Ok(a) => return owned(&serde_json::json!({ "token": a.token, "page": a.page }).to_string()).into_raw(),
        Err(f) => f,
    };
    if !code.is_null() {
        unsafe { *code = failed };
    }
    if let (false, Some(extra)) = (detail.is_null(), extra.filter(|s| !s.is_empty())) {
        unsafe { *detail = owned(&extra).into_raw() };
    }
    std::ptr::null_mut()
}

/// Signs in to the scrobbling account `form` describes, JSON `{kind, fields: {id: typed}, approved}` by the ids
/// [`nori_ios_scrobbler_kinds`] gives, `approved` the token allowed on the service's page where it signs
/// in there ([`nori_ios_scrobbler_approval`]), and keeps it in saved server `id`. Blocks on the network. Returns a
/// [`LOGIN_OK`] code, `detail` as [`nori_ios_login`]'s; [`LOGIN_DATABASE`] while no profile is open.
///
/// # Safety
/// The strings are NUL-terminated UTF-8, or NULL. `detail` is NULL or a pointer this function writes.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_add_scrobbler(data_dir: *const c_char, id: *const c_char, form: *const c_char, detail: *mut *mut c_char) -> i32 {
    if !detail.is_null() {
        unsafe { *detail = std::ptr::null_mut() };
    }
    let v: serde_json::Value = serde_json::from_str(&text(form)).unwrap_or_default();
    let Some(form) = nori_core::scrobblers::ScrobblerForm::from_json(v["kind"].as_str().unwrap_or_default(), &v["fields"], v["approved"].as_str().unwrap_or_default()) else {
        return LOGIN_INCOMPLETE;
    };
    let (Ok(settings), Some(client)) = (live_settings(Path::new(&text(data_dir))), crate::session::server().map(|(client, _)| client)) else { return LOGIN_DATABASE };
    let (code, extra) = match block_on(client.sign_in_scrobbler(form)) {
        Ok(account) => {
            nori_host::scrobbler_kept(&settings, &text(id), account);
            settings.writer.flush();
            (LOGIN_OK, None)
        }
        Err(e) => fail(e),
    };
    if let (false, Some(extra)) = (detail.is_null(), extra.filter(|s| !s.is_empty())) {
        unsafe { *detail = owned(&extra).into_raw() };
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_core::providers::profile_activated;

    /// Writes `profile` as the active server and waits until the database has it.
    fn keep(dir: &Path, profile: SavedServer) -> Result<(), String> {
        let (settings, prefs) = opened(dir)?;
        settings.put(profile_activated(prefs, profile));
        settings.writer.flush();
        Ok(())
    }

    /// Changes to the saved list go to the open session's settings when one is open: these tests wait for
    /// the tests that open one.
    fn no_session() -> std::sync::MutexGuard<'static, ()> {
        crate::pages::ABI_IN_USE.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn profile(id: &str, user: &str) -> SavedServer {
        SavedServer {
            id: id.into(),
            url: format!("http://127.0.0.1:9/{id}"),
            user: user.into(),
            ..SavedServer::default()
        }
    }

    #[test]
    fn a_server_picked_on_the_welcome_shows_its_form_then_the_questions() {
        let read = |flow: *const WelcomeFlow| -> serde_json::Value {
            let raw = unsafe { nori_ios_welcome(flow) };
            let v = serde_json::from_str(unsafe { CStr::from_ptr(raw) }.to_str().unwrap()).unwrap();
            unsafe { crate::nori_ios_free(raw) };
            v
        };
        let server = CString::new("subsonic").unwrap();
        let on = |flow, event| unsafe { nori_ios_welcome_on(flow, event, server.as_ptr()) };
        let flow = nori_ios_welcome_new();
        assert_eq!(on(flow, WELCOME_TOGGLED), WELCOME_NOTHING);
        assert_eq!(read(flow), serde_json::json!({ "picked": ["subsonic"], "one": null }));
        on(flow, WELCOME_CONTINUE);
        assert_eq!(read(flow)["one"], "subsonic");
        on(flow, WELCOME_BACK);
        assert_eq!(read(flow)["one"], serde_json::Value::Null, "back on the sources");
        on(flow, WELCOME_CONTINUE);
        assert_eq!(on(flow, WELCOME_ADDED), WELCOME_QUESTIONS);
        unsafe { nori_ios_welcome_free(flow) };
    }

    #[test]
    fn a_picked_server_becomes_active_and_a_removed_one_hands_over() {
        let _turn = no_session();
        let dir = nori_testdir::TempDir::new("ios-pick");
        keep(dir.path(), profile("one", "ada")).unwrap();
        keep(dir.path(), profile("two", "bob")).unwrap();
        assert_eq!(active(dir.path()).unwrap().unwrap().0, "bob");
        assert!(pick(dir.path(), "one").unwrap());
        assert!(!pick(dir.path(), "nope").unwrap());
        assert_eq!(active(dir.path()).unwrap().unwrap().0, "ada");
        remove(dir.path(), "one").unwrap();
        let left = servers(dir.path()).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!((&left[0]["id"], &left[0]["active"]), (&serde_json::json!("two"), &serde_json::json!(true)));
    }

    /// A plugin's profile is listed as one that never opens here; a server's opens.
    #[test]
    fn a_plugin_profile_is_listed_as_waiting() {
        let dir = nori_testdir::TempDir::new("ios-waits");
        keep(dir.path(), profile("one", "ada")).unwrap();
        keep(dir.path(), SavedServer { kind: ServerKind::Plugin { id: "fm.nori.youtube-music".into() }, ..profile("two", "") }).unwrap();
        let waits: Vec<_> = servers(dir.path()).unwrap().iter().map(|s| (s["id"].clone(), s["waits"].clone())).collect();
        assert_eq!(waits, [(serde_json::json!("one"), serde_json::json!(false)), (serde_json::json!("two"), serde_json::json!(true))]);
    }

    /// A chosen icon is listed with its profile, the others' stay; the letter is who signs in, else the
    /// name, else the address's host.
    #[test]
    fn a_chosen_icon_is_listed_with_its_letter() {
        let _turn = no_session();
        let dir = nori_testdir::TempDir::new("ios-icon");
        keep(dir.path(), profile("one", "ada")).unwrap();
        keep(dir.path(), SavedServer { url: "https://music.example.com/x".into(), ..profile("two", "") }).unwrap();
        set_icon(dir.path(), "two", ProfileIcon::LookUp).unwrap();
        set_icon(dir.path(), "nope", ProfileIcon::Think).unwrap();
        let rows: Vec<_> = servers(dir.path()).unwrap().iter().map(|s| (s["id"].clone(), s["icon"].clone(), s["letter"].clone())).collect();
        let row = |id: &str, icon: &str, letter: &str| (serde_json::json!(id), serde_json::json!(icon), serde_json::json!(letter));
        assert_eq!(rows, [row("one", "letter", "A"), row("two", "look-up", "M")]);
    }

    #[test]
    fn a_renamed_server_is_listed_by_its_new_name_and_by_its_user_once_blank() {
        let _turn = no_session();
        let dir = nori_testdir::TempDir::new("ios-rename");
        keep(dir.path(), profile("one", "ada")).unwrap();
        rename(dir.path(), "one", "  Kitchen ").unwrap();
        assert_eq!(servers(dir.path()).unwrap()[0]["label"], "Kitchen");
        rename(dir.path(), "one", " ").unwrap();
        assert_eq!(servers(dir.path()).unwrap()[0]["label"], "ada");
    }

    #[test]
    fn a_saved_server_is_the_active_one() {
        let dir = nori_testdir::TempDir::new("ios-keep");
        let profile = SavedServer {
            id: "one".into(),
            url: "http://127.0.0.1:9".into(),
            user: "ada".into(),
            password: "secret".into(),
            ..SavedServer::default()
        };
        keep(dir.path(), profile).unwrap();
        assert_eq!(active(dir.path()).unwrap(), Some(("ada".into(), ServerKind::Subsonic)));
    }

    #[test]
    fn an_unreachable_server_is_not_saved() {
        let dir = nori_testdir::TempDir::new("ios-login");
        let form = Form { url: "http://127.0.0.1:1".into(), user: "ada".into(), password: "secret".into(), ..Form::default() };
        let (code, _) = login(dir.path(), &form);
        assert_eq!(code, LOGIN_UNREACHABLE);
        assert_eq!(active(dir.path()).unwrap(), None);
    }

    #[test]
    fn the_advanced_fields_reach_the_profile_and_a_key_stands_in_for_a_user() {
        let form = Form {
            url: "music.example.org".into(),
            name: " Home ".into(),
            alt: "192.168.1.5:4533".into(),
            key: "k1".into(),
            ..Form::default()
        };
        let p = draft(&form).unwrap();
        assert_eq!((p.name.as_str(), p.api_key.as_str(), p.user.as_str()), ("Home", "k1", ""));
        assert_eq!(p.alt_url, address("192.168.1.5:4533"));
        assert!(p.url.starts_with("http"), "{}", p.url);
        assert_eq!(draft(&Form { key: String::new(), ..form.clone() }).err(), Some(LOGIN_INCOMPLETE), "neither a user nor a key");
        let proxied = Form { proxy: "proxy.lan:1081".into(), proxy_user: " me ".into(), proxy_password: " pw".into(), ..form.clone() };
        let login = nori_core::proxy::ProxyLogin { user: "me".into(), password: " pw".into() };
        assert_eq!(draft(&proxied).unwrap().proxy, Some(nori_core::proxy::SavedProxy { host: "proxy.lan".into(), port: 1081, login: Some(login) }));
        assert_eq!(draft(&Form { proxy: "http://proxy.lan".into(), ..form }).err(), Some(LOGIN_PROXY));
        let shown = |address: &str, user: &str| {
            let (address, user, password) = (owned(address), owned(user), owned(""));
            let mut error = 0;
            let got = unsafe { nori_ios_proxy(address.as_ptr(), user.as_ptr(), password.as_ptr(), &mut error) };
            let got = (!got.is_null()).then(|| unsafe { CString::from_raw(got) }.into_string().unwrap());
            (got, error)
        };
        assert_eq!(shown(" socks5://proxy.lan ", "me"), (Some("proxy.lan:1080".into()), 0), "as the page shows it");
        assert_eq!(shown("", "me"), (Some(String::new()), 0), "no address, no proxy");
        assert_eq!(shown("proxy.lan:0", ""), (None, PROXY_PORT));
        let parsed = Form::parse(r#"{"url":"u","user":"ada","alt":"a","proxy":"p","proxyUser":"pu","proxyPassword":"pp","headers":"X-A: b","selfSigned":true}"#);
        let read = [&parsed.user, &parsed.alt, &parsed.key, &parsed.proxy, &parsed.proxy_user, &parsed.proxy_password, &parsed.headers].map(String::as_str);
        assert_eq!(read, ["ada", "a", "", "p", "pu", "pp", "X-A: b"]);
        assert!(parsed.self_signed && !parsed.legacy_auth);
    }

    /// Headers, the switches and the certificate kept reach the profile, under the form's id.
    #[test]
    fn the_connection_options_reach_the_profile() {
        let form = Form { id: "p1".into(), url: "music.example.org".into(), user: "ada".into(), headers: "X-A: b\nbad line".into(), legacy_auth: true, self_signed: true, client_cert: "p1.p12".into(), client_cert_password: "pw".into(), ..Form::default() };
        let p = draft(&form).unwrap();
        assert_eq!((p.id.as_str(), p.headers.get("X-A").map(String::as_str), p.headers.len()), ("p1", Some("b"), 1));
        assert_eq!((p.legacy_auth, p.allow_self_signed, p.client_cert.as_str(), p.client_cert_password.as_str()), (true, true, "p1.p12", "pw"));
    }

    /// The page shows what the core lists for the iPod, in order: no Wi-Fi only on a device that has only
    /// Wi-Fi, and no user or password while an API key is typed.
    #[test]
    fn the_form_shows_what_the_ipod_honors() {
        let shown = |json: &str| -> Vec<String> {
            let json = owned(json);
            let raw = unsafe { nori_ios_form_fields(json.as_ptr()) };
            let v: serde_json::Value = serde_json::from_str(unsafe { CStr::from_ptr(raw) }.to_str().unwrap()).unwrap();
            unsafe { crate::nori_ios_free(raw) };
            v.as_array().unwrap().iter().map(|f| f["what"].as_str().unwrap().to_string()).collect()
        };
        let all = ["url", "user", "password", "name", "alt", "key", "headers", "legacyAuth", "selfSigned", "clientCert", "proxy"];
        assert_eq!(shown(r#"{"kind":"subsonic"}"#), all);
        assert_eq!(shown(r#"{"kind":"subsonic","key":"k"}"#), all.iter().filter(|w| !["user", "password"].contains(w)).copied().collect::<Vec<_>>());
    }

    #[test]
    fn a_certificate_is_kept_by_the_forms_id() {
        let dir = nori_testdir::TempDir::new("ios-cert");
        let import = |path: &str, password: &str| {
            let (d, id, path, password) = (owned(dir.path().to_str().unwrap()), owned("p1"), owned(path), owned(password));
            let mut error = 0;
            let got = unsafe { nori_ios_import_cert(d.as_ptr(), id.as_ptr(), path.as_ptr(), password.as_ptr(), &mut error) };
            ((!got.is_null()).then(|| unsafe { CString::from_raw(got) }.into_string().unwrap()), error)
        };
        let p12 = concat!(env!("CARGO_MANIFEST_DIR"), "/../cert/tests/data/client-legacy.p12");
        assert_eq!(import(p12, "nope"), (None, CERT_PASSWORD));
        let (Some(name), 0) = import(p12, "secret") else { panic!("taken") };
        assert!(name.starts_with("p1-") && dir.path().join("certs").join(name).exists());
    }

    #[test]
    fn a_blank_login_is_not_saved() {
        let dir = nori_testdir::TempDir::new("ios-blank");
        let (code, _) = login(dir.path(), &Form::default());
        assert_eq!(code, LOGIN_INCOMPLETE);
        assert!(dir.path().read_dir().unwrap().next().is_none());
    }
}
