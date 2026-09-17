use serde_json::{Value, json};
use std::{
    collections::HashMap,
    env,
    ffi::OsStr,
    fmt,
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const APP_SERVER_ARGS: [&str; 2] = ["app-server", "--stdio"];
const VERSION_ARG: &str = "--version";

#[derive(Debug, Clone, PartialEq)]
pub enum AdapterError {
    ExecutableNotFound,
    VersionCheckFailed {
        reason: String,
    },
    SpawnFailed {
        reason: String,
    },
    Io {
        operation: &'static str,
        reason: String,
    },
    Protocol {
        reason: String,
    },
    MalformedJson,
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    Timeout {
        method: String,
    },
    UnexpectedChildExit,
    StderrFlooding {
        limit: usize,
    },
    AlreadyShutdown,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExecutableNotFound => write!(
                formatter,
                "could not find a trusted codex executable on PATH; install Codex CLI or add codex.exe to an absolute PATH directory"
            ),
            Self::VersionCheckFailed { reason } => {
                write!(formatter, "codex --version verification failed: {reason}")
            }
            Self::SpawnFailed { reason } => {
                write!(formatter, "failed to start codex app-server: {reason}")
            }
            Self::Io { operation, reason } => write!(formatter, "{operation} failed: {reason}"),
            Self::Protocol { reason } => write!(formatter, "JSON-RPC protocol error: {reason}"),
            Self::MalformedJson => write!(formatter, "Codex App Server emitted malformed JSON"),
            Self::Rpc { code, message, .. } => {
                write!(formatter, "JSON-RPC error {code}: {message}")
            }
            Self::Timeout { method } => write!(formatter, "JSON-RPC request timed out: {method}"),
            Self::UnexpectedChildExit => write!(formatter, "Codex App Server exited unexpectedly"),
            Self::StderrFlooding { limit } => write!(
                formatter,
                "Codex App Server stderr exceeded the in-memory {limit}-byte safety limit"
            ),
            Self::AlreadyShutdown => {
                write!(formatter, "Codex App Server client is already shut down")
            }
        }
    }
}

impl std::error::Error for AdapterError {}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerNotification {
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub request_timeout: Duration,
    pub shutdown_grace: Duration,
    pub max_stderr_bytes: usize,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(5),
            shutdown_grace: Duration::from_millis(500),
            max_stderr_bytes: 64 * 1024,
        }
    }
}

trait ManagedChild: Send {
    fn take_stdin(&mut self) -> io::Result<Box<dyn Write + Send>>;
    fn take_stdout(&mut self) -> io::Result<Box<dyn Read + Send>>;
    fn take_stderr(&mut self) -> io::Result<Box<dyn Read + Send>>;
    fn try_wait(&mut self) -> io::Result<Option<i32>>;
    fn wait(&mut self) -> io::Result<i32>;
    fn kill(&mut self) -> io::Result<()>;
}

struct SystemChild {
    child: Child,
}

impl ManagedChild for SystemChild {
    fn take_stdin(&mut self) -> io::Result<Box<dyn Write + Send>> {
        self.child
            .stdin
            .take()
            .map(|stream| Box::new(stream) as Box<dyn Write + Send>)
            .ok_or_else(|| io::Error::other("child stdin was not piped"))
    }

    fn take_stdout(&mut self) -> io::Result<Box<dyn Read + Send>> {
        self.child
            .stdout
            .take()
            .map(|stream| Box::new(stream) as Box<dyn Read + Send>)
            .ok_or_else(|| io::Error::other("child stdout was not piped"))
    }

    fn take_stderr(&mut self) -> io::Result<Box<dyn Read + Send>> {
        self.child
            .stderr
            .take()
            .map(|stream| Box::new(stream) as Box<dyn Read + Send>)
            .ok_or_else(|| io::Error::other("child stderr was not piped"))
    }

    fn try_wait(&mut self) -> io::Result<Option<i32>> {
        Ok(self
            .child
            .try_wait()?
            .map(|status| status.code().unwrap_or(-1)))
    }

    fn wait(&mut self) -> io::Result<i32> {
        Ok(self.child.wait()?.code().unwrap_or(-1))
    }

    fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }
}

type ResponseSender = Sender<Result<Value, AdapterError>>;

enum OutboundMessage {
    Write {
        bytes: Vec<u8>,
        completion: Sender<Result<(), AdapterError>>,
    },
    Shutdown,
}

struct ClientState {
    pending: Mutex<HashMap<u64, ResponseSender>>,
    fatal: Mutex<Option<AdapterError>>,
    notification_sender: Mutex<Option<Sender<ServerNotification>>>,
    stderr: Mutex<Vec<u8>>,
    shutdown: AtomicBool,
}

struct BackgroundThread {
    handle: Option<JoinHandle<()>>,
    finished: Receiver<()>,
}

type ReaperResources = (
    Option<Box<dyn ManagedChild>>,
    Option<BackgroundThread>,
    Option<BackgroundThread>,
    Option<BackgroundThread>,
);

static REAPER_SENDER: OnceLock<Sender<ReaperResources>> = OnceLock::new();

fn ensure_reaper() -> io::Result<&'static Sender<ReaperResources>> {
    if let Some(sender) = REAPER_SENDER.get() {
        return Ok(sender);
    }
    let (sender, receiver) = mpsc::channel::<ReaperResources>();
    thread::Builder::new()
        .name("codex-app-server-reaper".into())
        .spawn(move || {
            while let Ok((child, writer, stdout, stderr)) = receiver.recv() {
                reap_resources(child, writer, stdout, stderr);
            }
        })?;
    let _ = REAPER_SENDER.set(sender);
    Ok(REAPER_SENDER.get().expect("reaper sender initialized"))
}

impl BackgroundThread {
    fn spawn(name: &str, task: impl FnOnce() + Send + 'static) -> io::Result<Self> {
        let (finished_tx, finished) = mpsc::channel();
        let handle = thread::Builder::new().name(name.into()).spawn(move || {
            task();
            let _ = finished_tx.send(());
        })?;
        Ok(Self {
            handle: Some(handle),
            finished,
        })
    }

    fn join_until(&mut self, deadline: Instant) -> bool {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match self.finished.recv_timeout(remaining) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => self
                .handle
                .take()
                .is_none_or(|handle| handle.join().is_ok()),
            Err(mpsc::RecvTimeoutError::Timeout) => false,
        }
    }
}

impl ClientState {
    fn register_pending(&self, id: u64, sender: ResponseSender) -> Result<(), AdapterError> {
        let fatal = self.fatal.lock().expect("fatal mutex poisoned");
        if let Some(error) = fatal.as_ref() {
            return Err(error.clone());
        }
        self.pending
            .lock()
            .expect("pending mutex poisoned")
            .insert(id, sender);
        Ok(())
    }

    fn fail(&self, error: AdapterError) {
        let should_broadcast = {
            let mut fatal = self.fatal.lock().expect("fatal mutex poisoned");
            if fatal.is_some() {
                false
            } else {
                *fatal = Some(error.clone());
                true
            }
        };
        if should_broadcast {
            let pending = {
                let mut pending = self.pending.lock().expect("pending mutex poisoned");
                pending
                    .drain()
                    .map(|(_, sender)| sender)
                    .collect::<Vec<_>>()
            };
            for sender in pending {
                let _ = sender.send(Err(error.clone()));
            }
        }
    }

    fn fatal_error(&self) -> Option<AdapterError> {
        self.fatal.lock().expect("fatal mutex poisoned").clone()
    }
}

pub struct CodexClient {
    state: Arc<ClientState>,
    outbound: SyncSender<OutboundMessage>,
    child: Mutex<Option<Box<dyn ManagedChild>>>,
    writer_thread: Mutex<Option<BackgroundThread>>,
    stdout_thread: Mutex<Option<BackgroundThread>>,
    stderr_thread: Mutex<Option<BackgroundThread>>,
    notification_receiver: Mutex<Option<Receiver<ServerNotification>>>,
    shutdown_lock: Mutex<()>,
    next_id: AtomicU64,
    options: ClientOptions,
}

