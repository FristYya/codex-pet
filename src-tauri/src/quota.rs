use crate::codex_adapter::{AdapterError, ServerNotification};
use serde::Serialize;
use serde_json::Value;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{Receiver, RecvTimeoutError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const QUOTA_UPDATED_EVENT: &str = "quota://updated";
const NOTIFICATION_DEBOUNCE: Duration = Duration::from_millis(800);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    pub id: String,
    pub name: String,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub window_duration_mins: Option<f64>,
    pub resets_at: Option<f64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshot {
    pub availability: String,
    pub windows: Vec<QuotaWindow>,
    pub plan_type: Option<String>,
    pub fetched_at: i64,
    pub stale: bool,
    pub message: Option<String>,
}

pub(crate) fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn as_number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64)
}

pub(crate) fn snapshot_from_response(response: &Value) -> QuotaSnapshot {
    let bucket = response
        .get("rateLimitsByLimitId")
        .and_then(|v| v.get("codex"))
        .or_else(|| response.get("rateLimits"));
    let mut windows = Vec::new();
    for (id, key) in [("primary", "primary"), ("secondary", "secondary")] {
        if let Some(window) = bucket.and_then(|v| v.get(key)).and_then(Value::as_object) {
            let used = as_number(window.get("usedPercent"))
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            let duration = as_number(window.get("windowDurationMins"));
            let name = match duration {
                Some(10080.0) => "Weekly".to_string(),
                Some(value) if value > 0.0 && value % 1440.0 == 0.0 => {
                    format!("{}D", value / 1440.0)
                }
                Some(value) if value > 0.0 && value % 60.0 == 0.0 => format!("{}H", value / 60.0),
                Some(value) => format!("{}m", value),
                None => "Unknown".to_string(),
            };
            windows.push(QuotaWindow {
                id: id.into(),
                name,
                used_percent: used,
                remaining_percent: 100.0 - used,
                window_duration_mins: duration,
                resets_at: as_number(window.get("resetsAt")),
            });
        }
    }
    let allowed = response
        .get("ordinaryUsageAllowed")
        .or_else(|| bucket.and_then(|v| v.get("ordinaryUsageAllowed")));
    let availability = match allowed.and_then(Value::as_bool) {
        Some(true) => "allowed",
        Some(false) => "blocked",
        None => "unknown",
    }
    .into();
    QuotaSnapshot {
        availability,
        windows,
        plan_type: response
            .get("planType")
            .or_else(|| bucket.and_then(|v| v.get("planType")))
            .and_then(Value::as_str)
            .map(str::to_owned),
        fetched_at: now_seconds(),
        stale: false,
        message: None,
    }
}

#[derive(Default)]
struct Flight {
    result: Mutex<Option<QuotaSnapshot>>,
    completed: Condvar,
}

#[derive(Default)]
struct RefreshState {
    active: Option<Arc<Flight>>,
    event_pending: bool,
    last_success: Option<QuotaSnapshot>,
}

type Reader = dyn Fn() -> Result<Value, AdapterError> + Send + Sync;
type Emitter = dyn Fn(&str, &QuotaSnapshot) + Send + Sync;

struct RefreshInner {
    state: Mutex<RefreshState>,
    read: Box<Reader>,
    emit: Box<Emitter>,
}

pub struct QuotaRefreshCoordinator {
    inner: Arc<RefreshInner>,
}

impl QuotaRefreshCoordinator {
    /// The reader may block on RPC. The emitter must return promptly and must not
    /// synchronously call `refresh` on this coordinator.
    pub fn new(
        read: impl Fn() -> Result<Value, AdapterError> + Send + Sync + 'static,
        emit: impl Fn(&str, &QuotaSnapshot) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(RefreshInner {
                state: Mutex::new(RefreshState::default()),
                read: Box::new(read),
                emit: Box::new(emit),
            }),
        }
    }

    /// Polls and manual calls share the exact result of the active read.
    pub fn refresh(&self) -> QuotaSnapshot {
        let flight = self.request(false);
        let result = flight.result.lock().expect("flight mutex poisoned");
        flight
            .completed
            .wait_while(result, |result| result.is_none())
            .expect("flight mutex poisoned")
            .as_ref()
            .expect("completed flight has a result")
            .clone()
    }

    /// Called after debouncing. Does not block the notification receiver on RPC.
    pub fn request_notification_refresh(&self) {
        self.request(true);
    }

    fn request(&self, event: bool) -> Arc<Flight> {
        let mut state = self.inner.state.lock().expect("refresh mutex poisoned");
        if let Some(flight) = state.active.clone() {
            state.event_pending |= event;
            return flight;
        }
        let flight = Arc::new(Flight::default());
        state.active = Some(Arc::clone(&flight));
        drop(state);
        let inner = Arc::clone(&self.inner);
        let worker_flight = Arc::clone(&flight);
        thread::spawn(move || run_refreshes(inner, worker_flight));
        flight
    }
}

