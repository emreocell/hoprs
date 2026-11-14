// GLSL ES 3.00 for the fluid passes and the glyph display. One full-screen triangle per pass.

const HEAD = "#version 300 es\nprecision highp float;\nprecision highp sampler2D;\n";

export const VERT = `${HEAD}out vec2 vUv;
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  vUv = p;
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

const FRAG = `${HEAD}in vec2 vUv;\nout vec4 o;\n`;

// Neighbour samples on the sim grid.
const NEIGHBOURS = `uniform vec2 uTexel;
#define L(t) texture(t, vUv - vec2(uTexel.x, 0.0))
#define R(t) texture(t, vUv + vec2(uTexel.x, 0.0))
#define B(t) texture(t, vUv - vec2(0.0, uTexel.y))
#define T(t) texture(t, vUv + vec2(0.0, uTexel.y))
`;

export const SPLAT = `${FRAG}uniform sampler2D uTarget;
uniform float uAspect, uRadius;
uniform vec2 uPoint;
uniform vec3 uValue;
void main() {
  vec2 p = vUv - uPoint;
  p.x *= uAspect;
  o = vec4(texture(uTarget, vUv).xyz + exp(-dot(p, p) / uRadius) * uValue, 1.0);
}`;

export const ADVECT = `${FRAG}uniform sampler2D uVelocity, uSource;
uniform vec2 uTexel;
uniform float uDt, uFade;
void main() {
  vec2 from = vUv - uDt * texture(uVelocity, vUv).xy * uTexel;
  o = texture(uSource, from) / (1.0 + uFade * uDt);
}`;

export const DIVERGENCE = `${FRAG}${NEIGHBOURS}uniform sampler2D uVelocity;
void main() {
  vec2 c = texture(uVelocity, vUv).xy;
  float l = vUv.x - uTexel.x < 0.0 ? -c.x : L(uVelocity).x;
  float r = vUv.x + uTexel.x > 1.0 ? -c.x : R(uVelocity).x;
  float b = vUv.y - uTexel.y < 0.0 ? -c.y : B(uVelocity).y;
  float t = vUv.y + uTexel.y > 1.0 ? -c.y : T(uVelocity).y;
  o = vec4(0.5 * (r - l + t - b), 0.0, 0.0, 1.0);
}`;

export const SCALE = `${FRAG}uniform sampler2D uSource;
uniform float uValue;
void main() { o = uValue * texture(uSource, vUv); }`;

export const PRESSURE = `${FRAG}${NEIGHBOURS}uniform sampler2D uPressure, uDivergence;
void main() {
  float d = texture(uDivergence, vUv).x;
  o = vec4((L(uPressure).x + R(uPressure).x + B(uPressure).x + T(uPressure).x - d) * 0.25, 0.0, 0.0, 1.0);
}`;

export const GRADIENT = `${FRAG}${NEIGHBOURS}uniform sampler2D uPressure, uVelocity;
void main() {
  vec2 g = 0.5 * vec2(R(uPressure).x - L(uPressure).x, T(uPressure).x - B(uPressure).x);
  o = vec4(texture(uVelocity, vUv).xy - g, 0.0, 1.0);
}`;

// Per character cell: sample dye, quantise like levelFor, draw the cell's glyph from the atlas in the grey ramp.
// Dim cells (levels 1 and 2 lit by dye alone) draw at 62% size; the field at rest keeps full size, as the <pre>.
export const DISPLAY = `${FRAG}uniform sampler2D uDye, uAtlas;
uniform highp usampler2D uCells;
uniform vec2 uGrid, uCanvas;
uniform float uSlots;
uniform vec3 uRamp[8];
void main() {
  vec2 px = vec2(gl_FragCoord.x, uCanvas.y - gl_FragCoord.y);
  vec2 pos = px / (uCanvas / uGrid);
  vec2 cell = floor(pos);
  vec2 inner = pos - cell;
  uvec2 info = texelFetch(uCells, ivec2(cell), 0).rg;
  bool rest = info.g > 0u;
  float d = texture(uDye, vec2((cell.x + 0.5) / uGrid.x, 1.0 - (cell.y + 0.5) / uGrid.y)).r;
  float level = floor(clamp(d, 0.0, 1.0) * 8.0 + 0.5);
  if (rest) level = max(level, 1.0);
  if (level < 1.0) { o = vec4(0.0); return; }
  float size = (level <= 2.0 && !rest) ? 0.62 : 1.0;
  vec2 g = (inner - 0.5) / size + 0.5;
  if (g.x < 0.0 || g.y < 0.0 || g.x > 1.0 || g.y > 1.0) { o = vec4(0.0); return; }
  float a = texture(uAtlas, vec2((float(info.r) + g.x) / uSlots, g.y)).a;
  o = vec4(uRamp[int(level) - 1] * a, a);
}`;
