//! Every word on the site. Facts come from the repository's README, docs/PERFORMANCE.md and
//! docs/RELEASING.md, so the site never promises more than the app does. Bodies marked "HTML" are
//! trusted markup written here and rendered unescaped.

pub const REPO: &str = "https://github.com/oddbench-co/jellybridge";
pub const ISSUES: &str = "https://github.com/oddbench-co/jellybridge/issues";
pub const RELEASES: &str = "https://github.com/oddbench-co/jellybridge/releases";

/// The version the Download page offers. Must match `rust/app/Cargo.toml` when a release is published.
pub const VERSION: &str = "0.1.0";
/// False until the GitHub release for `VERSION` is published: until then every download button opens
/// the Releases page instead of linking a file that does not exist yet.
pub const RELEASED: bool = false;
/// Last content change, for the sitemap's `lastmod`.
pub const UPDATED: &str = "2026-10-17";

/// An icon from `static/img/icons.svg`, by symbol id.
pub type Icon = &'static str;

pub struct Highlight {
    pub icon: Icon,
    pub title: &'static str,
    pub note: &'static str,
}

/// The four tiles under the home hero.
pub static HIGHLIGHTS: &[Highlight] = &[
    Highlight {
        icon: "layout",
        title: "Familiar UI",
        note: "Like jellyfin-web",
    },
    Highlight {
        icon: "play",
        title: "Direct play",
        note: "No transcoding",
    },
    Highlight {
        icon: "chip",
        title: "Hardware decoding",
        note: "Via mpv",
    },
    Highlight {
        icon: "devices",
        title: "Cross-platform",
        note: "Linux, Windows, macOS",
    },
];

pub struct Feature {
    pub icon: Icon,
    pub title: &'static str,
    /// HTML.
    pub body: &'static str,
}

pub static FEATURES: &[Feature] = &[
    Feature {
        icon: "play",
        title: "Direct play, always",
        body: "Plays the original file with <a href=\"https://mpv.io\">mpv</a>, so your server never transcodes and the \
               picture is never re-encoded. H.264, HEVC, AV1, VP9, 10-bit and HDR video, with hardware decoding where \
               your graphics card supports it.",
    },
    Feature {
        icon: "gauge",
        title: "Native performance",
        body: "Shows your Home screen about a tenth of a second after launch, uses around 120 MB of memory while you \
               browse and draws nothing while idle. Scrolling keeps up with 240 Hz displays.",
    },
    Feature {
        icon: "layout",
        title: "Familiar interface",
        body: "Pages, cards, menus and settings follow Jellyfin's web app, so everything is where you expect it. \
               Progress, watched state and favorites are saved on your server and sync with your other devices.",
    },
    Feature {
        icon: "shield",
        title: "Open and private",
        body: "Free software under GPL-3.0-or-later, with no telemetry. JellyBridge talks only to your server, and \
               your access token never reaches the player.",
    },
    Feature {
        icon: "devices",
        title: "Runs on all major platforms",
        body: "Packages for Linux, Windows and macOS bring their own player and codecs: there is no mpv or FFmpeg to \
               install, and portable builds run without installing anything.",
    },
];

/// More of what the app does, listed under the feature rows.
pub static FEATURE_LIST: &[(&str, &str)] = &[
    (
        "Home",
        "Your libraries, Continue Watching, Next Up and the latest additions, arranged by your Jellyfin Home settings.",
    ),
    (
        "Libraries",
        "Poster grids with sorting, filters, an A–Z jump bar and Suggestions, Favorites, Collections and Genres tabs.",
    ),
    (
        "Search",
        "Across the whole server, with results as you type.",
    ),
    (
        "Item pages",
        "Films, series, seasons, episodes and people: backdrop, logo, ratings, media info, cast and crew, trailers and similar titles.",
    ),
    (
        "Player",
        "Resume, audio and subtitle tracks, chapters, playback speed, previous and next episode, and skip buttons for intros, recaps and outros.",
    ),
    (
        "Stays awake",
        "While a video plays, no screen lock, blanking or sleep until you pause.",
    ),
    (
        "Playlists and collections",
        "Add items to them from any item's menu.",
    ),
    (
        "Sign-in",
        "With a password or Quick Connect, and you stay signed in.",
    ),
];

pub struct Screenshot {
    /// File stem in `static/img/screens/` (`<stem>-640.webp`, `<stem>-1280.webp`).
    pub stem: &'static str,
    pub title: &'static str,
    pub caption: &'static str,
    pub alt: &'static str,
    pub icon: Icon,
}

