//! The fluid sim's GLSL ES 1.00 passes (`crate::fluid`), each declaring what
//! the `//_DEFINES_` variants expect; `size` is the input's texels.

use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, UniformName, UniformType,
};

use crate::fluid_gl::{self, Prog};

const HEAD: &str = r#"#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform vec2 size;
"#;

/// Bilinear lookup of sampler `NAME` with `n` texels, in full float precision.
const BILERP: &str = r#"
vec4 bilerp_NAME(vec2 uv, vec2 n) {
    vec2 st = uv * n - 0.5;
    vec2 i = floor(st);
    vec2 f = st - i;
    vec2 a = (i + 0.5) / n;
    vec2 b = (i + 1.5) / n;
    return mix(mix(texture2D(NAME, a), texture2D(NAME, vec2(b.x, a.y)), f.x),
               mix(texture2D(NAME, vec2(a.x, b.y)), texture2D(NAME, b), f.x), f.y);
}
"#;

/// One force's change, blended onto its grid: dye plus glitter points in A (`dye` 1), or velocity
/// with A the pull toward it. Columns: (end px, radius px, kind), (cells/s, rad/s), dye, (start px).
const SPLAT: &str = r#"
uniform mat4 force;
uniform float cell;
uniform float dye;
uniform float seed;
uniform float glitter;
//_HASH_

void main() {
    vec2 p = v_coords * size * cell;
    mat4 f = force;
    vec2 d = p - f[0].xy;
    // A splat narrower than a texel would fall between texel centres.
    float r = max(f[0].z, cell);
    vec4 c = vec4(0.0);
    if (f[0].w < 0.5) {
        vec2 ab = f[0].xy - f[3].xy;
        d -= ab * (clamp(dot(p - f[3].xy, ab) / max(dot(ab, ab), 0.0001), 0.0, 1.0) - 1.0);
        float dd = dot(d, d) / (r * r);
        // As far as the draw's rect reaches (`SPLAT_REACH`).
        float g = dd > 9.0 ? 0.0 : exp(-dd);
        if (dye > 0.5) {
            c.rgb = f[2].rgb * g;
            vec2 at = floor(v_coords * size) + f[0].xy + seed;
            float amount = min(max(f[2].r, max(f[2].g, f[2].b)), 1.0);
            // Each hash steps by about 1/1000, too coarse for one chance this small.
            float chance = sqrt(glitter * g * amount);
            c.a = glitter > 0.0 && hash(at) < chance && hash(at + 71.3) < chance ? 1.0 : 0.0;
        } else {
            c = vec4(f[1].xy * g, 0.0, g);
        }
    } else {
        float k = 1.0 - smoothstep(0.8 * r, r, length(d));
        // Screen y points down: (d.y, -d.x) turns counter-clockwise.
        c.xy = f[1].z * vec2(d.y, -d.x) / cell * k;
    }
    gl_FragColor = c;
}
"#;

const CURL: &str = r#"
void main() {
    vec2 dx = vec2(1.0 / size.x, 0.0);
    vec2 dy = vec2(0.0, 1.0 / size.y);
    float dvdx = texture2D(tex, v_coords + dx).y - texture2D(tex, v_coords - dx).y;
    float dudy = texture2D(tex, v_coords + dy).x - texture2D(tex, v_coords - dy).x;
    gl_FragColor = vec4(0.5 * (dvdx - dudy), 0.0, 0.0, 1.0);
}
"#;

/// Vorticity confinement: pushes along N x curl, N the unit gradient of |curl|.
const VORTICITY: &str = r#"
uniform sampler2D curl;
uniform float strength;
uniform float dt;

void main() {
    vec2 dx = vec2(1.0 / size.x, 0.0);
    vec2 dy = vec2(0.0, 1.0 / size.y);
    vec2 n = 0.5 * vec2(abs(texture2D(curl, v_coords + dx).x) - abs(texture2D(curl, v_coords - dx).x),
                        abs(texture2D(curl, v_coords + dy).x) - abs(texture2D(curl, v_coords - dy).x));
    n /= length(n) + 0.0001;
    float w = texture2D(curl, v_coords).x;
    vec4 v = texture2D(tex, v_coords);
    v.xy += strength * w * vec2(n.y, -n.x) * dt;
    gl_FragColor = v;
}
"#;

