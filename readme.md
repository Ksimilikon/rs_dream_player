# Ideology / Идеология проекта

## ENG

Universal access without any restrictions. The project aims to enhance the offline music experience on user devices by providing convenient mechanisms for data synchronization and transfer between devices.
The core development philosophy focuses on minimal resource consumption without compromising security, alongside high flexibility in app configuration.

The core provides an API for third-party library integration (app mods), allowing users to extend and customize functionality to meet their specific needs.

## RU

Всеобщий доступ без каких-либо ограничений при использовании. Проект должен сделать более комфортным использование оффлайн музыки на устройствах пользователя, предоставляя удобные механизмы синхронизации и переноса данных между устройсвами.
Главная идея при разработке: минимальное потребление ресурсов, не жертвуя безопасностью, и гибкость настройки самого приложения.

Ядро предоставляет api для взаимодействия сторонних библиотек (моды для приложения), которые расширяют и меняют функционал под потребности пользователя.

# Interface Architecture / Архитектура взаимодействия

[ENG]
The core is strictly designed as a headless system component. Any communication 
with the core (both from Frontends and Plugins/Mods) is strictly limited to **FFI 
(Foreign Function Interface)**. No other data transfer methods (IPC, Sockets, RPC) 
are permitted inside the core to guarantee maximum memory safety, ultra-low latency, 
and zero architectural overhead.

[RU]
Ядро спроектировано строго как headless-компонент системы. Любое взаимодействие 
с ядром (как со стороны фронтендов, так и со стороны плагинов/модов) ограничено 
исключительно через **FFI (Foreign Function Interface)**. Любые другие способы 
передачи данных (IPC, сокеты, RPC) внутри ядра находятся под строгим запретом для 
обеспечения максимальной безопасности памяти, сверхнизкой задержки и отсутствия 
архитектурного оверхеда.

## Schema
```text
+-----------------------------------------------------------+  
|               FRONTENDS (Kotlin / Swift / C#)             |
+-----------------------------------------------------------+
                              │
     ================= FFI BOUNDARY =================
                              ▼
+───────────────────────────────────────────────────────────+
|                        CORE ENGINE                        |
|   [ Metadata Parser ]  [ Sync Stack ]  [ Audio State ]    |
+───────────────────────────────────────────────────────────+
                              ▲
     ================= FFI BOUNDARY =================
                              │
+-----------------------------------------------------------+
|                      THIRD-PARTY MODS                     |
+-----------------------------------------------------------+
```

# Features

- Editing metadata music files
- Synchronization/sending/receiving media between local devices
- System of modification (in development)
- Consumption resources of the device is very low 
- Crossplatform for all popular operating systems
- Tracks have individual volume to make fast fix volume of track

# Build

In order to build need install cargo and rustc in your system.

### Build release with native TUI:
```bash
  cargo build --features cli
```
```bash 
  cargo build --features cli --release 
```
### Just build core link dyn lib: 
```bash
  cargo build
```
```bash  
  cargo build --release
```
Bin files are located in target/

# Crossplatform

Supported OS:
- linux
- windows
- android
- ios
- macos

# LICENSE

This project is licensed under the **Mozilla Public License 2.0** - see the [LICENSE](LICENSE) file for details.

