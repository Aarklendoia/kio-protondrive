# Changelog

## [0.2.0](https://github.com/Aarklendoia/kio-protondrive/compare/v0.1.0...v0.2.0) (2026-07-24)


### Features

* **i18n:** add es, zh_CN, hi, ar, pt_BR, ru, ja, de translations ([e5c0539](https://github.com/Aarklendoia/kio-protondrive/commit/e5c053984bd4a71ed2609dfa9755b4e1ddcc29a9))
* initial protondrive:// KIO worker for Dolphin ([e172f95](https://github.com/Aarklendoia/kio-protondrive/commit/e172f9500fff13f3bedc55eed1c2fa8b6c4d833f))
* **worker:** translate virtual root section names via KDE i18n ([58b7e2d](https://github.com/Aarklendoia/kio-protondrive/commit/58b7e2dc85b8c47edad5ec03ca7b28b472d240d9))
* **worker:** translate virtual root section names via KDE i18n ([5ec6b70](https://github.com/Aarklendoia/kio-protondrive/commit/5ec6b70b375ebd8de28bd1965438ba062ac5b23d))


### Bug Fixes

* **core:** time out proton-drive CLI calls instead of hanging forever ([c142891](https://github.com/Aarklendoia/kio-protondrive/commit/c14289141d6c0fd03a242052e5bf8a27909bfaaf))
* **core:** time out proton-drive CLI calls instead of hanging forever ([cd4cbd8](https://github.com/Aarklendoia/kio-protondrive/commit/cd4cbd8d1dbb979df023ed74b23e430ebad87ae3))
* **debian:** point FindRust straight at the real rustc/cargo binaries ([fa9bcdd](https://github.com/Aarklendoia/kio-protondrive/commit/fa9bcdd5a7bb162707445ac51a3a97e1909b9479))
* repair CMake/cxx bridge wiring so the project actually builds ([2a859f4](https://github.com/Aarklendoia/kio-protondrive/commit/2a859f4029722a489b71be905e0c56c71f799410))
* repair CMake/cxx bridge wiring so the project actually builds ([c285190](https://github.com/Aarklendoia/kio-protondrive/commit/c2851908b6fe0677e6bf2eef701b01606cbd559a))
* **worker:** add kdemain() entry point so kioworker can actually launch the plugin ([be5f3a0](https://github.com/Aarklendoia/kio-protondrive/commit/be5f3a05865ca14e83f655ba372a6d904d73d7be))
* **worker:** emit a "." UDSEntry in listDir to satisfy KIO's convention ([457a2ab](https://github.com/Aarklendoia/kio-protondrive/commit/457a2abb747e3b111019657bd40c44d074662cf7))
* **worker:** make the KIO worker actually launch (kdemain entry point + "." UDSEntry) ([61523a4](https://github.com/Aarklendoia/kio-protondrive/commit/61523a4244512b182578231f294d033ea8f96b84))
