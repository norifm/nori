//! The sign-in form's own values and its option lines. A form holds the profile as typed from the moment
//! it opens (its id too, so a client certificate is kept by it) and draws the fields the core lists for
//! its kind on a computer (`form_fields`): the boxed ones the window binds itself (a name, an address, a
//! user and a password the form shows plainly), every other option a line of a group, those the kind
//! folds away under Advanced settings.

use std::path::Path;
use std::rc::Rc;

use nori_core::providers::{FieldSpec, FirstStep, FormField, Platform, ProviderInfo};
use nori_core::settings::SavedServer;
use slint::{Model, ModelRc, VecModel};

use crate::{words, AppWindow, FormLine};

/// The form's fields by their `FormLine::field`.
const FIELDS: [FormField; 12] = {
    use FormField::*;
    [Name, Address, User, Password, SecondAddress, ApiKey, Headers, LegacyAuth, SelfSigned, WifiOnly, ClientCert, Proxy]
};

fn field_number(what: FormField) -> i32 {
    FIELDS.iter().position(|&f| f == what).map_or(-1, |i| i as i32)
}

/// A field the window draws as a box of its own, bound to its own property.
fn boxed(f: &FieldSpec) -> bool {
    !f.advanced && matches!(f.what, FormField::Name | FormField::Address | FormField::User | FormField::Password)
}

pub struct Form {
    info: ProviderInfo,
    step: FirstStep,
    adding: bool,
    /// The profile as its lines and sheets have it; the boxed fields are the window's until it is sent.
    typed: SavedServer,
    /// The headers as typed, one per line.
    headers: String,
    plain: Rc<VecModel<FormLine>>,
    advanced: Rc<VecModel<FormLine>>,
}

impl Form {
    /// The form of `info`'s kind at `step` over `profile` (a new one, or one saved when not `adding`),
    /// drawn on `ui`: its boxes asked, its lines, Advanced settings open when a value of its own is set.
    pub fn open(ui: &AppWindow, info: ProviderInfo, step: FirstStep, profile: SavedServer, adding: bool) -> Form {
        let headers = nori_core::settings::server_headers_text(profile.headers.clone());
        let form = Form { info, step, adding, typed: profile, headers, plain: Rc::default(), advanced: Rc::default() };
        let fields = form.fields();
        form.lines_drawn(ui, &fields);
        form.boxes_drawn(ui, &fields);
        ui.set_login_advanced_open(!adding && nori_core::providers::advanced_shown(&fields, &form.typed));
        ui.set_login_proxy_open(false);
        ui.set_login_cert_open(false);
        crate::app::form_proxy_drawn(ui, form.typed.proxy.as_ref());
        ui.set_login_cert_set(!form.typed.client_cert.is_empty());
        form
    }

    /// The fields shown now, in the core's order.
    fn fields(&self) -> Vec<FieldSpec> {
        nori_core::providers::form_fields(&self.info, Platform::Computer, &self.typed, self.adding)
    }

    /// Whether the form saves anything: a saved profile with nothing to edit here has no button.
    pub fn saves(&self) -> bool {
        !self.fields().is_empty()
    }

    /// Which of the window's boxes it asks: those of `fields`, on a page that is the form.
    fn boxes_drawn(&self, ui: &AppWindow, fields: &[FieldSpec]) {
        let asks = |what| self.step == FirstStep::Form && fields.iter().any(|f| boxed(f) && f.what == what);
        let mut kind = ui.get_login_form();
        (kind.asks_name, kind.asks_address, kind.asks_user, kind.asks_password) = (asks(FormField::Name), asks(FormField::Address), asks(FormField::User), asks(FormField::Password));
        ui.set_login_form(kind);
    }

    /// The lines of `fields`: those shown plainly on a page that is the form, and the advanced ones on any
    /// (the proxy carries signing in elsewhere too).
    fn lines_drawn(&self, ui: &AppWindow, fields: &[FieldSpec]) {
        let plain: Vec<FormLine> = fields.iter().filter(|f| self.step == FirstStep::Form && !f.advanced && !boxed(f)).map(|f| self.line(f.what)).collect();
        let advanced: Vec<FormLine> = fields.iter().filter(|f| f.advanced).map(|f| self.line(f.what)).collect();
        self.plain.set_vec(plain);
        self.advanced.set_vec(advanced);
        ui.set_login_plain(ModelRc::from(self.plain.clone()));
        ui.set_login_advanced(ModelRc::from(self.advanced.clone()));
    }

