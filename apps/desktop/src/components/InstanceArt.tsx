import { useId } from "react";
import type { LoaderKind } from "../lib/ipc";

/** FNV-1a: stable 32-bit hash so each instance always gets the same art. */
export function hash32(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** Small deterministic PRNG (mulberry32) seeded from the hash. */
function rng(seed: number) {
  let a = seed;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Restrained per-loader tint, from the design tokens. */
const TINT: Record<LoaderKind, string> = {
  vanilla: "var(--color-info)",
  fabric: "var(--color-warning)",
  quilt: "var(--color-success)",
  neoforge: "var(--color-accent)",
  forge: "var(--color-danger)",
};

/**
 * Generated "night sky" artwork for an instance: a quiet graphite field with
 * a few stars and a horizon in the loader's tint. Decorative (aria-hidden).
 */
export function InstanceArt({ seed, loader, name, className = "" }: { seed: string; loader: LoaderKind; name: string; className?: string }) {
  const gid = useId();
  const r = rng(hash32(seed));
  const stars = Array.from({ length: 14 }, () => ({ x: r() * 320, y: r() * 120, s: 0.6 + r() * 1.3, o: 0.25 + r() * 0.6 }));
  const h = (i: number) => 128 + r() * 26 - i * 2;
  const [h0, h1, h2, h3, h4] = [h(0), h(1), h(2), h(3), h(4)];
  const tint = TINT[loader];
  return (
    <svg viewBox="0 0 320 180" preserveAspectRatio="xMidYMid slice" aria-hidden="true" className={`block ${className}`}>
      <defs>
        <linearGradient id={`${gid}-sky`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="var(--color-surface-1)" />
          <stop offset="1" stopColor="var(--color-surface-3)" />
        </linearGradient>
      </defs>
      <rect width="320" height="180" fill={`url(#${gid}-sky)`} />
      {stars.map((s, i) => (
        <circle key={i} cx={s.x} cy={s.y} r={s.s} fill="var(--color-fg)" opacity={s.o} />
      ))}
      <path
        d={`M0 ${h0} C 64 ${h1}, 128 ${h2}, 192 ${h3} S 288 ${h4}, 320 ${h2} L320 180 L0 180 Z`}
        fill={tint}
        opacity="0.16"
      />
      <path d={`M0 ${h2 + 18} C 96 ${h4 + 10}, 200 ${h0 + 20}, 320 ${h1 + 16} L320 180 L0 180 Z`} fill="var(--color-bg)" opacity="0.85" />
      <text x="20" y="160" fill={tint} opacity="0.9" fontFamily="var(--font-sans)" fontSize="44" fontWeight="600" letterSpacing="-1">
        {name.trim().slice(0, 1).toUpperCase() || "?"}
      </text>
    </svg>
  );
}
