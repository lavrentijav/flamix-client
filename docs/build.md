# Сборка Flamix / Building Flamix

Flamix — обычный Cargo-workspace. Нужен стабильный Rust (edition 2021,
`rust-version = 1.85`) и системные библиотеки, которые использует Slint/winit.

## 1. Инструменты

- Rust через [rustup](https://rustup.rs/):

      rustup toolchain install stable
      rustup component add rustfmt clippy

- Системные зависимости:

  **Debian / Ubuntu**

      sudo apt install pkg-config \
        libxkbcommon-dev libfontconfig1-dev libwayland-dev libx11-dev \
        libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev

  **Fedora / RHEL**

      sudo dnf install pkgconf-pkg-config fontconfig-devel libxkbcommon-devel \
        wayland-devel libX11-devel libXcursor-devel libXrandr-devel libXi-devel mesa-libGL-devel

  **Arch**

      sudo pacman -S base-devel pkgconf fontconfig libxkbcommon wayland libx11 \
        libxcursor libxrandr libxi mesa

  **macOS** — Xcode Command Line Tools (`xcode-select --install`).
  **Windows** — MSVC-тулчейн (Visual Studio Build Tools, компонент «Desktop development with C++»).

## 2. Сборка, тесты, линт

Из корня репозитория:

    cargo build --release -p flamix     # бинарник: target/release/flamix[.exe]
    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --all --check

## 3. Ядра Xray-core / sing-box и geo-базы

Приложению нужны `xray[.exe]`, `sing-box[.exe]`, `geoip.dat`, `geosite.dat`.
Кладём их рядом с бинарником (в подкаталог `cores/`):

    scripts/fetch-cores.sh <os> <arch> <out-dir>
    # os:   linux | macos | windows
    # arch: amd64 | arm64
    # пример:
    scripts/fetch-cores.sh linux amd64 cores

Скрипт скачивает пиннутые версии **Xray-core**, **sing-box** и geo-данных из
**Loyalsoldier/v2ray-rules-dat**, проверяет SHA256 (там, где upstream публикует
дайджесты) и кладёт файлы в `<out-dir>`. Версии закреплены переменными в начале скрипта.

## 4. Упаковка

Скрипты рассчитаны на запуск на любой машине соответствующей ОС (и используются CI —
см. `.github/workflows/release.yml`).

### Linux — `scripts/package-linux.sh`

Собирает в `dist/`:

- `flamix-<ver>-linux-<arch>.tar.gz` (portable, с ядрами);
- `PKGBUILD` для Arch (x86_64; генерируется из `packaging/arch/PKGBUILD.in`);
- `flamix_<ver>_<arch>.deb` — если установлен `cargo-deb`;
- `flamix-<ver>-1.<arch>.rpm` — если установлен `cargo-generate-rpm`;
- `flamix-<ver>-linux-<arch>.AppImage` — appimagetool скачивается автоматически.

    cargo install --locked cargo-deb cargo-generate-rpm
    scripts/package-linux.sh

Результат — в `dist/` (не коммитится).

### macOS — `scripts/package-macos.sh`

Собирает universal-бинарник (`aarch64-apple-darwin` + `x86_64-apple-darwin` через `lipo`),
кладывает ядра в `Flamix.app/Contents/Resources/cores/`, подписывает ad-hoc и делает `.dmg`:

    rustup target add aarch64-apple-darwin x86_64-apple-darwin
    scripts/package-macos.sh

### Windows — `scripts/package-windows.sh`

Делает портативный `.zip` и установщик Inno Setup (`packaging/windows/flamix.iss`).
Нужны ImageMagick (иконка `.ico`) и Inno Setup 6; запускается из Git Bash:

    scripts/package-windows.sh

## 5. CI

`.github/workflows/ci.yml` на push/PR в `main` гоняет `fmt`, `clippy` и тесты на Linux,
плюс `cargo check -p flamix` на Windows и macOS. `.github/workflows/release.yml` по тегу `v*`
(или вручную) собирает все пакеты и прикрепляет их вместе с `SHA256SUMS.txt` к GitHub Release.