    /// Field `what` as its line draws it.
    fn line(&self, what: FormField) -> FormLine {
        use FormField::*;
        let [title, hint, placeholder] = words::form_option(what);
        let p = &self.typed;
        let set = |set: bool, value: String| if set { value } else { words::NOTHING_SET.to_string() };
        let (kind, text, on) = match what {
            Name => (0, p.name.clone(), false),
            Address => (0, p.url.clone(), false),
            User => (0, p.user.clone(), false),
            Password => (0, p.password.clone(), false),
            SecondAddress => (0, p.alt_url.clone(), false),
            ApiKey => (0, p.api_key.clone(), false),
            Headers => (1, self.headers.clone(), false),
            LegacyAuth => (2, String::new(), p.legacy_auth),
            SelfSigned => (2, String::new(), p.allow_self_signed),
            WifiOnly => (2, String::new(), p.wifi_only),
            ClientCert => (3, set(!p.client_cert.is_empty(), words::CERT_SET.into()), false),
            Proxy => (3, set(p.proxy.is_some(), nori_core::proxy::proxy_address(p.proxy.clone())), false),
        };
        FormLine { field: field_number(what), kind, title: title.into(), hint: hint.into(), text: text.into(), placeholder: placeholder.into(), on }
    }

    /// Field `what`'s line drawn anew with its value, wherever it shows.
    fn redrawn(&self, what: FormField) {
        let number = field_number(what);
        for model in [&self.plain, &self.advanced] {
            if let Some(i) = model.iter().position(|l| l.field == number) {
                model.set_row_data(i, self.line(what));
            }
        }
    }

    /// Text typed into option `field`. An API key typed or cleared shows or hides the user and password.
    pub fn typed(&mut self, ui: &AppWindow, field: i32, text: String) {
        let Some(&what) = FIELDS.get(field as usize) else { return };
        let before = self.fields();
        let p = &mut self.typed;
        match what {
            FormField::Name => p.name = text,
            FormField::Address => p.url = text,
            FormField::User => p.user = text,
            FormField::Password => p.password = text,
            FormField::SecondAddress => p.alt_url = text,
            FormField::ApiKey => p.api_key = text,
            FormField::Headers => self.headers = text,
            _ => return,
        }
        let after = self.fields();
        if after != before {
            self.boxes_drawn(ui, &after);
        }
        if after.iter().filter(|f| !boxed(f)).ne(before.iter().filter(|f| !boxed(f))) {
            self.lines_drawn(ui, &after);
        } else {
            // Kept in its row, so the line drawn again after folding has it; the input typed into holds
            // its own text meanwhile.
            self.redrawn(what);
        }
    }

    /// Option `field`'s switch turned `on`.
    pub fn toggled(&mut self, field: i32, on: bool) {
        let Some(&what) = FIELDS.get(field as usize) else { return };
        let p = &mut self.typed;
        match what {
            FormField::LegacyAuth => p.legacy_auth = on,
            FormField::SelfSigned => p.allow_self_signed = on,
            FormField::WifiOnly => p.wifi_only = on,
            _ => return,
        }
        self.redrawn(what);
    }

    /// Option `field`'s line pressed: its sheet opens.
    pub fn opened(&self, ui: &AppWindow, field: i32) {
        match FIELDS.get(field as usize) {
            Some(FormField::Proxy) => {
                ui.set_login_proxy_error("".into());
                ui.set_login_proxy_open(true);
            }
            Some(FormField::ClientCert) => {
                ui.set_login_cert_error("".into());
                ui.set_login_cert_open(true);
            }
            _ => {}
        }
    }

    /// The proxy's sheet saved what was typed in it ([`crate::app::proxy_sheet_saved`]).
    pub fn proxy_saved(&mut self, ui: &AppWindow, address: String, user: String, password: String) {
        if let Some(proxy) = crate::app::proxy_sheet_saved(ui, address, user, password) {
            self.typed.proxy = proxy;
            self.redrawn(FormField::Proxy);
        }
    }