fn run_refreshes(inner: Arc<RefreshInner>, mut flight: Arc<Flight>) {
    loop {
        // Neither potentially blocking callback runs under the state lock.
        let result = (inner.read)();
        let snapshot = {
            let mut state = inner.state.lock().expect("refresh mutex poisoned");
            match result {
                Ok(value) => {
                    let snapshot = snapshot_from_response(&value);
                    state.last_success = Some(snapshot.clone());
                    snapshot
                }
                Err(error) => match state.last_success.clone() {
                    Some(mut snapshot) => {
                        snapshot.stale = true;
                        snapshot.message = Some(format!("更新失败：{error}"));
                        snapshot
                    }
                    None => QuotaSnapshot {
                        availability: "unavailable".into(),
                        windows: vec![],
                        plan_type: None,
                        fetched_at: now_seconds(),
                        stale: false,
                        message: Some(format!("额度暂不可用：{error}")),
                    },
                },
            }
        };
        (inner.emit)(QUOTA_UPDATED_EVENT, &snapshot);
        let next = {
            let mut state = inner.state.lock().expect("refresh mutex poisoned");
            let next = if std::mem::take(&mut state.event_pending) {
                Some(Arc::new(Flight::default()))
            } else {
                None
            };
            state.active = next.clone();
            *flight.result.lock().expect("flight mutex poisoned") = Some(snapshot);
            flight.completed.notify_all();
            next
        };
        match next {
            Some(next) => flight = next,
            None => return,
        }
    }
}