pub static SCREENSHOTS: &[Screenshot] = &[
    Screenshot {
        stem: "home",
        title: "Home",
        caption: "All your media in one place",
        alt: "JellyBridge Home screen with My Media, Continue Watching and Next Up rows",
        icon: "home",
    },
    Screenshot {
        stem: "library",
        title: "Library",
        caption: "Poster grids with filters and A–Z",
        alt: "Movies library shown as a poster grid with sort and filter controls",
        icon: "grid",
    },
    Screenshot {
        stem: "detail",
        title: "Details",
        caption: "Beautiful media pages",
        alt: "Film page with backdrop, logo, ratings, media info and cast",
        icon: "info",
    },
    Screenshot {
        stem: "player",
        title: "Player",
        caption: "Powerful mpv playback",
        alt: "Video player with seek bar, track menus and playback controls",
        icon: "play",
    },
];

pub struct Package {
    pub label: &'static str,
    pub note: &'static str,
    /// File name on the GitHub release; `{v}` is replaced by `VERSION`.
    pub file: &'static str,
}

pub struct Platform {
    /// Anchor on /download and the value the hero script picks.
    pub id: &'static str,
    pub name: &'static str,
    pub requirement: &'static str,
    pub icon: Icon,
    pub packages: &'static [Package],
}

pub static PLATFORMS: &[Platform] = &[
    Platform {
        id: "windows",
        name: "Windows",
        requirement: "10 or 11, 64-bit",
        icon: "windows",
        packages: &[
            Package {
                label: "Installer",
                note: "Recommended. Installs for your user only, no administrator prompt",
                file: "JellyBridge-{v}-setup.exe",
            },
            Package {
                label: "Portable .zip",
                note: "Unpack anywhere and run jellybridge.exe",
                file: "JellyBridge-{v}-portable.zip",
            },
        ],
    },
    Platform {
        id: "linux",
        name: "Linux",
        requirement: "x86_64, glibc 2.35 or newer",
        icon: "linux",
        packages: &[
            Package {
                label: "Ubuntu, Debian, Mint (.deb)",
                note: "Open it with the Software app",
                file: "jellybridge_{v}_amd64.deb",
            },
            Package {
                label: "Fedora, openSUSE (.rpm)",
                note: "dnf or zypper install",
                file: "jellybridge-{v}-1.x86_64.rpm",
            },
            Package {
                label: "Arch, Manjaro (.pkg.tar.zst)",
                note: "pacman -U, uses Arch's own mpv",
                file: "jellybridge-{v}-1-x86_64.pkg.tar.zst",
            },
            Package {
                label: "AppImage",
                note: "Any distribution: make it executable and run it",
                file: "JellyBridge-{v}-x86_64.AppImage",
            },
            Package {
                label: "Tarball (.tar.gz)",
                note: "No root needed: installs to ~/.local",
                file: "jellybridge-{v}-x86_64.tar.gz",
            },
        ],
    },
    Platform {
        id: "macos",
        name: "macOS",
        requirement: "Apple silicon or Intel, signed and notarized",
        icon: "apple",
        packages: &[
            Package {
                label: "Apple silicon (.dmg)",
                note: "macOS 14 or later",
                file: "JellyBridge-{v}-macos-arm64.dmg",
            },
            Package {
                label: "Intel (.dmg)",
                note: "macOS 15 or later",
                file: "JellyBridge-{v}-macos-x86_64.dmg",
            },
        ],
    },
];

pub static REQUIREMENTS: &[&str] = &[
    "64-bit Linux (x86_64), Windows 10 or 11, or macOS 14+ (Apple silicon) / 15+ (Intel)",
    "A graphics driver with OpenGL 2.0 or newer, which every current driver provides",
    "A Jellyfin server you can reach: tested with Jellyfin 10.11 and 12",
    "Nothing else: every package brings its own player and codecs",
];

pub struct Faq {
    pub question: &'static str,
    /// HTML.
    pub answer: &'static str,
}