    /// The certificate's sheet saved: the .p12 file at `path`, opened with `password`, kept under `data` as
    /// the form's, the sheet closed; or the sheet says what is wrong. With no path, a certificate set stays.
    pub fn cert_saved(&mut self, ui: &AppWindow, data: &Path, path: &str, password: String) {
        let path = path.trim();
        if path.is_empty() {
            match self.typed.client_cert.is_empty() {
                true => ui.set_login_cert_error(words::CERT_FILE_MISSING.into()),
                false => ui.set_login_cert_open(false),
            }
            return;
        }
        match nori_core::cert::import_client_cert_file(data, self.typed.clone(), Path::new(path), password) {
            Ok(p) => self.cert_shown(ui, p.client_cert, p.client_cert_password),
            Err(e) => ui.set_login_cert_error(words::cert_error(e).into()),
        }
    }

    /// The certificate's sheet took the one set off.
    pub fn cert_removed(&mut self, ui: &AppWindow) {
        self.cert_shown(ui, String::new(), String::new());
    }

    fn cert_shown(&mut self, ui: &AppWindow, cert: String, password: String) {
        (self.typed.client_cert, self.typed.client_cert_password) = (cert, password);
        self.redrawn(FormField::ClientCert);
        ui.set_login_cert_set(!self.typed.client_cert.is_empty());
        ui.set_login_cert_open(false);
    }

