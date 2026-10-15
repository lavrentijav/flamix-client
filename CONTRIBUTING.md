# Участие в разработке Flamix

Спасибо за интерес к проекту! Ниже — краткие правила для контрибьюторов.

## Прежде чем начать

- Проект в статусе **alpha** — возможны ломающие изменения.
- Для крупных изменений сначала откройте [Issue](https://github.com/lavrentijav/flamix-client/issues)
  и обсудите подход.
- Не добавляйте новые тяжёлые зависимости без необходимости; предпочитайте зависимости из корневого
  `Cargo.toml`.

## Требования

- [Rust](https://rustup.rs) (stable, через rustup) — версия указана в `Cargo.toml`.
- Системные зависимости для сборки интерфейса (Slint):

  - **Debian / Ubuntu**
    ```
    sudo apt install libxkbcommon-dev libfontconfig1-dev libwayland-dev \
        libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev
    ```
  - **Fedora / RHEL**: соответствующие `-devel`-пакеты из списка выше.
  - **macOS**: инструменты командной строки Xcode (`xcode-select --install`).
  - **Windows**: MSVC Build Tools (Visual Studio Build Tools).

## Сборка и проверки

```
cargo build --release -p flamix
cargo test --workspace
```

Перед отправкой Pull Request обязательно прогоните:

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

Подробнее о сборке и упаковке — в [docs/build.md](docs/build.md).

## Код

- `crates/flamix-core/src/model.rs` — общий контракт между модулями. Обсуждайте изменения
  существующих типов и полей заранее (в Issue/PR); добавление нового поля с `#[serde(default)]`
  обычно безопасно.
- Никаких `unwrap()`/`expect()` на данных из сети или от пользователя — возвращайте `Result`.
- Каждый парсер должен иметь unit-тесты на реальных примерах.
- `cargo clippy -D warnings` без замечаний, код отформатирован `rustfmt`.
- Комментарии — на английском, строки интерфейса — на русском и английском.

## Git и Pull Request

1. Форкните репозиторий и создайте ветку с говорящим именем.
2. Делайте небольшие осмысленные коммиты с понятными сообщениями.
3. Прогоните проверки из раздела «Сборка и проверки».
4. Откройте Pull Request с описанием: что изменено, зачем, как проверено.

## Иконки и ассеты

Набор иконок — [Alfa-Bank icons](https://github.com/core-ds/icons) (`@alfalab/icons-glyph`, MIT).
Текст лицензии: [app/flamix/assets/icons/LICENSE-alfalab.txt](app/flamix/assets/icons/LICENSE-alfalab.txt).
Добавляя ассеты, указывайте их источник и лицензию.

## Лицензия

Отправляя код, вы соглашаетесь с тем, что он распространяется под **GPL-3.0-or-later** (см. [LICENSE](LICENSE)).