impl CodexClient {
    pub fn connect(options: ClientOptions) -> Result<Self, AdapterError> {
        let executable = locate_codex()?;
        verify_codex(&executable, options.request_timeout)?;
        let child = spawn_app_server(&executable)?;
        Self::start_with_child(Box::new(child), options)
    }

    fn start_with_child(
        mut child: Box<dyn ManagedChild>,
        options: ClientOptions,
    ) -> Result<Self, AdapterError> {
        if let Err(error) = ensure_reaper() {
            abort_startup(child.as_mut(), options.shutdown_grace);
            return Err(AdapterError::Io {
                operation: "start resource reaper",
                reason: error.to_string(),
            });
        }
        let writer = match child.take_stdin() {
            Ok(writer) => writer,
            Err(error) => {
                abort_startup(child.as_mut(), options.shutdown_grace);
                return Err(AdapterError::Io {
                    operation: "take child stdin",
                    reason: error.to_string(),
                });
            }
        };
        let stdout = match child.take_stdout() {
            Ok(stdout) => stdout,
            Err(error) => {
                abort_startup(child.as_mut(), options.shutdown_grace);
                return Err(AdapterError::Io {
                    operation: "take child stdout",
                    reason: error.to_string(),
                });
            }
        };
        let stderr = match child.take_stderr() {
            Ok(stderr) => stderr,
            Err(error) => {
                abort_startup(child.as_mut(), options.shutdown_grace);
                return Err(AdapterError::Io {
                    operation: "take child stderr",
                    reason: error.to_string(),
                });
            }
        };
        let (notification_sender, notification_receiver) = mpsc::channel();
        let state = Arc::new(ClientState {
            pending: Mutex::new(HashMap::new()),
            fatal: Mutex::new(None),
            notification_sender: Mutex::new(Some(notification_sender)),
            stderr: Mutex::new(Vec::with_capacity(options.max_stderr_bytes)),
            shutdown: AtomicBool::new(false),
        });

        let (outbound, outbound_receiver) = mpsc::sync_channel(32);
        let writer_state = Arc::clone(&state);
        let writer_thread = match BackgroundThread::spawn("codex-app-server-stdin", move || {
            write_stdin(writer, outbound_receiver, writer_state)
        }) {
            Ok(handle) => handle,
            Err(error) => {
                abort_startup(child.as_mut(), options.shutdown_grace);
                return Err(AdapterError::Io {
                    operation: "start stdin writer",
                    reason: error.to_string(),
                });
            }
        };

        let stdout_state = Arc::clone(&state);
        let stdout_thread = match BackgroundThread::spawn("codex-app-server-stdout", move || {
            read_stdout(stdout, stdout_state)
        }) {
            Ok(handle) => handle,
            Err(error) => {
                abort_startup(child.as_mut(), options.shutdown_grace);
                let _ = outbound.try_send(OutboundMessage::Shutdown);
                let mut writer_thread = writer_thread;
                let _ = writer_thread.join_until(Instant::now() + options.shutdown_grace);
                return Err(AdapterError::Io {
                    operation: "start stdout reader",
                    reason: error.to_string(),
                });
            }
        };
        let stderr_state = Arc::clone(&state);
        let stderr_limit = options.max_stderr_bytes;
        let stderr_thread = match BackgroundThread::spawn("codex-app-server-stderr", move || {
            read_stderr(stderr, stderr_state, stderr_limit)
        }) {
            Ok(handle) => handle,
            Err(error) => {
                abort_startup(child.as_mut(), options.shutdown_grace);
                let _ = outbound.try_send(OutboundMessage::Shutdown);
                let deadline = Instant::now() + options.shutdown_grace;
                let mut writer_thread = writer_thread;
                let mut stdout_thread = stdout_thread;
                let _ = writer_thread.join_until(deadline);
                let _ = stdout_thread.join_until(deadline);
                return Err(AdapterError::Io {
                    operation: "start stderr reader",
                    reason: error.to_string(),
                });
            }
        };

        let client = Self {
            state,
            outbound,
            child: Mutex::new(Some(child)),
            writer_thread: Mutex::new(Some(writer_thread)),
            stdout_thread: Mutex::new(Some(stdout_thread)),
            stderr_thread: Mutex::new(Some(stderr_thread)),
            notification_receiver: Mutex::new(Some(notification_receiver)),
            shutdown_lock: Mutex::new(()),
            next_id: AtomicU64::new(1),
            options,
        };

        let initialize_result = client.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "codex-pet",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {}
            }),
        );
        if let Err(error) = initialize_result {
            let _ = client.shutdown();
            return Err(error);
        }
        if let Err(error) = client.notify("initialized", json!({})) {
            let _ = client.shutdown();
            return Err(error);
        }
        Ok(client)
    }

    pub fn read_rate_limits(&self) -> Result<Value, AdapterError> {
        self.request("account/rateLimits/read", json!({}))
    }

    pub fn take_notification_receiver(&self) -> Option<Receiver<ServerNotification>> {
        self.notification_receiver
            .lock()
            .expect("notification receiver mutex poisoned")
            .take()
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, AdapterError> {
        if self.state.shutdown.load(Ordering::Acquire) {
            return Err(AdapterError::AlreadyShutdown);
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let deadline = Instant::now() + self.options.request_timeout;
        let (sender, receiver) = mpsc::channel();
        self.state.register_pending(id, sender)?;

        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(error) = self.write_message_until(&message, deadline, method) {
            self.state
                .pending
                .lock()
                .expect("pending mutex poisoned")
                .remove(&id);
            return Err(error);
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.state
                    .pending
                    .lock()
                    .expect("pending mutex poisoned")
                    .remove(&id);
                Err(AdapterError::Timeout {
                    method: method.to_owned(),
                })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(self
                .state
                .fatal_error()
                .unwrap_or(AdapterError::UnexpectedChildExit)),
        }
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), AdapterError> {
        self.write_message_until(
            &json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
            }),
            Instant::now() + self.options.request_timeout,
            method,
        )
    }

    fn write_message_until(
        &self,
        message: &Value,
        deadline: Instant,
        method: &str,
    ) -> Result<(), AdapterError> {
        let mut bytes = serde_json::to_vec(message).map_err(|error| AdapterError::Protocol {
            reason: format!("failed to serialize request: {error}"),
        })?;
        bytes.push(b'\n');
        let (completion, written) = mpsc::channel();
        let mut outbound = OutboundMessage::Write { bytes, completion };
        loop {
            if self.state.shutdown.load(Ordering::Acquire) {
                return Err(AdapterError::AlreadyShutdown);
            }
            if let Some(error) = self.state.fatal_error() {
                return Err(error);
            }
            match self.outbound.try_send(outbound) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) if Instant::now() < deadline => {
                    outbound = returned;
                    thread::sleep(Duration::from_millis(2));
                }
                Err(TrySendError::Full(_)) => {
                    return Err(AdapterError::Timeout {
                        method: method.to_owned(),
                    });
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(self
                        .state
                        .fatal_error()
                        .unwrap_or(AdapterError::UnexpectedChildExit));
                }
            }
        }

        loop {
            if self.state.shutdown.load(Ordering::Acquire) {
                return Err(AdapterError::AlreadyShutdown);
            }
            if let Some(error) = self.state.fatal_error() {
                return Err(error);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AdapterError::Timeout {
                    method: method.to_owned(),
                });
            }
            match written.recv_timeout(remaining.min(Duration::from_millis(5))) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(self
                        .state
                        .fatal_error()
                        .unwrap_or(AdapterError::UnexpectedChildExit));
                }
            }
        }
    }

    pub fn shutdown(&self) -> Result<(), AdapterError> {
        let _shutdown_guard = self.shutdown_lock.lock().expect("shutdown mutex poisoned");
        let first_shutdown = !self.state.shutdown.swap(true, Ordering::AcqRel);
        if first_shutdown {
            self.state.fail(AdapterError::AlreadyShutdown);
            let _ = self.outbound.try_send(OutboundMessage::Shutdown);
            self.state
                .notification_sender
                .lock()
                .expect("notification sender mutex poisoned")
                .take();
        }

        let started = Instant::now();
        let deadline = started + self.options.shutdown_grace;
        let graceful_deadline = started + self.options.shutdown_grace / 2;
        let mut first_error = None;
        let mut child = self.child.lock().expect("child mutex poisoned").take();
        let mut exited = child.is_none();
        if let Some(child_ref) = child.as_mut() {
            loop {
                match child_ref.try_wait() {
                    Ok(Some(_)) => {
                        exited = true;
                        break;
                    }
                    Ok(None) if Instant::now() < graceful_deadline => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Ok(None) => break,
                    Err(error) => {
                        first_error = Some(AdapterError::Io {
                            operation: "poll child exit",
                            reason: error.to_string(),
                        });
                        break;
                    }
                }
            }
            if !exited {
                match child_ref.kill() {
                    Ok(()) => {
                        while Instant::now() < deadline {
                            match child_ref.try_wait() {
                                Ok(Some(_)) => {
                                    exited = true;
                                    break;
                                }
                                Ok(None) => thread::sleep(Duration::from_millis(2)),
                                Err(error) => {
                                    first_error.get_or_insert_with(|| AdapterError::Io {
                                        operation: "poll killed child exit",
                                        reason: error.to_string(),
                                    });
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        first_error.get_or_insert_with(|| AdapterError::Io {
                            operation: "kill child",
                            reason: error.to_string(),
                        });
                    }
                }
            }
            if exited && let Err(error) = child_ref.wait() {
                first_error.get_or_insert_with(|| AdapterError::Io {
                    operation: "wait for child",
                    reason: error.to_string(),
                });
            }
        }
        if child.is_some() && !exited {
            first_error.get_or_insert_with(|| AdapterError::Io {
                operation: "reap child",
                reason: "child did not exit before the shutdown deadline".into(),
            });
            *self.child.lock().expect("child mutex poisoned") = child.take();
        }
        drop(child);

        let mut writer_slot = self
            .writer_thread
            .lock()
            .expect("writer thread mutex poisoned");
        if let Some(mut handle) = writer_slot.take()
            && !handle.join_until(deadline)
        {
            *writer_slot = Some(handle);
            first_error.get_or_insert_with(|| AdapterError::Io {
                operation: "join stdin writer",
                reason: "stdin writer did not exit before the shutdown deadline".into(),
            });
        }
        drop(writer_slot);

        let mut stdout_slot = self
            .stdout_thread
            .lock()
            .expect("stdout thread mutex poisoned");
        if let Some(mut handle) = stdout_slot.take()
            && !handle.join_until(deadline)
        {
            *stdout_slot = Some(handle);
            first_error.get_or_insert_with(|| AdapterError::Io {
                operation: "join stdout reader",
                reason: "stdout reader did not exit before the shutdown deadline".into(),
            });
        }
        drop(stdout_slot);

        let mut stderr_slot = self
            .stderr_thread
            .lock()
            .expect("stderr thread mutex poisoned");
        if let Some(mut handle) = stderr_slot.take()
            && !handle.join_until(deadline)
        {
            *stderr_slot = Some(handle);
            first_error.get_or_insert_with(|| AdapterError::Io {
                operation: "join stderr reader",
                reason: "stderr reader did not exit before the shutdown deadline".into(),
            });
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn spawn_resource_reaper(&self) {
        let child = self.child.lock().expect("child mutex poisoned").take();
        let writer = self
            .writer_thread
            .lock()
            .expect("writer thread mutex poisoned")
            .take();
        let stdout = self
            .stdout_thread
            .lock()
            .expect("stdout thread mutex poisoned")
            .take();
        let stderr = self
            .stderr_thread
            .lock()
            .expect("stderr thread mutex poisoned")
            .take();

        if child.is_none() && writer.is_none() && stdout.is_none() && stderr.is_none() {
            return;
        }

        // Drop 不能无限等待，但也不能把仍然拥有子进程和管道的句柄直接丢掉。
        // 将所有权交给有名字的后台回收线程，持续终止子进程并等待读写线程退出。
        if let Some(sender) = REAPER_SENDER.get()
            && sender.send((child, writer, stdout, stderr)).is_ok()
        {
            return;
        }
        // supervisor 只在进程级线程异常退出时才会走到这里；避免 Drop 再次无限等待。
        eprintln!("codex-app-server reaper supervisor unavailable; resources retained");
    }

    #[cfg(test)]
    fn pending_request_count(&self) -> usize {
        self.state
            .pending
            .lock()
            .expect("pending mutex poisoned")
            .len()
    }

    #[cfg(test)]
    fn retained_stderr_len(&self) -> usize {
        self.state
            .stderr
            .lock()
            .expect("stderr mutex poisoned")
            .len()
    }

    #[cfg(test)]
    fn has_child_handle(&self) -> bool {
        self.child.lock().expect("child mutex poisoned").is_some()
    }
}

fn write_stdin(
    mut writer: Box<dyn Write + Send>,
    receiver: Receiver<OutboundMessage>,
    state: Arc<ClientState>,
) {
    while let Ok(message) = receiver.recv() {
        match message {
            OutboundMessage::Write { bytes, completion } => {
                let result = writer
                    .write_all(&bytes)
                    .and_then(|_| writer.flush())
                    .map_err(|error| AdapterError::Io {
                        operation: "write JSON-RPC request",
                        reason: error.to_string(),
                    });
                let failed = result.as_ref().err().cloned();
                let _ = completion.send(result);
                if state.shutdown.load(Ordering::Acquire) {
                    return;
                }
                if let Some(error) = failed {
                    if !state.shutdown.load(Ordering::Acquire) {
                        state.fail(error);
                    }
                    return;
                }
            }
            OutboundMessage::Shutdown => return,
        }
    }
}

fn abort_startup(child: &mut dyn ManagedChild, timeout: Duration) {
    if child.kill().is_err() {
        return;
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => {
                let _ = child.wait();
                return;
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(_) => return,
        }
    }
}

impl Drop for CodexClient {
    fn drop(&mut self) {
        let _ = self.shutdown();
        // 第一次关闭可能只等到 deadline；再尝试一次，给已被 kill 的子进程和读写线程机会完成回收。
        let _ = self.shutdown();
        self.spawn_resource_reaper();
    }
}

fn reap_resources(
    mut child: Option<Box<dyn ManagedChild>>,
    mut writer: Option<BackgroundThread>,
    mut stdout: Option<BackgroundThread>,
    mut stderr: Option<BackgroundThread>,
) {
    loop {
        let mut child_exited = true;
        if let Some(child_ref) = child.as_mut() {
            child_exited = match child_ref.try_wait() {
                Ok(Some(_)) => true,
                Ok(None) => {
                    let _ = child_ref.kill();
                    false
                }
                Err(_) => {
                    let _ = child_ref.kill();
                    false
                }
            };
            if child_exited {
                let _ = child_ref.wait();
                child = None;
            }
        }

        if child_exited {
            let deadline = Instant::now() + Duration::from_millis(50);
            let writer_done = writer
                .as_mut()
                .is_none_or(|handle| handle.join_until(deadline));
            if writer_done {
                writer = None;
            }
            let stdout_done = stdout
                .as_mut()
                .is_none_or(|handle| handle.join_until(deadline));
            if stdout_done {
                stdout = None;
            }
            let stderr_done = stderr
                .as_mut()
                .is_none_or(|handle| handle.join_until(deadline));
            if stderr_done {
                stderr = None;
            }
            if writer.is_none() && stdout.is_none() && stderr.is_none() {
                return;
            }
        }

        thread::sleep(Duration::from_millis(10));
    }
}

fn read_stdout(stdout: Box<dyn Read + Send>, state: Arc<ClientState>) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                if !state.shutdown.load(Ordering::Acquire) {
                    state.fail(AdapterError::UnexpectedChildExit);
                }
                return;
            }
            Ok(_) => {
                let value = match serde_json::from_str::<Value>(&line) {
                    Ok(value) => value,
                    Err(_) => {
                        state.fail(AdapterError::MalformedJson);
                        return;
                    }
                };
                if let Err(error) = dispatch_message(&state, value) {
                    state.fail(error);
                    return;
                }
            }
            Err(error) => {
                if !state.shutdown.load(Ordering::Acquire) {
                    state.fail(AdapterError::Io {
                        operation: "read child stdout",
                        reason: error.to_string(),
                    });
                }
                return;
            }
        }
    }
}

