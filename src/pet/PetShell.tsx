import { useEffect, useMemo, useRef, useState } from "react";
import type { QuotaSnapshot } from "../quota/types";

type PetShellProps = {
  snapshot: QuotaSnapshot;
  onExpandedChange: (expanded: boolean) => void;
};

const AUTO_COLLAPSE_MS = 800;

function PetFace({ availability }: Pick<QuotaSnapshot, "availability">) {
  const offline = availability === "unavailable" || availability === "unknown";
  const blocked = availability === "blocked";
  return (
    <svg className="pet-face" viewBox="0 0 112 92" aria-hidden="true">
      <defs><linearGradient id="petBody" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stopColor={offline ? "#9ca3af" : "#9ef3d0"} /><stop offset="1" stopColor={offline ? "#64748b" : "#35c799"} /></linearGradient></defs>
      <path className="pet-shadow" d="M21 80c9 8 61 9 72 0-8 13-63 15-72 0Z" />
      <path className="pet-ear" d="M24 28 12 13c-3-4 1-8 5-6l19 10M88 28l12-15c3-4 7 0 5 5L94 35" />
      <rect x="17" y="17" width="78" height="66" rx="28" fill="url(#petBody)" />
      <rect x="25" y="28" width="62" height="38" rx="17" className="pet-screen" />
      {blocked ? <><path d="m38 42 8 8m0-8-8 8m27-8 8 8m0-8-8 8" className="pet-eye-line" /><path d="M48 58h16" className="pet-eye-line" /></> : <><circle cx="43" cy="47" r="4.5" className="pet-eye" /><circle cx="69" cy="47" r="4.5" className="pet-eye" /><path d={offline ? "M49 58h14" : "M49 56c4 4 10 4 14 0"} className="pet-mouth" /></>}
      <path d="M36 82v5m40-5v5" className="pet-feet" />
    </svg>
  );
}

function resetLabel(resetsAt: number | null) {
  if (!resetsAt) return "重置时间未知";
  return `重置于 ${new Date(resetsAt * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}`;
}

export function PetShell({ snapshot, onExpandedChange }: PetShellProps) {
  const [expanded, setExpanded] = useState(false);
  const collapseTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const tightestWindow = useMemo(() => [...snapshot.windows].sort((a, b) => a.remainingPercent - b.remainingPercent)[0], [snapshot.windows]);

  useEffect(() => () => { if (collapseTimer.current) clearTimeout(collapseTimer.current); }, []);
  const changeExpanded = (next: boolean) => { setExpanded(next); onExpandedChange(next); };
  const scheduleCollapse = () => {
    collapseTimer.current = setTimeout(() => changeExpanded(false), AUTO_COLLAPSE_MS);
  };
  const cancelCollapse = () => { if (collapseTimer.current) clearTimeout(collapseTimer.current); };
  const headline = tightestWindow ? `${Math.round(tightestWindow.remainingPercent)}%` : snapshot.availability === "blocked" ? "已暂停" : "额度暂不可用";

  return (
    <main className={`pet-shell pet-${snapshot.availability}${expanded ? " is-expanded" : ""}`} onPointerEnter={cancelCollapse} onPointerLeave={scheduleCollapse}>
      <div className="drag-handle" data-tauri-drag-region aria-label="拖动桌宠" />
      <button type="button" className="pet-button" aria-label={expanded ? "收起额度详情" : "展开额度详情"} aria-expanded={expanded} onClick={() => changeExpanded(!expanded)}>
        <PetFace availability={snapshot.availability} />
        <span className="quota-pill">{headline}</span>
      </button>
      {expanded && <section className="quota-card" aria-label="Codex 额度详情">
        <header><div><p className="eyebrow">CODEX PET</p><h1>额度状态</h1></div><span className={`status-dot status-${snapshot.availability}`}>{snapshot.stale ? "更新失败" : "本机读取"}</span></header>
        {snapshot.windows.length ? <div className="quota-list">{snapshot.windows.map((window) => <article className="quota-row" key={window.id}>
          <div className="quota-row-heading"><strong>{window.name}</strong><span>{Math.round(window.remainingPercent)}% 剩余</span></div>
          <div className="quota-track" aria-hidden="true"><span style={{ width: `${window.remainingPercent}%` }} /></div>
          <small>{resetLabel(window.resetsAt)}</small>
        </article>)}</div> : <div className="empty-state"><strong>额度暂不可用</strong><span>请确认 Codex CLI 已登录</span></div>}
        <footer>{snapshot.planType ? `${snapshot.planType} 方案` : "等待本机 Codex"}</footer>
      </section>}
    </main>
  );
}
