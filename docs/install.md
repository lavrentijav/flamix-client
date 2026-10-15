# Установка Flamix / Installing Flamix

Готовые сборки публикуются на странице Releases (их собирает GitHub Actions по тегу `v*`).
Ядра Xray-core / sing-box и geo-базы (`geoip.dat`, `geosite.dat`) уже внутри пакетов —
отдельно ставить ничего не нужно.

## Linux

- **Debian / Ubuntu** — `flamix_<ver>_amd64.deb` (или `_arm64.deb`):

      sudo apt install ./flamix_<ver>_amd64.deb

- **Fedora / RHEL** — `flamix-<ver>-1.x86_64.rpm` (или `aarch64`):

      sudo dnf install ./flamix-<ver>-1.x86_64.rpm

- **Arch Linux** — `PKGBUILD` из релиза:

      makepkg -si

- **AppImage** (portable) — `flamix-<ver>-linux-x86_64.AppImage`:

      chmod +x flamix-<ver>-linux-x86_64.AppImage && ./flamix-<ver>-linux-x86_64.AppImage

- **Tar.gz** (portable) — `flamix-<ver>-linux-x86_64.tar.gz`.

Пакеты ставят приложение в `/usr/bin/flamix`, ядра — в `/usr/lib/flamix/cores/`,
`.desktop` — в `/usr/share/applications`, иконки — в `hicolor`. Postinstall
проставляет `setcap cap_net_admin,cap_net_bind_service+ep` на `sing-box`,
чтобы TUN-режим работал без запроса root.

## macOS

`flamix-<ver>-macos-universal.dmg` (universal: Apple Silicon + Intel).
Сборка подписана ad-hoc, поэтому при первом запуске Gatekeeper может ругаться;
снимите карантин:

    xattr -dr com.apple.quarantine /Applications/Flamix.app

## Windows

- `flamix-<ver>-windows-x64-setup.exe` — установщик (per-user, по умолчанию без UAC;
  ярлык в меню «Пуск» и опционально на рабочем столе, деинсталлятор, иконка).
- `flamix-<ver>-windows-x64.zip` — портативная сборка.

Установщик регистрирует схему `flamix://`.

## Схема `flamix://`

Ссылки-подписки и серверные ссылки открываются через URL-схему `flamix://`.
Она регистрируется установщиком (Windows/macOS/`.desktop`), для портативных
сборок — приложением при первом запуске, где это поддерживается.