fn dispatch_message(state: &ClientState, value: Value) -> Result<(), AdapterError> {
    if value.get("jsonrpc") != Some(&Value::String("2.0".into())) {
        return Err(AdapterError::Protocol {
            reason: "message is missing jsonrpc=2.0".into(),
        });
    }

    if let Some(id_value) = value.get("id") {
        if value.get("method").is_some() {
            return Err(AdapterError::Protocol {
                reason: "server requests with both id and method are unsupported".into(),
            });
        }
        let has_result = value.get("result").is_some();
        let has_error = value.get("error").is_some();
        if has_result == has_error {
            return Err(AdapterError::Protocol {
                reason: "response must contain exactly one of result or error".into(),
            });
        }
        let id = id_value.as_u64().ok_or_else(|| AdapterError::Protocol {
            reason: "response id must be an unsigned integer".into(),
        })?;
        let result = if let Some(error) = value.get("error") {
            let error = error.as_object().ok_or_else(|| AdapterError::Protocol {
                reason: "JSON-RPC error must be an object".into(),
            })?;
            let code = error.get("code").and_then(Value::as_i64).ok_or_else(|| {
                AdapterError::Protocol {
                    reason: "JSON-RPC error is missing an integer code".into(),
                }
            })?;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| AdapterError::Protocol {
                    reason: "JSON-RPC error is missing a message".into(),
                })?;
            Err(AdapterError::Rpc {
                code,
                message: message.to_owned(),
                data: error.get("data").cloned(),
            })
        } else {
            Ok(value
                .get("result")
                .expect("result presence validated")
                .clone())
        };
        let sender = state
            .pending
            .lock()
            .expect("pending mutex poisoned")
            .remove(&id);
        if let Some(sender) = sender {
            let _ = sender.send(result);
        }
        return Ok(());
    }

    if value.get("result").is_some() || value.get("error").is_some() {
        return Err(AdapterError::Protocol {
            reason: "response result or error is missing an id".into(),
        });
    }

    let method =
        value
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| AdapterError::Protocol {
                reason: "notification is missing a method".into(),
            })?;
    if matches!(method, "account/rateLimits/updated" | "account/updated") {
        let notification = ServerNotification {
            method: method.to_owned(),
            params: value.get("params").cloned().unwrap_or(Value::Null),
        };
        if let Some(sender) = state
            .notification_sender
            .lock()
            .expect("notification sender mutex poisoned")
            .as_ref()
        {
            let _ = sender.send(notification);
        }
    }
    Ok(())
}

