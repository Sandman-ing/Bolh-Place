//! Интерфейс Bolh Place на GPUI. Всё, что можно проверить без экрана,
//! живёт в `bolh-core`; здесь — окно и то, как человек с ним работает.

pub mod actions;
pub mod assets;
pub mod i18n;
pub mod palette;
pub mod paths;
pub mod theme;
pub mod updater;
pub mod workspace;

use gpui_kit::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};

use bolh_core::settings::Settings;

/// Версия программы — из Cargo.toml, она же номер выпуска на GitHub.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Общая настройка приложения: тема, язык, клавиши. Вызывается и при
/// запуске, и в тестах интерфейса.
pub fn init(settings: &Settings, cx: &mut App) {
    gpui_kit::init(cx);
    i18n::init(settings, cx);
    theme::init(cx);
    actions::bind_keys(cx);
}

pub fn run() {
    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(|cx| {
            let settings_path = paths::settings_file();
            let settings = settings_path
                .as_deref()
                .map(Settings::load)
                .unwrap_or_default();
            init(&settings, cx);

            let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(i18n::tr(cx, "app.title")),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(640.), px(420.))),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| workspace::Workspace::new(settings, settings_path, window, cx))
            })
            .expect("окно Bolh Place не открылось");
            cx.activate(true);
        });
}
