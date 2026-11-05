//! Переводы интерфейса. Все строки, которые видит человек, лежат в
//! `locales/<язык>.toml`, а не в коде (раздел 9 спецификации).
//!
//! Чтобы добавить язык, достаточно положить новый файл в `locales` и
//! дописать одну строку в [`LANGUAGES`]. Тест проверит, что в нём есть
//! все ключи.

use std::collections::HashMap;

/// Код языка и содержимое его файла. Первый — запасной: если в другом
/// языке нет ключа, строка берётся отсюда.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.toml")),
    ("ru", include_str!("../locales/ru.toml")),
];

pub const FALLBACK: &str = "en";

/// Разбирает файл перевода в плоский словарь `раздел.ключ → строка`.
pub fn parse(text: &str) -> Result<HashMap<String, String>, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut out = HashMap::new();
    flatten("", &table, &mut out)?;
    Ok(out)
}

fn flatten(prefix: &str, t: &toml::Table, out: &mut HashMap<String, String>) -> Result<(), String> {
    for (k, v) in t {
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            toml::Value::String(s) => {
                out.insert(key, s.clone());
            }
            toml::Value::Table(inner) => flatten(&key, inner, out)?,
            _ => return Err(format!("{key}: ожидалась строка")),
        }
    }
    Ok(())
}

/// Язык по системной настройке Windows: русский для `ru-…`, иначе английский.
pub fn detect(system_locale: Option<&str>) -> &'static str {
    let wanted = system_locale
        .and_then(|l| l.split(['-', '_']).next())
        .map(str::to_ascii_lowercase);
    LANGUAGES
        .iter()
        .map(|(c, _)| *c)
        .find(|c| Some(c.to_string()) == wanted)
        .unwrap_or(FALLBACK)
}

#[derive(Debug, Clone)]
pub struct I18n {
    code: &'static str,
    strings: HashMap<String, String>,
    fallback: HashMap<String, String>,
}

impl I18n {
    /// Неизвестный код языка — английский.
    pub fn new(code: &str) -> I18n {
        let load = |c: &str| {
            LANGUAGES
                .iter()
                .find(|(k, _)| *k == c)
                .map(|(k, text)| (*k, parse(text).expect("файлы переводов проверены тестами")))
        };
        let (code, strings) = load(code)
            .or_else(|| load(FALLBACK))
            .expect("запасной язык есть");
        let fallback = load(FALLBACK).expect("запасной язык есть").1;
        I18n {
            code,
            strings,
            fallback,
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Строка по ключу. Если её нет нигде — сам ключ: так пропуск заметен
    /// глазами, но программа не падает.
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.strings
            .get(key)
            .or_else(|| self.fallback.get(key))
            .map(String::as_str)
            .unwrap_or(key)
    }

    /// Строка с подстановками: `{name}` заменяется значением.
    pub fn tf(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut s = self.t(key).to_string();
        for (name, value) in args {
            s = s.replace(&format!("{{{name}}}"), value);
        }
        s
    }

    /// Название языка на нём самом — для списка языков.
    pub fn native_name(code: &str) -> String {
        I18n::new(code).t("language.name").to_string()
    }
}

/// Имена подстановок `{…}` в строке, по порядку.
pub fn placeholders(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                out.push(after[..end].to_string());
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_every_key_with_same_placeholders() {
        let base = parse(LANGUAGES[0].1).unwrap();
        for (code, text) in LANGUAGES {
            let map = parse(text).unwrap_or_else(|e| panic!("{code}: {e}"));
            for (k, v) in &base {
                let other = map
                    .get(k)
                    .unwrap_or_else(|| panic!("{code}: нет ключа {k}"));
                assert!(!other.trim().is_empty(), "{code}: пустой {k}");
                assert_eq!(
                    placeholders(v),
                    placeholders(other),
                    "{code}: подстановки в {k}"
                );
            }
            for k in map.keys() {
                assert!(base.contains_key(k), "{code}: лишний ключ {k}");
            }
        }
    }

    #[test]
    fn lookup_fallback_and_substitution() {
        let ru = I18n::new("ru");
        assert_eq!(ru.code(), "ru");
        assert_eq!(ru.t("language.name"), "Русский");
        assert_eq!(ru.t("нет.такого"), "нет.такого");
        assert_eq!(I18n::new("xx").code(), "en");
        let s = ru.tf("update.available", &[("version", "1.2.3")]);
        assert!(s.contains("1.2.3") && !s.contains('{'), "{s}");
    }

    #[test]
    fn detects_system_language() {
        assert_eq!(detect(Some("ru-RU")), "ru");
        assert_eq!(detect(Some("RU")), "ru");
        assert_eq!(detect(Some("en-US")), "en");
        assert_eq!(detect(Some("de_DE")), "en");
        assert_eq!(detect(None), "en");
    }

    #[test]
    fn placeholder_names() {
        assert_eq!(placeholders("{b} и {a}"), vec!["a", "b"]);
        assert!(placeholders("нет").is_empty());
    }
}
