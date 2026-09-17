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
};

function App() {
  const resizeForDetails = (expanded: boolean) => {
    // 浏览器预览没有 Tauri runtime；只在桌面壳中调整原生窗口尺寸。
    if (!("__TAURI_INTERNALS__" in window)) return;
    void getCurrentWindow().setSize(
      expanded ? new LogicalSize(340, 390) : new LogicalSize(164, 154),
    );
  };

  return (
    <PetShell snapshot={initialSnapshot} onExpandedChange={resizeForDetails} />
  );
}

export default App;
