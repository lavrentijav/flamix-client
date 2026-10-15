# flamix-client

Кратко: что это и как запускать.

## Запуск
...

## Разработка
Общая среда claude+opencode описана в `/srv/dev/AGENTS.md`.
Разработка — здесь (`~/projects/flamix-client`); релиз — `release-sync flamix-client`.

## Download / Install

Готовые сборки публикуются на странице Releases (их собирает GitHub Actions по тегу `v*`).
Ядра Xray-core / sing-box и geo-базы уже внутри пакетов — отдельно ставить ничего не нужно.

- **Debian / Ubuntu**: `flamix_<ver>_amd64.deb` (или `_arm64.deb`)

      sudo apt install ./flamix_<ver>_amd64.deb

- **Fedora / RHEL**: `flamix-<ver>-1.x86_64.rpm` (или `aarch64`)

      sudo dnf install ./flamix-<ver>-1.x86_64.rpm

- **Arch Linux**: `PKGBUILD` из релиза

      makepkg -si

- **AppImage** (portable): `flamix-<ver>-linux-x86_64.AppImage`

      chmod +x flamix-<ver>-linux-x86_64.AppImage && ./flamix-<ver>-linux-x86_64.AppImage

- **Tar.gz** (portable): `flamix-<ver>-linux-x86_64.tar.gz`
- **macOS**: `flamix-<ver>-macos-universal.dmg` (universal: Apple Silicon + Intel).
  Сборка подписана ad-hoc, поэтому при первом запуске Gatekeeper может ругаться; снимите карантин:

      xattr -dr com.apple.quarantine /Applications/Flamix.app

- **Windows**: `flamix-<ver>-windows-x64-setup.exe` (установщик, per-user, регистрирует `flamix://`)
  или `flamix-<ver>-windows-x64.zip` (портативная сборка).

Схема `flamix://` (ссылки-подписки и серверы) регистрируется установщиком/приложением.

## Build

Вся компиляция, тесты и clippy идут на сервере ru1 через `scripts/ru1` (не локально):

    scripts/ru1 cargo build --release
    scripts/ru1 cargo test --workspace
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo fmt --all --check

Релизные пакеты собирает CI: `.github/workflows/release.yml` (Linux deb/rpm/AppImage/tar.gz + PKGBUILD,
macOS universal .dmg, Windows .zip + Inno Setup installer). То же можно повторить вручную на ru1:

    cargo install --locked cargo-deb cargo-generate-rpm
    FLAMIX_FETCH=dist scripts/ru1 scripts/package-linux.sh

Ядра и geo-базы качаются скриптом `scripts/fetch-cores.sh <linux|macos|windows> <amd64|arm64> <out-dir>`
(пиннутые версии Xray-core, sing-box и Loyalsoldier/v2ray-rules-dat, SHA256 проверяется).
