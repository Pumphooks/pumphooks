"use client";

import { useEffect, useRef } from "react";

/**
 * Ordered-dither halftone of a painting, with a hook-shaped lens that follows the cursor and
 * reveals the original colour underneath. WebGL2, one full-screen triangle, no dependencies.
 *
 * `mode="page"`: fixed full-viewport stage; the image is framed by `[data-frame]` anchors in the
 * document (see FRAMES), interpolated as they cross the viewport centre.
 * `mode="banner"`: fills its parent; the image sits to the right.
 */

type Rect = { x: number; y: number; w: number };
type FrameFn = (vw: number, vh: number, el?: HTMLElement) => Rect;

/** Top of the hero image: just under the element marked `data-stage-top`, in page coordinates. */
function stageTop(el: HTMLElement | undefined, fallback: number) {
  const m = el?.querySelector<HTMLElement>("[data-stage-top]");
  if (!m) return fallback;
  return m.getBoundingClientRect().bottom + window.scrollY + 8;
}

const FRAMES: Record<string, FrameFn> = {
  hero: (vw, vh, el) => {
    const top = stageTop(el, vh * 0.5);
    const w = vw < 820 ? vw * 1.25 : Math.min(vw * 0.84, vh * 1.3, 1300);
    return { x: (vw - w) / 2, y: top - w * 0.012, w };
  },
  // The painting is widest at the bottom (hooks at ~0.1w and ~0.9w), so lower framings sit further left.
  s1: (vw, vh) => side(vw, vh, Math.max(vh * 1.2, vw * 0.56), 0.29, 0.3),
  s2: (vw, vh) => side(vw, vh, Math.max(vh * 1.45, vw * 0.68), 0.2, 0.55),
  s3: (vw, vh) => side(vw, vh, Math.max(vh * 1.3, vw * 0.62), 0.17, 0.7),
  s4: (vw, vh) => side(vw, vh, Math.min(vh * 1.02, vw * 0.55), 0.27, 0.5),
  end: (vw, vh) => {
    const r = side(vw, vh, Math.min(vh * 1.02, vw * 0.55), 0.27, 0.5);
    return { ...r, y: r.y - vh * 1.1 };
  },
};

function side(vw: number, vh: number, w: number, cx: number, focusY: number): Rect {
  if (vw < 820) {
    const mw = vw * 1.5;
    return { x: (vw - mw) / 2, y: vh * 0.5 - mw * focusY, w: mw };
  }
  return { x: vw * cx - w * 0.5, y: vh * 0.5 - w * focusY, w };
}

const smooth = (a: number, b: number, t: number) => {
  const x = Math.min(1, Math.max(0, (t - a) / (b - a)));
  return x * x * (3 - 2 * x);
};
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

const VERT = `#version 300 es
in vec2 p;
void main(){ gl_Position = vec4(p, 0.0, 1.0); }`;

