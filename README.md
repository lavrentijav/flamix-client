# Flamix

Кроссплатформенный VPN/proxy-клиент на Rust с интерфейсом на Slint (вид WinUI 3 / Fluent).

[![CI](https://github.com/lavrentijav/flamix-client/actions/workflows/ci.yml/badge.svg)](https://github.com/lavrentijav/flamix-client/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/lavrentijav/flamix-client?include_prereleases&label=release)](https://github.com/lavrentijav/flamix-client/releases)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-informational)](#платформы)

**🇷🇺 Русский | [🇬🇧 English](README.en.md)**

> **Статус: ранняя разработка (alpha).** Функции и интерфейс меняются, сборки появятся на странице
> [Releases](https://github.com/lavrentijav/flamix-client/releases), когда будет готов CI.

Flamix — открытый клиент, который запускает внешние ядра **Xray-core** и **sing-box** (как v2rayN) и объединяет
возможности Happ, v2RayTun и v2rayN в одном приложении.

<!-- Скриншот появится в ближайших сборках: ![Главное окно Flamix](docs/design/screenshot-main.png) -->

![Макет главного окна](docs/design/main-window.png)

*Макет главного окна.*

## Возможности

**Протоколы**
- VLESS (Reality, XTLS Vision, XHTTP), VMess, Trojan, Shadowsocks (SIP002, плагины)
- Hysteria2 (port hopping), TUIC, WireGuard, SOCKS, HTTP
- Транспорты: TCP/RAW, WS, gRPC, HTTPUpgrade, XHTTP, H2, mKCP, QUIC

**Режимы работы**
- **Proxy** — системный прокси, локальные SOCKS5 `10808` и HTTP `10809`
- **TUN** — туннель всего трафика (нужны права администратора/root)

**Подписки**
- Форматы: base64, обычный список ссылок, JSON Xray/sing-box, Clash YAML
- Заголовки `subscription-userinfo`, `profile-title`, `profile-update-interval`, `support-url`,
  `profile-web-page-url`, `announce`
- HWID-заголовки, совместимые с Remnawave
- **Резервные URL подписки** (`profile-backup-urls`) и **трафик по каждому серверу**
  (`server-userinfo` / параметр `userinfo=`) — см. [спецификацию](docs/subscription-headers.md)

**Ссылки и импорт**
- Своя схема `flamix://import/<url>`
- Понимает ссылки других клиентов: `happ://add/…`, `v2raytun://import/…`, `v2rayng://install-sub?url=…`,
  `clash://install-config?url=…`, `sing-box://import-remote-profile?url=…`
- Импорт из буфера обмена

**Маршрутизация и диагностика**
- Пресеты: Global, «Bypass RU» (RU-сайты и IP напрямую, реклама блокируется), Custom (домены, IP, процессы в TUN)
- Пинг: TCP и реальный URL-тест через ядро
- Полоса трафика подписки, полоса трафика каждого сервера, страница статистики
- Трей, автозапуск, светлая/тёмная тема, интерфейс RU/EN

**Резервные серверы и мультитрафик (в планах)**
- Основной сервер + упорядоченный список резервных, которые держатся «тёплыми».
- Мгновенное переключение новых соединений, когда основной сервер падает; во время переключения — «заморозка»
  вместо ошибки (приложение видит медленное подключение, а не сбой).
- Бесшовный UDP/QUIC, если резервные серверы используют тот же выходной IP (поток переживает переключение).
- Режимы load-balance и выбор сервера по правилу маршрутизации.
- Честно: уже открытые TCP-соединения перенести на другой сервер невозможно — приложение переподключится само.
- Дизайн и детали — в [docs/failover.md](docs/failover.md).

## Сравнение

| Возможность | Flamix | Happ | v2RayTun | v2rayN |
|---|:---:|:---:|:---:|:---:|
| Открытый исходный код | ✅ | ❌ | ❌ | ✅ (GPL-3.0) |
| Десктоп: Windows / macOS / Linux | ✅ | ? | ? | ✅ |
| TUN-режим | ✅ | ✅ | ? | ✅ |
| Резервные URL подписки | ✅ | ? | ? | ? |
| Трафик по каждому серверу | ✅ | ? | ? | ? |
| Резервные серверы и мультитрафик | 🚧 | ? | ? | ? |
| Xray-core | ✅ | ✅ | ✅ | ✅ |
| sing-box | ✅ | ? | ? | ✅ |
| Понимает deep links других клиентов | ✅ | ? | ? | ? |

«?» — данных нет, чтобы не угадывать.

## Платформы

- **Windows** 10/11 x64
- **macOS** 11+ (universal)
- **Linux** x86_64 / aarch64: Debian/Ubuntu (`.deb`), Fedora/RHEL (`.rpm`), Arch (PKGBUILD),
  любой дистрибутив (AppImage, `.tar.gz`)
- **Мобильные** (Android, iOS) — в планах

## Скачать и установить

Готовые сборки публикуются на странице [Releases](https://github.com/lavrentijav/flamix-client/releases).

Пошаговая инструкция по платформам (deb, rpm, PKGBUILD, AppImage, `tar.gz`, macOS Gatekeeper и т. д.) —
в документе **[docs/install.md](docs/install.md)**.

## Быстрый старт

1. **Добавьте подписку**: «Добавить» → вставьте URL подписки (или ссылку `flamix://…`, `vless://…` из буфера).
2. **Выберите сервер** в списке (кнопка «Тест пинга» покажет задержку).
3. **Подключитесь**: режим **Proxy** (системный прокси + локальные SOCKS5/HTTP) или **TUN** (весь трафик,
   нужны права администратора/root).
4. Обновляйте подписку кнопкой «Обновить» — клиент сам переключится на резервный URL, если основной недоступен.

Пресеты маршрутизации (Global, «Bypass RU», Custom) настраиваются на странице **Routing**.

## Для провайдеров

Flamix читает стандартные заголовки подписки (`subscription-userinfo`, `profile-title`,
`profile-update-interval`, `support-url`, `profile-web-page-url`, `announce`), HWID-заголовки,
совместимые с Remnawave, и две необязательные возможности:

- **Резервные URL подписки** — `profile-backup-urls: https://a.example/sub, https://b.example/sub`
- **Трафик по каждому серверу** — заголовок `server-userinfo` или параметр `userinfo=` в ссылке:

```
vless://uuid@de1.example.com:443?type=ws&security=tls&userinfo=download%3D5368709120%3Btotal%3D53687091200#🇩🇪 Germany
```

Полная спецификация форматов, заголовков и метаданных в теле — в документе
**[docs/subscription-headers.md](docs/subscription-headers.md)**.

## Сборка из исходников

Инструкция по сборке (включая сборку на сервере через `scripts/ru1` и CI) — в документе
**[docs/build.md](docs/build.md)**.

## Структура проекта

```
crates/flamix-core   # платформо-независимая логика: модель, ссылки, подписки, конфиги, хранилище, пинг
crates/flamix-sys    # интеграция с ОС: системный прокси, процессы ядер, elevation, автозапуск
app/flamix           # десктопное приложение на Slint (ui/*.slint, src/*.rs)
docs/                # спецификации, задачи, дизайн
scripts/             # скрипты сборки и упаковки (scripts/ru1 — сборка на сервере)
```

## План развития

- [ ] 🚧 Логика ядра: модель, ссылки, подписки, конфиги, хранилище, пинг — в работе
- [ ] 🚧 Интеграция с ОС: системный прокси, процессы ядер, elevation, автозапуск — в работе
- [ ] 🚧 Главное окно на Slint, темы, RU/EN — в работе
- [ ] 🚧 Упаковка и CI: deb, rpm, PKGBUILD, AppImage, tar.gz, Windows-инсталлятор, macOS `.dmg` — в работе
- [ ] Резервные серверы и мультитрафик — в планах ([docs/failover.md](docs/failover.md))
- [ ] Android — в планах
- [ ] iOS — в планах

## Благодарности

- [Xray-core](https://github.com/XTLS/Xray-core) — прокси-ядро
- [sing-box](https://github.com/SagerNet/sing-box) — прокси-ядро
- [Slint](https://slint.dev) — UI-фреймворк
- [Alfa-Bank icons](https://github.com/core-ds/icons) (`@alfalab/icons-glyph`, MIT) — набор иконок;
  текст лицензии в [app/flamix/assets/icons/LICENSE-alfalab.txt](app/flamix/assets/icons/LICENSE-alfalab.txt)

## Участие

См. [CONTRIBUTING.md](CONTRIBUTING.md). Вопросы и баги — в
[Issues](https://github.com/lavrentijav/flamix-client/issues).

## Лицензия

**GPL-3.0-or-later**. Полный текст — в файле [LICENSE](LICENSE).