/// The edges are walls: past one, a neighbour mirrors this texel's velocity.
const DIVERGENCE: &str = r#"
void main() {
    vec2 dx = vec2(1.0 / size.x, 0.0);
    vec2 dy = vec2(0.0, 1.0 / size.y);
    vec2 c = texture2D(tex, v_coords).xy;
    float x0 = v_coords.x - dx.x < 0.0 ? -c.x : texture2D(tex, v_coords - dx).x;
    float x1 = v_coords.x + dx.x > 1.0 ? -c.x : texture2D(tex, v_coords + dx).x;
    float y0 = v_coords.y - dy.y < 0.0 ? -c.y : texture2D(tex, v_coords - dy).y;
    float y1 = v_coords.y + dy.y > 1.0 ? -c.y : texture2D(tex, v_coords + dy).y;
    gl_FragColor = vec4(0.5 * (x1 - x0 + y1 - y0), 0.0, 0.0, 1.0);
}
"#;

/// Damped Jacobi on pressure (`tex`) with a two-texel Laplacian, the one `DIVERGENCE` after
/// `GRADIENT` makes; undamped, the mode alternating every two texels never decays.
const JACOBI: &str = r#"
uniform sampler2D divergence;

void main() {
    vec2 dx = vec2(2.0 / size.x, 0.0);
    vec2 dy = vec2(0.0, 2.0 / size.y);
    float sum = texture2D(tex, v_coords - dx).x + texture2D(tex, v_coords + dx).x
              + texture2D(tex, v_coords - dy).x + texture2D(tex, v_coords + dy).x;
    float p = 0.25 * (sum - 4.0 * texture2D(divergence, v_coords).x);
    p = mix(texture2D(tex, v_coords).x, p, 0.8);
    gl_FragColor = vec4(p, 0.0, 0.0, 1.0);
}
"#;

const GRADIENT: &str = r#"
uniform sampler2D pressure;

void main() {
    vec2 dx = vec2(1.0 / size.x, 0.0);
    vec2 dy = vec2(0.0, 1.0 / size.y);
    vec2 g = 0.5 * vec2(texture2D(pressure, v_coords + dx).x - texture2D(pressure, v_coords - dx).x,
                        texture2D(pressure, v_coords + dy).x - texture2D(pressure, v_coords - dy).x);
    vec4 v = texture2D(tex, v_coords);
    v.xy -= g;
    gl_FragColor = v;
}
"#;

/// Semi-Lagrangian advection of `tex` by `velocity` (`sim` texels), each channel times `decay`;
/// `tex` by hand in full precision for the velocity's own slow drift.
const ADVECT: &str = r#"
uniform sampler2D velocity;
uniform vec2 sim;
uniform float dt;
uniform vec4 decay;
//_BILERP_tex_

void main() {
    vec2 v = texture2D(velocity, v_coords).xy;
    gl_FragColor = bilerp_tex(v_coords - dt * v / sim, size) * decay;
}
"#;

/// As `ADVECT` for the dye, filtered by the GPU.
const ADVECT_DYE: &str = r#"
uniform sampler2D velocity;
uniform vec2 sim;
uniform float dt;
uniform vec4 decay;

void main() {
    vec2 v = texture2D(velocity, v_coords).xy;
    gl_FragColor = texture2D(tex, v_coords - dt * v / sim) * decay;
}
"#;