fn read_stderr(mut stderr: Box<dyn Read + Send>, state: Arc<ClientState>, limit: usize) {
    let mut buffer = [0_u8; 4096];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => return,
            Ok(count) => {
                let mut retained = state.stderr.lock().expect("stderr mutex poisoned");
                let available = limit.saturating_sub(retained.len());
                retained.extend_from_slice(&buffer[..count.min(available)]);
                let flooded = count > available;
                drop(retained);
                if flooded {
                    state.fail(AdapterError::StderrFlooding { limit });
                    return;
                }
            }
            Err(error) => {
                if !state.shutdown.load(Ordering::Acquire) {
                    state.fail(AdapterError::Io {
                        operation: "read child stderr",
                        reason: error.to_string(),
                    });
                }
                return;
            }
        }
    }
}

fn locate_codex() -> Result<PathBuf, AdapterError> {
    let path = env::var_os("PATH").ok_or(AdapterError::ExecutableNotFound)?;
    find_codex_in_path(&path)
}

fn find_codex_in_path(path: &OsStr) -> Result<PathBuf, AdapterError> {
    let executable_name = if cfg!(windows) { "codex.exe" } else { "codex" };
    for directory in env::split_paths(path) {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(executable_name);
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .map_err(|_| AdapterError::ExecutableNotFound);
        }
    }
    Err(AdapterError::ExecutableNotFound)
}

#[derive(Clone, Copy)]
struct VerificationExit {
    success: bool,
    code: Option<i32>,
}

trait VerificationChild {
    fn try_wait(&mut self) -> io::Result<Option<VerificationExit>>;
    fn kill(&mut self) -> io::Result<()>;
}

struct SystemVerificationChild {
    child: Child,
}

impl VerificationChild for SystemVerificationChild {
    fn try_wait(&mut self) -> io::Result<Option<VerificationExit>> {
        Ok(self.child.try_wait()?.map(|status| VerificationExit {
            success: status.success(),
            code: status.code(),
        }))
    }

    fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }
}

fn verify_codex(executable: &Path, timeout: Duration) -> Result<(), AdapterError> {
    let child = Command::new(executable)
        .arg(VERSION_ARG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| AdapterError::VersionCheckFailed {
            reason: error.to_string(),
        })?;
    verify_spawned_codex(Box::new(SystemVerificationChild { child }), timeout)
}

fn verify_spawned_codex(
    mut child: Box<dyn VerificationChild>,
    timeout: Duration,
) -> Result<(), AdapterError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success => return Ok(()),
            Ok(Some(status)) => {
                return Err(AdapterError::VersionCheckFailed {
                    reason: format!("process exited with code {:?}", status.code),
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                terminate_verification_child(child.as_mut(), deadline);
                return Err(AdapterError::VersionCheckFailed {
                    reason: "process did not exit before the verification timeout".into(),
                });
            }
            Err(error) => {
                terminate_verification_child(child.as_mut(), deadline);
                return Err(AdapterError::VersionCheckFailed {
                    reason: error.to_string(),
                });
            }
        }
    }
}

fn terminate_verification_child(child: &mut dyn VerificationChild, deadline: Instant) {
    let _ = child.kill();
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(_) => return,
        }
    }
}