    /// The profile the form saves, with the window's boxes as typed (`[name, address, user, password]`):
    /// over `saved`, the profile it edits as saved now, or a new one with the form's id. What the form
    /// does not show stays as it was, and so does a saved profile's password when none is typed.
    pub fn draft(&self, [name, url, user, password]: [String; 4], saved: Option<SavedServer>) -> SavedServer {
        let mut fields = self.fields();
        let mut typed = self.typed.clone();
        for f in fields.iter().filter(|f| boxed(f)) {
            match f.what {
                FormField::Name => typed.name = name.clone(),
                FormField::Address => typed.url = url.clone(),
                FormField::User => typed.user = user.clone(),
                _ => typed.password = password.clone(),
            }
        }
        for text in [&mut typed.name, &mut typed.user, &mut typed.api_key] {
            *text = text.trim().to_string();
        }
        if saved.is_some() && typed.password.is_empty() {
            fields.retain(|f| f.what != FormField::Password);
        }
        let mut typed = nori_core::settings::server_from_form(typed, self.headers.clone());
        typed.url = typed.url.trim_end_matches('/').to_string();
        let base = saved.unwrap_or_else(|| SavedServer { id: self.typed.id.clone(), ..Default::default() });
        SavedServer { kind: self.info.kind.clone(), ..nori_core::providers::form_taken(base, &typed, &fields) }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use i_slint_backend_testing::ElementHandle;
    use nori_core::settings::ServerKind;
    use slint::ComponentHandle;

    use super::*;
    use crate::app::tests::{press, window};
    use crate::{LoginKind, View};

    const SUBSONIC_ADVANCED: [&str; 8] = ["Name", "Second address", "API key", "Extra HTTP headers", "Legacy authentication", "Accept self-signed certificate", "Client certificate", "Proxy"];

    fn subsonic() -> ProviderInfo {
        nori_core::providers::Kinds::builtin().providers().into_iter().find(|i| i.kind == ServerKind::Subsonic).unwrap()
    }

    /// The form of `info`'s kind over `profile` on a window, wired to it as the app wires it, its
    /// certificates kept under `data`.
    struct Shown {
        ui: AppWindow,
        form: Rc<RefCell<Form>>,
    }

    impl Shown {
        fn new(info: ProviderInfo, profile: SavedServer, adding: bool, data: &Path) -> Shown {
            // Once a test's thread: a test may show two forms.
            let _ = slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
                i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
            )));
            let ui = window();
            ui.window().set_size(slint::PhysicalSize::new(1280, 1400));
            ui.set_login_form(LoginKind { add: "Add".into(), ..Default::default() });
            ui.set_view(View::Login);
            let form = Rc::new(RefCell::new(Form::open(&ui, info, FirstStep::Form, profile, adding)));
            let wired = |ui: &AppWindow| (ui.as_weak(), form.clone());
            let (w, f) = wired(&ui);
            ui.on_option_typed(move |field, text| f.borrow_mut().typed(&w.unwrap(), field, text.into()));
            let (_, f) = wired(&ui);
            ui.on_option_toggled(move |field, on| f.borrow_mut().toggled(field, on));
            let (w, f) = wired(&ui);
            ui.on_option_opened(move |field| f.borrow().opened(&w.unwrap(), field));
            let (w, f) = wired(&ui);
            ui.on_proxy_saved(move |a, u, p| f.borrow_mut().proxy_saved(&w.unwrap(), a.into(), u.into(), p.into()));
            let (w, f, data) = (ui.as_weak(), form.clone(), data.to_path_buf());
            ui.on_cert_saved(move |path, password| f.borrow_mut().cert_saved(&w.unwrap(), &data, &path, password.into()));
            let (w, f) = wired(&ui);
            ui.on_cert_removed(move || f.borrow_mut().cert_removed(&w.unwrap()));
            ui.show().unwrap();
            let shown = Shown { ui, form };
            shown.settle();
            shown
        }

        fn settle(&self) {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            self.ui.window().take_snapshot().unwrap();
        }

        /// The option lines' titles, in order.
        fn lines(&self) -> Vec<String> {
            ElementHandle::find_by_element_type_name(&self.ui, "OptionLine").filter_map(|l| l.accessible_label()).map(|l| l.to_string()).collect()
        }

        fn line(&self, title: &str) -> ElementHandle {
            ElementHandle::find_by_element_type_name(&self.ui, "OptionLine").find(|l| l.accessible_label().as_deref() == Some(title)).unwrap_or_else(|| panic!("{title} shows"))
        }

        /// What an entry's line says is set.
        fn value(&self, title: &str) -> String {
            let entry = self.line(title).query_descendants().match_accessible_role(i_slint_backend_testing::AccessibleRole::Button).find_first().expect("an entry");
            entry.accessible_value().unwrap_or_default().to_string()
        }

        fn input(&self, within: &ElementHandle) -> ElementHandle {
            within.query_descendants().match_inherits("TextInput").find_first().expect("a text to type into")
        }

        /// What `title`'s line has typed.
        fn text(&self, title: &str) -> String {
            self.input(&self.line(title)).accessible_value().unwrap_or_default().to_string()
        }

        fn type_into(&self, title: &str, text: &str) {
            self.input(&self.line(title)).set_accessible_value(text);
            self.settle();
        }

        fn press_on(&self, e: &ElementHandle) {
            press(self.ui.window(), e.absolute_position().x + e.size().width / 2.0, e.absolute_position().y + e.size().height / 2.0, slint::platform::PointerEventButton::Left);
            self.settle();
        }

        fn toggle(&self, title: &str) {
            let switch = self.line(title).query_descendants().match_inherits("Toggle").find_first().expect("a switch");
            self.press_on(&switch);
        }

        fn click(&self, label: &str) {
            let e = ElementHandle::find_by_accessible_label(&self.ui, label).next().unwrap_or_else(|| panic!("{label} shows"));
            self.press_on(&e);
        }

        fn said(&self, words: &str) -> bool {
            ElementHandle::find_by_accessible_label(&self.ui, words).next().is_some()
        }

        /// The sheet's field `i` takes `text`.
        fn sheet_typed(&self, i: usize, text: &str) {
            let sheet = ElementHandle::find_by_element_type_name(&self.ui, "Dialog").next().expect("a sheet");
            let input = sheet.query_descendants().match_inherits("TextInput").find_all().remove(i);
            input.set_accessible_value(text);
            self.settle();
        }

        fn boxes(&self) -> usize {
            ElementHandle::find_by_element_type_name(&self.ui, "Field").count()
        }

        fn draft(&self, saved: Option<SavedServer>) -> SavedServer {
            let ui = &self.ui;
            self.form.borrow().draft([ui.get_login_name(), ui.get_login_url(), ui.get_login_user(), ui.get_login_password()].map(|t| t.to_string()), saved)
        }
    }

    /// A server's form folds every option a computer honors under Advanced settings, in the core's order
    /// (Wi-Fi only is not one); a kind that lists none has only the proxy there.
    #[test]
    fn advanced_settings_hold_what_the_kind_lists() {
        let data = nori_testdir::TempDir::new("form");
        let s = Shown::new(subsonic(), SavedServer { id: "p1".into(), ..Default::default() }, true, data.path());
        assert_eq!((s.boxes(), s.lines().len()), (3, 0), "the address, user and password; the rest folded away");
        s.click("Advanced settings");
        assert_eq!(s.lines(), SUBSONIC_ADVANCED);
        assert_eq!((s.value("Client certificate"), s.value("Proxy")), ("None".to_string(), "None".to_string()));
        s.click("Advanced settings");
        assert!(s.lines().is_empty(), "folded again");

        let plugin = ProviderInfo { kind: ServerKind::Plugin { id: "a.b".into() }, fields: vec![FieldSpec { what: FormField::Address, advanced: false }, FieldSpec { what: FormField::Proxy, advanced: true }], sign_in: Vec::new(), identities: None };
        let s = Shown::new(plugin, SavedServer::default(), true, data.path());
        s.click("Advanced settings");
        assert_eq!((s.boxes(), s.lines()), (1, vec!["Proxy".to_string()]));
    }

    /// Texts typed and switches turned stay through folding, and Save takes them with the boxes.
    #[test]
    fn what_is_set_is_kept_through_folding_and_save() {
        let data = nori_testdir::TempDir::new("form");
        let s = Shown::new(subsonic(), SavedServer { id: "p1".into(), kind: ServerKind::Subsonic, ..Default::default() }, true, data.path());
        s.ui.set_login_url("https://music.example/".into());
        s.ui.set_login_user(" ann ".into());
        s.ui.set_login_password("pw".into());
        s.click("Advanced settings");
        s.type_into("Name", "Home");
        s.type_into("Second address", "http://10.0.0.2:4533");
        s.type_into("Extra HTTP headers", "X-Token: t0ken\nX-Other: 1");
        s.toggle("Accept self-signed certificate");
        s.toggle("Legacy authentication");
        s.toggle("Legacy authentication");
        s.click("Advanced settings");
        s.click("Advanced settings");
        assert_eq!((s.text("Second address"), s.text("Extra HTTP headers")), ("http://10.0.0.2:4533".into(), "X-Token: t0ken\nX-Other: 1".into()), "folding loses nothing");
        let p = s.draft(None);
        let headers = std::collections::HashMap::from([("X-Token".to_string(), "t0ken".to_string()), ("X-Other".to_string(), "1".to_string())]);
        assert_eq!((p.id.as_str(), p.name.as_str(), p.url.as_str(), p.user.as_str(), p.password.as_str(), p.alt_url.as_str()), ("p1", "Home", "https://music.example", "ann", "pw", "http://10.0.0.2:4533"));
        assert_eq!((p.headers, p.allow_self_signed, p.legacy_auth), (headers, true, false), "a switch turned on, one on and off again");
    }

    /// An API key takes the place of the user and password: their boxes go while it is typed.
    #[test]
    fn an_api_key_stands_in_for_the_user_and_password() {
        let data = nori_testdir::TempDir::new("form");
        let s = Shown::new(subsonic(), SavedServer::default(), true, data.path());
        s.ui.set_login_user("ann".into());
        s.click("Advanced settings");
        s.type_into("API key", "k3y");
        assert_eq!(s.boxes(), 1, "the address alone");
        let p = s.draft(None);
        assert_eq!((p.api_key.as_str(), p.user.as_str()), ("k3y", ""));
        s.type_into("API key", "");
        assert_eq!(s.boxes(), 3);
        assert_eq!(s.lines(), SUBSONIC_ADVANCED, "the lines stay as they were");
    }

    /// The certificate's line opens its sheet, which keeps a .p12 that opens with its password, says why one
    /// doesn't, and takes the one set off.
    #[test]
    fn the_certificate_line_takes_a_p12_through_its_sheet() {
        let data = nori_testdir::TempDir::new("form");
        let p12 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../cert/tests/data/client-aes.p12");
        let s = Shown::new(subsonic(), SavedServer { id: "p1".into(), ..Default::default() }, true, data.path());
        s.click("Advanced settings");
        s.press_on(&s.line("Client certificate"));
        assert!(s.said("Client certificate") && !s.said("Remove certificate"), "its sheet, nothing to remove");
        s.click("Save");
        assert!(s.said(words::CERT_FILE_MISSING));
        s.sheet_typed(0, p12.to_str().unwrap());
        s.sheet_typed(1, "wrong");
        s.click("Save");
        assert!(s.said(words::cert_error(nori_core::cert::CertError::Password)), "the sheet stays and says why");
        s.sheet_typed(1, "secret");
        s.click("Save");
        assert_eq!(s.value("Client certificate"), words::CERT_SET);
        let p = s.draft(None);
        assert!(p.client_cert.starts_with("p1-") && p.client_cert_password == "secret", "{p:?}");
        assert!(data.path().join("certs").join(&p.client_cert).exists(), "kept by the profile's id");
        s.press_on(&s.line("Client certificate"));
        s.click("Remove certificate");
        assert_eq!((s.value("Client certificate"), s.draft(None).client_cert), ("None".to_string(), String::new()));
    }

    /// The proxy's line, last among the others, says what is set and opens its sheet: Save goes through the
    /// core (an error stays in the sheet), Cancel keeps what was, Remove saves none.
    #[test]
    fn the_proxy_line_opens_its_sheet() {
        let data = nori_testdir::TempDir::new("form");
        let s = Shown::new(subsonic(), SavedServer::default(), true, data.path());
        s.click("Advanced settings");
        assert_eq!(s.lines().last().map(String::as_str), Some("Proxy"));
        s.press_on(&s.line("Proxy"));
        assert!(s.boxes() == 6 && !s.said("Remove proxy"), "the sheet's three fields over the form's; nothing to remove");
        s.sheet_typed(0, "http://proxy.example:1080");
        s.click("Save");
        assert!(s.boxes() == 6 && s.said(words::proxy_error(nori_core::proxy::ProxyError::NotSocks5)), "the sheet stays and says why");
        s.sheet_typed(0, "proxy.example");
        s.click("Save");
        let proxy = nori_core::proxy::proxy_from_form("proxy.example:1080".into(), String::new(), String::new()).unwrap();
        assert_eq!((s.boxes(), s.value("Proxy"), s.draft(None).proxy), (3, "proxy.example:1080".to_string(), proxy.clone()), "saved as the core reads it");
        s.click("Advanced settings");
        s.click("Advanced settings");
        assert_eq!(s.value("Proxy"), "proxy.example:1080", "folding loses nothing");
        s.press_on(&s.line("Proxy"));
        s.sheet_typed(0, "other.example:1080");
        s.click("Cancel");
        assert_eq!((s.value("Proxy"), s.draft(None).proxy), ("proxy.example:1080".to_string(), proxy), "canceled, the proxy stays");
        s.press_on(&s.line("Proxy"));
        s.click("Remove proxy");
        assert_eq!((s.value("Proxy"), s.draft(None).proxy), ("None".to_string(), None), "removed");
    }

    /// A saved profile's Edit page opens Advanced settings on a value set there, its values in their lines
    /// (its name is renamed from its menu); Save keeps what the page doesn't show, and the password when
    /// none is typed.
    #[test]
    fn a_saved_profile_opens_on_what_it_has_set() {
        let data = nori_testdir::TempDir::new("form");
        let proxy = nori_core::proxy::proxy_from_form("proxy.lan:1080".into(), String::new(), String::new()).unwrap();
        let saved = SavedServer {
            id: "p1".into(),
            kind: ServerKind::Subsonic,
            name: "Home".into(),
            url: "https://music.example".into(),
            alt_url: "http://10.0.0.2".into(),
            user: "ann".into(),
            password: "secret".into(),
            allow_self_signed: true,
            proxy: proxy.clone(),
            music_folder_ids: vec!["f".into()],
            ..Default::default()
        };
        let s = Shown::new(subsonic(), saved.clone(), false, data.path());
        assert_eq!(s.lines(), SUBSONIC_ADVANCED[1..], "open, without the name");
        assert_eq!((s.text("Second address"), s.value("Proxy")), ("http://10.0.0.2".to_string(), "proxy.lan:1080".to_string()));
        s.ui.set_login_url(saved.url.as_str().into());
        s.ui.set_login_user(saved.user.as_str().into());
        s.toggle("Legacy authentication");
        assert_eq!(s.draft(Some(saved.clone())), SavedServer { legacy_auth: true, ..saved.clone() }, "only the switch turned changed");
        let plain = Shown::new(subsonic(), SavedServer { alt_url: String::new(), allow_self_signed: false, proxy: None, ..saved }, false, data.path());
        assert!(plain.lines().is_empty(), "nothing advanced set: folded");
    }
}