/// A hash of `p` in 0..1.
const HASH: &str = r#"
float hash(vec2 p) {
    vec3 q = fract(vec3(p.xyx) * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}
"#;

/// Look pass inputs: dye `tex` (`size` texels), `velocity` (`sim` texels), `sparks`, `time` (s),
/// `tile`. Writes added light in RGB and backdrop cover in A; with `light`, a backdrop multiplier.
const LOOK: &str = r#"
uniform sampler2D velocity;
uniform vec2 sim;
uniform float time;
uniform float tile;
uniform float light;
uniform sampler2D sparks;

vec4 dye(vec2 o) {
    return texture2D(tex, v_coords + o / size);
}

float peak(vec3 c) {
    return max(c.r, max(c.g, c.b));
}

vec3 cover(vec3 c) {
    return 1.0 - exp(-2.0 * c);
}

// The dye's hue with its brightest channel at 1.
vec3 hue(vec3 c) {
    return c / max(peak(c), 0.0001);
}

// How sharply the covered dye changes here: its outlines.
float edge(float k) {
    vec3 gx = cover(dye(vec2(1.0, 0.0)).rgb * k) - cover(dye(vec2(-1.0, 0.0)).rgb * k);
    vec3 gy = cover(dye(vec2(0.0, 1.0)).rgb * k) - cover(dye(vec2(0.0, -1.0)).rgb * k);
    return 0.5 * sqrt(dot(gx, gx) + dot(gy, gy));
}

// Steps at a faint density and at four and sixteen times it, so dense dye keeps outlines.
float bands(vec2 o) {
    float m = peak(dye(o).rgb);
    return smoothstep(0.08, 0.3, m) + smoothstep(0.8, 1.6, m) + smoothstep(3.2, 6.4, m);
}

// Thin lines where the dye crosses a band: the blob's outlines.
float contour() {
    vec2 g = 0.5 * vec2(bands(vec2(1.0, 0.0)) - bands(vec2(-1.0, 0.0)),
                        bands(vec2(0.0, 1.0)) - bands(vec2(0.0, -1.0)));
    return min(length(g), 1.0);
}

vec2 slope() {
    float x = peak(dye(vec2(1.0, 0.0)).rgb) - peak(dye(vec2(-1.0, 0.0)).rgb);
    float y = peak(dye(vec2(0.0, 1.0)).rgb) - peak(dye(vec2(0.0, -1.0)).rgb);
    return vec2(x, y);
}

vec3 rainbow(float t) {
    return 0.5 + 0.5 * cos(6.2832 * (t + vec3(0.0, 0.33, 0.67)));
}

// The dye's hue deepened, and dimmed to a third as it nears white: many dyes mixed sum toward
// white, and this keeps them coloured.
vec3 deep(vec3 c) {
    vec3 h = hue(c);
    return h * h * mix(0.35, 1.0, 1.0 - min(h.r, min(h.g, h.b)));
}

// No dye here or next to it worth a look, nor a glitter point here: the look of empty dye.
bool empty() {
    float m = max(max(peak(dye(vec2(0.0)).rgb), peak(dye(vec2(1.0, 0.0)).rgb)),
                  max(peak(dye(vec2(-1.0, 0.0)).rgb),
                      max(peak(dye(vec2(0.0, 1.0)).rgb), peak(dye(vec2(0.0, -1.0)).rgb))));
    return m < 0.0001 && dye(vec2(0.0)).a < 0.005;
}
"#;

const INK: &str = r#"
void main() {
    if (empty()) {
        gl_FragColor = light > 0.5 ? vec4(1.0, 1.0, 1.0, 0.0) : vec4(0.0);
        return;
    }
    vec3 c = dye(vec2(0.0)).rgb;
    vec2 s = slope();
    vec3 n = normalize(vec3(s, 0.08));
    float shade = clamp(dot(n, normalize(vec3(-0.4, -0.6, 1.0))) + 0.75, 0.75, 1.1);
    float m = peak(c);
    if (light > 0.5) {
        // Tone-mapped by the peak, so dense dye stays its deepest hue instead of black.
        float d = 1.5 * (1.0 - exp(-m / 1.5));
        vec3 t = pow(max(hue(c), 0.001), vec3(5.0 * d)) * exp(-0.5 * d) * shade;
        gl_FragColor = vec4(min(t, vec3(1.0)), 0.0);
    } else {
        vec3 t = deep(c) * (1.0 - exp(-m)) * shade * 1.6;
        gl_FragColor = vec4(t, 0.6 * (1.0 - exp(-3.0 * m)));
    }
}
"#;

const GLOW: &str = r#"
void main() {
    if (empty()) {
        gl_FragColor = vec4(0.0);
        return;
    }
    vec3 c = dye(vec2(0.0)).rgb;
    float m = peak(c);
    float e = min(contour() * 1.5 + edge(1.5) * 0.6, 1.5);
    vec2 s = slope();
    float turn = atan(s.y, s.x) / 6.2832 + 0.05 * time + 0.6 * m;
    vec3 rim = mix(hue(c), rainbow(turn), 0.9 * smoothstep(0.2, 0.9, m));
    rim = mix(rim, vec3(1.0), 0.2);
    vec3 inside = hue(c) * 0.06 * (1.0 - exp(-m));
    gl_FragColor = vec4(rim * e * 2.2 + inside, 0.85 * (1.0 - exp(-4.0 * m)));
}
"#;

/// A glitter point's light where the dye's alpha peaks over its eight neighbours:
/// advection blurs each point, so its peak is where it still is.
const SPARK: &str = r#"
float spark() {
    float a = dye(vec2(0.0)).a;
    if (a < 0.005) {
        return 0.0;
    }
    float cross = max(max(dye(vec2(1.0, 0.0)).a, dye(vec2(-1.0, 0.0)).a),
                      max(dye(vec2(0.0, 1.0)).a, dye(vec2(0.0, -1.0)).a));
    // Most texels lose to a nearest neighbour, and need not read the corners.
    if (a <= cross) {
        return 0.0;
    }
    float corner = max(max(dye(vec2(1.0, 1.0)).a, dye(vec2(-1.0, 1.0)).a),
                       max(dye(vec2(1.0, -1.0)).a, dye(vec2(-1.0, -1.0)).a));
    return a > corner ? smoothstep(0.005, 0.04, a) : 0.0;
}
"#;

const SPARKS: &str = r#"
void main() {
    if (empty()) {
        gl_FragColor = vec4(0.0);
        return;
    }
    vec4 c = dye(vec2(0.0));
    float m = peak(c.rgb);
    vec3 lit = hue(c.rgb) * (0.1 * (1.0 - exp(-2.0 * m)) + contour() * 1.4 + edge(2.0) * 0.8);
    lit += mix(hue(c.rgb), vec3(1.0), 0.6) * spark() * 5.0;
    gl_FragColor = vec4(lit, 0.4 * (1.0 - exp(-3.0 * m)));
}
"#;

/// Neon's sparks, streaked along the flow by the look pass.
const SPARKLE: &str = r#"
void main() {
    vec3 c = dye(vec2(0.0)).rgb;
    float s = spark();
    gl_FragColor = vec4(hue(c) * s, s);
}
"#;

const NEON: &str = r#"
void main() {
    vec2 v = texture2D(velocity, v_coords).xy / sim;
    // Dashes stay visible in slow flow and short in fast flow: 2 to 24 texels.
    float speed = length(v * size);
    vec2 back = v / max(length(v), 0.000001) * clamp(speed * 0.08, 2.0, 24.0) / size / 5.0;
    vec3 dash = vec3(0.0);
    for (int i = 0; i < 6; i++) {
        dash += texture2D(sparks, v_coords - back * float(i)).rgb;
    }
    // 2.5 times the sparks' light, tone-mapped by its peak so piled-up dashes keep their colour.
    dash = deep(dash) * (1.0 - exp(-2.5 * peak(dash) / 3.0)) * 3.0;
    // The dye is smooth along a dash, so three reads average it.
    vec3 body = (texture2D(tex, v_coords).rgb + texture2D(tex, v_coords - back * 2.5).rgb
                 + texture2D(tex, v_coords - back * 5.0).rgb) / 3.0;
    vec2 p = v_coords * size;
    vec2 cell = floor(p / tile);
    vec2 q = p - (cell + 0.5) * tile;
    float side = 0.5 * tile - 0.1 * tile;
    float round_r = 0.22 * tile;
    float d = length(max(abs(q) - (side - round_r), 0.0)) - round_r;
    float outline = 1.0 - smoothstep(0.2, 0.9, abs(d));
    vec3 under = texture2D(tex, (cell + 0.5) * tile / size).rgb;
    vec3 lit = vec3(0.015, 0.02, 0.045) + deep(under) * (1.0 - exp(-3.0 * peak(under))) * 0.55;
    vec3 c = dash + deep(body) * (1.0 - exp(-2.0 * peak(body))) * 0.6 + lit * outline;
    gl_FragColor = vec4(c, 0.3 * (1.0 - exp(-2.0 * peak(body))));
}
"#;

/// Downsamples `tex` (`size` texels) by four bilinear taps, keeping only light
/// above `threshold`.
const DOWN: &str = r#"
uniform float threshold;

void main() {
    vec2 o = 1.0 / size;
    vec3 c = 0.25 * (texture2D(tex, v_coords + vec2(-o.x, -o.y)).rgb
                   + texture2D(tex, v_coords + vec2(o.x, -o.y)).rgb
                   + texture2D(tex, v_coords + vec2(-o.x, o.y)).rgb
                   + texture2D(tex, v_coords + vec2(o.x, o.y)).rgb);
    float b = max(c.r, max(c.g, c.b));
    c *= max(b - threshold, 0.0) / max(b, 0.0001);
    gl_FragColor = vec4(c, 1.0);
}
"#;

/// A nine-texel Gaussian along `dir` (texels) in five bilinear taps.
const BLUR: &str = r#"
uniform vec2 dir;

void main() {
    vec2 o1 = dir * 1.3846153846 / size;
    vec2 o2 = dir * 3.2307692308 / size;
    vec3 c = texture2D(tex, v_coords).rgb * 0.2270270270
           + (texture2D(tex, v_coords + o1).rgb + texture2D(tex, v_coords - o1).rgb) * 0.3162162162
           + (texture2D(tex, v_coords + o2).rgb + texture2D(tex, v_coords - o2).rgb) * 0.0702702703;
    gl_FragColor = vec4(c, 1.0);
}
"#;

/// The fluid element: the wallpaper (`tex`) dimmed toward `backdrop` by `dim`,
/// then `display` and its bloom scaled by `level`.
const DRAW: &str = r#"
uniform sampler2D display;
uniform sampler2D bloom0;
uniform sampler2D bloom1;
uniform vec3 backdrop;
uniform float dim;
uniform float level;
uniform float light;
uniform vec2 bloom;

void main() {
    vec4 w = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    w.a = 1.0;
#endif
    vec3 base = mix(w.rgb, backdrop, dim);
    vec4 d = texture2D(display, v_coords);
    vec3 c;
    if (light > 0.5) {
        c = base * mix(vec3(1.0), d.rgb, level);
    } else {
        vec3 e = d.rgb + texture2D(bloom0, v_coords).rgb * bloom.x
               + texture2D(bloom1, v_coords).rgb * bloom.y;
        c = base * (1.0 - d.a * level) + (1.0 - exp(-e)) * level;
    }
    gl_FragColor = vec4(clamp(c, 0.0, 1.0), 1.0) * w.a * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0) {
        gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
    }
#endif
}
"#;

