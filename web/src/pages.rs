//! Every page is known at startup: the content is compiled in. So each one is rendered and
//! compressed exactly once, and requests only choose which prepared bytes to send.

use crate::assets::Prepared;
use crate::content::{
    self, ABOUT, DOCS, Doc, FAQS, FEATURE_LIST, FEATURES, Faq, Feature, Group, HIGHLIGHTS,
    Highlight, PLATFORMS, PRIVACY, Platform, Point, REQUIREMENTS, SCREENSHOTS, Screenshot,
};
use askama::Template;
use axum::http::StatusCode;
use std::collections::HashMap;

/// Third-party hooks, set per deployment; all public, since they ship in every page's `<head>`:
/// search-engine ownership tokens (Google Search Console, Ahrefs) and the Ahrefs Web Analytics key.
#[derive(Clone, Copy, Default)]
pub struct Integrations {
    pub google: Option<&'static str>,
    pub ahrefs: Option<&'static str>,
    /// Cookie-free page-view counting. When set, the CSP admits analytics.ahrefs.com (see `main::csp`).
    pub ahrefs_analytics: Option<&'static str>,
}

pub struct Head {
    pub title: String,
    pub description: String,
    /// Path of this page, for the canonical URL.
    pub path: String,
    /// Which main navigation item is current: features, screenshots, download, docs or none.
    pub nav: &'static str,
    pub site_url: &'static str,
    /// False for 404s: `noindex`, and no canonical URL.
    pub index: bool,
    /// Structured data (schema.org JSON-LD), already escaped for a `<script>` element.
    pub json_ld: Option<String>,
    pub integrations: Integrations,
}

impl Head {
    pub fn canonical(&self) -> String {
        format!("{}{}", self.site_url, self.path)
    }

    /// Absolute URL of the social preview card (1200 x 630).
    pub fn image(&self) -> String {
        format!(
            "{}{}",
            self.site_url,
            crate::assets::url("/static/img/og.jpg")
        )
    }
}

/// An icon from the sprite. The sprite is one cached file shared by every page.
pub fn icon(name: &str) -> String {
    format!(
        r#"<svg class="icon" aria-hidden="true" focusable="false"><use href="{}#{name}"></use></svg>"#,
        crate::assets::url("/static/img/icons.svg")
    )
}

/// `srcset` for a screenshot stem: the 640 and 1280 px WebP files.
pub fn srcset(stem: &str) -> String {
    format!(
        "{} 640w, {} 1280w",
        crate::assets::url(&format!("/static/img/screens/{stem}-640.webp")),
        crate::assets::url(&format!("/static/img/screens/{stem}-1280.webp"))
    )
}

pub fn screen(stem: &str, width: u32) -> String {
    crate::assets::url(&format!("/static/img/screens/{stem}-{width}.webp"))
}

