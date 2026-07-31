# Changelog

## [0.3.0](https://github.com/Aarklendoia/kio-protondrive/compare/v0.2.0...v0.3.0) (2026-07-31)


### Features

* **daemon:** add Phase 1 sync daemon (one-way local -&gt; Drive upload) ([5162812](https://github.com/Aarklendoia/kio-protondrive/commit/5162812af557f842098ae1dc6deb98a957753b17))
* **daemon:** Phase 1 sync daemon — one-way local -&gt; Drive upload ([4ea34ad](https://github.com/Aarklendoia/kio-protondrive/commit/4ea34ad5bb8252a9783274481204e6edd0ec076c))
* surface missing/expired Proton Drive authentication actionably ([2632c57](https://github.com/Aarklendoia/kio-protondrive/commit/2632c576faee9ad931b4e6dccad9138e8dc67833))
* surface missing/expired Proton Drive authentication actionably ([aae13ae](https://github.com/Aarklendoia/kio-protondrive/commit/aae13ae267cca504b5216076f9bab7ee001978b0))


### Bug Fixes

* **daemon:** default to a non-keyring credentials store for the CLI ([506636d](https://github.com/Aarklendoia/kio-protondrive/commit/506636d7936b19f71f74213942a4169353bc64a6))
* **packaging:** vendor cxxbridge-cmd's own deps for real offline builds ([0aa3e3e](https://github.com/Aarklendoia/kio-protondrive/commit/0aa3e3e0786c7b12371d073bd6f37f053095ac0c))
* **packaging:** vendor cxxbridge-cmd's own deps for real offline builds ([107deea](https://github.com/Aarklendoia/kio-protondrive/commit/107deea9e4ba62fe395355b8543eb4f4d9541806))
* **worker:** translate the breadcrumb label too, not just the icon grid ([4f8babb](https://github.com/Aarklendoia/kio-protondrive/commit/4f8babb91224ec647d0715a89b658e3c105ab672))
* **worker:** translate the breadcrumb label too, not just the icon grid ([f394bf6](https://github.com/Aarklendoia/kio-protondrive/commit/f394bf6ffc46cdaa6804977641ad055b99de14da))

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
