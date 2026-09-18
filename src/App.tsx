import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { PetShell } from "./pet/PetShell";
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

function App() {
  const [snapshot, setSnapshot] = useState<QuotaSnapshot>(initialSnapshot);
  const hasSuccessfulSnapshot = useRef(false);
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
  const refresh = useCallback(async () => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    try {
      acceptSnapshot(await invoke<QuotaSnapshot>("read_quota"));
    } catch (error) {
      markUpdateFailed(error);
    }
  }, [acceptSnapshot, markUpdateFailed]);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 60_000);
    return () => window.clearInterval(timer);
  }, [refresh]);

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

  const resizeForDetails = (expanded: boolean) => {
    // 浏览器预览没有 Tauri runtime；只在桌面壳中调整原生窗口尺寸。
    if (!("__TAURI_INTERNALS__" in window)) return;
    void getCurrentWindow().setSize(
      expanded ? new LogicalSize(340, 390) : new LogicalSize(164, 154),
    );
  };

  return (
    <PetShell snapshot={snapshot} onExpandedChange={resizeForDetails} onRefresh={refresh} />
  );
}

export default App;
