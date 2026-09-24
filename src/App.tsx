import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { PetShell } from "./pet/PetShell";
import type { AccountStatus } from "./account/types";
import type { QuotaSnapshot } from "./quota/types";
import "./App.css";

const initialSnapshot: QuotaSnapshot = {
  availability: "unavailable",
  windows: [],
  planType: null,
  fetchedAt: 0,
  stale: false,
  message: "正在读取 Codex 额度…",
};

type RefreshFeedback = "idle" | "loading" | "success" | "error";

function App() {
  const [snapshot, setSnapshot] = useState<QuotaSnapshot>(initialSnapshot);
  const [accountStatus, setAccountStatus] = useState<AccountStatus>("checking");
  const [refreshFeedback, setRefreshFeedback] = useState<RefreshFeedback>("idle");
  const accountEventVersion = useRef(0);
  const [locked, setLocked] = useState(false);
  const hasSuccessfulSnapshot = useRef(false);
  const isMounted = useRef(false);
  const quotaRefreshPromise = useRef<Promise<boolean> | null>(null);
  const manualRefreshActive = useRef(false);
  const acceptSnapshot = useCallback((nextSnapshot: QuotaSnapshot) => {
    hasSuccessfulSnapshot.current = true;
    setSnapshot(nextSnapshot);
  }, []);
  const markUpdateFailed = useCallback((error: unknown) => {
    const message = `更新失败：${String(error)}`;
    setSnapshot((previous) => hasSuccessfulSnapshot.current
      ? { ...previous, stale: true, message }
      : { ...initialSnapshot, message });
  }, []);
  const refresh = useCallback((source: "automatic" | "manual" = "manual"): Promise<boolean> => {
    if (!("__TAURI_INTERNALS__" in window)) return Promise.resolve(false);
    if (quotaRefreshPromise.current) return quotaRefreshPromise.current;

    const operation = Promise.resolve()
      .then(() => invoke<QuotaSnapshot>("read_quota", { source }))
      .then((nextSnapshot) => {
        if (isMounted.current) acceptSnapshot(nextSnapshot);
        return true;
      })
      .catch((error: unknown) => {
        if (isMounted.current) markUpdateFailed(error);
        return false;
      });
    let trackedOperation: Promise<boolean>;
    trackedOperation = operation.finally(() => {
      if (quotaRefreshPromise.current === trackedOperation) quotaRefreshPromise.current = null;
    });
    quotaRefreshPromise.current = trackedOperation;
    return trackedOperation;
  }, [acceptSnapshot, markUpdateFailed]);

  const refreshManually = useCallback(async () => {
    if (manualRefreshActive.current) return;
    manualRefreshActive.current = true;
    setRefreshFeedback("loading");
    try {
      const succeeded = await refresh("manual");
      if (isMounted.current) setRefreshFeedback(succeeded ? "success" : "error");
    } finally {
      manualRefreshActive.current = false;
    }
  }, [refresh]);

  useEffect(() => {
    isMounted.current = true;
    return () => {
      isMounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (refreshFeedback !== "success" && refreshFeedback !== "error") return;
    const timer = window.setTimeout(() => {
      if (isMounted.current) setRefreshFeedback("idle");
    }, 2_500);
    return () => window.clearTimeout(timer);
  }, [refreshFeedback]);

  useEffect(() => {
    if (accountStatus !== "loggedIn") return;
    void refresh("automatic");
    const timer = window.setInterval(() => void refresh("automatic"), 60_000);
    return () => window.clearInterval(timer);
  }, [accountStatus, refresh]);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;

    let disposed = false;
    let eventReceived = false;
    let unlisten: (() => void) | undefined;
    void listen<AccountStatus>("account://updated", (event) => {
      eventReceived = true;
      accountEventVersion.current += 1;
      if (!disposed) setAccountStatus(event.payload);
    }).then((registeredUnlisten) => {
      if (disposed) registeredUnlisten();
      else unlisten = registeredUnlisten;
    }).catch(() => undefined);
    void invoke<AccountStatus>("read_account_status").then((status) => {
      if (!disposed && !eventReceived) setAccountStatus(status);
    }).catch(() => {
      if (!disposed) setAccountStatus("unavailable");
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;

    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<QuotaSnapshot>("quota://updated", (event) => {
      if (!disposed) acceptSnapshot(event.payload);
    }).then((registeredUnlisten) => {
      if (disposed) registeredUnlisten();
      else unlisten = registeredUnlisten;
    }).catch((error) => {
      if (!disposed) markUpdateFailed(error);
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [acceptSnapshot, markUpdateFailed]);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;

    let disposed = false;
    let eventReceived = false;
    let unlisten: (() => void) | undefined;
    void listen<boolean>("window://locked", (event) => {
      eventReceived = true;
      if (!disposed) setLocked(event.payload);
    }).then((registeredUnlisten) => {
      if (disposed) registeredUnlisten();
      else unlisten = registeredUnlisten;
    }).catch(() => undefined);
    void invoke<boolean>("read_window_locked").then((initialLocked) => {
      if (!disposed && !eventReceived) setLocked(initialLocked);
    }).catch(() => undefined);

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const resizeForDetails = (expanded: boolean) => {
    // 浏览器预览没有 Tauri runtime；只在桌面壳中调整原生窗口尺寸。
    if (!("__TAURI_INTERNALS__" in window)) return;
    void getCurrentWindow().setSize(
      expanded ? new LogicalSize(340, 390) : new LogicalSize(164, 154),
    );
  };

  const startDragging = useCallback(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    void getCurrentWindow().startDragging();
  }, []);

  const startChatgptLogin = useCallback(async () => {
    const version = accountEventVersion.current;
    try {
      const status = await invoke<AccountStatus>("start_chatgpt_login");
      if (accountEventVersion.current === version) setAccountStatus(status);
    } catch {
      if (accountEventVersion.current === version) setAccountStatus("loginFailed");
    }
  }, []);

  const retryAccountStatus = useCallback(async () => {
    const version = accountEventVersion.current;
    setAccountStatus("checking");
    try {
      const status = await invoke<AccountStatus>("read_account_status");
      if (accountEventVersion.current === version) setAccountStatus(status);
    } catch {
      if (accountEventVersion.current === version) setAccountStatus("unavailable");
    }
  }, []);

  const cancelChatgptLogin = useCallback(async () => {
    const version = accountEventVersion.current;
    try {
      const status = await invoke<AccountStatus>("cancel_chatgpt_login");
      if (accountEventVersion.current === version) setAccountStatus(status);
    } catch {
      if (accountEventVersion.current === version) setAccountStatus("cancelled");
    }
  }, []);

  return (
    <PetShell
      snapshot={snapshot}
      accountStatus={accountStatus}
      locked={locked}
      refreshState={refreshFeedback}
      onExpandedChange={resizeForDetails}
      onStartDragging={startDragging}
      onRefresh={() => void refreshManually()}
      onRetryAccountStatus={() => void retryAccountStatus()}
      onStartChatgptLogin={startChatgptLogin}
      onCancelChatgptLogin={cancelChatgptLogin}
    />
  );
}

export default App;
