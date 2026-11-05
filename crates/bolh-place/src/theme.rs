//! Тема Bolh: тёмная, светлая и «как в Windows» (раздел 9 спецификации).
//! Цвета — из `assets/bolh-theme.json`, а он — из `docs/design-tokens.md`.

use std::rc::Rc;

use bolh_core::settings::ThemeChoice;
use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeSet};
use gpui_kit::{App, Window};

pub const THEME_JSON: &str = include_str!("../assets/bolh-theme.json");

/// Разбирает обе темы Bolh: (тёмная, светлая).
pub fn bolh_themes() -> (ThemeConfig, ThemeConfig) {
    let set: ThemeSet = serde_json::from_str(THEME_JSON).expect("тема Bolh проверена тестом");
    let mut dark = None;
    let mut light = None;
    for mut t in set.themes {
        // В Windows — Segoe UI, как в дизайн-токенах. В остальных системах
        // (тесты в Linux) — системный шрифт по умолчанию.
        if cfg!(windows) {
            t.font_family = Some("Segoe UI".into());
        }
        if t.mode.is_dark() {
            dark = Some(t);
        } else {
            light = Some(t);
        }
    }
    (
        dark.expect("в теме есть тёмный вариант"),
        light.expect("в теме есть светлый вариант"),
    )
}

pub fn init(cx: &mut App) {
    let (dark, light) = bolh_themes();
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |t| {
        t.dark_theme = Rc::new(dark);
        t.light_theme = Rc::new(light);
    });
    Theme::change(ThemeMode::Dark, None, cx);
}

/// Какой режим нужен сейчас: для «как в Windows» — по виду окна.
pub fn mode_for(choice: ThemeChoice, window: Option<&Window>, cx: &App) -> ThemeMode {
    match choice {
        ThemeChoice::Dark => ThemeMode::Dark,
        ThemeChoice::Light => ThemeMode::Light,
        ThemeChoice::System => window
            .map(|w| w.appearance())
            .unwrap_or_else(|| cx.window_appearance())
            .into(),
    }
}

pub fn apply(choice: ThemeChoice, window: &mut Window, cx: &mut App) {
    let mode = mode_for(choice, Some(window), cx);
    Theme::change(mode, Some(window), cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw() -> serde_json::Value {
        serde_json::from_str(THEME_JSON).unwrap()
    }

    #[test]
    fn both_modes_parse() {
        let (dark, light) = bolh_themes();
        assert!(dark.mode.is_dark());
        assert!(!light.mode.is_dark());
    }

    /// Неизвестный ключ цвета библиотека молча пропускает — и цвет не
    /// применяется. Проверяем, что каждый ключ из файла она поняла.
    #[test]
    fn every_color_key_is_known_to_the_library() {
        for theme in raw()["themes"].as_array().unwrap() {
            let config: ThemeConfig = serde_json::from_value(theme.clone()).unwrap();
            let back = serde_json::to_value(&config).unwrap();
            for key in theme["colors"].as_object().unwrap().keys() {
                let v = &back["colors"][key];
                assert!(!v.is_null(), "{}: ключ {key} не понят", theme["name"]);
            }
        }
    }

    /// Цвета совпадают с docs/design-tokens.md — источником вида Bolh.
    #[test]
    fn colors_match_design_tokens() {
        let doc = include_str!("../../../docs/design-tokens.md");
        let hexes = |row: &str| -> (String, String) {
            let line = doc
                .lines()
                .find(|l| l.starts_with(&format!("| {row}")))
                .unwrap_or_else(|| panic!("в токенах нет строки {row}"));
            let cols: Vec<&str> = line.split('|').collect();
            let first = |s: &str| {
                let i = s.find('#').unwrap();
                s[i..i + 7].to_string()
            };
            (first(cols[2]), first(cols[3]))
        };
        let pairs = [
            ("Фон", "background"),
            ("Поверхность 1", "popover.background"),
            ("Поверхность 2", "secondary.background"),
            ("Разделитель", "border"),
            ("Обводка карточки", "input.border"),
            ("Текст основной", "foreground"),
            ("Текст второстепенный", "muted.foreground"),
            ("Бирюзовый", "primary.background"),
            ("Внимание", "warning.background"),
            ("Проблема", "danger.background"),
            ("Успех", "success.background"),
        ];
        let themes = raw();
        let (dark, light) = (
            &themes["themes"][0]["colors"],
            &themes["themes"][1]["colors"],
        );
        for (row, key) in pairs {
            let (d, l) = hexes(row);
            assert_eq!(
                dark[key].as_str().unwrap().to_uppercase(),
                d.to_uppercase(),
                "тёмная {row}"
            );
            assert_eq!(
                light[key].as_str().unwrap().to_uppercase(),
                l.to_uppercase(),
                "светлая {row}"
            );
        }
    }
}
