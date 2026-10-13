//! A caching DNS resolver for the Jellyfin client.
//!
//! hyper's default resolver runs one `getaddrinfo` per connection and does not
//! deduplicate lookups that are already in flight. The dashboard opens six
//! connections to the same host at once, so a single launch paid the same
//! lookup six times concurrently: measured against `jellyfin.local`, one lookup is
//! ~40 ms and the contended six cost ~80 ms of a 267 ms dashboard fetch. Every
//! artwork connection afterwards paid it again.
//!
//! Caching with a short TTL rather than pinning the address keeps a server
//! whose address changes reachable within a minute, which a `resolve_to_addrs`
//! override would not.
//!
//! A known address is never waited on: once it is older than the TTL it is
//! still returned at once and refreshed in the background. The addresses are
//! also kept on disk, so a launch connects without a lookup. That matters
//! because a lookup occasionally takes the resolver's 5 s retry timeout (a
//! lost UDP reply): measured twice in 38 launches against `jellyfin.local`, 1.1 s
//! and 5.3 s, each holding the whole first screen back by that much.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// How long a resolved address is reused. Long enough that a burst of
/// connections resolves once, short enough that a moved server recovers
/// without a restart.
const TTL: Duration = Duration::from_secs(60);
/// Addresses kept on disk older than this are not trusted at launch.
const DISK_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

struct Entry {
    addrs: Vec<SocketAddr>,
    /// When it was resolved; `None` for an entry read from disk, which is
    /// always refreshed on first use.
    refreshed: Option<Instant>,
    refreshing: bool,
}

#[derive(Default)]
pub struct CachingResolver {
    // An async mutex held across a first lookup, which is what deduplicates a
    // burst: the first caller resolves and the rest wake to a warm entry. This
    // app talks to exactly one host, so serializing distinct hosts costs
    // nothing here.
    entries: tokio::sync::Mutex<HashMap<String, Entry>>,
    /// Where the addresses are kept between runs; `None` in tests.
    disk: Option<std::path::PathBuf>,
    loaded: std::sync::atomic::AtomicBool,
}

/// The on-disk form: host → (addresses, seconds since the epoch).
type Stored = HashMap<String, (Vec<SocketAddr>, u64)>;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

impl CachingResolver {
    /// Reads the addresses kept on disk, once, into the cache.
    fn load_disk(&self, entries: &mut HashMap<String, Entry>) {
        if self.loaded.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        let Some(path) = &self.disk else { return };
        // A few hundred bytes; reading it is cheaper than any lookup.
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(stored) = serde_json::from_str::<Stored>(&text) else {
            return;
        };
        let now = now_secs();
        for (host, (addrs, saved)) in stored {
            if now.saturating_sub(saved) < DISK_MAX_AGE.as_secs() && !addrs.is_empty() {
                entries.entry(host).or_insert(Entry {
                    addrs,
                    refreshed: None,
                    refreshing: false,
                });
            }
        }
    }

    fn save_disk(&self, entries: &HashMap<String, Entry>) {
        let Some(path) = self.disk.clone() else {
            return;
        };
        let now = now_secs();
        let stored: Stored = entries
            .iter()
            .map(|(host, entry)| (host.clone(), (entry.addrs.clone(), now)))
            .collect();
        let Ok(text) = serde_json::to_string(&stored) else {
            return;
        };
        tokio::task::spawn_blocking(move || {
            let temporary = path.with_extension("tmp");
            if std::fs::write(&temporary, text).is_ok() {
                let _ = std::fs::rename(&temporary, &path);
            }
        });
    }

    async fn lookup(host: &str) -> std::io::Result<Vec<SocketAddr>> {
        // Port 0: reqwest substitutes the scheme's conventional port.
        let started = Instant::now();
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 0)).await?.collect();
        tracing::debug!(
            %host,
            ms = started.elapsed().as_secs_f64() * 1000.0,
            addresses = addrs.len(),
            "dns lookup"
        );
        Ok(addrs)
    }

    /// Re-resolves `host` in the background, keeping the old addresses until
    /// the new ones arrive (or for good, if the lookup fails).
    fn refresh_later(self: Arc<Self>, host: String) {
        tokio::spawn(async move {
            let result = Self::lookup(&host).await;
            let mut entries = self.entries.lock().await;
            if let Some(entry) = entries.get_mut(&host) {
                entry.refreshing = false;
                match result {
                    Ok(addrs) if !addrs.is_empty() => {
                        entry.addrs = addrs;
                        entry.refreshed = Some(Instant::now());
                        self.save_disk(&entries);
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%host, %error, "dns refresh failed"),
                }
            }
        });
    }
}

