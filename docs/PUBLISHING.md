# Publishing to the Linux stores

Where Linux users can install JellyBridge besides the files on the Releases
page, how each channel is fed, and what has to exist before it works. The
release itself is cut as in [RELEASING.md](RELEASING.md); everything here
starts from a **published** GitHub release, because every channel downloads
from it.

| Channel | Users run | What they get | Fed by |
| --- | --- | --- | --- |
| Snap Store | `sudo snap install jellybridge` | the portable bundle (bundled LGPL libmpv) in a strict snap | `publish.yml` → Snap Store, stable channel |
| AUR `jellybridge` | `yay -S jellybridge` | built from source against Arch's mpv | `publish.yml` → `scripts/aur.sh` |
| AUR `jellybridge-bin` | `yay -S jellybridge-bin` | the portable bundle, installs in seconds | same |
| Fedora COPR | `sudo dnf copr enable oddbench/jellybridge && sudo dnf install jellybridge` | built from source against Fedora's mpv | `publish.yml` → `scripts/copr.sh` |
| AppImage | download, make executable | the portable bundle; updates itself with AppImageUpdate | `release.yml` (`.AppImage` + `.zsync`) |
| Flathub | `flatpak install flathub co.oddbench.JellyBridge` | built from source on Flathub's builders | a manifest you write and submit by hand (below) |

`publish.yml` runs when a release is published and can be rerun for a tag
from the Actions tab (choose the channels). A channel whose secrets are not
set is skipped with a notice, so it is safe to publish before every account
exists. Every channel reads its version from a file in the repository, and
`release.yml` refuses to build when one of them disagrees with
`rust/app/Cargo.toml`:

| File | Field |
| --- | --- |
| `packaging/linux/co.oddbench.JellyBridge.metainfo.xml` | the first `<release version="…">` (also the snap's version and description) |
| `packaging/arch/PKGBUILD`, `packaging/arch/PKGBUILD-bin` | `pkgver` |
| `packaging/copr/jellybridge.spec` | `Version` (add a `%changelog` entry too) |

## Snap Store

`snap/snapcraft.yaml` wraps `target/linux/bundle` (the output of
`scripts/build-linux.sh`), so the snap is the same binary and libmpv as the
.deb. `release.yml` builds it with `snapcore/action-build` and attaches
`jellybridge_<version>_amd64.snap` to the release; the publish job uploads
that file to the **stable** channel. Strict confinement with the GNOME
extension: `network`, `audio-playback`, `opengl`, `wayland`/`x11`,
`desktop` and `screen-inhibit-control` (so the screen stays on while a
video plays). No `home` access: the app streams from the server and keeps
its config and cache in its own snap directory.

One-time setup:

1. Create an Ubuntu One account and install snapcraft on any machine
   (`sudo snap install snapcraft --classic`).
2. Register the name: `snapcraft login`, then `snapcraft register jellybridge`.
3. Export store credentials limited to this snap and add them as the
   repository secret `SNAPCRAFT_STORE_CREDENTIALS`:

   ```bash
   snapcraft export-login --snaps=jellybridge \
     --acls package_access,package_push,package_update,package_release \
     --expires 2027-12-31 exported.txt
   # paste the content of exported.txt into the secret, then delete the file
   ```

4. On the snap's store page (Listing), add the screenshots from
   `docs/screenshots/` and the website link; the name, summary, description
   and icon come from the snap itself.

The first upload goes through the store's automatic review (interfaces used
are all auto-connected ones, so no manual review is expected). Later
releases appear in `snap refresh` within hours.

Building and testing without the store: `scripts/build-linux.sh && snapcraft
pack` on a machine with snapcraft (LXD builds), or
`scripts/package-linux.sh snap`, which builds in a container like the other
packages; then `sudo snap install --dangerous jellybridge_*.snap`. The VM
harness installs it the same way: `scripts/vm-test/run-guest.sh ubuntu-2404
snap` (see `docs/rust-port/platform-matrix.md`).

## AUR

Two packages, both maintained from this repository:

- `jellybridge` (`packaging/arch/PKGBUILD`): builds from the tag's source
  tarball with Arch's Rust and depends on Arch's `mpv`, which is current and
  has the full FFmpeg. `scripts/package-linux.sh arch` builds exactly this
  PKGBUILD from a tarball of the checked-out commit.
- `jellybridge-bin` (`packaging/arch/PKGBUILD-bin`): unpacks the release's
  `jellybridge-<version>-x86_64.tar.gz`, so it installs without compiling and
  brings its own libmpv.

`scripts/aur.sh prepare <version> <dir>` downloads both release files,
writes PKGBUILDs with their checksums and generates `.SRCINFO`;
`scripts/aur.sh push` commits them to `ssh://aur@aur.archlinux.org/<name>.git`
(the first push creates the package). The AUR host key is pinned in the
script. The publish job runs both in an Arch container.

One-time setup:

1. Create an account on https://aur.archlinux.org and add an SSH public key
   to it (My Account → SSH Public Key). Use a key made for this purpose:
   `ssh-keygen -t ed25519 -f aur -C aur@jellybridge -N ''`.
2. Add the private key (`aur`, the whole file) as the repository secret
   `AUR_SSH_PRIVATE_KEY`.

Pushing by hand from an Arch machine: `scripts/aur.sh prepare 0.1.0 /tmp/aur
&& scripts/aur.sh push 0.1.0 /tmp/aur`.

## Fedora COPR