const FRAG = `#version 300 es
precision highp float;
uniform sampler2D uImg;
uniform vec2 uRes;
uniform vec3 uRect;
uniform vec2 uMouse;
uniform float uLens;
uniform float uCell;
uniform float uTime;
uniform float uReveal;
uniform vec3 uPaper;
uniform vec3 uInk;
out vec4 o;

// Exact 8x8 Bayer threshold via bit interleaving (float formulations lose precision far from the origin).
float bayer8(vec2 c){
  ivec2 p = ivec2(mod(c, 8.0));
  int x = p.x, xy = p.x ^ p.y;
  int v = ((xy & 1) << 5) | ((x & 1) << 4) | ((xy & 2) << 2) | ((x & 2) << 1) | ((xy & 4) >> 1) | ((x & 4) >> 2);
  return (float(v) + 0.5) / 64.0;
}
float hash(vec2 p){ return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453); }

float sdSeg(vec2 p, vec2 a, vec2 b){
  vec2 pa = p - a, ba = b - a;
  float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
  return length(pa - ba * h);
}

// The Pump Hooks mark (y up), matching components/Logo.tsx.
float sdHook(vec2 p){
  p += vec2(0.165, 0.225);
  float eye = abs(length(p - vec2(0.0, 0.62)) - 0.15);
  float shank = sdSeg(p, vec2(0.0, 0.47), vec2(0.0, -0.08));
  vec2 q = p - vec2(0.24, -0.08);
  float bowl = q.y < 0.0 ? abs(length(q) - 0.24) : min(length(q - vec2(-0.24, 0.0)), length(q - vec2(0.24, 0.0)));
  float tip = sdSeg(p, vec2(0.48, -0.08), vec2(0.48, 0.10));
  float barb = sdSeg(p, vec2(0.48, 0.10), vec2(0.40, 0.03));
  return min(min(eye, shank), min(bowl, min(tip, barb))) - 0.075;
}

vec4 img(vec2 px, float bias){
  vec2 uv = (px - uRect.xy) / uRect.z;
  if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) return vec4(1.0, 1.0, 1.0, 0.0);
  vec3 c = texture(uImg, uv, bias).rgb;
  float a = 1.0 - smoothstep(0.86, 0.975, min(c.r, min(c.g, c.b)));
  return vec4(c, a);
}

void main(){
  vec2 px = vec2(gl_FragCoord.x, uRes.y - gl_FragCoord.y);

  vec2 cell = floor(px / uCell);
  vec2 cc = (cell + 0.5) * uCell;
  vec4 s = img(cc, 0.6);
  float lum = dot(s.rgb, vec3(0.299, 0.587, 0.114));
  float dark = smoothstep(0.07, 0.9, 1.0 - lum);
  dark = pow(dark, 0.82) * s.a * uReveal;
  float th = bayer8(cell);
  float ink = step(th, dark);
  vec3 col = mix(uPaper, uInk, ink);
  col += (hash(cell) - 0.5) * 0.018;

  vec2 q = (px - uMouse) / uLens;
  q.y = -q.y;
  float d = sdHook(q) * uLens;
  float edge = 0.018 * uLens;
  float inside = 1.0 - smoothstep(-edge, edge * 0.35, d);
  float rim = exp(-pow(d / (0.05 * uLens), 2.0));
  float outerRim = exp(-pow(max(d, 0.0) / (0.11 * uLens), 2.0));

  vec4 full = img(px, 0.0);
  vec4 blur = img(px, 4.5);
  vec3 lensCol = mix(uPaper, full.rgb * 1.04, full.a);
  vec3 rimCol = mix(uPaper * 0.86, blur.rgb, blur.a);

  float on = uReveal;
  col = mix(col, rimCol, outerRim * 0.42 * on);
  col = mix(col, lensCol, inside * on);
  col = mix(col, rimCol, rim * 0.5 * (1.0 - inside * 0.55) * on);
  float line = 1.0 - smoothstep(0.0, 1.2 * uCell * 0.5, abs(d));
  col = mix(col, uInk, line * 0.22 * on);

  o = vec4(col, 1.0);
}`;

