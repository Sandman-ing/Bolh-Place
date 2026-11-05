//! Раздел 9 спецификации: русский и английский, и другие языки без
//! переделок. Поэтому в коде интерфейса нет ни одной фразы для человека —
//! только ключи переводов, и каждый ключ есть в файлах переводов.

use std::path::Path;

use bolh_core::i18n::{LANGUAGES, parse};

fn sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect()
}

/// Строка кода без комментария `//` (грубо, но для нашего кода хватает:
/// `//` внутри строковых литералов интерфейса не встречается).
fn code_part(line: &str) -> &str {
    let t = line.trim_start();
    if t.starts_with("//") || t.starts_with("///") || t.starts_with("//!") {
        return "";
    }
    match line.find(" // ") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Строковые литералы в строке кода.
fn literals(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = code.chars().peekable();
    let mut cur: Option<String> = None;
    while let Some(c) = chars.next() {
        match (&mut cur, c) {
            (None, '"') => cur = Some(String::new()),
            (Some(_), '\\') => {
                chars.next();
            }
            (Some(s), '"') => {
                out.push(std::mem::take(s));
                cur = None;
            }
            (Some(s), c) => s.push(c),
            (None, _) => {}
        }
    }
    out
}

#[test]
fn no_cyrillic_text_in_ui_code() {
    let mut bad = Vec::new();
    for (file, text) in sources() {
        let mut in_tests = false;
        for (n, line) in text.lines().enumerate() {
            // Тесты внутри модулей могут сравнивать с русским переводом.
            if line.contains("#[cfg(test)]") {
                in_tests = true;
            }
            if in_tests {
                continue;
            }
            for lit in literals(code_part(line)) {
                // Сообщения паники и ошибок ядра — для разработчика, не для человека.
                let developer_only = line.contains("expect(")
                    || line.contains("panic!(")
                    || line.contains("UpdateError::Other(");
                if lit
                    .chars()
                    .any(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
                    && !developer_only
                {
                    bad.push(format!("{file}:{}: {lit}", n + 1));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "текст для человека прямо в коде:\n{}",
        bad.join("\n")
    );
}

#[test]
fn every_key_used_in_code_exists_in_every_language() {
    let mut keys = Vec::new();
    for (_, text) in sources() {
        for call in [
            "tr(cx, \"",
            "trf(cx, \"",
            ".t(\"",
            "\"names.",
            "\"document.",
            "\"canvas.",
            "\"theme.",
        ] {
            let mut rest = text.as_str();
            while let Some(i) = rest.find(call) {
                let start = i + call.len();
                let tail = &rest[start..];
                let end = tail.find('"').unwrap_or(0);
                let key = match call.strip_prefix('"') {
                    Some(prefix) => format!("{prefix}{}", &tail[..end]),
                    None => tail[..end].to_string(),
                };
                if !key.is_empty() && key.contains('.') {
                    keys.push(key);
                }
                rest = &rest[start..];
            }
        }
    }
    keys.sort();
    keys.dedup();
    assert!(keys.len() > 30, "ключи не нашлись: {keys:?}");
    for (code, text) in LANGUAGES {
        let map = parse(text).unwrap();
        for k in &keys {
            assert!(map.contains_key(k), "в языке {code} нет ключа {k}");
        }
    }
}