impl Resolve for CachingResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        // The resolver is kept alive by the process-wide `shared()` handle, so
        // the future can safely outlive this call.
        let shared = shared();
        Box::pin(async move {
            let mut entries = shared.entries.lock().await;
            shared.load_disk(&mut entries);
            if let Some(entry) = entries.get_mut(&host) {
                let fresh = entry.refreshed.is_some_and(|at| at.elapsed() < TTL);
                if !fresh && !entry.refreshing {
                    entry.refreshing = true;
                    shared.clone().refresh_later(host.clone());
                }
                return Ok(Box::new(entry.addrs.clone().into_iter()) as Addrs);
            }
            let addrs = CachingResolver::lookup(&host).await?;
            entries.insert(
                host,
                Entry {
                    addrs: addrs.clone(),
                    refreshed: Some(Instant::now()),
                    refreshing: false,
                },
            );
            shared.save_disk(&entries);
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// The process-wide resolver. Shared by every client the app builds, so the
/// sign-in client's lookup warms the one the dashboard uses.
pub fn shared() -> Arc<CachingResolver> {
    static SHARED: OnceLock<Arc<CachingResolver>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            Arc::new(CachingResolver {
                disk: if cfg!(test) {
                    None
                } else {
                    // Beside the artwork cache, not in it: that one is pruned.
                    crate::services::session::cache_directory()
                        .ok()
                        .and_then(|images| images.parent().map(|cache| cache.join("dns.json")))
                },
                ..CachingResolver::default()
            })
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_second_lookup_of_the_same_host_is_served_from_the_cache() {
        let resolver = shared();
        // localhost resolves without a network, and the assertion is about the
        // cache entry rather than the addresses.
        let first: Vec<SocketAddr> = resolver
            .resolve("localhost".parse().expect("valid name"))
            .await
            .expect("localhost resolves")
            .collect();
        assert!(!first.is_empty());

        let refreshed_at = {
            let entries = resolver.entries.lock().await;
            entries.get("localhost").expect("cached").refreshed
        };
        assert!(refreshed_at.is_some());

        let second: Vec<SocketAddr> = resolver
            .resolve("localhost".parse().expect("valid name"))
            .await
            .expect("localhost resolves")
            .collect();
        assert_eq!(first, second);

        let entries = resolver.entries.lock().await;
        assert_eq!(
            entries.get("localhost").expect("cached").refreshed,
            refreshed_at,
            "the second lookup re-resolved instead of reusing the entry"
        );
    }

    #[tokio::test]
    async fn a_stale_address_is_served_at_once_and_refreshed_behind_it() {
        let resolver = Arc::new(CachingResolver::default());
        let stale: SocketAddr = "192.0.2.7:0".parse().unwrap();
        resolver
            .loaded
            .store(true, std::sync::atomic::Ordering::Release);
        resolver.entries.lock().await.insert(
            "localhost".into(),
            Entry {
                addrs: vec![stale],
                refreshed: None,
                refreshing: false,
            },
        );
        {
            let mut entries = resolver.entries.lock().await;
            let entry = entries.get_mut("localhost").unwrap();
            // What `resolve` does with an entry read from disk.
            assert!(entry.refreshed.is_none());
            entry.refreshing = true;
        }
        resolver.clone().refresh_later("localhost".into());
        for _ in 0..100 {
            if !resolver.entries.lock().await["localhost"].refreshing {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let entries = resolver.entries.lock().await;
        let entry = &entries["localhost"];
        assert!(entry.refreshed.is_some());
        assert!(!entry.addrs.contains(&stale));
    }
}
