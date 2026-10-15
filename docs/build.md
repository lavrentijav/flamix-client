# Сборка Flamix / Building Flamix

## Разработка

Вся компиляция, тесты и clippy идут на сервере ru1 через `scripts/ru1` (не локально):

    scripts/ru1 cargo build --release
    scripts/ru1 cargo test --workspace
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo fmt --all --check

## Релизные пакеты

Релизные пакеты собирает CI: `.github/workflows/release.yml` по тегу `v*` или вручную
(`workflow_dispatch`):

- Linux: `flamix_<ver>_<arch>.deb` (cargo-deb), `flamix-<ver>-1.<arch>.rpm`
  (cargo-generate-rpm), `flamix-<ver>-linux-<arch>.tar.gz`, `flamix-<ver>-linux-<arch>.AppImage`
  и `PKGBUILD` для Arch (генерируется из `packaging/arch/PKGBUILD.in` с sha256 tarball'а);
- Windows: портативный `.zip` и установщик Inno Setup (`packaging/windows/flamix.iss`);
- macOS: universal `.app` + `.dmg`.

Финальная job собирает артефакты в GitHub Release и добавляет `SHA256SUMS.txt`.

## Локальная (ru1) проверка упаковки

Linux-упаковку можно повторить вручную на ru1 — тем же скриптом, что использует CI:

    cargo install --locked cargo-deb cargo-generate-rpm
    FLAMIX_FETCH=dist scripts/ru1 scripts/package-linux.sh

Результат — в `dist/`: `.deb`, `.rpm`, `.tar.gz`, `.AppImage` и `PKGBUILD`
(`dist/` в `.gitignore`).

## Ядра и geo-базы

`scripts/fetch-cores.sh <linux|macos|windows> <amd64|arm64> <out-dir>` скачивает
пиннутые версии **Xray-core**, **sing-box** и geo-базы из **Loyalsoldier/v2ray-rules-dat**,
проверяет SHA256 (там, где upstream публикует дайджесты) и кладёт `xray[.exe]`,
`sing-box[.exe]`, `geoip.dat`, `geosite.dat` в `<out-dir>`. Версии закреплены
переменными в начале скрипта.
