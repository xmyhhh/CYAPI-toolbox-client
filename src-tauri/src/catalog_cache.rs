//! Persistent two-hour refresh window for the server catalogs.
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const TTL_MILLIS: u64 = 2 * 60 * 60 * 1000;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn record_at(data: &Path, cache: &str, time: u64) {
    let path = data.join(format!("{cache}.checked-at"));
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, time.to_string());
}

/// A 304 response renews the window even though the catalog file stays untouched.
pub(crate) fn checked(data: &Path, cache: &str) {
    record_at(data, cache, now());
}

fn claim_at(data: &Path, cache: &str, valid: bool, force: bool, time: u64) -> bool {
    let last_check = fs::read_to_string(data.join(format!("{cache}.checked-at")))
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .or_else(|| {
            // Existing installations have no timestamp yet. Keep the remaining
            // lifetime of their downloaded catalog instead of fetching again.
            fs::metadata(data.join(cache))
                .ok()?
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|age| age.as_millis() as u64)
        });
    let fresh = last_check
        .and_then(|last| time.checked_sub(last))
        .is_some_and(|age| age < TTL_MILLIS);
    if valid && !force && fresh {
        return false;
    }
    // Reserve the window before network I/O. Concurrent page reads and failed
    // automatic attempts must not repeatedly request an existing stale cache.
    record_at(data, cache, time);
    true
}

/// Call while holding the service lock, then fetch outside the lock.
/// Missing/invalid caches and explicit manual refreshes always fetch.
pub(crate) fn claim(data: &Path, cache: &str, valid: bool, force: bool) -> bool {
    claim_at(data, cache, valid, force, now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshes_once_after_two_hours_and_manual_refresh_bypasses_the_window() {
        let root = std::env::temp_dir().join(format!("cyapi-catalog-ttl-{}", uuid::Uuid::new_v4()));
        let cache = "integrations/catalog.json";
        let start = 10 * TTL_MILLIS;
        record_at(&root, cache, start);
        assert!(!claim_at(&root, cache, true, false, start + TTL_MILLIS - 1));
        assert!(claim_at(&root, cache, true, false, start + TTL_MILLIS));
        // A second read (including after a failed attempt or restart) reuses it.
        assert!(!claim_at(&root, cache, true, false, start + TTL_MILLIS + 1));
        assert!(claim_at(&root, cache, true, true, start + TTL_MILLIS + 2));
        assert!(claim_at(&root, cache, false, false, start + TTL_MILLIS + 3));
        // Catalogs have independent refresh windows.
        assert!(claim_at(&root, "mcp/catalog.json", true, false, start));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uses_existing_file_age_and_recovers_from_future_timestamps() {
        let root = std::env::temp_dir().join(format!("cyapi-catalog-age-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let cache = "catalog.json";
        fs::write(root.join(cache), b"cached catalog").unwrap();
        let modified = fs::metadata(root.join(cache))
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(!claim_at(
            &root,
            cache,
            true,
            false,
            modified + TTL_MILLIS - 1
        ));
        assert!(claim_at(&root, cache, true, false, modified + TTL_MILLIS));
        // Rolling the system clock back must not suppress refresh indefinitely.
        assert!(claim_at(&root, cache, true, false, modified));
        fs::remove_dir_all(root).unwrap();
    }
}
