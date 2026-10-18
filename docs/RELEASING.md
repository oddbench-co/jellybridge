# Releasing JellyBridge

Every package is built by GitHub Actions (`.github/workflows/release.yml`)
when a version tag is pushed. The workflow puts them in a **draft** release,
so nothing is public until you publish it.

## Cutting a release

1. Set the version in `rust/app/Cargo.toml`, add a `<release>` entry with
   notes to `packaging/linux/co.oddbench.JellyBridge.metainfo.xml`, and set
   the same version in `packaging/arch/PKGBUILD`, `packaging/arch/PKGBUILD-bin`
   and `packaging/copr/jellybridge.spec` (with a `%changelog` entry). The
   workflow fails at once when they disagree. Commit and push.
2. Tag that commit and push the tag:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

3. Wait for the **Release** workflow (about 30–45 minutes; the first run
   builds the Linux media libraries and takes longer). It fails at once if
   the tag does not match the version in `Cargo.toml`.
4. Open the draft on the Releases page, check the files and the generated
   notes, and publish it. Publishing starts the **Publish** workflow, which
   sends the release to the Snap Store, the AUR and Fedora COPR; what each
   channel needs is in [PUBLISHING.md](PUBLISHING.md).

A failed build can be rerun from the Actions tab. To build again from a fixed
commit, delete the draft release and the tag (`git push --delete origin
v0.1.0`), then tag again.

To test the packages without a release, run the workflow by hand: Actions →
Release → Run workflow, or `gh workflow run release.yml --ref main -f
macos=false` (leave out `-f macos=false` to build the .dmg files too). The
files are attached to the run as artifacts; no tag or release is created.

## What the workflow builds

| Job | Runner | Files |
| --- | --- | --- |
| Linux | ubuntu-24.04, in Ubuntu 22.04 and Arch containers, plus snapcraft | `.deb`, `.rpm`, `.AppImage` (+ `.zsync`), `.tar.gz`, `.pkg.tar.zst`, `.snap`, `-vendor.tar.xz` (the crates, for offline distribution builds) |
| Windows | ubuntu-24.04, cross-built in a container | `-setup.exe`, `-portable.zip` |
| macOS arm64 | macos-14 | `-macos-arm64.dmg` (runs on macOS 14 and newer) |
| macOS x86_64 | macos-15-intel | `-macos-x86_64.dmg` (runs on macOS 15 and newer) |

The draft also gets `SHA256SUMS.txt` with a checksum for every file. The
macOS minimum is the runner's macOS, because the bundled Homebrew libraries
are built for it (see `docs/rust-port/packaging.md`).

## macOS signing secrets

macOS releases are signed with a Developer ID and notarized by Apple, so they
open without a warning. That needs a paid Apple Developer Program membership
and five repository secrets (Settings → Secrets and variables → Actions →
New repository secret). The workflow stops with a list of what is missing.

| Secret | What it is |
| --- | --- |
| `MACOS_CERTIFICATE` | The "Developer ID Application" certificate with its private key, exported as .p12, base64-encoded |
| `MACOS_CERTIFICATE_PWD` | The password chosen when exporting the .p12 |
| `APPLE_API_KEY_P8_BASE64` | An App Store Connect API key (.p8 file), base64-encoded |
| `APPLE_API_KEY_ID` | That key's id (10 characters) |
| `APPLE_API_ISSUER_ID` | The issuer id shown above the keys list (a UUID) |

**The certificate.** In the Apple Developer account, Certificates → "+" →
*Developer ID Application*, following Apple's steps to create it on a Mac.
Then, in Keychain Access, find "Developer ID Application: …", select the
certificate together with its private key, export as .p12 with a strong
password, and encode it:

```bash
base64 -i DeveloperID.p12 | pbcopy     # paste into MACOS_CERTIFICATE
```

**The API key.** In App Store Connect, Users and Access → Integrations → App
Store Connect API → Team Keys → "+", with the *Developer* role. Download the
.p8 (it can be downloaded only once), note the key id and the issuer id, and
encode the file:

```bash
base64 -i AuthKey_XXXXXXXXXX.p8 | pbcopy   # paste into APPLE_API_KEY_P8_BASE64
```

Delete the local .p12 and .p8 copies once the secrets are saved, or keep them
in a password manager. The workflow writes them only to the runner's
temporary directory and removes its keychain when the job ends.

## Building locally

Each job is a script you can run yourself; see "Building packages" in the
README. On a Mac, `scripts/package-macos.sh` signs ad hoc unless
`MACOS_SIGNING_IDENTITY` (and, for notarization, `APPLE_API_KEY_PATH`,
`APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`) are set.

## Costs

Actions minutes are free for public repositories. On a private repository
they count against the plan's quota, and macOS minutes count ten times; one
release uses roughly 30–40 Linux minutes and 40–60 macOS minutes.
