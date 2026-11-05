//! Иконки. Стандартный набор gpui-kit содержит только иконки самих
//! компонентов; остальные нужные программе иконки Lucide подключаются
//! здесь поштучно, чтобы не тащить в программу все 1800.

use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString};

gpui_kit::assets::icon_assets!(
    pub ExtraIcons,
    [
        Download,
        FilePlus,
        FolderInput,
        FolderPlus,
        Grid3x3,
        Languages,
        Monitor,
        RotateCcw,
        Shapes,
        SquarePlus,
        Trash,
        X
    ]
);

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::assets::IconName;

    /// Каждая иконка, упомянутая в коде интерфейса, действительно есть в
    /// программе. Иначе на её месте будет пустота — так уже было.
    #[test]
    fn every_icon_used_in_code_is_bundled() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut used = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let mut rest = text.as_str();
            while let Some(i) = rest.find("IconName::") {
                rest = &rest[i + "IconName::".len()..];
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
                if !name.is_empty() && name != "ALL" {
                    used.push(name);
                }
            }
        }
        used.sort();
        used.dedup();
        assert!(used.len() > 5, "иконки не нашлись: {used:?}");
        for name in used {
            let icon = IconName::ALL
                .iter()
                .find(|i| format!("{i:?}") == name)
                .unwrap_or_else(|| panic!("нет такой иконки: {name}"));
            let loaded = AppAssets.load(&icon.path()).unwrap();
            assert!(loaded.is_some(), "иконка {name} не встроена в программу");
        }
    }
}