#[derive(Clone, Debug)]
pub(crate) struct Programs {
    pub(crate) splat: Prog,
    pub(crate) curl: Prog,
    pub(crate) vorticity: Prog,
    pub(crate) divergence: Prog,
    pub(crate) jacobi: Prog,
    pub(crate) gradient: Prog,
    pub(crate) advect: Prog,
    pub(crate) advect_dye: Prog,
    pub(crate) ink: Prog,
    pub(crate) glow: Prog,
    pub(crate) sparks: Prog,
    pub(crate) sparkle: Prog,
    pub(crate) neon: Prog,
    pub(crate) down: Prog,
    pub(crate) blur: Prog,
    /// Drawn by the renderer in the output's frame, so one of its programs.
    pub(crate) draw: GlesTexProgram,
}

pub(crate) fn compile(r: &mut GlesRenderer) -> Result<Programs, GlesError> {
    let source = |body: &str| {
        format!("{HEAD}{body}")
            .replace("//_DEFINES_", "")
            .replace("//_BILERP_tex_", &BILERP.replace("NAME", "tex"))
            .replace("//_HASH_", HASH)
    };
    let u = UniformName::new;
    let draw = r.compile_custom_texture_shader(
        format!("{HEAD}{DRAW}"),
        &[
            u("size", UniformType::_2f),
            u("display", UniformType::_1i),
            u("bloom0", UniformType::_1i),
            u("bloom1", UniformType::_1i),
            u("backdrop", UniformType::_3f),
            u("dim", UniformType::_1f),
            u("level", UniformType::_1f),
            u("light", UniformType::_1f),
            u("bloom", UniformType::_2f),
        ],
    )?;
    r.with_context(|gl| -> Result<_, GlesError> {
        let link = |body: &str| fluid_gl::link(gl, &source(body));
        Ok(Programs {
            splat: link(SPLAT)?,
            curl: link(CURL)?,
            vorticity: link(VORTICITY)?,
            divergence: link(DIVERGENCE)?,
            jacobi: link(JACOBI)?,
            gradient: link(GRADIENT)?,
            advect: link(ADVECT)?,
            advect_dye: link(ADVECT_DYE)?,
            ink: link(&format!("{LOOK}{INK}"))?,
            glow: link(&format!("{LOOK}{GLOW}"))?,
            sparks: link(&format!("{LOOK}{SPARK}{SPARKS}"))?,
            sparkle: link(&format!("{LOOK}{SPARK}{SPARKLE}"))?,
            neon: link(&format!("{LOOK}{NEON}"))?,
            down: link(DOWN)?,
            blur: link(BLUR)?,
            draw,
        })
    })?
}
