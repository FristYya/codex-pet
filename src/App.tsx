import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
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
  const refresh = useCallback(async () => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    try {
      setSnapshot(await invoke<QuotaSnapshot>("read_quota"));
    } catch (error) {
      setSnapshot((previous) => ({ ...previous, stale: previous.windows.length > 0, message: String(error) }));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 60_000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const resizeForDetails = (expanded: boolean) => {
    // 浏览器预览没有 Tauri runtime；只在桌面壳中调整原生窗口尺寸。
    if (!("__TAURI_INTERNALS__" in window)) return;
    void getCurrentWindow().setSize(
      expanded ? new LogicalSize(340, 390) : new LogicalSize(164, 154),
    );
  };

  return (
    <PetShell snapshot={snapshot} onExpandedChange={resizeForDetails} />
  );
}

export default App;
