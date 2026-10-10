/** Soft glass shader shared by the conversation entrance. CSS remains the fallback. */
export function attachSoftGlass(btn) {
  const events = new AbortController();
  const listen = (target, type, handler) => target.addEventListener(type, handler, {signal: events.signal});
  const light = btn.dataset.glass === 'light';
  const always = 'glassAlways' in btn.dataset;      // glass from the start, never white again; a pointer stirs the water
  // The button's own white, read before the glass makes its background transparent.
  const face = (getComputedStyle(btn).backgroundColor.match(/[\d.]+/g) || [244, 244, 244]).slice(0, 3).map((x) => +x / 255);
  const fine = matchMedia('(hover: hover) and (pointer: fine)');
  const still = matchMedia('(prefers-reduced-motion: reduce)');
  const canvas = document.createElement('canvas');
  canvas.setAttribute('aria-hidden', 'true');
  const gl = canvas.getContext('webgl2', { alpha: true, premultipliedAlpha: true, antialias: false });
  if (!gl) { document.documentElement.classList.add('no-webgl'); return; }
  btn.prepend(canvas);

  const VS = `#version 300 es
  in vec2 a; out vec2 v;
  void main() { v = a * .5 + .5; v.y = 1. - v.y; gl_Position = vec4(a, 0., 1.); }`;
  const FS = `#version 300 es
  precision highp float;
  in vec2 v; out vec4 o;
  uniform vec2 uSize;      // button, CSS px
  uniform float uDpr, uTime, uRadius, uMotion, uSpeed, uStrength, uOutline, uPrism, uAlpha;
  uniform vec3 uAccent;
  uniform sampler2D uCov;  // distance (CSS px) to where the current front started: its point and the face it already had
  uniform vec4 uEv;        // the current front: x, y, start (s), 1 = to glass / 0 = back to white
  uniform vec4 uP[8];      // pulses: x, y, start (s), amplitude
  uniform vec4 uLabel;     // the label's box, CSS px: x0, y0, x1, y1
  uniform float uTone;     // 0 = black glass, 1 = white glass (the button keeps its white)
  uniform vec3 uFace;      // the button's own white, as it is at rest
  const float TAU = 6.2831853;

  float sdBox(vec2 p, vec2 b, float r) { vec2 q = abs(p) - b + r; return length(max(q, 0.)) + min(max(q.x, q.y), 0.) - r; }

  // The water surface is one height field h (CSS px), kept with its slope g. Everything that moves is read from it:
  // the reflection bends with the slope, glints sit where the slope faces the light, the outline's tint follows it.
  struct W { float h; vec2 g; };
  void addWave(inout W w, vec2 p, vec2 dir, float len, float amp, float ph, float t) {
    float k = TAU / len;
    float om = sqrt(25. * k);                 // deep-water dispersion: longer waves travel faster
    vec2 kv = dir * k;
    float a = dot(kv, p) - om * t + ph;
    float s = sin(a), c = cos(a);
    w.h += amp * s;
    w.g += amp * c * kv;
  }
  // The swell: six waves from one side, spread over +-75 degrees, 28-110 px long, each about as steep as the next.
  W swell(vec2 p, float t) {
    W w = W(0., vec2(0.));
    addWave(w, p, normalize(vec2(-1., .25)), 110., 1.00, 0.0, t);
    addWave(w, p, normalize(vec2(-.82, .82)), 80., .75, 1.7, t);
    addWave(w, p, normalize(vec2(-.86, -.5)), 62., .60, 4.1, t);
    addWave(w, p, normalize(vec2(-.34, .94)), 47., .45, 2.6, t);
    addWave(w, p, normalize(vec2(-.5, -.87)), 36., .33, 5.3, t);
    addWave(w, p, normalize(vec2(-.97, .26)), 28., .25, .9, t);
    return w;
  }
  // A ring from a pulse: its envelope travels with the glass front, its crests more slowly (as small ripples on
  // water do), so a point sees about three crests a second rather than a flicker.
  float ring(float d, float dt, float amp) {
    float x = uSpeed * dt - d;                // distance behind the envelope's front
    if (x <= 0.) return 0.;
    return amp * sin((d - 150. * dt) * TAU / 50.) * exp(-x / 120.) * smoothstep(0., 12., x) * exp(-dt * .9);
  }
  void addRings(inout W w, vec2 p) {
    for (int i = 0; i < 8; i++) {
      vec4 P = uP[i];
      if (P.w <= 0.) continue;
      float dt = uTime - P.z;
      if (dt < 0. || dt > 4.) continue;
      vec2 r = p - P.xy;
      float d = max(length(r), .5);
      vec2 u = r / d;
      float f0 = ring(d, dt, P.w), fp = ring(d + 1., dt, P.w), fm = ring(max(d - 1., 0.), dt, P.w);
      w.h += f0;
      w.g += (fp - fm) * .5 * u;
    }
  }
  // Glass or white: the front's face spreads at the front's speed from where it started (the pointer's point and the part
  // of the button that already showed it), with an edge that wavers outside that part.
  float coverage(vec2 p) {
    float dt = uTime - uEv.z, m;
    if (uMotion > .5) {
      float d = min(texture(uCov, p / uSize).r, length(p - uEv.xy));
      float wob = (3. * sin(p.x * .07 + uTime * 1.6) + 2. * sin(p.y * .23 - uTime * 2.2) + 1.5 * sin((p.x + p.y) * .11 + uTime * 1.1)) * smoothstep(0., 12., d);
      m = 1. - smoothstep(-4., 4., d + wob - uSpeed * dt);
    } else {
      m = clamp(dt / .2, 0., 1.);
    }
    return uEv.w > .5 ? m : 1. - m;
  }
  // Thin-film colour from the film's thickness, per channel; mixed towards white so it tints rather than paints.
  vec3 film(float h) {
    float d = 400. + 120. * h;                // nm
    vec3 ph = 4. * 3.14159265 * 1.33 * d / vec3(620., 540., 460.);
    return mix(vec3(1.), (.5 - .5 * cos(ph)) * 1.35, .55);
  }
  // The studio, as seen in the glass: one large soft panel over the upper left with a sharp lower edge, like the
  // plates beside the button, and the thin bright line of a window just past it.
  float studio(vec2 q) {
    vec2 a = vec2(0., uSize.y * .64), b = vec2(uSize.x * .56, 0.);
    vec2 nrm = normalize(vec2(b.y - a.y, b.x - a.x) * vec2(-1., 1.));
    float e = dot(q - a, nrm);                // < 0 inside the panel
    float panel = (1. - smoothstep(-.9, .9, e)) * mix(.05, .15, smoothstep(-46., -2., e));
    float along = dot(q - a, normalize(b - a)) / length(b - a);
    float line = exp(-pow((e - 7.) / .9, 2.)) * smoothstep(.04, .22, along) * (1. - smoothstep(.62, .9, along)) * .32;
    return panel + line;
  }

  void main() {
    vec2 p = v * uSize;
    float d = sdBox(p - uSize * .5, uSize * .5, uRadius);
    float shape = clamp(.5 - d * uDpr, 0., 1.);
    if (shape <= 0.) { o = vec4(0.); return; }

    float c = coverage(p);
    float t = uTime * uMotion * .52;
    W w = swell(p, t);
    w.h *= c * uStrength; w.g *= c * uStrength;                      // the swell lives on the glass only
    addRings(w, p);
    vec3 n = normalize(vec3(-w.g, 1.));
    float wet = smoothstep(.6, 1., c);       // the water shows once a cell has turned dark

    vec2 lc = (uLabel.xy + uLabel.zw) * .5, lb = (uLabel.zw - uLabel.xy) * .5;
    float tz = 1. - smoothstep(0., 10., sdBox(p - lc, lb + 4., 4.));
    float rim = smoothstep(-1.8, -.5, d) * (1. - smoothstep(-.5, .5, d));
    float lit = .35 + .65 * clamp(dot(normalize(-(p - uSize * .5)), vec2(.4, .8)) * .5 + .5, 0., 1.);
    float react = smoothstep(.03, .18, length(w.g));
    vec3 g;
    if (uTone > .5) {
      // White glass: the button keeps its white. The water shows as soft light and shade on its slopes (lit from the
      // upper left), a faint thin-film colour where it tilts, and a grey outline that takes the tint where waves pass.
      // The mark stays dark on it, so the area behind it is kept calm.
      g = uFace * .975 + vec3(.012) * (1. - v.y);
      g += vec3(dot(n.xy, normalize(vec2(-.4, -.75))) * .38) * wet;
      g = mix(g, g * film(w.h), .16 * uPrism * react * wet);
      g = mix(g, uFace * .975, tz * .7);
      g = mix(g, mix(uAccent * .9, uAccent * film(w.h), .22 * uPrism * react), rim * (.32 + .3 * lit) * uOutline);
    } else {
      // Black glass, as dark as the plates' body. Face-on it reflects little of the studio; where the water tilts it,
      // more, as glass does. Glints: small and sharp, where the surface leans to face the light (from the upper left).
      vec3 refl = vec3(studio(p + n.xy * 40.)) * 1.7 * mix(.45, 1., smoothstep(.04, .5, length(n.xy)));
      vec2 dn = n.xy - vec2(-.07, -.12);
      float glint = exp(-dot(dn, dn) / (.03 * .03)) * .38;
      g = vec3(.010, .010, .012) + vec3(.006) * (1. - v.y);
      g += (refl + glint) * wet;
      // Behind the label the glass stays dark and quiet, so the white label (difference blend) keeps its contrast.
      g = mix(g, min(g, vec3(.22)), tz);
      g = mix(g, vec3(dot(g, vec3(.3333))), tz * .6);
      // A thin outline all round, brighter towards the light. White while the water is calm; where the surface tilts
      // (rings passing, the swell's slopes) it takes the thin-film tint, so it answers the water. Drawn after the quiet
      // zone, which reaches the top and bottom edges.
      g += vec3(.9) * rim * lit * mix(vec3(1.), film(w.h), .5 * react);
    }

    // White face, as the button is at rest.
    vec3 wht = uFace;
    vec3 col = mix(wht, g, c);
    // The front: a faint shadow on the white side and a thin edge on the glass side, like a water's edge (bright on
    // black glass, a soft dark line on white glass).
    col -= vec3(.05) * smoothstep(.04, .25, c) * (1. - smoothstep(.25, .45, c));
    col += vec3(uTone > .5 ? -.12 : .32) * smoothstep(.45, .75, c) * (1. - smoothstep(.75, .95, c));
    float alpha = mix(1., uAlpha, c) * shape;
    o = vec4(clamp(col, 0., 1.) * alpha, alpha);
  }`;

  const sh = (type, src) => { const s = gl.createShader(type); gl.shaderSource(s, src); gl.compileShader(s); if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s)); return s; };
  const prog = gl.createProgram();
  try { gl.attachShader(prog, sh(gl.VERTEX_SHADER, VS)); gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FS)); } catch (err) { console.error(err); canvas.remove(); document.documentElement.classList.add('no-webgl'); return; }
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) { console.error(gl.getProgramInfoLog(prog)); canvas.remove(); document.documentElement.classList.add('no-webgl'); return; }
  gl.useProgram(prog);
  const buf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
  const aLoc = gl.getAttribLocation(prog, 'a');
  gl.enableVertexAttribArray(aLoc);
  gl.vertexAttribPointer(aLoc, 2, gl.FLOAT, false, 0, 0);
  const U = {};
  ['uSize', 'uDpr', 'uTime', 'uRadius', 'uMotion', 'uSpeed', 'uCov', 'uP', 'uLabel', 'uEv', 'uTone', 'uFace', 'uStrength', 'uOutline', 'uPrism', 'uAlpha', 'uAccent'].forEach((k) => { U[k] = gl.getUniformLocation(prog, k); });
  const tex = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  gl.uniform1i(U.uCov, 0);

  const SPEED = 360;        // px/s, the glass front and the rings it carries
  const CELL = 3;           // px, roughly: the cell of the distance field's grid
  let W = 0, H = 0, cols = 0, rows = 0, cw = CELL, ch = CELL;
  let dist = null, far = 0; // the current front's distance field (CSS px) and its largest value
  let ev = null;            // the current front: { glass, x, y, t }
  const pulses = [];        // { x, y, t, a }
  let raf = 0, live = false, last = { x: 0, y: 0, t: -1 };
  const clock0 = performance.now();
  const now = () => (performance.now() - clock0) / 1000;
  const profile = () => btn.dataset.glassProfile || 'soft';
  const motion = () => (still.matches || profile() === 'still' ? 0 : 1);
  const active = () => !document.hidden && W > 0 && H > 0 && !btn.disabled && !btn.closest('[inert]') && !!btn.getClientRects().length;
  const smooth = (a, b, x) => { const s = Math.min(1, Math.max(0, (x - a) / (b - a))); return s * s * (3 - 2 * s); };

  // A front grows at a steady speed from where its face already is: from the pointer's point, and from every part of
  // the button that already shows that face. So re-entering soon after leaving takes back the white that was left at
  // once, from its edges, instead of waiting for a circle from the pointer to cross the glass that never left.
  // The shader draws it per pixel from a distance field; this is the same rule on the CPU, for the field and for
  // knowing when to stop. Its edge wavers, but not inside the face it starts from.
  const wobble = (x, y, d, t) => (3 * Math.sin(x * .07 + t * 1.6) + 2 * Math.sin(y * .23 - t * 2.2) + 1.5 * Math.sin((x + y) * .11 + t * 1.1)) * smooth(0, 12, d);
  const reach = (k, x, y, t) => {
    if (!motion()) return Math.min(1, Math.max(0, (t - ev.t) / .2));
    const d = Math.min(dist[k], Math.hypot(x - ev.x, y - ev.y));
    return 1 - smooth(-4, 4, d + wobble(x, y, d, t) - SPEED * (t - ev.t));
  };
  const glassAt = (k, x, y, t) => (ev ? (ev.glass ? reach(k, x, y, t) : 1 - reach(k, x, y, t)) : 0);
  const upload = () => { gl.bindTexture(gl.TEXTURE_2D, tex); gl.texImage2D(gl.TEXTURE_2D, 0, gl.R16F, cols, rows, 0, gl.RED, gl.FLOAT, dist); };
  const begin = (glass, x, y, t) => {
    const n = cols * rows, D = new Float32Array(n), BIG = 1e4, dg = Math.hypot(cw, ch);
    // 0 where the button already shows the face this front brings, else the distance to it (two-pass chamfer).
    for (let j = 0; j < rows; j++) for (let i = 0; i < cols; i++) {
      const k = j * cols + i, cx = (i + .5) * cw, cy = (j + .5) * ch;
      const g = dist ? glassAt(k, cx, cy, t) : 0;
      D[k] = (glass ? g >= .5 : g < .5) ? 0 : BIG;
    }
    const at = (i, j) => (i < 0 || j < 0 || i >= cols || j >= rows ? BIG : D[j * cols + i]);
    for (let j = 0; j < rows; j++) for (let i = 0; i < cols; i++) {
      const k = j * cols + i;
      D[k] = Math.min(D[k], at(i - 1, j) + cw, at(i, j - 1) + ch, at(i - 1, j - 1) + dg, at(i + 1, j - 1) + dg);
    }
    for (let j = rows - 1; j >= 0; j--) for (let i = cols - 1; i >= 0; i--) {
      const k = j * cols + i;
      D[k] = Math.min(D[k], at(i + 1, j) + cw, at(i, j + 1) + ch, at(i + 1, j + 1) + dg, at(i - 1, j + 1) + dg);
    }
    far = 0;
    for (let j = 0; j < rows; j++) for (let i = 0; i < cols; i++) {
      const k = j * cols + i;
      D[k] = Math.min(D[k], Math.hypot((i + .5) * cw - x, (j + .5) * ch - y));
      far = Math.max(far, D[k]);
    }
    dist = D; ev = { glass, x, y, t };
    upload();
  };
  // Is the front still on its way across the button?
  const moving = (t) => (ev ? (motion() ? SPEED * (t - ev.t) < far + 16 : t - ev.t < .2) : false);

  const size = () => {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    // The padding box, unrounded: a button's own border stays outside the canvas.
    const r = btn.getBoundingClientRect(), cs = getComputedStyle(btn);
    W = Math.max(0, r.width - parseFloat(cs.borderLeftWidth) - parseFloat(cs.borderRightWidth));
    H = Math.max(0, r.height - parseFloat(cs.borderTopWidth) - parseFloat(cs.borderBottomWidth));
    if (!W || !H) return;
    canvas.width = Math.max(1, Math.round(W * dpr)); canvas.height = Math.max(1, Math.round(H * dpr));
    const nc = Math.max(1, Math.round(W / CELL)), nr = Math.max(1, Math.round(H / CELL));
    if (nc !== cols || nr !== rows) {
      // A new size settles the current front where it was going.
      cols = nc; rows = nr; cw = W / cols; ch = H / rows;
      dist = new Float32Array(cols * rows); far = 0;
      upload();
    } else { cw = W / cols; ch = H / rows; }
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.uniform1f(U.uDpr, dpr);
  };

  const draw = (t) => {
    const P = new Float32Array(32);
    pulses.forEach((p, i) => { if (i < 8) P.set([p.x, p.y, p.t, p.a * motion()], i * 4); });
    gl.uniform4fv(U.uP, P);
    gl.uniform4f(U.uEv, ev ? ev.x : 0, ev ? ev.y : 0, ev ? ev.t : 0, ev && ev.glass ? 1 : 0);
    gl.uniform2f(U.uSize, W, H);
    gl.uniform1f(U.uTime, t);
    gl.uniform1f(U.uMotion, motion());
    const crisp = profile() === 'crisp';
    gl.uniform1f(U.uStrength, crisp ? .9 : .45);
    gl.uniform1f(U.uOutline, crisp ? 1.2 : .75);
    gl.uniform1f(U.uPrism, crisp ? .85 : .35);
    gl.uniform1f(U.uAlpha, crisp ? .95 : .9);
    gl.uniform3f(U.uAccent, 125 / 255, 134 / 255, 226 / 255);
    gl.uniform1f(U.uSpeed, SPEED);
    // A radius of 50% reads as 50 here; no corner can be rounder than half the button.
    gl.uniform1f(U.uRadius, Math.min(parseFloat(getComputedStyle(btn).borderTopLeftRadius) || 0, Math.min(W, H) / 2));
    gl.uniform1f(U.uTone, light ? 1 : 0);
    gl.uniform3f(U.uFace, face[0], face[1], face[2]);
    const br = btn.getBoundingClientRect(), lr = (btn.querySelector('.label') || btn).getBoundingClientRect();
    const ox = br.left + btn.clientLeft, oy = br.top + btn.clientTop;
    gl.uniform4f(U.uLabel, lr.left - ox, lr.top - oy, lr.right - ox, lr.bottom - oy);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.clearColor(0, 0, 0, 0); gl.clear(gl.COLOR_BUFFER_BIT);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    // Keep drawing while the front moves, and while the glass shows water that moves.
    return moving(t) || (ev && ev.glass && motion() > 0);
  };

  const loop = () => {
    raf = 0;
    if (!active()) return;
    const keep = draw(now());
    if (keep) raf = requestAnimationFrame(loop);
    else if (!(ev && ev.glass)) { live = false; btn.classList.remove('is-live'); }   // back at rest: hand over to the CSS button
  };
  const wake = () => {
    if (!active()) return;
    if (!live) { live = true; draw(now()); btn.classList.add('is-live'); }
    if (!raf) raf = requestAnimationFrame(loop);
  };
  const pulse = (x, y, a) => { pulses.unshift({ x, y, t: now(), a }); pulses.length = Math.min(pulses.length, 8); };
  const local = (e) => { const r = btn.getBoundingClientRect(); return { x: e.clientX - r.left - btn.clientLeft, y: e.clientY - r.top - btn.clientTop }; };

  const enter = (x, y) => { begin(true, x, y, now()); pulse(x, y, 1.6); last = { x, y, t: now() }; wake(); };
  const leave = (x, y) => { begin(false, x, y, now()); pulse(x, y, .9); wake(); };

  size();
  const refresh = () => { size(); if (active()) wake(); };

  const resize = new ResizeObserver(refresh); resize.observe(btn);
  const intersection = new IntersectionObserver(entries => { if (entries[0].isIntersecting) refresh(); }); intersection.observe(btn);
  const mutation = new MutationObserver(refresh); mutation.observe(btn, { attributes: true, attributeFilter: ['disabled', 'data-glass-profile'] });
  listen(still, 'change', refresh);
  listen(document, 'visibilitychange', refresh);
  // data-glass-always: the glass spreads once from the middle when the page opens and stays; the water keeps moving.
  if (always) { begin(true, W / 2, H / 2, now()); wake(); }

  listen(btn, 'pointerenter', (e) => {
    if (e.pointerType !== 'mouse' && !fine.matches) return;
    const p = local(e);
    if (always) { pulse(p.x, p.y, 1.2); last = { x: p.x, y: p.y, t: now() }; wake(); return; }
    enter(p.x, p.y);
  });
  // Glass while the pointer is over the button or it has keyboard focus: losing one keeps it while the other holds.
  // The white comes back from where the pointer crossed the edge: a fast pointer is first seen far outside, and a front
  // starting there would take seconds to arrive, so the point is brought to the nearest place on the button.
  listen(btn, 'pointerleave', (e) => {
    if (always || !ev || !ev.glass || btn.matches(':focus-visible')) return;
    const p = local(e);
    leave(Math.min(Math.max(p.x, 0), W), Math.min(Math.max(p.y, 0), H));
  });
  listen(btn, 'pointermove', (e) => {
    if (!ev || !ev.glass) return;
    const p = local(e), t = now();
    if (Math.hypot(p.x - last.x, p.y - last.y) > 16 && t - last.t > .18) { pulse(p.x, p.y, .8); last = { x: p.x, y: p.y, t }; wake(); }
  });
  listen(btn, 'pointerdown', (e) => { if (!ev || !ev.glass) return; const p = local(e); pulse(p.x, p.y, 2); wake(); });
  // Keyboard: focus turns it to glass from the middle, and losing focus brings the white back unless the pointer is on it.
  listen(btn, 'focus', () => { if (btn.matches(':focus-visible')) enter(W / 2, H / 2); });
  listen(btn, 'blur', () => { if (!always && ev && ev.glass && !btn.matches(':hover')) leave(W / 2, H / 2); });
  listen(canvas, 'webglcontextlost', (e) => { e.preventDefault(); cancelAnimationFrame(raf); raf = 0; live = false; btn.classList.remove('is-live'); canvas.remove(); document.documentElement.classList.add('no-webgl'); });
  return () => {
    events.abort(); resize.disconnect(); intersection.disconnect(); mutation.disconnect();
    cancelAnimationFrame(raf); gl.deleteTexture(tex); gl.deleteBuffer(buf); gl.deleteProgram(prog);
    canvas.remove(); btn.classList.remove('is-live');
  };
}
