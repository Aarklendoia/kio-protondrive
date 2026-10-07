# Changelog

## [0.11.2](https://github.com/Aarklendoia/kio-protondrive/compare/v0.11.1...v0.11.2) (2026-10-07)


### Bug Fixes

* **core:** fetch the CLI manifest and binary over HTTPS only, redirects included ([#172](https://github.com/Aarklendoia/kio-protondrive/issues/172)) ([6a61781](https://github.com/Aarklendoia/kio-protondrive/commit/6a617811fb16a87f27ea37a1e19b4e2c754aeb27))
* **core:** refuse remote paths that would map outside the local cache root ([#175](https://github.com/Aarklendoia/kio-protondrive/issues/175)) ([31f84f0](https://github.com/Aarklendoia/kio-protondrive/commit/31f84f05bc69c853d210a053ddc1ff05c708b816))
* **core:** time out silent control-server connections and decode %-escapes byte-wise ([#174](https://github.com/Aarklendoia/kio-protondrive/issues/174)) ([670ab87](https://github.com/Aarklendoia/kio-protondrive/commit/670ab87c1a7742d23fe7355e28d7fe071ed4ef98))
* **daemon:** emit batched D-Bus signals with busctl so paths arrive intact ([#170](https://github.com/Aarklendoia/kio-protondrive/issues/170)) ([22b020d](https://github.com/Aarklendoia/kio-protondrive/commit/22b020dcaacf578db711a76f627cce4b4ad31206))
* never pass a user-supplied value starting with - as a bare command-line argument ([#171](https://github.com/Aarklendoia/kio-protondrive/issues/171)) ([8982f87](https://github.com/Aarklendoia/kio-protondrive/commit/8982f87e1ebe2f07d970ea5ed953ccfc41395f99))
* **wizard:** drop the control server's CORS headers and OPTIONS answer ([#173](https://github.com/Aarklendoia/kio-protondrive/issues/173)) ([9dd89ee](https://github.com/Aarklendoia/kio-protondrive/commit/9dd89ee127a2e6db8c7d0367d0a70684d88b7df4))
* **wizard:** feed the gpg key batch on stdin and reject control characters in it ([#169](https://github.com/Aarklendoia/kio-protondrive/issues/169)) ([70c2f47](https://github.com/Aarklendoia/kio-protondrive/commit/70c2f478c0c8d7c57f1077cb98f1bd37f1d69597))
* **wizard:** only accept known credentials stores, and quote the one written to the login script ([#168](https://github.com/Aarklendoia/kio-protondrive/issues/168)) ([722f6c8](https://github.com/Aarklendoia/kio-protondrive/commit/722f6c8fc06194dad2dbb00fe24b1eee317e98d4))
* **worker:** only download into the cache directory when the file can be recorded ([#176](https://github.com/Aarklendoia/kio-protondrive/issues/176)) ([9c02015](https://github.com/Aarklendoia/kio-protondrive/commit/9c020157e52adcaa58fcd0e2c067d4c5ba6f231c))

## [0.11.1](https://github.com/Aarklendoia/kio-protondrive/compare/v0.11.0...v0.11.1) (2026-10-07)


### Bug Fixes

* **daemon:** count suspended time toward the daily version check and cache eviction ([#142](https://github.com/Aarklendoia/kio-protondrive/issues/142)) ([9f137d6](https://github.com/Aarklendoia/kio-protondrive/commit/9f137d6b24f241edb950217293ef40d3f3691f39))
* **daemon:** drop the no-op network-online.target ordering from the user units ([#143](https://github.com/Aarklendoia/kio-protondrive/issues/143)) ([8a516f2](https://github.com/Aarklendoia/kio-protondrive/commit/8a516f2ae6b1ce50d16fc8321085561d6e1354e7))
* **daemon:** log at info level by default when RUST_LOG isn't set ([#144](https://github.com/Aarklendoia/kio-protondrive/issues/144)) ([eb435a4](https://github.com/Aarklendoia/kio-protondrive/commit/eb435a455962e8dfdf6673427f1e197c0fb1c188))
* **daemon:** retry a failed CLI version check after 15 minutes instead of a day ([#141](https://github.com/Aarklendoia/kio-protondrive/issues/141)) ([6580185](https://github.com/Aarklendoia/kio-protondrive/commit/6580185459be95adf034a9e93bb80543ce17a31f))
* **packaging:** add the wizard's and plugins' runtime deps to the PKGBUILD's makedepends ([#153](https://github.com/Aarklendoia/kio-protondrive/issues/153)) ([40f3073](https://github.com/Aarklendoia/kio-protondrive/commit/40f30730946e328cd5d85351e3de05a19e84bcb1))
* **packaging:** let release-please bump the PKGBUILD's pkgver ([#151](https://github.com/Aarklendoia/kio-protondrive/issues/151)) ([cdd687b](https://github.com/Aarklendoia/kio-protondrive/commit/cdd687bb4773f3a30b29f0fc6ca7ba30ca5f8a58))
* **wizard:** name protondrive:/, not /my-files, on the Places page ([#150](https://github.com/Aarklendoia/kio-protondrive/issues/150)) ([7a0f91d](https://github.com/Aarklendoia/kio-protondrive/commit/7a0f91dbcc3b6c53efbe74997799205907ec3def))

## [0.11.0](https://github.com/Aarklendoia/kio-protondrive/compare/v0.10.1...v0.11.0) (2026-10-06)


### Features

* require proton-drive CLI 0.9.0 or newer ([#129](https://github.com/Aarklendoia/kio-protondrive/issues/129)) ([036490b](https://github.com/Aarklendoia/kio-protondrive/commit/036490b7f0ff81bdae4e5e49d372b2151e63d874)), closes [#127](https://github.com/Aarklendoia/kio-protondrive/issues/127)
* **wizard:** add AppStream MetaInfo for kio-protondrive-wizard ([#101](https://github.com/Aarklendoia/kio-protondrive/issues/101)) ([1ff0f4d](https://github.com/Aarklendoia/kio-protondrive/commit/1ff0f4dc583d7da126abddb36cc49f86903e210f))
* **worker:** copy server-side within protondrive:/ ([#124](https://github.com/Aarklendoia/kio-protondrive/issues/124)) ([82ba956](https://github.com/Aarklendoia/kio-protondrive/commit/82ba956cbf946e6c6a3b334013c30fffe64fce26)), closes [#109](https://github.com/Aarklendoia/kio-protondrive/issues/109)


### Bug Fixes

* **aur:** add qt6-tools to makedepends ([#115](https://github.com/Aarklendoia/kio-protondrive/issues/115)) ([902d9ec](https://github.com/Aarklendoia/kio-protondrive/commit/902d9ec167e0062326ba181496fb8338d6318b17)), closes [#112](https://github.com/Aarklendoia/kio-protondrive/issues/112)
* **aur:** build without LTO so the KIO plugins actually load ([#114](https://github.com/Aarklendoia/kio-protondrive/issues/114)) ([df89d5c](https://github.com/Aarklendoia/kio-protondrive/commit/df89d5c50197d484bca5b5152c40c7e444ec051e)), closes [#111](https://github.com/Aarklendoia/kio-protondrive/issues/111)
* **core:** invalidate a renamed or trashed folder's whole cached subtree ([#132](https://github.com/Aarklendoia/kio-protondrive/issues/132)) ([a8f098c](https://github.com/Aarklendoia/kio-protondrive/commit/a8f098ceb25d898de13790daf12ada621e75f65a)), closes [#128](https://github.com/Aarklendoia/kio-protondrive/issues/128)
* **core:** move a pinned file's pin along when it's renamed through KIO ([#136](https://github.com/Aarklendoia/kio-protondrive/issues/136)) ([e0a0064](https://github.com/Aarklendoia/kio-protondrive/commit/e0a006419a7b46fb90e202887a70c0c92330f866)), closes [#135](https://github.com/Aarklendoia/kio-protondrive/issues/135)
* **core:** show the file's real size, not its encrypted storage size ([#126](https://github.com/Aarklendoia/kio-protondrive/issues/126)) ([12ef940](https://github.com/Aarklendoia/kio-protondrive/commit/12ef940e8178239f683303cc12bddc1d9f96e157)), closes [#125](https://github.com/Aarklendoia/kio-protondrive/issues/125)
* **wizard:** translate the Finish page's log-out hint ([#134](https://github.com/Aarklendoia/kio-protondrive/issues/134)) ([d0e8680](https://github.com/Aarklendoia/kio-protondrive/commit/d0e86803b8de147751bf9d2d0c7228ea6f91b1a9)), closes [#130](https://github.com/Aarklendoia/kio-protondrive/issues/130)
* **worker:** name stat() entries after the item, not "." ([#123](https://github.com/Aarklendoia/kio-protondrive/issues/123)) ([188ea63](https://github.com/Aarklendoia/kio-protondrive/commit/188ea63a7eddb92ae900ff12151c70e33b2308da))
* **worker:** synthesize the entry for the virtual root and its sections ([#120](https://github.com/Aarklendoia/kio-protondrive/issues/120)) ([d1494a2](https://github.com/Aarklendoia/kio-protondrive/commit/d1494a2a2c570da186443544a04b68f9b162e1e5)), closes [#108](https://github.com/Aarklendoia/kio-protondrive/issues/108)
* **worker:** trash a deleted folder whole instead of child by child ([#133](https://github.com/Aarklendoia/kio-protondrive/issues/133)) ([5a2bb42](https://github.com/Aarklendoia/kio-protondrive/commit/5a2bb428aaefbb1e92ae23b106e0abade65b9082)), closes [#131](https://github.com/Aarklendoia/kio-protondrive/issues/131)

## [0.10.1](https://github.com/Aarklendoia/kio-protondrive/compare/v0.10.0...v0.10.1) (2026-08-25)


### Bug Fixes

* **wizard:** sync KIO worker's credentials store via plasma-workspace env ([#99](https://github.com/Aarklendoia/kio-protondrive/issues/99)) ([82ea8a8](https://github.com/Aarklendoia/kio-protondrive/commit/82ea8a8c16f3e0e50dde859fe4d389beeb7cb61c))

## [0.10.0](https://github.com/Aarklendoia/kio-protondrive/compare/v0.9.1...v0.10.0) (2026-08-21)


### Features

* add AUR packaging (PKGBUILD) ([#93](https://github.com/Aarklendoia/kio-protondrive/issues/93)) ([8894fa8](https://github.com/Aarklendoia/kio-protondrive/commit/8894fa8e6398921f6ca1b803dfd5291e5de658f2)), closes [#89](https://github.com/Aarklendoia/kio-protondrive/issues/89)

## [0.9.1](https://github.com/Aarklendoia/kio-protondrive/compare/v0.9.0...v0.9.1) (2026-08-20)


### Bug Fixes

* exclude /photos/&lt;category&gt; filter folders from the Share action ([#82](https://github.com/Aarklendoia/kio-protondrive/issues/82)) ([ed9947a](https://github.com/Aarklendoia/kio-protondrive/commit/ed9947a6e722bed4f72c24aba49af3ed9a82a9b3)), closes [#74](https://github.com/Aarklendoia/kio-protondrive/issues/74)
* invalidate fs_stat_cache on NotFound in refresh_stat_cache ([#84](https://github.com/Aarklendoia/kio-protondrive/issues/84)) ([bbab8c5](https://github.com/Aarklendoia/kio-protondrive/commit/bbab8c5be3411b0613292468bfb133bdfb6f35dc)), closes [#76](https://github.com/Aarklendoia/kio-protondrive/issues/76)
* preload an existing public link's role/expiration in ShareDialog ([#81](https://github.com/Aarklendoia/kio-protondrive/issues/81)) ([200af63](https://github.com/Aarklendoia/kio-protondrive/commit/200af630c219f2bc51c1ea020befbd5639061ae9)), closes [#73](https://github.com/Aarklendoia/kio-protondrive/issues/73)


### Performance Improvements

* batch the daemon's periodic overlay-refresh D-Bus notifications ([#83](https://github.com/Aarklendoia/kio-protondrive/issues/83)) ([747c241](https://github.com/Aarklendoia/kio-protondrive/commit/747c2414ab5cef2a6c4c4572e0257461170e3d82)), closes [#75](https://github.com/Aarklendoia/kio-protondrive/issues/75)
* stop blocking the GUI thread on sharing actions' cache refresh ([#85](https://github.com/Aarklendoia/kio-protondrive/issues/85)) ([56625b1](https://github.com/Aarklendoia/kio-protondrive/commit/56625b144eee8b917b9ba34e0b6faa096ed2d61e)), closes [#77](https://github.com/Aarklendoia/kio-protondrive/issues/77)


### Code Refactoring

* factor ShareDialog's cursor/error boilerplate into tryOrWarn ([#87](https://github.com/Aarklendoia/kio-protondrive/issues/87)) ([2b71123](https://github.com/Aarklendoia/kio-protondrive/commit/2b711237dfe0a5dbf132b96a3210c690eebf70b6)), closes [#79](https://github.com/Aarklendoia/kio-protondrive/issues/79)
* rename the PinChanged overlay signal to OverlayChanged ([#86](https://github.com/Aarklendoia/kio-protondrive/issues/86)) ([702d5e3](https://github.com/Aarklendoia/kio-protondrive/commit/702d5e3acbf53c25088baa5a3fb7d397910e9f6c)), closes [#78](https://github.com/Aarklendoia/kio-protondrive/issues/78)

## [0.9.0](https://github.com/Aarklendoia/kio-protondrive/compare/v0.8.0...v0.9.0) (2026-08-20)


### Features

* sharing and public link support ([#69](https://github.com/Aarklendoia/kio-protondrive/issues/69)) ([18657a6](https://github.com/Aarklendoia/kio-protondrive/commit/18657a6b57b7c2981e3dfe9f321779be8960a4f3))

## [0.8.0](https://github.com/Aarklendoia/kio-protondrive/compare/v0.7.0...v0.8.0) (2026-08-19)


### Features

* restorable trash via context menu ([#7](https://github.com/Aarklendoia/kio-protondrive/issues/7)) ([#63](https://github.com/Aarklendoia/kio-protondrive/issues/63)) ([75c6ef9](https://github.com/Aarklendoia/kio-protondrive/commit/75c6ef9c96bfd5ec6d8f77e8ec0a0c098ddf0e14))
* wizard-installed/daemon-updated proton-drive CLI with a real version check ([#65](https://github.com/Aarklendoia/kio-protondrive/issues/65)) ([#66](https://github.com/Aarklendoia/kio-protondrive/issues/66)) ([66814c2](https://github.com/Aarklendoia/kio-protondrive/commit/66814c2655c0d4528dfac5a7fcadb121bb02f416))
* **worker:** filter Photos by category via the context-menu (favorites, screenshots, videos, ...) ([#68](https://github.com/Aarklendoia/kio-protondrive/issues/68)) ([9b1147e](https://github.com/Aarklendoia/kio-protondrive/commit/9b1147edce9e7e167c31e178d3d097c14df99c51))
* **worker:** give the Drive root's virtual sections distinct icons ([#67](https://github.com/Aarklendoia/kio-protondrive/issues/67)) ([580880a](https://github.com/Aarklendoia/kio-protondrive/commit/580880a93d5323b4fb643d3b09855b9279a40153))

## [0.7.0](https://github.com/Aarklendoia/kio-protondrive/compare/v0.6.0...v0.7.0) (2026-08-18)


### Features

* opportunistic local file cache with configurable retention ([#60](https://github.com/Aarklendoia/kio-protondrive/issues/60)) ([#61](https://github.com/Aarklendoia/kio-protondrive/issues/61)) ([d40ae37](https://github.com/Aarklendoia/kio-protondrive/commit/d40ae372e0b220512b7ffc0ea696e80df871d52f))

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
