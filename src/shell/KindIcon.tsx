import type { ObjectKind } from "../lib/meta";
import { FunctionIcon, ProcedureIcon, TableIcon, TriggerIcon, ViewIcon } from "./Icons";

export function KindIcon({ kind, size = 14 }: { kind: ObjectKind; size?: number }) {
  const Icon = { table: TableIcon, view: ViewIcon, function: FunctionIcon, procedure: ProcedureIcon, trigger: TriggerIcon }[kind];
  return (
    <span className={`kind-icon kind-${kind}`}>
      <Icon size={size} />
    </span>
  );
}

/** A name with the fuzzy-matched characters highlighted. */
export function Highlighted({ text, indices }: { text: string; indices?: number[] }) {
  if (!indices || indices.length === 0) return <>{text}</>;
  const marks = new Set(indices);
  const chars = Array.from(text);
  const parts: { s: string; hit: boolean }[] = [];
  chars.forEach((c, i) => {
    const hit = marks.has(i);
    const last = parts[parts.length - 1];
    if (last && last.hit === hit) last.s += c;
    else parts.push({ s: c, hit });
  });
  return (
    <>
      {parts.map((p, i) => (p.hit ? <mark key={i}>{p.s}</mark> : <span key={i}>{p.s}</span>))}
    </>
  );
}