fn spawn_app_server(executable: &Path) -> Result<SystemChild, AdapterError> {
    let child = Command::new(executable)
        .args(APP_SERVER_ARGS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| AdapterError::SpawnFailed {
            reason: error.to_string(),
        })?;
    Ok(SystemChild { child })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        collections::{HashMap, VecDeque},
        ffi::OsString,
        io::{self, Read, Write},
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc::{self, Receiver, Sender},
        },
        thread,
        time::Duration,
    };

    const TEST_TIMEOUT: Duration = Duration::from_secs(1);

    #[derive(Default)]
    struct FakeProcessState {
        exited: AtomicBool,
        killed: AtomicBool,
        waited: AtomicBool,
        dropped: AtomicUsize,
    }

    struct FakeChild {
        stdin: Option<Box<dyn Write + Send>>,
        stdout: Option<Box<dyn Read + Send>>,
        stderr: Option<Box<dyn Read + Send>>,
        state: Arc<FakeProcessState>,
    }

    impl Drop for FakeChild {
        fn drop(&mut self) {
            self.state.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl ManagedChild for FakeChild {
        fn take_stdin(&mut self) -> io::Result<Box<dyn Write + Send>> {
            self.stdin
                .take()
                .ok_or_else(|| io::Error::other("stdin already taken"))
        }

        fn take_stdout(&mut self) -> io::Result<Box<dyn Read + Send>> {
            self.stdout
                .take()
                .ok_or_else(|| io::Error::other("stdout already taken"))
        }

        fn take_stderr(&mut self) -> io::Result<Box<dyn Read + Send>> {
            self.stderr
                .take()
                .ok_or_else(|| io::Error::other("stderr already taken"))
        }

        fn try_wait(&mut self) -> io::Result<Option<i32>> {
            Ok(self.state.exited.load(Ordering::SeqCst).then_some(0))
        }

        fn wait(&mut self) -> io::Result<i32> {
            self.state.waited.store(true, Ordering::SeqCst);
            self.state.exited.store(true, Ordering::SeqCst);
            Ok(0)
        }

        fn kill(&mut self) -> io::Result<()> {
            self.state.killed.store(true, Ordering::SeqCst);
            self.state.exited.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    struct LineWriter {
        tx: Sender<String>,
        buffer: Vec<u8>,
    }

    impl Write for LineWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.buffer.extend_from_slice(bytes);
            while let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line = self.buffer.drain(..=index).collect::<Vec<_>>();
                self.tx
                    .send(String::from_utf8(line).map_err(io::Error::other)?)
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "server stopped"))?;
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct BlockingWriteState {
        entered: AtomicBool,
        release: AtomicBool,
    }

    struct BlockingRateLimitWriter {
        inner: LineWriter,
        control: Arc<BlockingWriteState>,
        process: Arc<FakeProcessState>,
    }

    impl Write for BlockingRateLimitWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if String::from_utf8_lossy(bytes).contains("account/rateLimits/read") {
                self.control.entered.store(true, Ordering::SeqCst);
                while !self.control.release.load(Ordering::SeqCst)
                    && !self.process.killed.load(Ordering::SeqCst)
                {
                    thread::sleep(Duration::from_millis(2));
                }
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "blocked pipe was closed",
                ));
            }
            self.inner.write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    struct ChunkReader {
        rx: Receiver<Vec<u8>>,
        pending: VecDeque<u8>,
        state: Arc<FakeProcessState>,
    }

    #[derive(Default)]
    struct HostileShutdownState {
        release: AtomicBool,
        wait_entered: AtomicBool,
    }

    struct ReleaseGatedReader {
        rx: Receiver<Vec<u8>>,
        pending: VecDeque<u8>,
        control: Arc<HostileShutdownState>,
    }

    impl Read for ReleaseGatedReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.pending.is_empty() {
                loop {
                    match self.rx.recv_timeout(Duration::from_millis(5)) {
                        Ok(chunk) => {
                            self.pending.extend(chunk);
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
                        Err(mpsc::RecvTimeoutError::Timeout)
                            if self.control.release.load(Ordering::SeqCst) =>
                        {
                            return Ok(0);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            }
            let count = output.len().min(self.pending.len());
            for target in &mut output[..count] {
                *target = self.pending.pop_front().expect("pending length checked");
            }
            Ok(count)
        }
    }

    struct KillFailingChild {
        stdin: Option<Box<dyn Write + Send>>,
        stdout: Option<Box<dyn Read + Send>>,
        stderr: Option<Box<dyn Read + Send>>,
        control: Arc<HostileShutdownState>,
    }

    impl ManagedChild for KillFailingChild {
        fn take_stdin(&mut self) -> io::Result<Box<dyn Write + Send>> {
            self.stdin
                .take()
                .ok_or_else(|| io::Error::other("stdin already taken"))
        }

        fn take_stdout(&mut self) -> io::Result<Box<dyn Read + Send>> {
            self.stdout
                .take()
                .ok_or_else(|| io::Error::other("stdout already taken"))
        }

        fn take_stderr(&mut self) -> io::Result<Box<dyn Read + Send>> {
            self.stderr
                .take()
                .ok_or_else(|| io::Error::other("stderr already taken"))
        }

        fn try_wait(&mut self) -> io::Result<Option<i32>> {
            Ok(None)
        }

        fn wait(&mut self) -> io::Result<i32> {
            self.control.wait_entered.store(true, Ordering::SeqCst);
            while !self.control.release.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(2));
            }
            Ok(0)
        }

        fn kill(&mut self) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "kill denied",
            ))
        }
    }

    impl Read for ChunkReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.pending.is_empty() {
                loop {
                    match self.rx.recv_timeout(Duration::from_millis(5)) {
                        Ok(chunk) => {
                            self.pending.extend(chunk);
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
                        Err(mpsc::RecvTimeoutError::Timeout)
                            if self.state.exited.load(Ordering::SeqCst) =>
                        {
                            return Ok(0);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            }
            let count = output.len().min(self.pending.len());
            for target in &mut output[..count] {
                *target = self.pending.pop_front().expect("pending length checked");
            }
            Ok(count)
        }
    }

    struct FakeServer {
        requests: Receiver<String>,
        stdout: Option<Sender<Vec<u8>>>,
        stderr: Option<Sender<Vec<u8>>>,
        state: Arc<FakeProcessState>,
    }

    impl FakeServer {
        fn recv_json(&self) -> Value {
            let line = self.requests.recv_timeout(TEST_TIMEOUT).unwrap();
            assert!(line.ends_with('\n'), "JSON-RPC frame must end in newline");
            assert_eq!(line[..line.len() - 1].matches('\n').count(), 0);
            serde_json::from_str(&line).unwrap()
        }

        fn send_json(&self, value: Value) {
            let mut bytes = serde_json::to_vec(&value).unwrap();
            bytes.push(b'\n');
            self.stdout.as_ref().unwrap().send(bytes).unwrap();
        }

        fn initialize(&self) {
            let request = self.recv_json();
            assert_eq!(request["jsonrpc"], "2.0");
            assert_eq!(request["method"], "initialize");
            assert_eq!(request["id"], 1);
            self.send_json(json!({"jsonrpc":"2.0","id":1,"result":{"serverInfo":{"name":"fake"}}}));

            let notification = self.recv_json();
            assert_eq!(notification["jsonrpc"], "2.0");
            assert_eq!(notification["method"], "initialized");
            assert!(notification.get("id").is_none());
        }
    }

    impl Drop for FakeServer {
        fn drop(&mut self) {
            self.stdout.take();
            self.stderr.take();
            self.state.exited.store(true, Ordering::SeqCst);
        }
    }

    fn fake_process(
        _exit_on_stdin_close: bool,
    ) -> (Box<dyn ManagedChild>, FakeServer, Arc<FakeProcessState>) {
        let (request_tx, request_rx) = mpsc::channel();
        let (stdout_tx, stdout_rx) = mpsc::channel();
        let (stderr_tx, stderr_rx) = mpsc::channel();
        let state = Arc::new(FakeProcessState::default());
        let child = FakeChild {
            stdin: Some(Box::new(LineWriter {
                tx: request_tx,
                buffer: Vec::new(),
            })),
            stdout: Some(Box::new(ChunkReader {
                rx: stdout_rx,
                pending: VecDeque::new(),
                state: Arc::clone(&state),
            })),
            stderr: Some(Box::new(ChunkReader {
                rx: stderr_rx,
                pending: VecDeque::new(),
                state: Arc::clone(&state),
            })),
            state: Arc::clone(&state),
        };
        let server = FakeServer {
            requests: request_rx,
            stdout: Some(stdout_tx),
            stderr: Some(stderr_tx),
            state: Arc::clone(&state),
        };
        (Box::new(child), server, Arc::clone(&state))
    }

    fn blocking_writer_process() -> (
        Box<dyn ManagedChild>,
        FakeServer,
        Arc<FakeProcessState>,
        Arc<BlockingWriteState>,
    ) {
        let (request_tx, request_rx) = mpsc::channel();
        let (stdout_tx, stdout_rx) = mpsc::channel();
        let (stderr_tx, stderr_rx) = mpsc::channel();
        let state = Arc::new(FakeProcessState::default());
        let control = Arc::new(BlockingWriteState::default());
        let child = FakeChild {
            stdin: Some(Box::new(BlockingRateLimitWriter {
                inner: LineWriter {
                    tx: request_tx,
                    buffer: Vec::new(),
                },
                control: Arc::clone(&control),
                process: Arc::clone(&state),
            })),
            stdout: Some(Box::new(ChunkReader {
                rx: stdout_rx,
                pending: VecDeque::new(),
                state: Arc::clone(&state),
            })),
            stderr: Some(Box::new(ChunkReader {
                rx: stderr_rx,
                pending: VecDeque::new(),
                state: Arc::clone(&state),
            })),
            state: Arc::clone(&state),
        };
        let server = FakeServer {
            requests: request_rx,
            stdout: Some(stdout_tx),
            stderr: Some(stderr_tx),
            state: Arc::clone(&state),
        };
        (Box::new(child), server, state, control)
    }

    fn kill_failing_process() -> (Box<dyn ManagedChild>, FakeServer, Arc<HostileShutdownState>) {
        let (request_tx, request_rx) = mpsc::channel();
        let (stdout_tx, stdout_rx) = mpsc::channel();
        let control = Arc::new(HostileShutdownState::default());
        let server_state = Arc::new(FakeProcessState::default());
        let child = KillFailingChild {
            stdin: Some(Box::new(LineWriter {
                tx: request_tx,
                buffer: Vec::new(),
            })),
            stdout: Some(Box::new(ReleaseGatedReader {
                rx: stdout_rx,
                pending: VecDeque::new(),
                control: Arc::clone(&control),
            })),
            stderr: Some(Box::new(io::empty())),
            control: Arc::clone(&control),
        };
        let server = FakeServer {
            requests: request_rx,
            stdout: Some(stdout_tx),
            stderr: None,
            state: server_state,
        };
        (Box::new(child), server, control)
    }

    fn stuck_reader_process() -> (Box<dyn ManagedChild>, FakeServer, Arc<HostileShutdownState>) {
        let (request_tx, request_rx) = mpsc::channel();
        let (stdout_tx, stdout_rx) = mpsc::channel();
        let control = Arc::new(HostileShutdownState::default());
        let state = Arc::new(FakeProcessState::default());
        let child = FakeChild {
            stdin: Some(Box::new(LineWriter {
                tx: request_tx,
                buffer: Vec::new(),
            })),
            stdout: Some(Box::new(ReleaseGatedReader {
                rx: stdout_rx,
                pending: VecDeque::new(),
                control: Arc::clone(&control),
            })),
            stderr: Some(Box::new(io::empty())),
            state: Arc::clone(&state),
        };
        let server = FakeServer {
            requests: request_rx,
            stdout: Some(stdout_tx),
            stderr: None,
            state,
        };
        (Box::new(child), server, control)
    }

    fn options() -> ClientOptions {
        ClientOptions {
            request_timeout: Duration::from_millis(200),
            shutdown_grace: Duration::from_millis(100),
            max_stderr_bytes: 32,
        }
    }

    #[test]
    fn frames_json_lines_and_performs_initialization_before_rate_limit_read() {
        let (child, server, _) = fake_process(true);
        let handle = thread::spawn(move || {
            server.initialize();
            let request = server.recv_json();
            assert_eq!(request["method"], "account/rateLimits/read");
            assert_eq!(request["id"], 2);
            server.send_json(
                json!({"jsonrpc":"2.0","id":2,"result":{"rateLimits":{"primary":true}}}),
            );
        });

        let client = CodexClient::start_with_child(child, options()).unwrap();
        assert_eq!(
            client.read_rate_limits().unwrap(),
            json!({"rateLimits":{"primary":true}})
        );
        drop(client);
        handle.join().unwrap();
    }

    #[test]
    fn correlates_out_of_order_responses_by_numeric_id() {
        let (child, server, _) = fake_process(true);
        let handle = thread::spawn(move || {
            server.initialize();
            let first = server.recv_json();
            let second = server.recv_json();
            assert!(first["id"].is_number() && second["id"].is_number());
            server.send_json(
                json!({"jsonrpc":"2.0","id":second["id"],"result":{"method":second["method"]}}),
            );
            server.send_json(
                json!({"jsonrpc":"2.0","id":first["id"],"result":{"method":first["method"]}}),
            );
        });

        let client = Arc::new(CodexClient::start_with_child(child, options()).unwrap());
        let first_client = Arc::clone(&client);
        let first = thread::spawn(move || first_client.request("test/first", json!({})).unwrap());
        let second_client = Arc::clone(&client);
        let second =
            thread::spawn(move || second_client.request("test/second", json!({})).unwrap());

        assert_eq!(first.join().unwrap(), json!({"method":"test/first"}));
        assert_eq!(second.join().unwrap(), json!({"method":"test/second"}));
        drop(client);
        handle.join().unwrap();
    }

    #[test]
    fn surfaces_malformed_json_as_a_protocol_error() {
        let (child, server, _) = fake_process(true);
        let handle = thread::spawn(move || {
            server.initialize();
            let _request = server.recv_json();
            server
                .stdout
                .as_ref()
                .unwrap()
                .send(b"{not json}\n".to_vec())
                .unwrap();
        });

        let client = CodexClient::start_with_child(child, options()).unwrap();
        let error = client.read_rate_limits().unwrap_err();
        assert!(matches!(error, AdapterError::MalformedJson));
        drop(client);
        handle.join().unwrap();
    }

    #[test]
    fn surfaces_json_rpc_error_objects() {
        let (child, server, _) = fake_process(true);
        let handle = thread::spawn(move || {
            server.initialize();
            let request = server.recv_json();
            server.send_json(json!({
                "jsonrpc":"2.0",
                "id":request["id"],
                "error":{"code":-32001,"message":"not available","data":{"retry":false}}
            }));
        });

        let client = CodexClient::start_with_child(child, options()).unwrap();
        let error = client.read_rate_limits().unwrap_err();
        assert!(
            matches!(error, AdapterError::Rpc { code: -32001, ref message, .. } if message == "not available")
        );
        drop(client);
        handle.join().unwrap();
    }

    #[test]
    fn times_out_a_request_and_removes_it_from_pending_requests() {
        let (child, server, _) = fake_process(true);
        let handle = thread::spawn(move || {
            server.initialize();
            let _request = server.recv_json();
            thread::sleep(Duration::from_millis(300));
        });

        let client = CodexClient::start_with_child(child, options()).unwrap();
        let error = client.read_rate_limits().unwrap_err();
        assert!(
            matches!(error, AdapterError::Timeout { ref method } if method == "account/rateLimits/read")
        );
        assert_eq!(client.pending_request_count(), 0);
        drop(client);
        handle.join().unwrap();
    }

    #[test]
    fn forwards_rate_limit_and_account_notifications_without_merging() {
        let (child, server, _) = fake_process(true);
        let handle = thread::spawn(move || {
            server.initialize();
            server.send_json(json!({"jsonrpc":"2.0","method":"account/rateLimits/updated","params":{"primary":{"usedPercent":20}}}));
            server.send_json(
                json!({"jsonrpc":"2.0","method":"account/updated","params":{"planType":"team"}}),
            );
            thread::sleep(Duration::from_millis(100));
        });

        let client = CodexClient::start_with_child(child, options()).unwrap();
        let notifications = client.take_notification_receiver().unwrap();
        assert_eq!(
            notifications.recv_timeout(TEST_TIMEOUT).unwrap(),
            ServerNotification {
                method: "account/rateLimits/updated".into(),
                params: json!({"primary":{"usedPercent":20}}),
            }
        );
        assert_eq!(
            notifications.recv_timeout(TEST_TIMEOUT).unwrap(),
            ServerNotification {
                method: "account/updated".into(),
                params: json!({"planType":"team"}),
            }
        );
        drop(client);
        handle.join().unwrap();
    }

    #[test]
    fn bounds_stderr_and_surfaces_flooding_without_retaining_contents() {
        let (child, mut server, _) = fake_process(true);
        let stderr = server.stderr.take().unwrap();
        let handle = thread::spawn(move || {
            server.initialize();
            let _request = server.recv_json();
            stderr.send(vec![b'x'; 128]).unwrap();
            thread::sleep(Duration::from_millis(20));
        });

        let client = CodexClient::start_with_child(child, options()).unwrap();
        let error = client.read_rate_limits().unwrap_err();
        assert!(matches!(error, AdapterError::StderrFlooding { limit: 32 }));
        assert_eq!(client.retained_stderr_len(), 32);
        drop(client);
        handle.join().unwrap();
    }

    #[test]
    fn graceful_shutdown_closes_stdin_waits_and_releases_child_handle() {
        let (child, server, state) = fake_process(true);
        let state_for_server = Arc::clone(&state);
        let handle = thread::spawn(move || {
            server.initialize();
            assert!(server.requests.recv_timeout(TEST_TIMEOUT).is_err());
            state_for_server.exited.store(true, Ordering::SeqCst);
        });

        let client = CodexClient::start_with_child(child, options()).unwrap();
        client.shutdown().unwrap();
        handle.join().unwrap();
        assert!(state.waited.load(Ordering::SeqCst));
        assert!(!state.killed.load(Ordering::SeqCst));
        assert_eq!(state.dropped.load(Ordering::SeqCst), 1);
        assert!(!client.has_child_handle());
    }

    #[test]
    fn shutdown_kills_only_a_child_that_ignores_closed_stdin() {
        let (child, server, state) = fake_process(false);
        let handle = thread::spawn(move || {
            server.initialize();
            thread::sleep(Duration::from_millis(250));
        });
        let client = CodexClient::start_with_child(child, options()).unwrap();
        client.shutdown().unwrap();
        assert!(state.killed.load(Ordering::SeqCst));
        assert!(state.waited.load(Ordering::SeqCst));
        assert!(!client.has_child_handle());
        handle.join().unwrap();
    }

    #[test]
    fn reports_unexpected_child_exit_to_pending_request() {
        let (child, mut server, state) = fake_process(true);
        let handle = thread::spawn(move || {
            server.initialize();
            let _request = server.recv_json();
            let stdout = server.stdout.take().unwrap();
            drop(stdout);
            state.exited.store(true, Ordering::SeqCst);
        });
        let client = CodexClient::start_with_child(child, options()).unwrap();
        let error = client.read_rate_limits().unwrap_err();
        assert!(matches!(error, AdapterError::UnexpectedChildExit));
        drop(client);
        handle.join().unwrap();
    }

    #[test]
    fn finds_only_direct_executables_in_absolute_path_entries() {
        let temp = std::env::temp_dir().join(format!(
            "codex-pet-adapter-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&temp).unwrap();
        let executable = temp.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        std::fs::write(&executable, []).unwrap();
        let joined = std::env::join_paths([PathBuf::from("relative"), temp.clone()]).unwrap();
        assert_eq!(
            find_codex_in_path(&joined).unwrap(),
            executable.canonicalize().unwrap()
        );
        assert!(find_codex_in_path(&OsString::from("relative")).is_err());
        std::fs::remove_file(executable).unwrap();
        std::fs::remove_dir(temp).unwrap();
    }

    #[test]
    fn app_server_arguments_are_exact_and_shell_free() {
        assert_eq!(APP_SERVER_ARGS, ["app-server", "--stdio"]);
    }

    #[test]
    fn startup_stream_failure_kills_reaps_and_releases_the_child() {
        let state = Arc::new(FakeProcessState::default());
        let child = FakeChild {
            stdin: Some(Box::new(io::sink())),
            stdout: None,
            stderr: Some(Box::new(io::empty())),
            state: Arc::clone(&state),
        };

        let error = match CodexClient::start_with_child(Box::new(child), options()) {
            Ok(_) => panic!("startup should fail when stdout is unavailable"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AdapterError::Io {
                operation: "take child stdout",
                ..
            }
        ));
        assert!(state.killed.load(Ordering::SeqCst));
        assert!(state.waited.load(Ordering::SeqCst));
        assert_eq!(state.dropped.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fatal_state_rejects_registration_without_stranding_a_pending_request() {
        let (notification_sender, _notification_receiver) = mpsc::channel();
        let state = ClientState {
            pending: Mutex::new(HashMap::new()),
            fatal: Mutex::new(Some(AdapterError::MalformedJson)),
            notification_sender: Mutex::new(Some(notification_sender)),
            stderr: Mutex::new(Vec::new()),
            shutdown: AtomicBool::new(false),
        };
        let (sender, _receiver) = mpsc::channel();

        let error = state.register_pending(42, sender).unwrap_err();

        assert_eq!(error, AdapterError::MalformedJson);
        assert!(state.pending.lock().unwrap().is_empty());
    }

    #[test]
    fn blocked_pipe_write_respects_the_request_deadline() {
        let (child, server, _state, control) = blocking_writer_process();
        let server_thread = thread::spawn(move || {
            server.initialize();
            thread::sleep(Duration::from_millis(500));
        });
        let mut test_options = options();
        test_options.request_timeout = Duration::from_millis(50);
        let client = Arc::new(CodexClient::start_with_child(child, test_options).unwrap());
        let (result_tx, result_rx) = mpsc::channel();
        let request_client = Arc::clone(&client);
        let request_thread = thread::spawn(move || {
            let _ = result_tx.send(request_client.read_rate_limits());
        });

        while !control.entered.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        let result = result_rx.recv_timeout(Duration::from_millis(200));
        control.release.store(true, Ordering::SeqCst);
        let _ = request_thread.join();
        let _ = client.shutdown();
        let _ = server_thread.join();

        assert!(matches!(
            result,
            Ok(Err(AdapterError::Timeout { ref method }))
                if method == "account/rateLimits/read"
        ));
    }

    #[test]
    fn shutdown_completes_while_pipe_writer_is_blocked() {
        let (child, server, _state, control) = blocking_writer_process();
        let server_thread = thread::spawn(move || {
            server.initialize();
            thread::sleep(Duration::from_millis(500));
        });
        let mut test_options = options();
        test_options.request_timeout = Duration::from_secs(2);
        test_options.shutdown_grace = Duration::from_millis(80);
        let client = Arc::new(CodexClient::start_with_child(child, test_options).unwrap());
        let request_client = Arc::clone(&client);
        let request_thread = thread::spawn(move || request_client.read_rate_limits());
        while !control.entered.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let shutdown_client = Arc::clone(&client);
        let shutdown_thread = thread::spawn(move || {
            let _ = shutdown_tx.send(shutdown_client.shutdown());
        });

        let result = shutdown_rx.recv_timeout(Duration::from_millis(250));
        control.release.store(true, Ordering::SeqCst);
        let _ = shutdown_thread.join();
        let _ = request_thread.join();
        let _ = server_thread.join();

        assert!(result.is_ok(), "shutdown exceeded its bounded deadline");
    }

    #[test]
    fn shutdown_is_bounded_when_kill_fails_and_stdout_never_reaches_eof() {
        let (child, server, control) = kill_failing_process();
        let server_control = Arc::clone(&control);
        let server_thread = thread::spawn(move || {
            server.initialize();
            while !server_control.release.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(2));
            }
        });
        let mut test_options = options();
        test_options.shutdown_grace = Duration::from_millis(80);
        let client = Arc::new(CodexClient::start_with_child(child, test_options).unwrap());
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let shutdown_client = Arc::clone(&client);
        let shutdown_thread = thread::spawn(move || {
            let _ = shutdown_tx.send(shutdown_client.shutdown());
        });

        let result = shutdown_rx.recv_timeout(Duration::from_millis(250));
        control.release.store(true, Ordering::SeqCst);
        let _ = shutdown_thread.join();
        let _ = server_thread.join();

        assert!(
            matches!(
                result,
                Ok(Err(AdapterError::Io {
                    operation: "kill child",
                    ..
                }))
            ),
            "shutdown must return the kill error within its deadline"
        );
        assert!(
            !control.wait_entered.load(Ordering::SeqCst),
            "shutdown must not call an unbounded wait after kill fails"
        );
    }

    #[test]
    fn shutdown_immediately_fails_registered_pending_requests() {
        let (child, server, _state) = fake_process(true);
        let (request_seen_tx, request_seen_rx) = mpsc::channel();
        let release_server = Arc::new(AtomicBool::new(false));
        let server_release = Arc::clone(&release_server);
        let server_thread = thread::spawn(move || {
            server.initialize();
            let _request = server.recv_json();
            request_seen_tx.send(()).unwrap();
            while !server_release.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(2));
            }
        });
        let mut test_options = options();
        test_options.request_timeout = Duration::from_secs(1);
        test_options.shutdown_grace = Duration::from_millis(80);
        let client = Arc::new(CodexClient::start_with_child(child, test_options).unwrap());
        let (result_tx, result_rx) = mpsc::channel();
        let request_client = Arc::clone(&client);
        let request_thread = thread::spawn(move || {
            let _ = result_tx.send(request_client.read_rate_limits());
        });
        request_seen_rx.recv_timeout(TEST_TIMEOUT).unwrap();

        client.shutdown().unwrap();
        let result = result_rx.recv_timeout(Duration::from_millis(150));
        release_server.store(true, Ordering::SeqCst);
        let _ = request_thread.join();
        let _ = server_thread.join();

        assert!(matches!(result, Ok(Err(AdapterError::AlreadyShutdown))));
        assert_eq!(client.pending_request_count(), 0);
    }

    #[derive(Default)]
    struct VerificationProcessState {
        killed: AtomicBool,
        dropped: AtomicBool,
    }

    struct TryWaitFailingVerificationChild {
        state: Arc<VerificationProcessState>,
    }

    impl Drop for TryWaitFailingVerificationChild {
        fn drop(&mut self) {
            self.state.dropped.store(true, Ordering::SeqCst);
        }
    }

    impl VerificationChild for TryWaitFailingVerificationChild {
        fn try_wait(&mut self) -> io::Result<Option<VerificationExit>> {
            Err(io::Error::other("try_wait failed"))
        }

        fn kill(&mut self) -> io::Result<()> {
            self.state.killed.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn version_try_wait_error_kills_waits_and_releases_the_child() {
        let state = Arc::new(VerificationProcessState::default());
        let child = TryWaitFailingVerificationChild {
            state: Arc::clone(&state),
        };

        let error = verify_spawned_codex(Box::new(child), Duration::from_millis(20)).unwrap_err();

        assert!(matches!(error, AdapterError::VersionCheckFailed { .. }));
        assert!(state.killed.load(Ordering::SeqCst));
        assert!(state.dropped.load(Ordering::SeqCst));
    }

    fn protocol_test_state() -> ClientState {
        let (notification_sender, _notification_receiver) = mpsc::channel();
        ClientState {
            pending: Mutex::new(HashMap::new()),
            fatal: Mutex::new(None),
            notification_sender: Mutex::new(Some(notification_sender)),
            stderr: Mutex::new(Vec::new()),
            shutdown: AtomicBool::new(false),
        }
    }

    #[test]
    fn rejects_missing_jsonrpc_and_invalid_response_envelopes() {
        let missing_version = dispatch_message(&protocol_test_state(), json!({"id":1,"result":{}}));
        assert!(matches!(
            missing_version,
            Err(AdapterError::Protocol { .. })
        ));

        let ambiguous_state = protocol_test_state();
        let (ambiguous_sender, _ambiguous_receiver) = mpsc::channel();
        ambiguous_state
            .register_pending(2, ambiguous_sender)
            .unwrap();
        let ambiguous = dispatch_message(
            &ambiguous_state,
            json!({
                "jsonrpc":"2.0",
                "id":2,
                "result":{},
                "error":{"code":-1,"message":"bad"}
            }),
        );
        assert!(matches!(ambiguous, Err(AdapterError::Protocol { .. })));

        let server_request = dispatch_message(
            &protocol_test_state(),
            json!({"jsonrpc":"2.0","id":99,"method":"server/call","params":{}}),
        );
        assert!(matches!(server_request, Err(AdapterError::Protocol { .. })));
    }

    #[test]
    fn writer_exits_after_shutdown_even_when_shutdown_control_is_not_queued() {
        let state = Arc::new(protocol_test_state());
        state.shutdown.store(true, Ordering::Release);
        let (outbound_tx, outbound_rx) = mpsc::sync_channel(1);
        let (completion_tx, completion_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let writer_state = Arc::clone(&state);
        let writer_thread = thread::spawn(move || {
            write_stdin(Box::new(io::sink()), outbound_rx, writer_state);
            let _ = done_tx.send(());
        });

        outbound_tx
            .send(OutboundMessage::Write {
                bytes: b"{}\n".to_vec(),
                completion: completion_tx,
            })
            .unwrap();
        assert!(completion_rx.recv_timeout(TEST_TIMEOUT).unwrap().is_ok());
        assert!(done_rx.recv_timeout(TEST_TIMEOUT).is_ok());
        writer_thread.join().unwrap();
    }

    #[test]
    fn shutdown_reports_and_retains_an_unfinished_stdout_reader_for_retry() {
        let (child, server, control) = stuck_reader_process();
        let server_control = Arc::clone(&control);
        let server_thread = thread::spawn(move || {
            server.initialize();
            while !server_control.release.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(2));
            }
        });
        let mut test_options = options();
        test_options.shutdown_grace = Duration::from_millis(80);
        let client = CodexClient::start_with_child(child, test_options).unwrap();

        let first = client.shutdown();
        let retained_after_first = client.stdout_thread.lock().unwrap().is_some();
        control.release.store(true, Ordering::SeqCst);
        server_thread.join().unwrap();
        let second = client.shutdown();

        assert!(matches!(
            first,
            Err(AdapterError::Io {
                operation: "join stdout reader",
                ..
            })
        ));
        assert!(retained_after_first);
        assert!(second.is_ok());
        assert!(client.stdout_thread.lock().unwrap().is_none());
    }

    #[test]
    fn drop_hands_unfinished_reader_to_a_bounded_reaper() {
        let (child, server, control) = stuck_reader_process();
        let server_control = Arc::clone(&control);
        let server_thread = thread::spawn(move || {
            server.initialize();
            while !server_control.release.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(2));
            }
        });
        let mut test_options = options();
        test_options.shutdown_grace = Duration::from_millis(40);
        let client = CodexClient::start_with_child(child, test_options).unwrap();
        let started = std::time::Instant::now();
        drop(client);
        assert!(started.elapsed() < Duration::from_millis(500));
        control.release.store(true, Ordering::SeqCst);
        server_thread.join().unwrap();
    }

    #[test]
    fn blocked_writer_returns_an_existing_fatal_error_instead_of_timeout() {
        let (child, mut server, _state, control) = blocking_writer_process();
        let stderr = server.stderr.take().unwrap();
        let server_control = Arc::clone(&control);
        let server_thread = thread::spawn(move || {
            server.initialize();
            while !server_control.release.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(2));
            }
        });
        let mut test_options = options();
        test_options.request_timeout = Duration::from_millis(500);
        let client = Arc::new(CodexClient::start_with_child(child, test_options).unwrap());
        let (result_tx, result_rx) = mpsc::channel();
        let request_client = Arc::clone(&client);
        let request_thread = thread::spawn(move || {
            let _ = result_tx.send(request_client.read_rate_limits());
        });
        while !control.entered.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }

        stderr.send(vec![b'x'; 128]).unwrap();
        let result = result_rx.recv_timeout(Duration::from_millis(150));
        control.release.store(true, Ordering::SeqCst);
        let _ = request_thread.join();
        let _ = client.shutdown();
        let _ = server_thread.join();

        assert!(matches!(
            result,
            Ok(Err(AdapterError::StderrFlooding { limit: 32 }))
        ));
    }
}
