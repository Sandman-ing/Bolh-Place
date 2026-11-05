//! Автообновление (раздел 10 спецификации): программа спрашивает GitHub о
//! последнем выпуске, скачивает установщик и его подпись, проверяет подпись
//! открытым ключом, зашитым в программу, и только тогда запускает.
//!
//! Всё здесь блокирующее — вызывать только в фоновой задаче.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    /// Подпись не сошлась — запускать нельзя.
    BadSignature,
    Other(String),
}

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(concat!("BolhPlace/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let resp = agent
        .get(url)
        .header(
            "Accept",
            "application/vnd.github+json, application/octet-stream",
        )
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(code) => UpdateError::Other(format!("HTTP {code}")),
            _ => UpdateError::Offline,
        })?;
    let mut bytes = Vec::new();
    resp.into_body()
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UpdateError::Offline)?;
    if bytes.len() as u64 > limit {
        return Err(UpdateError::Other("слишком большой файл".into()));
    }
    Ok(bytes)
}

/// Есть ли версия новее установленной. `None` — обновляться не нужно.
pub fn check(current: &str) -> Result<Option<Release>, UpdateError> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let json = get(&agent(), &url, 4 * 1024 * 1024)?;
    let release = parse_latest(&json).map_err(|e| UpdateError::Other(e.to_string()))?;
    Ok(is_newer(current, &release).then_some(release))
}

/// Скачивает установщик и подпись, проверяет и кладёт установщик во
/// временную папку. Возвращает путь к проверенному файлу.
pub fn download(release: &Release, dir: &Path) -> Result<PathBuf, UpdateError> {
    let agent = agent();
    let installer = get(&agent, &release.installer_url, MAX_INSTALLER_BYTES)?;
    let signature = get(&agent, &release.signature_url, 64 * 1024)?;
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