`packaging/copr/jellybridge.spec` builds from source in COPR's chroots. COPR
builds without network, so the crates come from
`jellybridge-<version>-vendor.tar.xz`, which `release.yml` produces with
`scripts/vendor-crates.sh` and attaches to the release. The package depends
on Fedora's `mpv-libs`; Fedora's own FFmpeg (`ffmpeg-free`) does not decode
H.264 or HEVC, so the description tells users to swap in RPM Fusion's FFmpeg.
Users who want everything included can use the release .rpm, the snap or
the AppImage instead.

One-time setup:

1. Sign in to https://copr.fedorainfracloud.org with a Fedora account and
   create the project `jellybridge` (so it is `oddbench/jellybridge`; set the
   repository variable `COPR_PROJECT` if the owner differs). Enable the
   chroots `fedora-<current>-x86_64`, `fedora-<current-1>-x86_64` and
   `fedora-rawhide-x86_64`. Description and instructions can repeat the RPM
   Fusion note from the spec.
2. On https://copr.fedorainfracloud.org/api/ copy the three values of the
   generated config into the secrets `COPR_LOGIN`, `COPR_USERNAME` and
   `COPR_TOKEN` (the token expires after 180 days; the API page shows when).

The publish job builds the SRPM in a Fedora container and submits it with
`copr-cli`, waiting for COPR's build so a failure shows in the workflow. By
hand: `scripts/copr.sh submit 0.1.0 /tmp/copr` on Fedora.

## AppImage

The AppImage carries update information
(`gh-releases-zsync|oddbench-co|jellybridge|latest|JellyBridge-*-x86_64.AppImage.zsync`),
and `release.yml` publishes the matching `.zsync` file, so AppImageUpdate and
AppImageLauncher fetch new releases as deltas. The AppImage can also be
listed on https://appimage.github.io: fork `AppImage/appimage.github.io`, add
`data/JellyBridge` containing the repository URL, and open a pull request;
their bot tests the latest release's AppImage. Do this once the first
release is public.

## Flathub

Flathub is the one channel this repository does not feed automatically, and
the manifest is not in the tree. Flathub's policy, as of October 2026,
requires that manifests contain no AI-generated or AI-assisted content and
that no AI tool opens or automates a submission pull request; it also asks
that AI-assisted material in the application be disclosed in the submission.
So the manifest and the submission are yours to write. What it needs:

- The app id `co.oddbench.JellyBridge` is domain-based, so verification is a
  token at `https://oddbench.co/.well-known/org.flathub.VerifiedApps.txt`
  (or a `_flathub.oddbench.co` TXT record), from the Flathub developer portal
  after acceptance. The file is served by the `oddbench-website` repository.
- Runtime `org.freedesktop.Platform` at the latest version (26.08 at the time
  of writing) with the `org.freedesktop.Sdk.Extension.rust-stable` SDK
  extension; the toolchain file in the repository is ignored by the
  extension's cargo, and the crate needs Rust 1.98 or newer.
- Builds are offline: the crates are declared from `Cargo.lock` with
  `flatpak-cargo-generator.py` from `flatpak/flatpak-builder-tools`
  (`python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json`;
  there are no git dependencies, so it runs without network).
- libmpv has to be built from source in the manifest; the recipe that the
  release bundle uses (versions, LGPL configuration, the static FFmpeg, dav1d,
  libass and libplacebo) is `scripts/linux/build-in-container.sh`, and the
  Flathub manifests of `io.mpv.Mpv` and Celluloid show the same libraries as
  Flatpak modules. The app loads `libmpv.so.2` from the default library path,
  which includes `/app/lib`.
- Permissions the app needs: `--share=network`, `--share=ipc`,
  `--socket=wayland`, `--socket=fallback-x11`, `--socket=pulseaudio`,
  `--device=dri`; the idle inhibitor uses the portal
  (`org.freedesktop.portal.Inhibit`), which needs no extra permission.
- The metainfo and desktop file in `packaging/linux/` already carry what
  Flathub lists: screenshots, release notes, branding, OARS rating, links.
- Submission is a pull request to `flathub/flathub` against the `new-pr`
  branch containing the manifest, `cargo-sources.json` and `flathub.json`
  (with `"only-arches": ["x86_64"]` until an aarch64 build is tested), after
  `flatpak-builder-lint` passes.

## Not done yet, and why

- **Ubuntu PPA**: Launchpad builds need Debian source packages signed with a
  GPG key, vendored crates and a libmpv built inside the package (Ubuntu
  22.04's libmpv is too old). The .deb, the snap and Flathub already cover
  Ubuntu and Debian users; a PPA is worth it only if a dnf/apt-updated
  package becomes a frequent request.
- **openSUSE (OBS)**: the COPR spec could be built on the Open Build
  Service with `libmpv2` in place of `mpv-libs`; openSUSE's mpv also needs
  Packman for full codecs. Flathub and the release .rpm serve openSUSE now.
- **Nixpkgs, Debian, Fedora proper**: distribution maintainers package from
  the source tarball; the vendor tarball and `packaging/` give them what
  they need. Worth starting once the release cadence is steady.

## Secrets

| Secret | Channel | Where it comes from |
| --- | --- | --- |
| `SNAPCRAFT_STORE_CREDENTIALS` | Snap Store | `snapcraft export-login` |
| `AUR_SSH_PRIVATE_KEY` | AUR | the private key whose public half is in the AUR account |
| `COPR_LOGIN`, `COPR_USERNAME`, `COPR_TOKEN` | Fedora COPR | https://copr.fedorainfracloud.org/api/ |