/// A JSON string literal. `<` is escaped too, so the text can never close the `<script>` it sits in.
fn js(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The text of trusted HTML, for structured data: tags dropped, the few entities used here decoded.
fn text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

pub struct Link {
    pub label: &'static str,
    pub note: &'static str,
    pub href: String,
}

pub struct PlatformView {
    pub p: &'static Platform,
    pub links: Vec<Link>,
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomePage {
    head: Head,
    highlights: &'static [Highlight],
    features: &'static [Feature],
    platforms: &'static [Platform],
}

#[derive(Template)]
#[template(path = "features.html")]
struct FeaturesPage {
    head: Head,
    features: &'static [Feature],
    more: &'static [(&'static str, &'static str)],
}

#[derive(Template)]
#[template(path = "screenshots.html")]
struct ScreenshotsPage {
    head: Head,
    shots: &'static [Screenshot],
}

#[derive(Template)]
#[template(path = "download.html")]
struct DownloadPage {
    head: Head,
    version: &'static str,
    released: bool,
    platforms: Vec<PlatformView>,
    requirements: &'static [&'static str],
}

pub struct NavGroup {
    pub label: &'static str,
    pub docs: Vec<&'static Doc>,
}

#[derive(Template)]
#[template(path = "doc.html")]
struct DocPage {
    head: Head,
    doc: &'static Doc,
    groups: &'static [NavGroup],
    prev: Option<&'static Doc>,
    next: Option<&'static Doc>,
}

#[derive(Template)]
#[template(path = "faq.html")]
struct FaqPage {
    head: Head,
    faqs: &'static [Faq],
}

#[derive(Template)]
#[template(path = "privacy.html")]
struct PrivacyPage {
    head: Head,
    points: &'static [Point],
}

#[derive(Template)]
#[template(path = "about.html")]
struct AboutPage {
    head: Head,
    points: &'static [Point],
}

#[derive(Template)]
#[template(path = "not_found.html")]
struct NotFoundPage {
    head: Head,
}

/// The whole site, prepared.
pub struct Site {
    pub home: Prepared,
    pub features: Prepared,
    pub screenshots: Prepared,
    pub download: Prepared,
    pub faq: Prepared,
    pub privacy: Prepared,
    pub about: Prepared,
    /// `/docs` is the empty slug.
    pub docs: HashMap<&'static str, Prepared>,
    pub not_found: Prepared,
    pub sitemap: Prepared,
}

fn html(t: &impl Template) -> Prepared {
    Prepared::html(StatusCode::OK, t.render().expect("templates render"))
}

impl Site {
    pub fn build(site_url: &'static str, integrations: Integrations, released: bool) -> Site {
        let head = |title: &str, description: &str, path: &str, nav: &'static str| Head {
            title: title.to_string(),
            description: description.to_string(),
            path: path.to_string(),
            nav,
            site_url,
            index: true,
            json_ld: None,
            integrations,
        };
        let org = r#"{"@type":"Organization","name":"Oddbench","url":"https://oddbench.co/"}"#;
        let app = |description: &str| {
            let os: Vec<&str> = PLATFORMS.iter().map(|p| p.name).collect();
            format!(
                r#"{{"@type":"SoftwareApplication","@id":"{site_url}/#app","name":"JellyBridge","url":"{site_url}/","description":{},"applicationCategory":"MultimediaApplication","operatingSystem":{},"softwareVersion":"{}","license":"https://www.gnu.org/licenses/gpl-3.0.html","isAccessibleForFree":true,"offers":{{"@type":"Offer","price":"0","priceCurrency":"EUR"}},"downloadUrl":"{site_url}/download","screenshot":{},"codeRepository":"{}","publisher":{org}}}"#,
                js(description),
                js(&os.join(", ")),
                content::VERSION,
                js(&format!("{site_url}{}", screen("home", 1280))),
                content::REPO,
            )
        };
        let breadcrumbs = |trail: &[(&str, &str)]| {
            let items: Vec<String> = trail
                .iter()
                .enumerate()
                .map(|(i, (name, path))| format!(r#"{{"@type":"ListItem","position":{},"name":{},"item":"{site_url}{path}"}}"#, i + 1, js(name)))
                .collect();
            format!(
                r#"{{"@type":"BreadcrumbList","itemListElement":[{}]}}"#,
                items.join(",")
            )
        };

        let mut h = head(
            "JellyBridge: native desktop client for Jellyfin",
            "JellyBridge is a fast, free and open source Jellyfin desktop app for Linux, Windows and macOS. The familiar \
             interface, with direct play through mpv.",
            "/",
            "home",
        );
        h.json_ld = Some(format!(
            r#"{{"@context":"https://schema.org","@graph":[{},{{"@type":"WebSite","name":"JellyBridge","url":"{site_url}/","publisher":{org}}}]}}"#,
            app(&h.description)
        ));
        let home = html(&HomePage {
            head: h,
            highlights: HIGHLIGHTS,
            features: FEATURES,
            platforms: PLATFORMS,
        });

        let features = html(&FeaturesPage {
            head: head(
                "Features: direct play, native speed | JellyBridge",
                "Direct play with mpv and hardware decoding, launch in about 100 ms, ~120 MB of memory, the familiar \
                 Jellyfin interface, no telemetry. See what JellyBridge does.",
                "/features",
                "features",
            ),
            features: FEATURES,
            more: FEATURE_LIST,
        });

        let screenshots = html(&ScreenshotsPage {
            head: head(
                "Screenshots of the JellyBridge app",
                "See JellyBridge, the native Jellyfin desktop client: the Home screen, library grids, item pages and the \
                 mpv player, in the familiar jellyfin-web design.",
                "/screenshots",
                "screenshots",
            ),
            shots: SCREENSHOTS,
        });

        let mut h = head(
            "Download JellyBridge for Linux, Windows and macOS",
            "Download JellyBridge, the native Jellyfin client: .deb, .rpm, AppImage, Arch, Windows installer and \
             portable zip, and signed macOS builds. Free and open source.",
            "/download",
            "download",
        );
        h.json_ld = Some(format!(
            r#"{{"@context":"https://schema.org","@graph":[{},{}]}}"#,
            app(&h.description),
            breadcrumbs(&[("JellyBridge", "/"), ("Download", "/download")])
        ));
        let platforms = PLATFORMS
            .iter()
            .map(|p| PlatformView {
                p,
                links: p
                    .packages
                    .iter()
                    .map(|pkg| Link {
                        label: pkg.label,
                        note: pkg.note,
                        href: if released {
                            format!(
                                "{}/download/v{}/{}",
                                content::RELEASES,
                                content::VERSION,
                                pkg.file.replace("{v}", content::VERSION)
                            )
                        } else {
                            content::RELEASES.into()
                        },
                    })
                    .collect(),
            })
            .collect();
        let download = html(&DownloadPage {
            head: h,
            version: content::VERSION,
            released,
            platforms,
            requirements: REQUIREMENTS,
        });

        let mut h = head(
            "JellyBridge FAQ: questions and answers",
            "Answers about JellyBridge: how it differs from Jellyfin Media Player, direct play, supported platforms, \
             privacy, settings, pricing and the first release.",
            "/faq",
            "none",
        );
        let questions: Vec<String> = FAQS
            .iter()
            .map(|f| format!(r#"{{"@type":"Question","name":{},"acceptedAnswer":{{"@type":"Answer","text":{}}}}}"#, js(f.question), js(&text(f.answer))))
            .collect();
        h.json_ld = Some(format!(
            r#"{{"@context":"https://schema.org","@type":"FAQPage","mainEntity":[{}]}}"#,
            questions.join(",")
        ));
        let faq = html(&FaqPage {
            head: h,
            faqs: FAQS,
        });

        let privacy = html(&PrivacyPage {
            head: head(
                "Privacy: your data stays with you | JellyBridge",
                "JellyBridge sends no telemetry, talks only to your Jellyfin server and keeps your access token away from \
                 the player. Open source, so you can check it.",
                "/privacy",
                "none",
            ),
            points: PRIVACY,
        });

        let about = html(&AboutPage {
            head: head(
                "About JellyBridge: built for open media",
                "JellyBridge is an independent, open source project by Oddbench that gives Jellyfin users a fast, \
                 lightweight and beautiful native desktop app.",
                "/about",
                "none",
            ),
            points: ABOUT,
        });

        // The sidebar is the same on every doc page; leak it once so pages can borrow it for 'static.
        let groups: &'static [NavGroup] = Group::ALL
            .iter()
            .map(|g| NavGroup {
                label: g.label(),
                docs: DOCS.iter().filter(|d| d.group == *g).collect(),
            })
            .collect::<Vec<_>>()
            .leak();
        let order: Vec<&'static Doc> = groups.iter().flat_map(|g| g.docs.iter().copied()).collect();
        let docs = order
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let title = if d.slug.is_empty() {
                    d.seo_title.to_string()
                } else {
                    format!("{} | Docs", d.seo_title)
                };
                let mut h = head(&title, d.description, &d.path(), "docs");
                let mut trail = vec![
                    ("JellyBridge", "/".to_string()),
                    ("Docs", "/docs".to_string()),
                ];
                if !d.slug.is_empty() {
                    trail.push((d.title, d.path()));
                }
                let trail: Vec<(&str, &str)> =
                    trail.iter().map(|(n, p)| (*n, p.as_str())).collect();
                h.json_ld = Some(format!(
                    r#"{{"@context":"https://schema.org","@graph":[{}]}}"#,
                    breadcrumbs(&trail)
                ));
                let page = DocPage {
                    head: h,
                    doc: d,
                    groups,
                    prev: i.checked_sub(1).map(|p| order[p]),
                    next: order.get(i + 1).copied(),
                };
                (d.slug, html(&page))
            })
            .collect();

        let not_found = Prepared::html(
            StatusCode::NOT_FOUND,
            NotFoundPage {
                head: Head {
                    index: false,
                    ..head(
                        "Page not found | JellyBridge",
                        "This page does not exist.",
                        "/",
                        "none",
                    )
                },
            }
            .render()
            .expect("templates render"),
        );

        Site {
            home,
            features,
            screenshots,
            download,
            faq,
            privacy,
            about,
            docs,
            not_found,
            sitemap: sitemap(site_url),
        }
    }
}

fn sitemap(site_url: &str) -> Prepared {
    let mut paths = vec![
        "/".to_string(),
        "/features".into(),
        "/screenshots".into(),
        "/download".into(),
        "/faq".into(),
        "/privacy".into(),
        "/about".into(),
    ];
    paths.extend(DOCS.iter().map(Doc::path));
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for p in paths {
        xml.push_str(&format!(
            "  <url><loc>{site_url}{p}</loc><lastmod>{}</lastmod></url>\n",
            content::UPDATED
        ));
    }
    xml.push_str("</urlset>\n");
    let mut prepared = Prepared::html(StatusCode::OK, xml);
    prepared.content_type = "application/xml; charset=utf-8";
    prepared
}