export function DitherStage({ src = "/art/hook-engine.jpg", mode = "page" }: { src?: string; mode?: "page" | "banner" }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const fallbackRef = useRef<HTMLImageElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current!;
    const gl = canvas.getContext("webgl2", { antialias: false, premultipliedAlpha: false, alpha: false });
    if (!gl) {
      canvas.style.display = "none";
      if (fallbackRef.current) fallbackRef.current.style.display = "block";
      return;
    }
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    const sh = (type: number, srcText: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, srcText);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS) && !gl.isContextLost()) throw new Error(gl.getShaderInfoLog(s) || "shader compile failed");
      return s;
    };
    const prog = gl.createProgram()!;
    gl.attachShader(prog, sh(gl.VERTEX_SHADER, VERT));
    gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FRAG));
    gl.linkProgram(prog);
    gl.useProgram(prog);
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, "p");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    const U = (n: string) => gl.getUniformLocation(prog, n);
    const u = {
      res: U("uRes"), rect: U("uRect"), mouse: U("uMouse"), lens: U("uLens"), cell: U("uCell"),
      time: U("uTime"), reveal: U("uReveal"), paper: U("uPaper"), ink: U("uInk"), img: U("uImg"),
    };
    gl.uniform3f(u.paper, 0.937, 0.929, 0.902);
    gl.uniform3f(u.ink, 0.055, 0.055, 0.047);

    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([255, 255, 255, 255]));
    let loaded = false;
    let loadedAt = 0;
    const image = new Image();
    image.decoding = "async";
    image.src = src;
    image.onload = () => {
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
      gl.generateMipmap(gl.TEXTURE_2D);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      loaded = true;
      loadedAt = performance.now();
    };
    gl.uniform1i(u.img, 0);

    let dpr = 1;
    let vw = 0;
    let vh = 0;
    const resize = () => {
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      const r = mode === "page" ? { width: window.innerWidth, height: window.innerHeight } : canvas.parentElement!.getBoundingClientRect();
      vw = r.width;
      vh = r.height;
      canvas.width = Math.round(vw * dpr);
      canvas.height = Math.round(vh * dpr);
      canvas.style.width = `${vw}px`;
      canvas.style.height = `${vh}px`;
      gl.viewport(0, 0, canvas.width, canvas.height);
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(mode === "page" ? document.documentElement : canvas.parentElement!);

    const pointer = { x: -1, y: -1, at: -1e9, inside: false };
    const onMove = (e: PointerEvent) => {
      const r = canvas.getBoundingClientRect();
      pointer.x = e.clientX - r.left;
      pointer.y = e.clientY - r.top;
      pointer.inside = pointer.x >= 0 && pointer.y >= 0 && pointer.x <= r.width && pointer.y <= r.height;
      if (e.pointerType === "mouse") pointer.at = performance.now();
    };
    window.addEventListener("pointermove", onMove, { passive: true });

    const frameRect = (): Rect => {
      if (mode === "banner") {
        const w = Math.max(vh * 1.35, vw * 0.5);
        return { x: vw - w * 0.8, y: vh * 0.5 - w * 0.42, w };
      }
      const anchors = Array.from(document.querySelectorAll<HTMLElement>("[data-frame]"));
      if (anchors.length === 0) return FRAMES.hero(vw, vh);
      const cur = window.scrollY + vh * 0.5;
      const centers = anchors.map((a) => {
        const r = a.getBoundingClientRect();
        return r.top + window.scrollY + r.height * 0.5;
      });
      const f = (i: number) => (FRAMES[anchors[i].dataset.frame!] ?? FRAMES.hero)(vw, vh, anchors[i]);
      if (cur <= centers[0]) return f(0);
      for (let i = 0; i < anchors.length - 1; i++) {
        if (cur < centers[i + 1]) {
          const t = smooth(0.3, 0.9, (cur - centers[i]) / (centers[i + 1] - centers[i]));
          const a = f(i);
          const b = f(i + 1);
          return { x: lerp(a.x, b.x, t), y: lerp(a.y, b.y, t), w: lerp(a.w, b.w, t) };
        }
      }
      return f(anchors.length - 1);
    };

    let rect = frameRect();
    let lens = { x: vw * 0.5, y: vh * 0.7 };
    let raf = 0;
    const t0 = performance.now();
    const draw = (now: number) => {
      raf = requestAnimationFrame(draw);
      if (document.hidden) return;
      const target = frameRect();
      const k = reduce ? 1 : 0.16;
      rect = { x: lerp(rect.x, target.x, k), y: lerp(rect.y, target.y, k), w: lerp(rect.w, target.w, k) };

      const t = (now - t0) / 1000;
      const idle = now - pointer.at > 2600 || !pointer.inside;
      let tx: number;
      let ty: number;
      if (idle) {
        const cx = rect.x + rect.w * 0.5;
        const cy = Math.min(Math.max(rect.y + rect.w * 0.45, vh * 0.25), vh * 0.8);
        const amp = Math.min(rect.w * 0.2, vw * 0.22);
        tx = cx + (reduce ? 0 : Math.sin(t * 0.37) * amp);
        ty = cy + (reduce ? 0 : Math.sin(t * 0.53 + 1.3) * amp * 0.45);
      } else {
        tx = pointer.x;
        ty = pointer.y;
      }
      const lk = idle ? 0.03 : 0.17;
      lens = { x: lerp(lens.x, tx, lk), y: lerp(lens.y, ty, lk) };

      const reveal = loaded ? (reduce ? 1 : smooth(0, 1, (now - loadedAt) / 900)) : 0;
      gl.uniform2f(u.res, canvas.width, canvas.height);
      gl.uniform3f(u.rect, rect.x * dpr, rect.y * dpr, rect.w * dpr);
      gl.uniform2f(u.mouse, lens.x * dpr, lens.y * dpr);
      gl.uniform1f(u.lens, Math.min(Math.max(Math.min(vw, vh) * 0.26, 130), 280) * dpr);
      gl.uniform1f(u.cell, Math.max(2, Math.round(2 * dpr)));
      gl.uniform1f(u.time, t);
      gl.uniform1f(u.reveal, reveal);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    };
    raf = requestAnimationFrame(draw);

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      window.removeEventListener("pointermove", onMove);
      image.onload = null;
      gl.deleteTexture(tex);
      gl.deleteBuffer(buf);
      gl.deleteProgram(prog);
    };
  }, [src, mode]);

  return (
    <div className={mode === "page" ? "stage stage-page" : "stage stage-banner"} aria-hidden>
      <canvas ref={canvasRef} />
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img ref={fallbackRef} src={src} alt="" className="stage-fallback" />
    </div>
  );
}