pub static FAQS: &[Faq] = &[
    Faq {
        question: "What is JellyBridge?",
        answer: "A native desktop client for <a href=\"https://jellyfin.org\">Jellyfin</a>, the free media server. It looks and \
                 works like Jellyfin's web app but runs as a real desktop program instead of a browser tab, and plays your \
                 files with a built-in mpv player.",
    },
    Faq {
        question: "How is it different from Jellyfin Media Player?",
        answer: "Jellyfin Media Player wraps the web app in an embedded browser. JellyBridge draws its own interface natively \
                 in Rust, so it starts in about a tenth of a second, uses around 120 MB of memory while browsing and uses \
                 next to no CPU while idle.",
    },
    Faq {
        question: "Does it support direct play?",
        answer: "Always. mpv plays the original file, with hardware decoding where your graphics card supports it, so your \
                 server's CPU stays idle and the picture is never re-encoded. JellyBridge never asks the server to transcode.",
    },
    Faq {
        question: "Which platforms are supported?",
        answer: "Linux (Ubuntu, Debian, Fedora, openSUSE, Arch and any other distribution with glibc 2.35 or newer), \
                 Windows 10 and 11, and macOS 14 or later on Apple silicon or 15 or later on Intel. See <a href=\"/download\">Download</a>.",
    },
    Faq {
        question: "Do I need to install mpv or FFmpeg?",
        answer: "No. Every package includes the player and the codecs it needs. The Arch package is the one exception: it uses \
                 Arch's own mpv package, which pacman installs for you.",
    },
    Faq {
        question: "Where are settings stored?",
        answer: "Your Jellyfin preferences (Home sections, playback and subtitle choices) are saved on your server, like in the \
                 web app. The sign-in and a few local settings are files in your user folder; \
                 <a href=\"/docs/file-locations\">File locations</a> lists them.",
    },
    Faq {
        question: "Is there any telemetry?",
        answer: "No. JellyBridge talks only to the Jellyfin server you sign in to. It sends no usage data, crash reports or \
                 analytics anywhere. See <a href=\"/privacy\">Privacy</a>.",
    },
    Faq {
        question: "Is JellyBridge free?",
        answer: "Yes. It is free software under the GNU General Public License v3.0 or later: free to use, study, change and share.",
    },
    Faq {
        question: "Can I use it with my existing Jellyfin server?",
        answer: "Yes. Enter your server's address, sign in with your Jellyfin account or Quick Connect, and your libraries, \
                 progress and favorites are all there. Nothing needs to change on the server.",
    },
    Faq {
        question: "A video will not play, or the picture is black. What can I do?",
        answer: "Start JellyBridge with <code>JELLYBRIDGE_HWDEC=no</code> to rule out hardware decoding. \
                 <a href=\"/docs/troubleshooting\">Troubleshooting</a> covers this and other common problems.",
    },
    Faq {
        question: "How do I report a bug or request a feature?",
        answer: "Open an issue on <a href=\"https://github.com/oddbench-co/jellybridge/issues\">GitHub</a>. For a playback problem, \
                 include your system, the file's format and what the player's status line shows.",
    },
    Faq {
        question: "When will the first public release be available?",
        answer: "Version 0.1.0 is built and tested on every supported system, and is being prepared for release. Until then \
                 you can <a href=\"/docs/building-from-source\">build it from source</a>.",
    },
];

pub struct Point {
    pub icon: Icon,
    pub title: &'static str,
    pub body: &'static str,
}

pub static PRIVACY: &[Point] = &[
    Point {
        icon: "eye-off",
        title: "No telemetry",
        body: "JellyBridge does not collect or send any usage data, crash reports or analytics.",
    },
    Point {
        icon: "server",
        title: "Your server, your data",
        body: "It only connects to the Jellyfin server you choose. No third-party services; online trailers open in your own web browser.",
    },
    Point {
        icon: "lock",
        title: "Secure by design",
        body: "Your access token is stored in your user folder and never passed to the player: mpv streams through a local proxy that adds it, so it never appears in a URL.",
    },
    Point {
        icon: "code",
        title: "Open source",
        body: "The full source code is on GitHub under GPL-3.0-or-later. You can read it, build it yourself and check every claim on this page.",
    },
];

pub static ABOUT: &[Point] = &[
    Point {
        icon: "code",
        title: "Open source",
        body: "GPL-3.0-or-later",
    },
    Point {
        icon: "heart",
        title: "Community driven",
        body: "Feedback and contributions are welcome",
    },
    Point {
        icon: "rust",
        title: "Built with Rust",
        body: "Performance, safety and reliability",
    },
    Point {
        icon: "users",
        title: "Focused on users",
        body: "A better desktop experience for the Jellyfin community",
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    GettingStarted,
    Guides,
    Reference,
    Development,
}

impl Group {
    pub const ALL: [Group; 4] = [
        Group::GettingStarted,
        Group::Guides,
        Group::Reference,
        Group::Development,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::GettingStarted => "Getting started",
            Group::Guides => "Guides",
            Group::Reference => "Reference",
            Group::Development => "Development",
        }
    }
}

