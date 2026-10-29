//! In-process priority for the two official download routes.
use std::sync::atomic::{AtomicBool, Ordering};

use super::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    Cloudflare,
    Origin,
}

impl Route {
    fn other(self) -> Self {
        match self {
            Self::Cloudflare => Self::Origin,
            Self::Origin => Self::Cloudflare,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Cloudflare => "Cloudflare",
            Self::Origin => "源站直连",
        }
    }
}

pub(super) struct RoutePriority {
    origin_first: AtomicBool,
}

impl RoutePriority {
    pub(super) const fn new() -> Self {
        Self {
            origin_first: AtomicBool::new(false),
        }
    }

    fn preferred(&self) -> Route {
        if self.origin_first.load(Ordering::Relaxed) {
            Route::Origin
        } else {
            Route::Cloudflare
        }
    }

    fn demote(&self, route: Route) {
        // An in-flight success must not undo a more recent failure. Only
        // failures change the order; concurrent requests do not hold a lock.
        self.origin_first
            .store(route == Route::Cloudflare, Ordering::Relaxed);
    }
}

// The API and download hosts use the same two routes. Their requests, including
// concurrent background refreshes, share this preference until the app exits.
pub(super) static PRIORITY: RoutePriority = RoutePriority::new();

pub(super) fn fetch<T>(
    priority: &RoutePriority,
    can_fallback: bool,
    is_cancelled: impl Fn() -> bool,
    mut attempt: impl FnMut(Route, bool) -> Result<T>,
) -> Result<T> {
    if is_cancelled() {
        return Err(crate::cancel::MESSAGE.into());
    }
    // Other hosts must neither use nor change the official-route preference.
    let first = if can_fallback {
        priority.preferred()
    } else {
        Route::Cloudflare
    };
    let first_error = match attempt(first, false) {
        Ok(value) => return Ok(value),
        Err(_) if is_cancelled() => return Err(crate::cancel::MESSAGE.into()),
        Err(error) if !can_fallback => return Err(error),
        Err(error) => error,
    };

    let second = first.other();
    priority.demote(first);
    crate::logging::event(
        "warn",
        "download",
        format!(
            "{}线路请求失败，已降低优先级，改用{}线路：{first_error}",
            first.name(),
            second.name()
        ),
    );
    if is_cancelled() {
        return Err(crate::cancel::MESSAGE.into());
    }
    // The retry flag tells the downloader to reset its progress, regardless of
    // which route was tried first.
    match attempt(second, true) {
        Ok(value) => Ok(value),
        Err(_) if is_cancelled() => Err(crate::cancel::MESSAGE.into()),
        Err(second_error) => {
            priority.demote(second);
            crate::logging::event(
                "warn",
                "download",
                format!(
                    "{}线路请求失败，已降低优先级：{second_error}",
                    second.name()
                ),
            );
            Err(format!(
                "{}线路失败：{first_error}；{}线路失败：{second_error}",
                first.name(),
                second.name()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        collections::VecDeque,
        sync::{mpsc, Arc},
        thread,
    };

    fn attempts(
        priority: &RoutePriority,
        can_fallback: bool,
        responses: &[(Route, Result<&'static str>)],
    ) -> (Result<&'static str>, Vec<(Route, bool)>) {
        let mut responses = VecDeque::from(responses.to_vec());
        let mut seen = Vec::new();
        let result = fetch(
            priority,
            can_fallback,
            || false,
            |route, retry| {
                seen.push((route, retry));
                let (expected, response) = responses.pop_front().expect("unexpected attempt");
                assert_eq!(route, expected);
                response
            },
        );
        assert!(responses.is_empty(), "missing attempt");
        (result, seen)
    }

    #[test]
    fn failure_demotes_cloudflare_for_subsequent_requests() {
        let priority = RoutePriority::new();
        let (result, seen) = attempts(
            &priority,
            true,
            &[
                (Route::Cloudflare, Err("timeout".into())),
                (Route::Origin, Ok("downloaded")),
            ],
        );
        assert_eq!(result.unwrap(), "downloaded");
        assert_eq!(seen, [(Route::Cloudflare, false), (Route::Origin, true)]);
        assert_eq!(priority.preferred(), Route::Origin);

        let (result, seen) = attempts(&priority, true, &[(Route::Origin, Ok("next file"))]);
        assert_eq!(result.unwrap(), "next file");
        assert_eq!(seen, [(Route::Origin, false)]);
    }

    #[test]
    fn origin_failure_restores_cloudflare_as_first_choice() {
        let priority = RoutePriority::new();
        priority.demote(Route::Cloudflare);
        let (result, seen) = attempts(
            &priority,
            true,
            &[
                (Route::Origin, Err("origin unavailable".into())),
                (Route::Cloudflare, Ok("downloaded")),
            ],
        );
        assert_eq!(result.unwrap(), "downloaded");
        assert_eq!(seen, [(Route::Origin, false), (Route::Cloudflare, true)]);
        let (_, seen) = attempts(&priority, true, &[(Route::Cloudflare, Ok("next file"))]);
        assert_eq!(seen, [(Route::Cloudflare, false)]);
    }

    #[test]
    fn both_failures_are_reported_and_each_route_is_demoted() {
        let priority = RoutePriority::new();
        let (result, _) = attempts(
            &priority,
            true,
            &[
                (Route::Cloudflare, Err("cloudflare error".into())),
                (Route::Origin, Err("origin error".into())),
            ],
        );
        let error = result.unwrap_err();
        assert!(error.contains("Cloudflare线路失败：cloudflare error"));
        assert!(error.contains("源站直连线路失败：origin error"));
        assert_eq!(priority.preferred(), Route::Cloudflare);
    }

    #[test]
    fn unrelated_hosts_do_not_use_or_change_the_preference() {
        let priority = RoutePriority::new();
        priority.demote(Route::Cloudflare);
        let (result, seen) = attempts(
            &priority,
            false,
            &[(Route::Cloudflare, Err("unrelated error".into()))],
        );
        assert_eq!(result.unwrap_err(), "unrelated error");
        assert_eq!(seen, [(Route::Cloudflare, false)]);
        assert_eq!(priority.preferred(), Route::Origin);
    }

    #[test]
    fn cancellation_does_not_demote_the_cancelled_attempt() {
        let priority = RoutePriority::new();
        let cancelled = Cell::new(false);
        let mut seen = Vec::new();
        let result: Result<()> = fetch(
            &priority,
            true,
            || cancelled.get(),
            |route, retry| {
                seen.push((route, retry));
                cancelled.set(true);
                Err("interrupted".into())
            },
        );
        assert_eq!(result.unwrap_err(), crate::cancel::MESSAGE);
        assert_eq!(seen, [(Route::Cloudflare, false)]);
        assert_eq!(priority.preferred(), Route::Cloudflare);

        cancelled.set(false);
        seen.clear();
        let result: Result<()> = fetch(
            &priority,
            true,
            || cancelled.get(),
            |route, retry| {
                seen.push((route, retry));
                if retry {
                    cancelled.set(true);
                }
                Err("interrupted".into())
            },
        );
        assert_eq!(result.unwrap_err(), crate::cancel::MESSAGE);
        assert_eq!(seen, [(Route::Cloudflare, false), (Route::Origin, true)]);
        assert_eq!(priority.preferred(), Route::Origin);
    }

    #[test]
    fn already_cancelled_request_never_starts_a_route() {
        let priority = RoutePriority::new();
        let result: Result<()> = fetch(
            &priority,
            true,
            || true,
            |_, _| panic!("a cancelled request must not start"),
        );
        assert_eq!(result.unwrap_err(), crate::cancel::MESSAGE);
        assert_eq!(priority.preferred(), Route::Cloudflare);
    }

    #[test]
    fn success_from_an_older_request_does_not_undo_a_failure() {
        let priority = Arc::new(RoutePriority::new());
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let old_priority = Arc::clone(&priority);
        let old_request = thread::spawn(move || {
            fetch(
                &old_priority,
                true,
                || false,
                |route, _| {
                    assert_eq!(route, Route::Cloudflare);
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                },
            )
        });
        started_rx.recv().unwrap();
        // A separate request can change the preference while the first is in
        // flight; neither request blocks the other during network I/O.
        let (result, _) = attempts(
            &priority,
            true,
            &[
                (Route::Cloudflare, Err("timeout".into())),
                (Route::Origin, Ok("downloaded")),
            ],
        );
        assert!(result.is_ok());
        release_tx.send(()).unwrap();
        assert!(old_request.join().unwrap().is_ok());
        assert_eq!(priority.preferred(), Route::Origin);
    }
}
