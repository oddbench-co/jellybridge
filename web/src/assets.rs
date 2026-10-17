//! Responses that are fully known ahead of time: static files embedded by `build.rs`, and pages
//! rendered once at startup. Both carry precompressed variants, so a request only picks bytes.

use axum::body::Bytes;
use axum::http::header::{
    ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, ETAG, IF_NONE_MATCH, VARY,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use std::io::Write as _;

pub struct Asset {
    pub path: &'static str,
    pub content_type: &'static str,
    /// Short content hash, used in URLs and as the ETag.
    pub version: &'static str,
    pub raw: &'static [u8],
    pub gzip: Option<&'static [u8]>,
    pub br: Option<&'static [u8]>,
}

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

pub fn find(path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|a| a.path == path)
}

/// Cache-busting URL for templates: `/static/css/site.css?v=1a2b3c4d5e6f`.
pub fn url(path: &str) -> String {
    match find(path) {
        Some(asset) => format!("{}?v={}", asset.path, asset.version),
        None => panic!("template references a missing asset: {path}"),
    }
}

/// A complete response body with its encodings, ready to serve many times.
pub struct Prepared {
    pub status: StatusCode,
    pub content_type: &'static str,
    pub etag: HeaderValue,
    pub raw: Bytes,
    pub gzip: Option<Bytes>,
    pub br: Option<Bytes>,
}

impl Prepared {
    pub fn asset(asset: &'static Asset) -> Self {
        Prepared {
            status: StatusCode::OK,
            content_type: asset.content_type,
            etag: HeaderValue::from_str(&format!("\"{}\"", asset.version)).expect("hex etag"),
            raw: Bytes::from_static(asset.raw),
            gzip: asset.gzip.map(Bytes::from_static),
            br: asset.br.map(Bytes::from_static),
        }
    }

    /// Compresses a rendered page once. Brotli 11 is slow but runs only at startup, on a few pages.
    pub fn html(status: StatusCode, html: String) -> Self {
        let bytes = html.into_bytes();
        let etag = format!("\"{}\"", hex(&Sha256::digest(&bytes)[..8]));

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&bytes).expect("in-memory gzip");
        let mut br = Vec::new();
        let params = brotli::enc::BrotliEncoderParams {
            quality: 11,
            ..Default::default()
        };
        brotli::BrotliCompress(&mut &bytes[..], &mut br, &params).expect("in-memory brotli");

        Prepared {
            status,
            content_type: "text/html; charset=utf-8",
            etag: HeaderValue::from_str(&etag).expect("hex etag"),
            raw: Bytes::from(bytes),
            gzip: Some(Bytes::from(gz.finish().expect("in-memory gzip"))),
            br: Some(Bytes::from(br)),
        }
    }

    pub fn serve(&self, headers: &HeaderMap, cache: &'static str) -> Response {
        let not_modified = self.status == StatusCode::OK
            && headers
                .get(IF_NONE_MATCH)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    v.split(',').any(|t| {
                        t.trim().trim_start_matches("W/").as_bytes() == self.etag.as_bytes()
                    })
                });

        let mut response = if not_modified {
            StatusCode::NOT_MODIFIED.into_response()
        } else {
            let accepts = headers
                .get(ACCEPT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let accepts = |enc: &str| {
                accepts
                    .split(',')
                    .any(|e| e.split(';').next().is_some_and(|n| n.trim() == enc))
            };
            let (body, encoding) = match (&self.br, &self.gzip) {
                (Some(br), _) if accepts("br") => (br.clone(), Some("br")),
                (_, Some(gz)) if accepts("gzip") => (gz.clone(), Some("gzip")),
                _ => (self.raw.clone(), None),
            };
            let mut r = (self.status, body).into_response();
            r.headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static(self.content_type));
            if let Some(enc) = encoding {
                r.headers_mut()
                    .insert(CONTENT_ENCODING, HeaderValue::from_static(enc));
            }
            r
        };
        let h = response.headers_mut();
        h.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
        h.insert(ETAG, self.etag.clone());
        if self.gzip.is_some() {
            h.insert(VARY, HeaderValue::from_static("Accept-Encoding"));
        }
        response
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