pub struct Doc {
    /// Empty for the overview at `/docs`.
    pub slug: &'static str,
    pub group: Group,
    pub title: &'static str,
    /// `<title>` without the site suffix.
    pub seo_title: &'static str,
    /// Meta description, 110–165 characters.
    pub description: &'static str,
    pub summary: &'static str,
    /// HTML.
    pub body: &'static str,
}

impl Doc {
    pub fn path(&self) -> String {
        if self.slug.is_empty() {
            "/docs".into()
        } else {
            format!("/docs/{}", self.slug)
        }
    }
}

pub static DOCS: &[Doc] = &[
    Doc {
        slug: "",
        group: Group::GettingStarted,
        title: "Getting started",
        seo_title: "JellyBridge documentation: install, connect, play",
        description: "Everything you need to install, configure and use JellyBridge, the native Jellyfin desktop client: setup, \
                      playback, shortcuts and troubleshooting.",
        summary: "JellyBridge connects to your existing Jellyfin server and gives you a native desktop experience with the same \
                  familiar interface.",
        body: r#"<ol class="steps">
<li><h2>Install</h2><p>Download and install JellyBridge for your system. <a href="/docs/installation">Installation</a> explains each package.</p></li>
<li><h2>Launch</h2><p>Start JellyBridge from your app menu, or run <code>jellybridge</code> in a terminal.</p></li>
<li><h2>Connect</h2><p>Enter your Jellyfin server's address and sign in. See <a href="/docs/connecting">Connecting to your server</a>.</p></li>
<li><h2>Enjoy</h2><p>Browse and play your media with native performance. <a href="/docs/basic-usage">Basic usage</a> shows you around.</p></li>
</ol>"#,
    },
    Doc {
        slug: "installation",
        group: Group::GettingStarted,
        title: "Installation",
        seo_title: "Install JellyBridge on Linux, Windows, macOS",
        description: "How to install JellyBridge on Ubuntu, Debian, Fedora, openSUSE, Arch, any Linux via AppImage, Windows 10 \
                      and 11, and macOS. Every package includes its own player.",
        summary: "Pick the file for your system. Every package includes everything playback needs: you do not need to install mpv or FFmpeg.",
        body: r#"<div class="table-wrap" tabindex="0" role="region" aria-label="Packages and how to install them"><table>
<thead><tr><th scope="col">System</th><th scope="col">File</th><th scope="col">How to install</th></tr></thead>
<tbody>
<tr><td>Ubuntu 22.04+, Debian 12+, Linux Mint, Pop!_OS, elementary OS</td><td><code>jellybridge_&lt;version&gt;_amd64.deb</code></td><td>Open it with the Software app, or <code>sudo apt install ./jellybridge_*_amd64.deb</code></td></tr>
<tr><td>Fedora, openSUSE</td><td><code>jellybridge-&lt;version&gt;-1.x86_64.rpm</code></td><td><code>sudo dnf install ./jellybridge-*.rpm</code> or <code>sudo zypper install ./jellybridge-*.rpm</code></td></tr>
<tr><td>Arch, Manjaro, EndeavourOS</td><td><code>jellybridge-&lt;version&gt;-1-x86_64.pkg.tar.zst</code></td><td><code>sudo pacman -U jellybridge-*.pkg.tar.zst</code> (uses Arch's own mpv)</td></tr>
<tr><td>Any other Linux (glibc 2.35+)</td><td><code>JellyBridge-&lt;version&gt;-x86_64.AppImage</code></td><td>Make it executable and run it</td></tr>
<tr><td>Linux, without root</td><td><code>jellybridge-&lt;version&gt;-x86_64.tar.gz</code></td><td>Unpack and run <code>./install.sh</code> (installs to <code>~/.local</code>)</td></tr>
<tr><td>Windows 10 and 11</td><td><code>JellyBridge-&lt;version&gt;-setup.exe</code></td><td>Run it. Installs for your user only, no administrator prompt</td></tr>
<tr><td>Windows, no install</td><td><code>JellyBridge-&lt;version&gt;-portable.zip</code></td><td>Unpack anywhere and run <code>jellybridge.exe</code></td></tr>
<tr><td>macOS 14+ on Apple silicon</td><td><code>JellyBridge-&lt;version&gt;-macos-arm64.dmg</code></td><td>Open it and drag JellyBridge to Applications</td></tr>
<tr><td>macOS 15+ on Intel</td><td><code>JellyBridge-&lt;version&gt;-macos-x86_64.dmg</code></td><td>Open it and drag JellyBridge to Applications</td></tr>
</tbody></table></div>
<p>Each package was installed and tested on Ubuntu, Debian, Fedora, openSUSE, Mint, Pop!_OS, elementary, Manjaro, EndeavourOS, Windows 10 and Windows 11. The macOS builds are signed and notarized by Apple.</p>
<h2>Uninstalling</h2>
<p>Remove JellyBridge like any other app: with your package manager, from Windows' Installed apps, or by moving it out of Applications on macOS. Your sign-in and settings stay in place, so a reinstall keeps you signed in; <a href="/docs/file-locations">File locations</a> lists them if you want to remove them too.</p>"#,
    },
    Doc {
        slug: "first-launch",
        group: Group::GettingStarted,
        title: "First launch",
        seo_title: "First launch of JellyBridge",
        description: "What happens the first time you start JellyBridge: where to find it, what it checks, and what to do if it \
                      will not start on your graphics driver.",
        summary: "Start JellyBridge from your app menu. The first screen asks for your server.",
        body: r#"<p>The packages add JellyBridge to your app menu (Start menu on Windows, Applications on macOS). From a terminal, run <code>jellybridge</code>.</p>
<p>On the first start there is nothing to set up: JellyBridge opens straight on the server screen. After you sign in once, later launches go directly to your Home screen and show it about a tenth of a second after the window opens, from the copy saved last time, while the server's latest version loads.</p>
<h2>If it will not start</h2>
<p>JellyBridge needs a graphics driver with OpenGL 2.0 or newer. If yours lacks it, JellyBridge shows a message saying so; install or update the driver. Started from a terminal, it prints the reason there too.</p>"#,
    },
    Doc {
        slug: "connecting",
        group: Group::GettingStarted,
        title: "Connecting to your server",
        seo_title: "Connect JellyBridge to your Jellyfin server",
        description: "Enter your Jellyfin server address and sign in to JellyBridge with a password or Quick Connect. Works with \
                      local addresses and HTTPS domains alike.",
        summary: "JellyBridge works with any Jellyfin server you can reach: on your network or over the internet.",
        body: r#"<ol>
<li>Enter your server's address, for example <code>192.168.1.10:8096</code> or <code>https://jellyfin.example.com</code>.</li>
<li>Sign in with your Jellyfin username and password, or choose <strong>Quick Connect</strong> and approve the code from a device where you are already signed in.</li>
</ol>
<p>You stay signed in until you choose <strong>Sign Out</strong>. Tested with Jellyfin 10.11 and 12.</p>
<h2>Asked to sign in again?</h2>
<p>The server no longer accepts the saved sign-in, usually because it was signed out from the server's dashboard or the password changed. Sign in again.</p>"#,
    },
    Doc {
        slug: "basic-usage",
        group: Group::GettingStarted,
        title: "Basic usage",
        seo_title: "Using JellyBridge: Home, libraries, search",
        description: "A tour of JellyBridge: the Home screen, library grids with filters and A–Z, search, item pages, playlists and \
                      collections, and progress that syncs.",
        summary: "Everything is where jellyfin-web puts it. A short tour:",
        body: r#"<dl>
<dt>Home</dt><dd>Your libraries, Continue Watching, Next Up and the latest additions, arranged by your Jellyfin Home settings.</dd>
<dt>Libraries</dt><dd>Poster grids with sorting, filters, an A–Z jump bar, and Suggestions, Favorites, Collections and Genres tabs.</dd>
<dt>Search</dt><dd>Searches the whole server, with results as you type.</dd>
<dt>Item pages</dt><dd>Films, series, seasons, episodes and people: backdrop, logo, ratings, media info, cast and crew, trailers and similar titles.</dd>
<dt>Playlists and collections</dt><dd>Add items to them from any item's menu.</dd>
<dt>Progress that syncs</dt><dd>Positions, watched state and favorites are saved on the server, so you can continue on any other device.</dd>
</dl>
<p>JellyBridge is built for video. It does not (yet) offer transcoding for files mpv cannot play, Live TV, SyncPlay, casting, offline sync or server administration; music and books can be browsed but have no dedicated player.</p>"#,
    },
    Doc {
        slug: "playback",
        group: Group::Guides,
        title: "Playback",
        seo_title: "Playback in JellyBridge: direct play with mpv",
        description: "How JellyBridge plays video: direct play of the original file with mpv, hardware decoding, resume, chapters, \
                      speed, next episode and intro skipping.",
        summary: "JellyBridge plays the original file through mpv. The server never transcodes.",
        body: r#"<p>mpv decodes H.264, HEVC, AV1, VP9, 10-bit and HDR video from the original file, with hardware decoding where your GPU supports it. Your server's CPU stays idle and the picture is never re-encoded. Formats your GPU cannot decode (10-bit H.264, 4:2:2 chroma and similar) fall back to software decoding automatically; the player's status line shows which decoder is in use.</p>
<h2>In the player</h2>
<ul>
<li><strong>Resume</strong> where you left off, on this or any other device.</li>
<li><strong>Chapters</strong> and <strong>playback speed</strong>.</li>
<li><strong>Previous and next episode</strong>, and the next episode plays automatically if you turn that on in Playback settings.</li>
<li><strong>Skip buttons</strong> for intros, recaps and outros where the server has detected them.</li>
<li>Your computer <strong>stays awake</strong> while a video plays: no screen lock, blanking or sleep until you pause.</li>
</ul>
<p>Everything can be done from the keyboard: see <a href="/docs/keyboard-shortcuts">Keyboard shortcuts</a>.</p>"#,
    },
    Doc {
        slug: "subtitles-and-audio",
        group: Group::Guides,
        title: "Subtitles and audio",
        seo_title: "Subtitles and audio tracks in JellyBridge",
        description: "Choose audio and subtitle tracks in JellyBridge, embedded or external, set preferred languages, and style \
                      subtitles to your liking from Settings.",
        summary: "Audio and subtitle tracks, embedded in the file or external, are in the player's track menus.",
        body: r#"<p>Open the track menus in the player, or press <kbd>A</kbd> for the next audio track and <kbd>C</kbd> for the next subtitle track. JellyBridge starts with the tracks your server picks for you, based on your language preferences.</p>
<h2>Preferences</h2>
<ul>
<li><strong>Settings → Playback</strong>: preferred audio language, maximum audio channels, whether to play the default track, and whether to remember your choices from the previous item.</li>
<li><strong>Settings → Subtitles</strong>: preferred subtitle language, when subtitles show, and their appearance: size, weight, color, shadow and position.</li>
</ul>
<p>These preferences are saved on your Jellyfin server, so they are the same in the web app and your other clients.</p>"#,
    },
    Doc {
        slug: "keyboard-shortcuts",
        group: Group::Guides,
        title: "Keyboard shortcuts",
        seo_title: "JellyBridge keyboard shortcuts for the player",
        description: "Every keyboard shortcut in the JellyBridge player: play and pause, seeking, volume, fullscreen, audio and \
                      subtitle tracks, speed, chapters and episodes.",
        summary: "The player can be driven entirely from the keyboard.",
        body: r#"<div class="table-wrap" tabindex="0" role="region" aria-label="Player keyboard shortcuts"><table>
<thead><tr><th scope="col">Key</th><th scope="col">Action</th></tr></thead>
<tbody>
<tr><td><kbd>Space</kbd>, <kbd>K</kbd></td><td>Play or pause</td></tr>
<tr><td><kbd>←</kbd> / <kbd>→</kbd></td><td>Back 10 s / forward 30 s (with <kbd>Shift</kbd>: 5 s)</td></tr>
<tr><td><kbd>J</kbd> / <kbd>L</kbd></td><td>Back 10 s / forward 30 s</td></tr>
<tr><td><kbd>0</kbd>–<kbd>9</kbd></td><td>Jump to 0 %–90 % of the video</td></tr>
<tr><td><kbd>Home</kbd> / <kbd>End</kbd></td><td>Jump to the start / end</td></tr>
<tr><td><kbd>↑</kbd> / <kbd>↓</kbd></td><td>Volume up / down</td></tr>
<tr><td><kbd>M</kbd></td><td>Mute</td></tr>
<tr><td><kbd>F</kbd></td><td>Fullscreen</td></tr>
<tr><td><kbd>A</kbd> / <kbd>C</kbd></td><td>Next audio track / next subtitle track</td></tr>
<tr><td><kbd>&lt;</kbd> / <kbd>&gt;</kbd></td><td>Slower / faster</td></tr>
<tr><td><kbd>Page Up</kbd> / <kbd>Page Down</kbd></td><td>Next / previous chapter</td></tr>
<tr><td><kbd>Shift</kbd>+<kbd>N</kbd> / <kbd>Shift</kbd>+<kbd>P</kbd></td><td>Next / previous episode</td></tr>
<tr><td><kbd>Esc</kbd></td><td>Leave fullscreen, or leave the player</td></tr>
<tr><td><kbd>Q</kbd></td><td>Leave the player</td></tr>
</tbody></table></div>"#,
    },
    Doc {
        slug: "troubleshooting",
        group: Group::Guides,
        title: "Troubleshooting",
        seo_title: "Troubleshooting JellyBridge",
        description: "Fixes for common JellyBridge problems: a video that will not play or shows a black picture, being asked to \
                      sign in again, and an app that will not start.",
        summary: "The most common problems and what to do about them.",
        body: r#"<h2>A video will not play, or the picture is black</h2>
<p>JellyBridge plays the original file and never asks the server to transcode. mpv falls back to software decoding for formats your GPU cannot decode. To rule out hardware decoding, start it with <code>JELLYBRIDGE_HWDEC=no jellybridge</code>; <code>JELLYBRIDGE_HWDEC=auto</code> allows modes the default (<code>auto-safe</code>) leaves out. The player's status line shows which decoder is in use.</p>
<h2>JellyBridge asks me to sign in again</h2>
<p>The server no longer accepts the saved sign-in, usually because it was signed out from the server's dashboard or the password changed. Sign in again.</p>
<h2>It will not start</h2>
<p>If your graphics driver lacks OpenGL 2.0, JellyBridge shows a message saying so; install or update the driver. Started from a terminal, it prints the reason there too.</p>
<h2>Still stuck?</h2>
<p>Open an issue on <a href="https://github.com/oddbench-co/jellybridge/issues">GitHub</a> with your system, what you did and what happened. For playback problems, include the file's format and the decoder shown in the status line.</p>"#,
    },
    Doc {
        slug: "settings",
        group: Group::Reference,
        title: "Settings",
        seo_title: "JellyBridge settings reference",
        description: "The JellyBridge settings pages: Profile, Quick Connect, Display, Home, Playback and Subtitles, and which of \
                      them are saved on your Jellyfin server.",
        summary: "Open Settings from the user menu. The pages match jellyfin-web's, and save to your Jellyfin server.",
        body: r#"<dl>
<dt>Profile</dt><dd>Your user name and picture.</dd>
<dt>Quick Connect</dt><dd>Approve a sign-in on another device with its code.</dd>
<dt>Display</dt><dd>How libraries and episodes are shown, such as missing episodes.</dd>
<dt>Home</dt><dd>Which sections appear on the Home screen and in what order, library order, libraries hidden from My Media or Latest, each library's default screen, and grouped library folders.</dd>
<dt>Playback</dt><dd>Preferred audio language, maximum audio channels, playing the next episode automatically, remembering audio and subtitle choices, and what to do at intro, recap, preview, commercial and outro segments.</dd>
<dt>Subtitles</dt><dd>Preferred subtitle language, subtitle mode and appearance.</dd>
</dl>
<p>Because these settings live on your server, changing them in JellyBridge changes them in the web app too, and the other way round.</p>"#,
    },
    Doc {
        slug: "command-line",
        group: Group::Reference,
        title: "Command line",
        seo_title: "JellyBridge environment variables",
        description: "Environment variables that change how JellyBridge plays video: hardware decoding mode and extra mpv options, \
                      for troubleshooting and tuning playback.",
        summary: "JellyBridge has no command-line flags. A few environment variables change playback, mostly for troubleshooting.",
        body: r#"<dl>
<dt><code>JELLYBRIDGE_HWDEC</code></dt><dd>mpv's hardware decoding mode. The default is <code>auto-safe</code>; <code>no</code> turns hardware decoding off, <code>auto</code> allows modes the default leaves out.</dd>
<dt><code>JELLYBRIDGE_MPV_OPTIONS</code></dt><dd>Any further mpv options, as <code>name=value,name=value</code>. They are applied last, so they override JellyBridge's own.</dd>
</dl>
<p>Example: <code>JELLYBRIDGE_HWDEC=no jellybridge</code></p>
<p>On Windows, set the variable in PowerShell before starting the app: <code>$env:JELLYBRIDGE_HWDEC="no"; jellybridge.exe</code>.</p>"#,
    },
    Doc {
        slug: "file-locations",
        group: Group::Reference,
        title: "File locations",
        seo_title: "Where JellyBridge stores its files",
        description: "Where JellyBridge keeps your sign-in, settings and artwork cache on Linux, Windows and macOS, and what \
                      uninstalling or signing out removes.",
        summary: "JellyBridge keeps three things on your computer.",
        body: r#"<div class="table-wrap" tabindex="0" role="region" aria-label="Where JellyBridge stores its files"><table>
<thead><tr><th scope="col">What</th><th scope="col">Linux and macOS</th><th scope="col">Windows</th></tr></thead>
<tbody>
<tr><td>Sign-in (server address, access token, user id)</td><td><code>~/.config/jellybridge/session.json</code></td><td><code>%APPDATA%\JellyBridge\session.json</code></td></tr>
<tr><td>Settings</td><td><code>~/.config/jellybridge/settings.json</code></td><td><code>%APPDATA%\JellyBridge\settings.json</code></td></tr>
<tr><td>Artwork cache</td><td><code>~/.cache/jellybridge/</code></td><td><code>%LOCALAPPDATA%\JellyBridge\</code></td></tr>
</tbody></table></div>
<p>Uninstalling leaves these in place, so a reinstall keeps you signed in. Sign Out in the app removes the sign-in. The session file holds an access token to your server: keep it private.</p>"#,
    },
    Doc {
        slug: "building-from-source",
        group: Group::Development,
        title: "Building from source",
        seo_title: "Build JellyBridge from source",
        description: "Build and run JellyBridge from source with Rust and libmpv, run the tests, and build the release packages \
                      for Linux, Windows and macOS in containers.",
        summary: "You need rustup and, to play media, libmpv from your system's mpv package.",
        body: r#"<p>Install <a href="https://rustup.rs">rustup</a>; the toolchain version is pinned by <code>rust-toolchain.toml</code>. For playback, install libmpv: <code>sudo apt install libmpv-dev</code>, <code>sudo pacman -S mpv</code> or <code>brew install mpv</code>. Browsing works without it; it is loaded on the first play.</p>
<p>Run every cargo command through <code>scripts/cargo-env.sh</code>, which pins the toolchain and points the build at libmpv:</p>
<pre><code>git clone https://github.com/oddbench-co/jellybridge
cd jellybridge
scripts/cargo-env.sh run --release                                  # run the app
scripts/cargo-env.sh test --locked                                  # tests
scripts/cargo-env.sh clippy --all-targets --locked -- -D warnings   # lint</code></pre>
<h2>Packages</h2>
<p>The release packages are built in throwaway containers (podman or docker), so nothing is installed on your machine:</p>
<pre><code>scripts/build-linux.sh      # portable bundle with libmpv and codecs
scripts/package-linux.sh    # .deb, .rpm, AppImage, tarball and Arch package
scripts/build-windows.sh    # setup.exe and portable zip
scripts/package-macos.sh    # on a Mac: .app and .dmg for its CPU</code></pre>
<p>The first <code>build-linux.sh</code> compiles mpv, FFmpeg, dav1d, libass, libplacebo and libva from source (about 15 minutes, cached afterwards).</p>"#,
    },
    Doc {
        slug: "contributing",
        group: Group::Development,
        title: "Contributing",
        seo_title: "Contributing to JellyBridge",
        description: "How to contribute to JellyBridge: report bugs, open pull requests, run the tests and lints, and bring \
                      measurements for changes that affect speed.",
        summary: "Bug reports and pull requests are welcome.",
        body: r#"<ul>
<li>Report bugs and ideas as <a href="https://github.com/oddbench-co/jellybridge/issues">GitHub issues</a>.</li>
<li>Before opening a pull request, run the tests, clippy and rustfmt (see <a href="/docs/building-from-source">Building from source</a>).</li>
<li><code>AGENTS.md</code> in the repository describes the code layout, conventions and test scripts in more depth.</li>
<li>Changes that touch startup, scrolling or playback should come with before and after numbers from <code>JELLYBRIDGE_MEASURE</code>; <code>docs/PERFORMANCE.md</code> explains how.</li>
</ul>
<p>JellyBridge is licensed under GPL-3.0-or-later; contributions are accepted under the same licence.</p>"#,
    },
    Doc {
        slug: "architecture",
        group: Group::Development,
        title: "Architecture",
        seo_title: "How JellyBridge is built: Rust, egui and mpv",
        description: "How JellyBridge is built: a Rust app with an egui interface on OpenGL, libmpv rendering into the same window, \
                      and all network and disk work off the UI thread.",
        summary: "JellyBridge is written in Rust.",
        body: r#"<p>The interface is drawn with <a href="https://github.com/emilk/egui">egui</a> on OpenGL, and libmpv renders video into the same window. Network and disk work runs on a tokio runtime, off the thread that draws, so the window never waits on the server. An idle window requests no frames, so it uses next to no CPU.</p>
<pre><code>rust/app/src
├── main.rs        startup and wiring
├── api.rs, api/   Jellyfin HTTP client
├── features/      what the app does: sign-in, pages, playback, settings
├── services/      artwork cache, session storage, mpv, the playback proxy
└── ui/            everything on screen (the only code that uses egui)</code></pre>
<p>libmpv is loaded the first time something plays rather than linked, which removes 70–80 ms of dynamic linking from every launch. <code>docs/PERFORMANCE.md</code> in the repository records what was measured and why each optimization works.</p>"#,
    },
];
