/** The hook mark. Geometry matches `sdHook` in the dither shader, so the lens is the logo. */
export function HookMark({ size = 22, stroke = "currentColor" }: { size?: number; stroke?: string }) {
  return (
    <svg width={size * 0.7} height={size} viewBox="-0.27 -0.86 0.88 1.27" fill="none" aria-hidden>
      <g stroke={stroke} strokeWidth={0.15} strokeLinecap="round" strokeLinejoin="round">
        <circle cx={0} cy={-0.62} r={0.15} />
        <path d="M0 -0.47 V0.08 A0.24 0.24 0 0 0 0.48 0.08 V-0.1 L0.4 -0.03" />
      </g>
    </svg>
  );
}

export function Wordmark() {
  return (
    <span className="wordmark">
      <HookMark size={22} />
      <span>Pump Hooks</span>
    </span>
  );
}
