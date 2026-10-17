//! jellybridge.oddbench.co: the JellyBridge website.
//!
//! All content is compiled in and every page is rendered and compressed once at startup, so a
//! request is a hash lookup plus a header match. The only script is a small same-origin
//! enhancement (`static/js/site.js`); every page works without it.

mod assets;
mod content;
mod pages;

use assets::{ASSETS, Prepared};
use axum::Router;
use axum::extract::{Path, RawQuery, State};
use axum::http::header::{
    CONTENT_SECURITY_POLICY, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{Redirect, Response};
use axum::routing::get;
use pages::{Integrations, Site};
use std::sync::Arc;

/// HTML changes only on deploy: let browsers and the CDN reuse it briefly, then revalidate by ETag.
const PAGE_CACHE: &str = "public, max-age=300, stale-while-revalidate=86400";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// Unversioned asset URLs (old links, crawlers) are revalidated hourly.
const ASSET_CACHE: &str = "public, max-age=3600";

/// Same-origin scripts, styles, images and fonts only; no inline code, nothing from other origins.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// The same, plus the Ahrefs Web Analytics script and the beacon it sends. Still no inline scripts.
const CSP_ANALYTICS: &str = "default-src 'none'; script-src 'self' https://analytics.ahrefs.com; connect-src https://analytics.ahrefs.com; style-src 'self'; img-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn csp(integrations: &Integrations) -> &'static str {
    if integrations.ahrefs_analytics.is_some() {
        CSP_ANALYTICS
    } else {
        CSP
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let site_url =
        std::env::var("SITE_URL").unwrap_or_else(|_| "https://jellybridge.oddbench.co".into());
    let site_url: &'static str = site_url.trim_end_matches('/').to_string().leak();
    let addr = format!(
        "{}:{}",
        std::env::var("BIND").unwrap_or_else(|_| "0.0.0.0".into()),
        std::env::var("PORT").unwrap_or_else(|_| "8080".into())
    );

    // Empty or unset means "no tag" (local runs, previews).
    let token = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .map(|v| &*v.leak())
    };
    let integrations = Integrations {
        google: token("GOOGLE_SITE_VERIFICATION"),
        ahrefs: token("AHREFS_SITE_VERIFICATION"),
        ahrefs_analytics: token("AHREFS_ANALYTICS_KEY"),
    };

    let started = std::time::Instant::now();
    let app = app(site_url, integrations, content::RELEASED);
    tracing::info!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        "site prepared"
    );

    // A busy port is an everyday mistake, not a crash: say what happened and how to choose another.
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            tracing::error!(
                "{addr} is already in use: another program (or another copy of the site) is listening there. Stop it, or pick a free port with PORT=8081"
            );
            std::process::exit(1);
        }
        Err(e) => {
            tracing::error!("cannot listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!(%addr, %site_url, "listening");
    eprintln!(
        "Serving the JellyBridge website at http://localhost:{} (Ctrl+C to stop)",
        listener.local_addr().map(|a| a.port()).unwrap_or_default()
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
        .expect("server");
}

type S = State<Arc<Site>>;

fn app(site_url: &'static str, integrations: Integrations, released: bool) -> Router {
    let site = Arc::new(Site::build(site_url, integrations, released));
    let csp = csp(&integrations);

    let mut router = Router::new()
        .route(
            "/",
            get(|State(s): S, h: HeaderMap| async move { s.home.serve(&h, PAGE_CACHE) }),
        )
        .route(
            "/features",
            get(|State(s): S, h: HeaderMap| async move { s.features.serve(&h, PAGE_CACHE) }),
        )
        .route(
            "/screenshots",
            get(|State(s): S, h: HeaderMap| async move { s.screenshots.serve(&h, PAGE_CACHE) }),
        )
        .route(
            "/download",
            get(|State(s): S, h: HeaderMap| async move { s.download.serve(&h, PAGE_CACHE) }),
        )
        .route(
            "/faq",
            get(|State(s): S, h: HeaderMap| async move { s.faq.serve(&h, PAGE_CACHE) }),
        )
        .route(
            "/privacy",
            get(|State(s): S, h: HeaderMap| async move { s.privacy.serve(&h, PAGE_CACHE) }),
        )
        .route(
            "/about",
            get(|State(s): S, h: HeaderMap| async move { s.about.serve(&h, PAGE_CACHE) }),
        )
        .route(
            "/docs",
            get(|State(s): S, h: HeaderMap| async move { s.docs[""].serve(&h, PAGE_CACHE) }),
        )
        .route("/docs/", get(|| async { Redirect::permanent("/docs") }))
        .route("/docs/{slug}", get(doc))
        .route(
            "/sitemap.xml",
            get(|State(s): S, h: HeaderMap| async move { s.sitemap.serve(&h, PAGE_CACHE) }),
        )
        .route(
            "/robots.txt",
            get(move || async move {
                (
                    [("content-type", "text/plain; charset=utf-8")],
                    format!("User-agent: *\nAllow: /\nSitemap: {site_url}/sitemap.xml\n"),
                )
            }),
        )
        .route("/healthz", get(|| async { "ok" }))
        .fallback(|State(s): S, h: HeaderMap| async move { s.not_found.serve(&h, PAGE_CACHE) });

    for asset in ASSETS {
        let prepared = Arc::new(Prepared::asset(asset));
        router = router.route(
            asset.path,
            get(move |h: HeaderMap, q: RawQuery| async move {
                let versioned = q.0.is_some_and(|q| {
                    q.split('&')
                        .any(|p| p.strip_prefix("v=") == Some(asset.version))
                });
                prepared.serve(&h, if versioned { IMMUTABLE } else { ASSET_CACHE })
            }),
        );
    }
    let favicon = Arc::new(Prepared::asset(
        assets::find("/static/favicon.svg").expect("favicon embedded"),
    ));
    router = router.route(
        "/favicon.ico",
        get(move |h: HeaderMap| async move { favicon.serve(&h, ASSET_CACHE) }),
    );

    router
        .with_state(site)
        .layer(axum::middleware::map_response(move |r: Response| {
            security_headers(r, csp)
        }))
}

async fn doc(State(s): S, Path(slug): Path<String>, h: HeaderMap) -> Response {
    // The overview lives at /docs, not at an empty slug.
    let page = if slug.is_empty() {
        None
    } else {
        s.docs.get(slug.as_str())
    };
    page.unwrap_or(&s.not_found).serve(&h, PAGE_CACHE)
}

async fn security_headers(mut response: Response, csp: &'static str) -> Response {
    let h = response.headers_mut();
    h.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp));
    h.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    response
}

