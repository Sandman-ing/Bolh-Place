//! Автообновление (раздел 10 спецификации): программа спрашивает GitHub о
//! последнем выпуске, скачивает установщик и его подпись, проверяет подпись
//! открытым ключом, зашитым в программу, и только тогда запускает.
//!
//! Всё здесь блокирующее — вызывать только в фоновой задаче.

use std::path::{Path, PathBuf};

use bolh_core::update::{Release, is_newer, parse_latest, verify};

pub const REPO: &str = "Sandman-ing/Bolh-Place";

/// Открытый ключ подписи обновлений. Закрытая половина — секрет
/// репозитория `UPDATE_SIGNING_KEY`, её нет ни в коде, ни в программе.
pub const PUBLIC_KEY: &str = include_str!("../../../packaging/update-key.pub");

/// Установщик больше этого — подозрительно; не качаем бесконечно.
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateError {
    /// Нет сети или GitHub не ответил.
    Offline,
    /// GitHub ответил «нет такого» — например, выпусков ещё нет.
    NotFound,
    /// Подпись не сошлась — запускать нельзя.
    BadSignature,
    Other(String),
}

fn get(url: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let resp = minreq::get(url)
        .with_header("Accept", "application/vnd.github+json, application/octet-stream")
        .with_header("User-Agent", concat!("BolhPlace/", env!("CARGO_PKG_VERSION")))
        .with_timeout(120)
        .with_max_redirects(10)
        .send_lazy()
        .map_err(|_| UpdateError::Offline)?;
    if resp.status_code == 404 {
        return Err(UpdateError::NotFound);
    }
    if !(200..300).contains(&resp.status_code) {
        return Err(UpdateError::Other(format!("HTTP {}", resp.status_code)));
    }
    let mut bytes = Vec::new();
    for b in resp {
        let (byte, _) = b.map_err(|_| UpdateError::Offline)?;
        bytes.push(byte);
        if bytes.len() as u64 > limit {
            return Err(UpdateError::Other("слишком большой файл".into()));
        }
    }
    Ok(bytes)
}

/// Есть ли версия новее установленной. `None` — обновляться не нужно.
pub fn check(current: &str) -> Result<Option<Release>, UpdateError> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let json = match get(&url, 4 * 1024 * 1024) {
        // Выпусков ещё нет — значит, и новее нет.
        Err(UpdateError::NotFound) => return Ok(None),
        other => other?,
    };
    let release = parse_latest(&json).map_err(|e| UpdateError::Other(e.to_string()))?;
    Ok(is_newer(current, &release).then_some(release))
}

/// Скачивает установщик и подпись, проверяет и кладёт установщик во
/// временную папку. Возвращает путь к проверенному файлу.
pub fn download(release: &Release, dir: &Path) -> Result<PathBuf, UpdateError> {
    let installer = get(&release.installer_url, MAX_INSTALLER_BYTES)?;
    let signature = get(&release.signature_url, 64 * 1024)?;
    let signature = String::from_utf8(signature).map_err(|_| UpdateError::BadSignature)?;
    verify(&installer, &signature, PUBLIC_KEY, release).map_err(|_| UpdateError::BadSignature)?;
    std::fs::create_dir_all(dir).map_err(|e| UpdateError::Other(e.to_string()))?;
    // Имя без пути из ответа сервера: в имени не может быть каталогов.
    let name = Path::new(&release.installer_name)
        .file_name()
        .ok_or_else(|| UpdateError::Other("имя установщика".into()))?;
    let path = dir.join(name);
    std::fs::write(&path, &installer).map_err(|e| UpdateError::Other(e.to_string()))?;
    Ok(path)
}

/// Запускает проверенный установщик в тихом режиме. Он сам закроет
/// программу, заменит файлы и запустит новую версию.
pub fn launch_installer(path: &Path) -> Result<(), UpdateError> {
    std::process::Command::new(path)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/UPDATE=1"])
        .spawn()
        .map(|_| ())
        .map_err(|e| UpdateError::Other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_key_is_a_valid_minisign_key() {
        assert!(minisign_key_ok(PUBLIC_KEY));
    }

    fn minisign_key_ok(text: &str) -> bool {
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        lines.len() == 2 && lines[0].starts_with("untrusted comment:") && lines[1].len() == 56
    }
}

#[cfg(test)]
mod live {
    /// Настоящий запрос к GitHub. Не в обычном прогоне: зависит от сети.
    /// `cargo test -p bolh-place -- --ignored live_check`
    #[test]
    #[ignore]
    fn live_check() {
        let r = super::check("0.0.1");
        eprintln!("{r:?}");
        assert!(r.is_ok(), "{r:?}");
    }
}
