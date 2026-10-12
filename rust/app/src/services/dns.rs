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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// How long a resolved address is reused. Long enough that a burst of
/// connections resolves once, short enough that a moved server recovers
/// without a restart.
const TTL: Duration = Duration::from_secs(60);

struct Entry {
    addrs: Vec<SocketAddr>,
    refreshed: Instant,
}

#[derive(Default)]
pub struct CachingResolver {
    // An async mutex held across the lookup, which is what deduplicates a
    // burst: the first caller resolves and the rest wake to a warm entry. This
    // app talks to exactly one host, so serializing distinct hosts costs
    // nothing here.
    entries: tokio::sync::Mutex<HashMap<String, Entry>>,
}

impl Resolve for CachingResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        // The resolver is kept alive by the process-wide `shared()` handle, so
        // the future can safely outlive this call.
        let shared = shared();
        Box::pin(async move {
            let mut entries = shared.entries.lock().await;
            if let Some(entry) = entries.get(&host)
                && entry.refreshed.elapsed() < TTL
            {
                return Ok(Box::new(entry.addrs.clone().into_iter()) as Addrs);
            }
            // Port 0: reqwest substitutes the scheme's conventional port.
            let started = Instant::now();
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .collect();
            tracing::debug!(
                %host,
                ms = started.elapsed().as_secs_f64() * 1000.0,
                addresses = addrs.len(),
                "dns lookup"
            );
            entries.insert(
                host,
                Entry {
                    addrs: addrs.clone(),
                    refreshed: Instant::now(),
                },
            );
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// The process-wide resolver. Shared by every client the app builds, so the
/// sign-in client's lookup warms the one the dashboard uses.
pub fn shared() -> Arc<CachingResolver> {
    static SHARED: OnceLock<Arc<CachingResolver>> = OnceLock::new();
    SHARED.get_or_init(|| Arc::new(CachingResolver::default())).clone()
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
}