async fn shutdown() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.expect("ctrl-c handler") };
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler")
            .recv()
            .await;
    };
    tokio::select! { () = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const SITE: &str = "https://jellybridge.oddbench.co";

    async fn get(
        app: &Router,
        uri: &str,
        headers: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, String) {
        let mut req = Request::get(uri);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (parts, body) = res.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        (
            parts.status,
            parts.headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    fn test_app() -> Router {
        app(
            SITE,
            Integrations {
                google: Some("g-token"),
                ahrefs: Some("a-token"),
                ahrefs_analytics: Some("k/e+y"),
            },
            false,
        )
    }

    fn indexed() -> Vec<String> {
        let mut paths: Vec<String> = [
            "/",
            "/features",
            "/screenshots",
            "/download",
            "/faq",
            "/privacy",
            "/about",
        ]
        .map(String::from)
        .to_vec();
        paths.extend(content::DOCS.iter().map(content::Doc::path));
        paths
    }

    #[tokio::test]
    async fn pages_render() {
        let app = test_app();
        for (uri, needle) in [
            ("/", "The native desktop client for Jellyfin"),
            ("/features", "Built for a better Jellyfin experience"),
            ("/screenshots", "The familiar interface"),
            ("/download", "Get JellyBridge"),
            ("/docs", "Learn and get started"),
            ("/docs/keyboard-shortcuts", "Next audio track"),
            ("/faq", "Frequently asked questions"),
            ("/privacy", "Your data stays with you"),
            ("/about", "Built by people who love open media"),
        ] {
            let (status, headers, body) = get(&app, uri, &[]).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(headers["content-type"], "text/html; charset=utf-8");
            assert!(body.contains(needle), "{uri} should contain {needle:?}");
            assert_eq!(body.matches("<h1").count(), 1, "{uri}: one h1");
        }
    }

    #[tokio::test]
    async fn missing_things_are_404_with_a_way_back() {
        let app = test_app();
        for uri in ["/nope", "/docs/nope", "/docs/installation/x"] {
            let (status, _, body) = get(&app, uri, &[]).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert!(body.contains("href=\"/\""), "{uri} offers a way back");
        }
    }

    #[tokio::test]
    async fn only_the_current_doc_is_current() {
        let (_, _, body) = get(&test_app(), "/docs/playback", &[]).await;
        assert_eq!(
            body.matches("aria-current=\"page\"").count(),
            2,
            "nav item + one sidebar entry"
        );
        assert!(body.contains("href=\"/docs/playback\" aria-current=\"page\""));
    }

    #[tokio::test]
    async fn download_links_follow_the_release_state() {
        let (_, _, before) = get(&test_app(), "/download", &[]).await;
        assert!(
            !before.contains("/releases/download/"),
            "no file links before the release"
        );
        assert!(before.contains("Pre-release"));

        let released = app(SITE, Integrations::default(), true);
        let (_, _, after) = get(&released, "/download", &[]).await;
        for p in content::PLATFORMS {
            for pkg in p.packages {
                let file = pkg.file.replace("{v}", content::VERSION);
                assert!(
                    after.contains(&format!(
                        "/releases/download/v{}/{file}\"",
                        content::VERSION
                    )),
                    "links {file}"
                );
            }
        }
        assert!(!after.contains("Pre-release"));
    }

    #[tokio::test]
    async fn every_internal_link_and_asset_resolves() {
        let app = test_app();
        let mut pages = indexed();
        pages.push("/nope".into());
        let mut checked = std::collections::HashSet::new();
        for page in &pages {
            let (_, _, body) = get(&app, page, &[]).await;
            for attr in ["href=\"", "src=\"", "srcset=\""] {
                for part in body.split(attr).skip(1) {
                    let value = part.split('"').next().unwrap().replace("&amp;", "&");
                    // A srcset lists several "url width" pairs.
                    for link in value
                        .split(", ")
                        .map(|c| c.split(' ').next().unwrap().to_string())
                    {
                        if !link.starts_with('/') || !checked.insert(link.clone()) {
                            continue;
                        }
                        let path = link.split('#').next().unwrap();
                        let (status, _, _) =
                            get(&app, if path.is_empty() { "/" } else { path }, &[]).await;
                        assert_eq!(status, StatusCode::OK, "{page} links to {link}");
                    }
                }
            }
        }
        assert!(checked.len() >= 30, "crawled {} links", checked.len());
    }

    #[tokio::test]
    async fn css_references_versioned_assets() {
        let (status, headers, css) =
            get(&test_app(), &assets::url("/static/css/site.css"), &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["cache-control"], IMMUTABLE);
        assert!(
            css.contains("url(/static/fonts/inter-var.woff2?v="),
            "font url is versioned"
        );
    }

    #[tokio::test]
    async fn compression_and_revalidation() {
        let app = test_app();
        let (_, headers, _) = get(&app, "/", &[("accept-encoding", "gzip, br")]).await;
        assert_eq!(headers["content-encoding"], "br");
        let etag = headers["etag"].to_str().unwrap().to_string();
        let (status, _, body) = get(&app, "/", &[("if-none-match", &etag)]).await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);
        assert!(body.is_empty());
    }

    fn between<'a>(html: &'a str, start: &str, end: &str) -> Option<&'a str> {
        let from = html.find(start)? + start.len();
        Some(&html[from..from + html[from..].find(end)?])
    }

    #[tokio::test]
    async fn search_metadata() {
        let app = test_app();
        let mut titles = std::collections::HashSet::new();
        for path in indexed() {
            let (_, headers, html) = get(&app, &path, &[]).await;
            let title = between(&html, "<title>", "</title>").unwrap();
            let description = between(&html, "name=\"description\" content=\"", "\"").unwrap();
            assert!(
                title.chars().count() <= 60,
                "{path}: title is {} characters",
                title.chars().count()
            );
            assert!(
                titles.insert(title.to_string()),
                "{path}: title {title:?} is not unique"
            );
            assert!(
                (110..=165).contains(&description.chars().count()),
                "{path}: description is {} characters",
                description.chars().count()
            );
            assert!(
                html.contains(&format!("<link rel=\"canonical\" href=\"{SITE}{path}\">")),
                "{path}: canonical"
            );
            assert!(!html.contains("noindex"), "{path}: indexable");
            assert!(
                html.contains(&format!(
                    "property=\"og:image\" content=\"{SITE}/static/img/og.jpg?v="
                )),
                "{path}: og:image"
            );
            if let Some(ld) = between(&html, "<script type=\"application/ld+json\">", "</script>") {
                serde_json::from_str::<serde_json::Value>(ld)
                    .unwrap_or_else(|e| panic!("{path}: JSON-LD: {e}"));
            }
            assert!(
                !headers["content-security-policy"]
                    .to_str()
                    .unwrap()
                    .contains("unsafe"),
                "{path}: strict CSP"
            );
            assert!(
                !html.contains(" style=\""),
                "{path}: no inline styles (the CSP blocks them)"
            );
        }
        let (_, _, html) = get(&app, "/nope", &[]).await;
        assert!(html.contains("<meta name=\"robots\" content=\"noindex, follow\">"));
        assert!(!html.contains("rel=\"canonical\""));

        // Ownership tags render everywhere, including noindex pages.
        for path in ["/", "/nope", "/docs/playback"] {
            let (_, _, html) = get(&app, path, &[]).await;
            assert!(
                html.contains(r#"<meta name="google-site-verification" content="g-token">"#),
                "{path}: google tag"
            );
            assert!(
                html.contains(r#"<meta name="ahrefs-site-verification" content="a-token">"#),
                "{path}: ahrefs tag"
            );
        }
        let plain = super::app(SITE, Integrations::default(), false);
        let (_, headers, html) = get(&plain, "/", &[]).await;
        assert!(
            !html.contains("site-verification"),
            "unset tokens render no tag"
        );
        assert!(
            !html.contains("analytics.ahrefs.com"),
            "unset analytics key renders no script"
        );
        assert_eq!(headers["content-security-policy"], CSP);

        let (_, headers, html) = get(&app, "/", &[]).await;
        assert!(
            html.contains("https://analytics.ahrefs.com/analytics.js"),
            "analytics script"
        );
        assert!(
            html.contains("no cookies, no personal data"),
            "analytics is disclosed"
        );
        assert_eq!(headers["content-security-policy"], CSP_ANALYTICS);

        let (_, _, faq) = get(&app, "/faq", &[]).await;
        let ld: serde_json::Value = serde_json::from_str(
            between(&faq, "<script type=\"application/ld+json\">", "</script>").unwrap(),
        )
        .unwrap();
        assert_eq!(
            ld["mainEntity"].as_array().unwrap().len(),
            content::FAQS.len()
        );
        assert!(
            !ld.to_string().contains("<a "),
            "FAQ answers are plain text"
        );

        let (_, _, sitemap) = get(&app, "/sitemap.xml", &[]).await;
        assert_eq!(sitemap.matches("<url>").count(), indexed().len());
    }
}
