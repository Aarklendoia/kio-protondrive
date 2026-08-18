# Changelog

## [0.6.0](https://github.com/Aarklendoia/kio-protondrive/compare/v0.5.0...v0.6.0) (2026-08-18)


### Features

* cancellable uploads/downloads with approximate progress ([#9](https://github.com/Aarklendoia/kio-protondrive/issues/9)) ([#59](https://github.com/Aarklendoia/kio-protondrive/issues/59)) ([5945aa6](https://github.com/Aarklendoia/kio-protondrive/commit/5945aa61e9bcddbbb2d2a0b7c6407da787c8febe))
* persistent filesystem listing/stat cache ([#8](https://github.com/Aarklendoia/kio-protondrive/issues/8)) ([#57](https://github.com/Aarklendoia/kio-protondrive/issues/57)) ([0babf73](https://github.com/Aarklendoia/kio-protondrive/commit/0babf73805111d65ed2c807ac0457128a29389c9))
* server-side rename and move support ([#56](https://github.com/Aarklendoia/kio-protondrive/issues/56)) ([75b8283](https://github.com/Aarklendoia/kio-protondrive/commit/75b8283c6850d260f8fd69b499540eac483e7ab7))

## [0.5.0](https://github.com/Aarklendoia/kio-protondrive/compare/v0.4.1...v0.5.0) (2026-08-18)


### Features

* browse /photos read-only, fix the Places bookmark to the Drive root ([#18](https://github.com/Aarklendoia/kio-protondrive/issues/18)) ([#54](https://github.com/Aarklendoia/kio-protondrive/issues/54)) ([941238c](https://github.com/Aarklendoia/kio-protondrive/commit/941238ccda0ff9eed13979092b226564e67a6e31))

## [0.4.1](https://github.com/Aarklendoia/kio-protondrive/compare/v0.4.0...v0.4.1) (2026-08-18)


### Bug Fixes

* capture the CLI's stdout for diagnostics on unrecognized failures ([#53](https://github.com/Aarklendoia/kio-protondrive/issues/53)) ([41c9013](https://github.com/Aarklendoia/kio-protondrive/commit/41c901384c9693b0c69c87caf7aba77829a32167))
* use the freedesktop bookmark:icon element for the Places entry ([#51](https://github.com/Aarklendoia/kio-protondrive/issues/51)) ([4587855](https://github.com/Aarklendoia/kio-protondrive/commit/45878559dad9f546dde9bad16288a88d7b87c9b3))

## [0.4.0](https://github.com/Aarklendoia/kio-protondrive/compare/v0.3.1...v0.4.0) (2026-08-17)


### Features

* **daemon:** localize desktop notification strings ([#31](https://github.com/Aarklendoia/kio-protondrive/issues/31)) ([#49](https://github.com/Aarklendoia/kio-protondrive/issues/49)) ([10f1839](https://github.com/Aarklendoia/kio-protondrive/commit/10f1839c7f06602744eb2bc9387ca94f6d1bcd6c))
* **daemon:** notify when a newer proton-drive CLI is available ([#46](https://github.com/Aarklendoia/kio-protondrive/issues/46)) ([29680cb](https://github.com/Aarklendoia/kio-protondrive/commit/29680cb5d98f8c1c08fdb7b24e7a5b340dc6a682))
* **worker:** add standalone mimetype() KIO method ([#45](https://github.com/Aarklendoia/kio-protondrive/issues/45)) ([43a9677](https://github.com/Aarklendoia/kio-protondrive/commit/43a9677aec1181372c095e385a369cda6cce466f))


### Bug Fixes

* **daemon:** run the CLI version check at startup, not after 24h ([#48](https://github.com/Aarklendoia/kio-protondrive/issues/48)) ([37c5f90](https://github.com/Aarklendoia/kio-protondrive/commit/37c5f901536c49d0053d0fc3fe4d1cb77cb5ea2a))

## [0.3.1](https://github.com/Aarklendoia/kio-protondrive/compare/v0.3.0...v0.3.1) (2026-08-01)


### Bug Fixes

* **core,worker:** escape upload glob metacharacters + treat already-existing folder as success ([b86e5bc](https://github.com/Aarklendoia/kio-protondrive/commit/b86e5bc2559a25c5d9d32cf5124ce018ff8c9122))
* **core,worker:** treat an already-existing folder as success, not a hard failure ([5596359](https://github.com/Aarklendoia/kio-protondrive/commit/55963590c5db8f4b478681d0aef6936c16cbaec4))
* **core:** escape glob metacharacters in upload's local path ([dc1c515](https://github.com/Aarklendoia/kio-protondrive/commit/dc1c51556996952d09cf2ae0ded9d7cb5bbf91dd))
* **daemon:** stop the systemd unit retrying forever with no config ([d35a4d4](https://github.com/Aarklendoia/kio-protondrive/commit/d35a4d4f61155ccb013425d6c5d4623370cc090f))
* **daemon:** stop the systemd unit retrying forever with no config ([013488f](https://github.com/Aarklendoia/kio-protondrive/commit/013488f851c342ab9001de1f0284a291ff12fff8))
* **worker:** call dataReq() before readData() in put() ([f341756](https://github.com/Aarklendoia/kio-protondrive/commit/f341756f4996705f8a4a03f6117a5edac6fc1289))
* **worker:** call dataReq() before readData() in put() ([b342eab](https://github.com/Aarklendoia/kio-protondrive/commit/b342eab0416e51a1d8ea11b26abdc8941b3c0865))

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
