//! Build-time asset pipeline.
//!
//! Every file under `static/` is embedded into the binary together with gzip and brotli variants
//! computed here, once, so the server never compresses assets at runtime and needs no files on disk.
//! A short content hash per file is emitted for cache-busting URLs (`?v=<hash>`), which lets
//! browsers and the CDN cache assets as immutable.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=static");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let assets_dir = out.join("assets");
    fs::create_dir_all(&assets_dir).unwrap();

    let mut files: Vec<(String, PathBuf)> = walk(Path::new("static"))
        .into_iter()
        .map(|p| {
            (
                format!(
                    "/static/{}",
                    p.strip_prefix("static")
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                ),
                p,
            )
        })
        .collect();
    files.sort();

    // Hashes of everything except stylesheets first, so CSS `url(...)`s can point at versioned files.
    let mut versions: Vec<(String, String)> = Vec::new();
    for (public, path) in &files {
        if !public.ends_with(".css") {
            versions.push((
                public.clone(),
                hex(&Sha256::digest(fs::read(path).unwrap())[..6]),
            ));
        }
    }

    let mut table = String::from("pub static ASSETS: &[Asset] = &[\n");
    for (i, (public, path)) in files.iter().enumerate() {
        let (bytes, raw) = if public.ends_with(".css") {
            let mut css = fs::read_to_string(path).unwrap();
            for (asset, version) in &versions {
                css = css.replace(
                    &format!("url({asset})"),
                    &format!("url({asset}?v={version})"),
                );
            }
            let raw = assets_dir.join(format!("{i}.css"));
            fs::write(&raw, &css).unwrap();
            (css.into_bytes(), raw)
        } else {
            // Absolute path into the source tree: no copy needed for the uncompressed variant.
            (fs::read(path).unwrap(), fs::canonicalize(path).unwrap())
        };
        let hash = hex(&Sha256::digest(&bytes)[..6]);

        // Only text formats shrink; woff2, webp and png are already compressed.
        let (gzip, br) = if compressible(public) {
            let gz = assets_dir.join(format!("{i}.gz"));
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            enc.write_all(&bytes).unwrap();
            fs::write(&gz, enc.finish().unwrap()).unwrap();

            let brp = assets_dir.join(format!("{i}.br"));
            let mut br_out = Vec::new();
            let params = brotli::enc::BrotliEncoderParams {
                quality: 11,
                ..Default::default()
            };
            brotli::BrotliCompress(&mut &bytes[..], &mut br_out, &params).unwrap();
            fs::write(&brp, br_out).unwrap();
            (
                format!("Some(include_bytes!({:?}))", gz.display().to_string()),
                format!("Some(include_bytes!({:?}))", brp.display().to_string()),
            )
        } else {
            ("None".into(), "None".into())
        };

        writeln!(
            table,
            "    Asset {{ path: {public:?}, content_type: {ct:?}, version: {hash:?}, raw: include_bytes!({raw:?}), gzip: {gzip}, br: {br} }},",
            ct = content_type(public),
            raw = raw.display().to_string(),
        )
        .unwrap();
    }
    table.push_str("];\n");
    fs::write(out.join("assets.rs"), table).unwrap();
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn compressible(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("css" | "js" | "svg" | "txt" | "ico")
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