pub fn spawn_notification_bridge(
    notifications: Receiver<ServerNotification>,
    coordinator: Arc<QuotaRefreshCoordinator>,
) -> JoinHandle<()> {
    // The coordinator's reader owns the client sending these notifications.
    // Keeping only a weak reference lets application state release that client.
    let coordinator = Arc::downgrade(&coordinator);
    thread::spawn(move || {
        let mut deadline: Option<Instant> = None;
        loop {
            let received = match deadline {
                Some(at) => {
                    let remaining = at.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        let Some(coordinator) = coordinator.upgrade() else {
                            return;
                        };
                        coordinator.request_notification_refresh();
                        deadline = None;
                        continue;
                    }
                    notifications.recv_timeout(remaining)
                }
                None => notifications
                    .recv()
                    .map_err(|_| RecvTimeoutError::Disconnected),
            };
            match received {
                Ok(notification) if notification.method == "account/rateLimits/updated" => {
                    deadline = Some(Instant::now() + NOTIFICATION_DEBOUNCE);
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    let Some(coordinator) = coordinator.upgrade() else {
                        return;
                    };
                    coordinator.request_notification_refresh();
                    deadline = None;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        sync::{
            Barrier, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    const TIMEOUT: Duration = Duration::from_secs(3);

    fn response(used: f64) -> Value {
        json!({
            "rateLimits": {"primary": {"usedPercent": 99}},
            "rateLimitsByLimitId": {"codex": {
                "primary": {"usedPercent": used, "windowDurationMins": 300, "resetsAt": 1900000000},
                "secondary": {"usedPercent": 42, "windowDurationMins": 10080, "resetsAt": 1900600000},
                "planType": "pro", "ordinaryUsageAllowed": true
            }}
        })
    }

    fn notification() -> ServerNotification {
        ServerNotification {
            method: "account/rateLimits/updated".into(),
            params: json!({"usedPercent": 99}),
        }
    }

    #[test]
    fn concurrent_refreshes_share_one_read_and_result() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let count = Arc::clone(&calls);
        let coordinator = Arc::new(QuotaRefreshCoordinator::new(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                entered_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv_timeout(TIMEOUT).unwrap();
                Ok(response(25.0))
            },
            |_, _| {},
        ));
        let barrier = Arc::new(Barrier::new(9));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let coordinator = Arc::clone(&coordinator);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    coordinator.refresh()
                })
            })
            .collect();
        barrier.wait();
        entered_rx.recv_timeout(TIMEOUT).unwrap();
        // Wait until all eight callers own the active flight. Its other two
        // owners are RefreshState and the worker; no production test hook is needed.
        let joined_deadline = Instant::now() + TIMEOUT;
        loop {
            let all_joined = {
                let state = coordinator.inner.state.lock().unwrap();
                state
                    .active
                    .as_ref()
                    .is_some_and(|flight| Arc::strong_count(flight) == 10)
            };
            if all_joined {
                break;
            }
            assert!(
                Instant::now() < joined_deadline,
                "all refresh callers must join the active flight"
            );
            thread::yield_now();
        }
        release_tx.send(()).unwrap();
        let snapshots: Vec<_> = handles
            .into_iter()
            .map(|handle| serde_json::to_value(handle.join().unwrap()).unwrap())
            .collect();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(snapshots.iter().all(|snapshot| snapshot == &snapshots[0]));
    }

    #[test]
    fn events_during_read_queue_only_one_follow_up_without_overlap() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (emitted_tx, emitted_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let calls = Arc::new(AtomicUsize::new(0));
        let active_reads = Arc::new(AtomicUsize::new(0));
        let max_active_reads = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let active_count = Arc::clone(&active_reads);
        let max_active_count = Arc::clone(&max_active_reads);
        let coordinator = Arc::new(QuotaRefreshCoordinator::new(
            move || {
                // Measure entry before the test's receiver mutex can serialize reads.
                let active = active_count.fetch_add(1, Ordering::SeqCst) + 1;
                max_active_count.fetch_max(active, Ordering::SeqCst);
                let call = count.fetch_add(1, Ordering::SeqCst) + 1;
                entered_tx.send(call).unwrap();
                release_rx.lock().unwrap().recv_timeout(TIMEOUT).unwrap();
                let result = Ok(response(call as f64));
                active_count.fetch_sub(1, Ordering::SeqCst);
                result
            },
            move |_, snapshot| {
                emitted_tx
                    .send(serde_json::to_value(snapshot).unwrap())
                    .unwrap();
            },
        ));
        let caller = Arc::clone(&coordinator);
        let first = thread::spawn(move || caller.refresh());
        assert_eq!(entered_rx.recv_timeout(TIMEOUT).unwrap(), 1);
        for _ in 0..10 {
            coordinator.request_notification_refresh();
        }
        assert!(entered_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        assert_eq!(entered_rx.recv_timeout(TIMEOUT).unwrap(), 2);
        assert_eq!(first.join().unwrap().windows[0].used_percent, 1.0);
        release_tx.send(()).unwrap();
        emitted_rx.recv_timeout(TIMEOUT).unwrap();
        emitted_rx.recv_timeout(TIMEOUT).unwrap();
        assert!(entered_rx.recv_timeout(Duration::from_millis(100)).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(max_active_reads.load(Ordering::SeqCst), 1);
        assert_eq!(active_reads.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn success_emits_complete_snapshot_with_fixed_event_name() {
        let (tx, rx) = mpsc::channel();
        let coordinator = QuotaRefreshCoordinator::new(
            || Ok(response(25.0)),
            move |name, snapshot| {
                tx.send((name.to_owned(), serde_json::to_value(snapshot).unwrap()))
                    .unwrap();
            },
        );
        let snapshot = coordinator.refresh();
        let (event, payload) = rx.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(event, "quota://updated");
        assert_eq!(payload, serde_json::to_value(&snapshot).unwrap());
        assert_eq!(payload["availability"], "allowed");
        assert_eq!(payload["planType"], "pro");
        assert_eq!(payload["windows"].as_array().unwrap().len(), 2);
        assert_eq!(payload["windows"][0]["usedPercent"], 25.0);
        assert_eq!(payload["windows"][0]["remainingPercent"], 75.0);
        assert_eq!(payload["windows"][0]["name"], "5H");
        assert_eq!(payload["windows"][1]["name"], "Weekly");
        assert_eq!(payload["stale"], false);
        assert!(payload["fetchedAt"].as_i64().unwrap() > 0);
        assert!(payload["message"].is_null());
    }

    #[test]
    fn failure_emits_last_success_as_stale_and_recovery_clears_error() {
        let calls = AtomicUsize::new(0);
        let (tx, rx) = mpsc::channel();
        let coordinator = QuotaRefreshCoordinator::new(
            move || match calls.fetch_add(1, Ordering::SeqCst) {
                0 => Ok(response(25.0)),
                1 | 2 => Err(AdapterError::UnexpectedChildExit),
                _ => Ok(response(30.0)),
            },
            move |_, snapshot| {
                tx.send(serde_json::to_value(snapshot).unwrap()).unwrap();
            },
        );
        let good = serde_json::to_value(coordinator.refresh()).unwrap();
        rx.recv_timeout(TIMEOUT).unwrap();
        for _ in 0..2 {
            let stale = serde_json::to_value(coordinator.refresh()).unwrap();
            assert_eq!(stale["windows"], good["windows"]);
            assert_eq!(stale["fetchedAt"], good["fetchedAt"]);
            assert_eq!(stale["availability"], good["availability"]);
            assert_eq!(stale["planType"], good["planType"]);
            assert_eq!(stale["stale"], true);
            assert!(stale["message"].as_str().unwrap().contains("更新失败"));
            assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), stale);
        }
        let recovered = serde_json::to_value(coordinator.refresh()).unwrap();
        assert_eq!(recovered["windows"][0]["usedPercent"], 30.0);
        assert_eq!(recovered["stale"], false);
        assert!(recovered["message"].is_null());
    }

    #[test]
    fn initial_failure_emits_unavailable_snapshot() {
        let (tx, rx) = mpsc::channel();
        let coordinator = QuotaRefreshCoordinator::new(
            || Err(AdapterError::ExecutableNotFound),
            move |_, snapshot| {
                tx.send(serde_json::to_value(snapshot).unwrap()).unwrap();
            },
        );
        let snapshot = serde_json::to_value(coordinator.refresh()).unwrap();
        assert_eq!(snapshot["availability"], "unavailable");
        assert!(snapshot["windows"].as_array().unwrap().is_empty());
        assert!(snapshot["message"].is_string());
        assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), snapshot);
    }

    #[test]
    fn three_notifications_debounce_for_800ms_and_ignore_notification_payload() {
        let (notifications_tx, notifications_rx) = mpsc::channel();
        let (emitted_tx, emitted_rx) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let coordinator = Arc::new(QuotaRefreshCoordinator::new(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(response(25.0))
            },
            move |_, snapshot| {
                emitted_tx
                    .send(serde_json::to_value(snapshot).unwrap())
                    .unwrap();
            },
        ));
        let bridge = spawn_notification_bridge(notifications_rx, Arc::clone(&coordinator));
        for _ in 0..2 {
            notifications_tx.send(notification()).unwrap();
            thread::sleep(Duration::from_millis(120));
        }
        let last_sent = Instant::now();
        notifications_tx.send(notification()).unwrap();
        assert!(emitted_rx.recv_timeout(Duration::from_millis(700)).is_err());
        let snapshot = emitted_rx.recv_timeout(TIMEOUT).unwrap();
        assert!(last_sent.elapsed() >= Duration::from_millis(800));
        assert_eq!(snapshot["windows"][0]["usedPercent"], 25.0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(notifications_tx);
        bridge.join().unwrap();
    }

    #[test]
    fn unrelated_notifications_do_not_read_and_disconnect_exits() {
        let (tx, rx) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let coordinator = Arc::new(QuotaRefreshCoordinator::new(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(response(25.0))
            },
            |_, _| {},
        ));
        let bridge = spawn_notification_bridge(rx, Arc::clone(&coordinator));
        tx.send(ServerNotification {
            method: "account/updated".into(),
            params: json!({}),
        })
        .unwrap();
        thread::sleep(Duration::from_millis(900));
        drop(tx);
        bridge.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn notification_bridge_does_not_keep_its_client_owner_alive() {
        let (tx, rx) = mpsc::channel();
        let coordinator = Arc::new(QuotaRefreshCoordinator::new(
            || Ok(response(25.0)),
            |_, _| {},
        ));
        let owner = Arc::downgrade(&coordinator);
        let bridge = spawn_notification_bridge(rx, Arc::clone(&coordinator));
        drop(coordinator);
        let kept_alive = owner.upgrade().is_some();
        drop(tx);
        bridge.join().unwrap();
        assert!(
            !kept_alive,
            "the bridge must not retain the coordinator that owns its client"
        );
    }
}
