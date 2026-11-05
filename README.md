# Bolh Place

Программа для Windows, где любые планы можно разложить и связать: документы,
как в Google Docs, бесконечный холст, как в Miro или Obsidian Canvas, ссылки
между всем и граф связей.

- Что именно делаем — [BOLH_PLACE_SPEC.md](BOLH_PLACE_SPEC.md).
- Как работаем — [CLAUDE.md](CLAUDE.md).
- Как выглядит — [docs/design-tokens.md](docs/design-tokens.md).
- Выпуски и автообновление — [docs/release.md](docs/release.md).
- Лицензии библиотек — [docs/licenses.md](docs/licenses.md).

Rust, интерфейс на GPUI с gpui-component. Windows 10 и 11.

Состояние: этап 1 — каркас.

## Устройство

| Папка | Что там |
|---|---|
| `crates/bolh-core` | ядро без интерфейса: пространства, формат `.bdoc`/`.bcanvas`, история, корзина, синхронизация, переводы |
| `crates/bolh-core/locales` | все строки интерфейса, `ru.toml` и `en.toml` |
| `crates/bolh-place` | окно: дерево, вкладки, темы, палитра Ctrl+P, обновления |
| `crates/bolh-sign` | подпись установщика при выпуске |
| `packaging` | установщик Inno Setup и открытый ключ обновлений |

## Проверки

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/check-licenses.py
```

Тесты интерфейса открывают настоящее окно без экрана и проходят и в Linux.
Установщик проверяется только в GitHub Actions на Windows.
