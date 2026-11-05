//! Переводы в интерфейсе. Строки берутся только отсюда: в коде нет ни
//! одной фразы для человека — тест `no_ui_text_in_code` это проверяет.

use bolh_core::Error;
use bolh_core::i18n::{I18n, detect};
use bolh_core::settings::Settings;
use gpui_kit::{App, Global, SharedString};

pub struct Lang(pub I18n);

impl Global for Lang {}

pub fn init(settings: &Settings, cx: &mut App) {
    set_language(language_for(settings), cx);
}

/// Язык из настроек, а если там пусто — язык Windows.
pub fn language_for(settings: &Settings) -> String {
    settings
        .language
        .clone()
        .unwrap_or_else(|| detect(sys_locale::get_locale().as_deref()).to_string())
}

pub fn set_language(code: impl AsRef<str>, cx: &mut App) {
    cx.set_global(Lang(I18n::new(code.as_ref())));
}

pub fn current(cx: &App) -> &I18n {
    &cx.global::<Lang>().0
}

/// Строка по ключу из `locales/*.toml`.
pub fn tr(cx: &App, key: &'static str) -> SharedString {
    SharedString::from(current(cx).t(key).to_string())
}

/// Строка с подстановками `{имя}`.
pub fn trf(cx: &App, key: &'static str, args: &[(&str, &str)]) -> SharedString {
    SharedString::from(current(cx).tf(key, args))
}

/// Понятное человеку сообщение об ошибке ядра.
pub fn error_text(cx: &App, err: &Error) -> SharedString {
    let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
    match err {
        Error::Io { path: p, .. } => trf(cx, "error.io", &[("path", &path(p))]),
        Error::Corrupt { path: p, .. } => trf(cx, "error.corrupt", &[("path", &path(p))]),
        Error::TooNew { path: p, .. } => trf(cx, "error.too_new", &[("path", &path(p))]),
        Error::NotASpace(p) => trf(cx, "error.not_a_space", &[("path", &path(p))]),
        Error::OutsideSpace(_) => tr(cx, "error.outside"),
        Error::AlreadyExists(p) => trf(cx, "error.exists", &[("path", &path(p))]),
        Error::NotFound(p) => trf(cx, "error.not_found", &[("path", &path(p))]),
        Error::InvalidName(n) => trf(cx, "error.invalid_name", &[("path", n)]),
        Error::Update(_) => tr(cx, "error.update"),
    }
}
