# Идеология проекта
## RU
Всеобщий доступ без каких-либо ограничений при использовании. Проект должен сделать более комфортным использование оффлайн музыки на устройствах пользователя, предоставляя удобные механизмы синхронизации и переноса данных между устройсвами.
Главная идея при разработке: минимальное потребление ресурсов, не жертвуя безопасностью, и гибкость настройки самого приложения.

Ядро предоставляет api для взаимодействия сторонних библиотек (моды для приложения), которые расширяют и меняют функционал под потребности пользователя.

## ENG 
Universal access without any restrictions. The project aims to enhance the offline music experience on user devices by providing convenient mechanisms for data synchronization and transfer between devices.
The core development philosophy focuses on minimal resource consumption without compromising security, alongside high flexibility in app configuration.

The core provides an API for third-party library integration (app mods), allowing users to extend and customize functionality to meet their specific needs.

# TODO 
- [ ] Storage
- [ ] Indexing musics
- [ ] Import/Export config and musics 
- [ ] Sync config and musics with other device (wifi/usb)
- [ ] FFI 

# Future features 
- Передача структуры плейлистов между устройствами (через любой доступный интерфейс обмена между устройствами) и возможность скачивать (на время передачи для прослушивания или навсегда сохранять у себя) песни с другого устройства. Также можно передавать целые плейлисты.

# Crossplatform
Supported OS:
 - linux 
 - windows (WARN: not tested)
 - ~~android~~
 - ~~ios~~
