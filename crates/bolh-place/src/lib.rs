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
    App, AppContext as _, Bounds, Pixels, Size, TitlebarOptions, WindowBounds, WindowOptions, px,
    size,
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

/// Начальный размер окна: 1280×820, но не больше 90% ширины и 85% высоты
/// экрана — на ноутбуке 1366×768 окно иначе уходило бы под панель задач.
pub fn fit_window(screen: Option<Size<Pixels>>) -> Size<Pixels> {
    let wanted = size(px(1280.), px(820.));
    match screen {
        Some(s) => size(
            wanted.width.min(s.width * 0.9),
            wanted.height.min(s.height * 0.85),
        ),
        None => wanted,
    }
}

fn window_size(cx: &App) -> Size<Pixels> {
    fit_window(cx.primary_display().map(|d| d.bounds().size))
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

            let bounds = Bounds::centered(None, window_size(cx), cx);
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
                cx.new(|cx| {
                    let mut ws = workspace::Workspace::new(settings, settings_path, window, cx);
                    // Раздел 10: программа сама смотрит, нет ли новой версии.
                    ws.check_update(false, window, cx);
                    ws
                })
            })
            .expect("окно Bolh Place не открылось");
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_fits_small_screens() {
        let small = fit_window(Some(size(px(1024.), px(768.))));
        assert!(small.width <= px(1024.) * 0.9 && small.height <= px(768.) * 0.85);
        let big = fit_window(Some(size(px(2560.), px(1440.))));
        assert_eq!(big, size(px(1280.), px(820.)));
        assert_eq!(fit_window(None), size(px(1280.), px(820.)));
    }
}
