//! Проверка обновлений (раздел 10 спецификации). Сеть — забота интерфейса;
//! здесь разбор ответа GitHub, сравнение версий и проверка подписи.
//!
//! Установщик в выпуске подписан ключом minisign. Открытая половина ключа
//! зашита в программу, закрытая — секрет репозитория. Подпись покрывает и
//! сам файл, и «доверенный комментарий» с номером версии: старый
//! подписанный установщик нельзя выдать за новый.

use semver::Version;
use serde::Deserialize;

use crate::error::{Error, Result};

/// Окончание имени установщика в выпуске. Подпись — рядом, с `.minisig`.
pub const INSTALLER_SUFFIX: &str = "-setup.exe";
pub const SIGNATURE_EXT: &str = ".minisig";

/// Текст, который подписывается вместе с файлом.
pub fn trusted_comment(version: &Version, file_name: &str) -> String {
    format!("bolh-place {version} {file_name}")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    pub installer_name: String,
    pub installer_url: String,
    pub signature_url: String,
    pub page_url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    html_url: String,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Разбирает ответ `GET /repos/{owner}/{repo}/releases/latest`.
pub fn parse_latest(json: &[u8]) -> Result<Release> {
    let bad = |m: &str| Error::Update(m.to_string());
    let r: GhRelease = serde_json::from_slice(json).map_err(|e| bad(&e.to_string()))?;
    if r.draft || r.prerelease {
        return Err(bad("выпуск не окончательный"));
    }
    let version = Version::parse(r.tag_name.trim_start_matches('v'))
        .map_err(|e| bad(&format!("версия {:?}: {e}", r.tag_name)))?;
    let installer = r
        .assets
        .iter()
        .find(|a| a.name.ends_with(INSTALLER_SUFFIX))
        .ok_or_else(|| bad("в выпуске нет установщика"))?;
    let sig_name = format!("{}{SIGNATURE_EXT}", installer.name);
    let signature = r
        .assets
        .iter()
        .find(|a| a.name == sig_name)
        .ok_or_else(|| bad("в выпуске нет подписи"))?;
    Ok(Release {
        version,
        installer_name: installer.name.clone(),
        installer_url: installer.browser_download_url.clone(),
        signature_url: signature.browser_download_url.clone(),
        page_url: r.html_url,
    })
}

/// Новее ли выпуск, чем установленная версия.
pub fn is_newer(current: &str, release: &Release) -> bool {
    Version::parse(current).is_ok_and(|c| release.version > c)
}

/// Проверяет, что установщик подписан нашим ключом и именно для этого
/// выпуска. Без успешной проверки запускать скачанное нельзя.
pub fn verify(
    installer: &[u8],
    signature: &str,
    public_key: &str,
    release: &Release,
) -> Result<()> {
    let bad = |m: String| Error::Update(m);
    let pk = minisign_verify::PublicKey::decode(public_key.trim())
        .or_else(|_| minisign_verify::PublicKey::from_base64(public_key.trim()))
        .map_err(|e| bad(format!("открытый ключ: {e}")))?;
    let sig =
        minisign_verify::Signature::decode(signature).map_err(|e| bad(format!("подпись: {e}")))?;
    pk.verify(installer, &sig, false)
        .map_err(|e| bad(format!("подпись не сходится: {e}")))?;
    let expected = trusted_comment(&release.version, &release.installer_name);
    if sig.trusted_comment() != expected {
        return Err(bad(format!(
            "подпись от другого файла: {:?}",
            sig.trusted_comment()
        )));
    }
    Ok(())
}
