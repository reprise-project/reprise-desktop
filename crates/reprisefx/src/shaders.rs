//! The GLSL ES 1.00 programs, as Smithay custom texture shaders: the blur's
//! Gaussian pass, the glass that lays the blurred backdrop into its shape and
//! lifts it for the chrome's glass, the 3D switcher's tilted card, the colour
//! filter and desktop water's step, normals and draw.
//!
//! Smithay replaces the `//_DEFINES_` line with its variant defines
//! (`EXTERNAL`, `NO_ALPHA`, `DEBUG_FLAGS`) and every variant must compile, so
//! each shader declares the sampler and uniforms those defines expect.

use smithay::backend::renderer::gles::{
    GlesError, GlesRenderer, GlesTexProgram, UniformName, UniformType,
};

/// One pass of a separable Gaussian along `dir`. Texels `2i+1` and `2i+2` either
/// side are read as one linear lookup at their weighted centre, so `pairs`
/// lookups each way cover `2·pairs` texels; weights are renormalised over the kernel.
const BLUR_FRAG: &str = r#"#version 100
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
// One texel along this pass's axis, in texture coordinates.
uniform vec2 dir;
// The Gaussian's sigma, in texels.
uniform float sigma;
// Linear lookups on either side; at most 16.
uniform float pairs;

void main() {
    float k = -0.5 / (sigma * sigma);
    vec4 sum = texture2D(tex, v_coords);
    float total = 1.0;
    for (int i = 0; i < 16; i++) {
        if (float(i) >= pairs) {
            break;
        }
        float a = float(2 * i + 1);
        float b = a + 1.0;
        float wa = exp(a * a * k);
        float wb = exp(b * b * k);
        float w = wa + wb;
        float o = (a * wa + b * wb) / w;
        sum += (texture2D(tex, v_coords + dir * o) + texture2D(tex, v_coords - dir * o)) * w;
        total += 2.0 * w;
    }
    gl_FragColor = sum / total * alpha;
}
"#;

/// The glass over one shape: the blurred backdrop B (streaks already in it)
/// lifted per pixel for the chrome's glass laid over it
/// (`reprisekit::backdrop::Lift`):
///
/// ```text
/// out = B + gain·(C·luma(B) - mean·B - (C - mean)/2)
/// ```
///
/// inside the shape's rounded rectangle (anti-aliased over one pixel by its
/// distance field), nothing outside. The caller cuts holes by leaving them out
/// of the drawn damage.
///
/// Molten first moves the lookup out along the lens box's normal by
/// `strength · (1 - smoothstep(0, band, depth))²`, red a little farther and
/// blue less, and saturates what it reads. After the lift the edge darkens a
/// little just inside and the highlight (rim and glow, brightest where the
/// edge faces the light) screens over it (`crate::lens`).
const GLASS_SRC: &str = r#"#version 100
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
// Where the backdrop texture lies, in the shape's own framebuffer pixels
// (turned with the output, as the texture is): its top-left (xy) and size
// (zw). A texture coordinate t is the shape-local point xy + t * zw.
uniform vec4 cap;
// The shape's size in framebuffer pixels.
uniform vec2 size;
// Corner radii in framebuffer pixels: top-left, top-right, bottom-right,
// bottom-left of the shape as it lies in the framebuffer.
uniform vec4 radii;
// The glass colour C, 0..1.
uniform vec3 colour;
// The lift's mean of C and gain; a gain of 0 shows the backdrop alone.
uniform vec2 lift;
// Which material: 0 Frosted, 1 Smoked, 2 Molten. Molten bends `uv` first.
uniform float material;
// Molten's lens: band, strength, rim, scale (crate::lens::Lens).
uniform vec4 lens;
// The box whose edges bend, shape-local: top-left (xy) and size (zw).
uniform vec4 lens_box;

float sd_round_box(vec2 p, vec2 half_size, vec4 r) {
    float rr = p.x > 0.0 ? (p.y > 0.0 ? r.z : r.y) : (p.y > 0.0 ? r.w : r.x);
    vec2 q = abs(p) - half_size + rr;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - rr;
}

float sd_lens(vec2 p) {
    vec2 h = lens_box.zw * 0.5;
    return sd_round_box(p - lens_box.xy - h, h, radii);
}

void main() {
    vec2 p = cap.xy + v_coords * cap.zw;
    float d = sd_round_box(p - size * 0.5, size * 0.5, radii);
    float cover = clamp(0.5 - d, 0.0, 1.0);
    vec3 b;
    float light = 0.0;
    float shade = 0.0;
    if (material > 1.5) {
        // The outward normal from the distance field's gradient (GLSL ES
        // 1.00 has no dFdx), and how deep inside the bending edge.
        vec2 e = vec2(0.5, 0.0);
        vec2 n = vec2(sd_lens(p + e.xy) - sd_lens(p - e.xy),
                      sd_lens(p + e.yx) - sd_lens(p - e.yx));
        n /= max(length(n), 1e-4);
        float depth = max(-sd_lens(p), 0.0);
        float t = 1.0 - smoothstep(0.0, lens.x, depth);
        vec2 bend = n * (lens.y * t * t) / cap.zw;
        b = vec3(texture2D(tex, v_coords + bend * (1.0 + DISPERSION)).r,
                 texture2D(tex, v_coords + bend).g,
                 texture2D(tex, v_coords + bend * (1.0 - DISPERSION)).b);
        float g = dot(b, vec3(0.299, 0.587, 0.114));
        b = clamp(g + (b - g) * VIBRANCE, 0.0, 1.0);
        vec2 to_light = vec2(-sin(radians(LIGHT_ANGLE)), -cos(radians(LIGHT_ANGLE)));
        float facing = dot(n, to_light);
        float spec = SPEC_FLOOR
            + (1.0 - SPEC_FLOOR) * pow(max(abs(facing), 1e-4), SPEC_POWER);
        if (facing < 0.0) {
            spec *= SPEC_BACK;
        }
        float glow = 1.0 - smoothstep(0.0, GLOW * lens.w, depth);
        light = lens.z * spec
            * (exp(-depth / (RIM_FADE * lens.w)) + GLOW_SHARE * glow);
        float s = (depth - SHADE_AT * lens.w) / (SHADE_WIDTH * lens.w);
        shade = SHADE * exp(-s * s);
    } else {
        b = texture2D(tex, v_coords).rgb;
    }
    float l = dot(b, vec3(0.299, 0.587, 0.114));
    vec3 c = clamp(b + lift.y * (colour * l - lift.x * b - (colour - lift.x) * 0.5), 0.0, 1.0);
    c *= 1.0 - shade;
    vec3 hue = b / max(max(b.r, max(b.g, b.b)), 1e-3);
    vec3 hl = clamp(light, 0.0, 1.0) * mix(vec3(1.0), hue, SPEC_TINT);
    c = 1.0 - (1.0 - c) * (1.0 - hl);
    gl_FragColor = vec4(c, 1.0) * cover * alpha;
}
"#;

/// The glass program with `crate::lens`'s constants in place of their names.
fn glass_frag() -> String {
    use crate::lens::*;
    [
        ("DISPERSION", DISPERSION),
        ("VIBRANCE", VIBRANCE),
        ("LIGHT_ANGLE", LIGHT_ANGLE),
        ("SPEC_POWER", SPEC_POWER),
        ("SPEC_FLOOR", SPEC_FLOOR),
        ("SPEC_BACK", SPEC_BACK),
        ("SPEC_TINT", SPEC_TINT),
        ("RIM_FADE", RIM_FADE),
        ("GLOW_SHARE", GLOW_SHARE),
        ("GLOW", GLOW),
        ("SHADE_AT", SHADE_AT),
        ("SHADE_WIDTH", SHADE_WIDTH),
        ("SHADE", SHADE),
    ]
    .iter()
    .fold(GLASS_SRC.to_string(), |src, (name, v)| {
        src.replace(name, &format!("{v:.4}"))
    })
}

/// A picture laid on a plane in perspective. The quad drawn is the card's
/// bounds on screen; each pixel maps back to the picture through the inverse
/// homography (rows `hx`, `hy`, `hw`, output pixels to texels) and shows
/// nothing off the picture. The edge is anti-aliased over one screen pixel.
/// A texel the picture leaves partly clear (a frame's glass, drawn without
/// its backdrop) fades toward clear: of alpha `a` it keeps `mix(glass, 1, a)`
/// of itself. The card's outermost screen pixel all round is a black outline
/// at `rim`, at any size (a frame's own outline doesn't survive minifying).
/// The picture is read before any pixel is discarded, so its mipmaps get the
/// derivatives of whole pixel quads.
const FLIP_FRAG: &str = r#"#version 100
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
// Where the quad lies on the output, in pixels: top-left (xy), size (zw).
uniform vec4 dst;
// The inverse homography's rows.
uniform vec3 hx;
uniform vec3 hy;
uniform vec3 hw;
// The picture's size in texels.
uniform vec2 tex_size;
// How much of a translucent texel shows at alpha 0, rising to all at 1.
uniform float glass;
// The outline's alpha: black over the picture's outermost screen pixel.
uniform float rim;

vec2 to_tex(vec2 p) {
    vec3 q = vec3(p, 1.0);
    return vec2(dot(hx, q), dot(hy, q)) / dot(hw, q);
}

void main() {
    vec2 p = dst.xy + v_coords * dst.zw;
    vec2 uv = to_tex(p);
    float span = max(length(to_tex(p + vec2(1.0, 0.0)) - uv),
                     length(to_tex(p + vec2(0.0, 1.0)) - uv));
    vec2 edge = min(uv, tex_size - uv);
    // Screen pixels from the pixel's centre in to the nearest edge.
    float inside = min(edge.x, edge.y) / max(span, 1e-4);
    float cover = clamp(inside + 0.5, 0.0, 1.0);
    vec4 c = texture2D(tex, clamp(uv / tex_size, 0.0, 1.0));
    if (cover <= 0.0) {
        discard;
    }
    c *= mix(glass, 1.0, c.a);
    // The outline covers the band one screen pixel in from the edge.
    float line = rim * clamp(1.5 - inside, 0.0, 1.0);
    c = c * (1.0 - line) + vec4(0.0, 0.0, 0.0, line);
    gl_FragColor = c * cover * alpha;
}
"#;

/// The packed field's helpers: a height in -1..1 as 16 bits in two 8-bit
/// channels (high, low) and back. `pack_dithered` rounds up with probability
/// equal to the fraction cut off (`r` uniform in 0..1), so a height too small
/// to change by half a step still falls on average.
const WATER_PACK: &str = r#"
vec2 pack_bits(float u) {
    float hi = floor(u / 256.0);
    return vec2(hi, u - hi * 256.0) / 255.0;
}
// The arithmetic stays about 0, where a float keeps its low bits: about
// 32767.5 a float holds only 1/256 of a step, and that rounding, the
// same way every step, piles up in the field as a drifting mean.
vec2 pack_dithered(float h, float r) {
    return pack_bits(clamp(floor(h * 32767.5 + 0.5 + r), -32767.0, 32768.0) + 32767.0);
}
vec2 pack(float h) {
    return pack_dithered(h, 0.5);
}
float unpack(vec2 c) {
    vec2 b = floor(c * 255.0 + 0.5);
    return ((b.x - 128.0) * 256.0 + b.y + 0.5) / 32767.5;
}
"#;

/// Desktop water's step: each texel's next height from its four neighbours
/// and previous height, damped, plus up to four capsule splashes (`seg*` ends
/// in texels, `radius` and `depth` one lane each; depth 0 is no splash).
/// Writes (next, current): next rounded with a dither from the texel and
/// `seed` (new each step), current as read, plus the splashes.
///
/// `next = d * (2h + c2 * lap) - d^2 * prev`, with `lap` the neighbours' sum
/// less 4h and `c2` the wave speed squared (`reprisekit::water::Step`): at 0.5
/// it is `d * (sum / 2 - d * prev)`. `prev` is damped too, so every wave's
/// height falls by `damping` per step (`reprisekit::water::Energy` takes
/// `damping` squared). A splash goes into both lanes: it lifts the water at
/// rest rather than pushing it, so the mean height doesn't drift from 0.
const WATER_STEP_FRAG: &str = r#"#version 100
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
uniform float damping;
uniform float c2;
uniform float seed;
uniform vec4 seg0;
uniform vec4 seg1;
uniform vec4 seg2;
uniform vec4 seg3;
uniform vec4 radius;
uniform vec4 depth;
//_PACK_

float at(vec2 uv) { return unpack(texture2D(tex, uv).rg); }

// A value in 0..1 from a texel and the step's seed.
float dither(vec2 p) {
    vec3 q = fract(vec3(p, seed) * 0.1031);
    q += dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

float splash(vec2 p, vec4 s, float r, float d) {
    if (d == 0.0) { return 0.0; }
    vec2 ab = s.zw - s.xy;
    float l = dot(ab, ab);
    float t = l > 0.0 ? clamp(dot(p - s.xy, ab) / l, 0.0, 1.0) : 0.0;
    float k = min(length(p - (s.xy + ab * t)) / r, 1.0);
    return d * 0.5 * (1.0 + cos(3.14159265 * k));
}

void main() {
    vec4 c = texture2D(tex, v_coords);
    float h = unpack(c.rg);
    float prev = unpack(c.ba);
    vec2 dx = vec2(1.0 / size.x, 0.0);
    vec2 dy = vec2(0.0, 1.0 / size.y);
    float sum = at(v_coords + dx) + at(v_coords - dx) + at(v_coords + dy) + at(v_coords - dy);
    float n = damping * ((2.0 - 4.0 * c2) * h + c2 * sum) - damping * damping * prev;
    vec2 p = v_coords * size;
    float s = splash(p, seg0, radius.x, depth.x) + splash(p, seg1, radius.y, depth.y)
            + splash(p, seg2, radius.z, depth.z) + splash(p, seg3, radius.w, depth.w);
    gl_FragColor = vec4(pack_dithered(n + s, dither(p)), pack(h + s));
}
"#;

/// Desktop water's normals: the field's slope times `gain` in R and G, and the
/// light the slope catches in B, each as 128 + 127 * v so a flat field is exactly (128, 128, 128).
const WATER_NORMALS_FRAG: &str = r#"#version 100
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
uniform float gain;
//_PACK_

float at(vec2 uv) { return unpack(texture2D(tex, uv).rg); }
float lane(float v) { return (128.0 + 127.0 * clamp(v, -1.0, 1.0)) / 255.0; }

void main() {
    vec2 dx = vec2(1.0 / size.x, 0.0);
    vec2 dy = vec2(0.0, 1.0 / size.y);
    vec2 g = vec2(at(v_coords + dx) - at(v_coords - dx), at(v_coords + dy) - at(v_coords - dy)) * gain;
    // Light from the top left.
    float light = dot(g, vec2(-0.7071, -0.7071));
    gl_FragColor = vec4(lane(g.x), lane(g.y), lane(light), 1.0);
}
"#;

/// Desktop water's draw: the wallpaper (`tex`) looked up `refraction` (texture
/// units) along the slope from the normals texture on unit 1, lightened or darkened by `shine`.
const WATER_DRAW_FRAG: &str = r#"#version 100
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
uniform sampler2D normals;
uniform vec2 refraction;
uniform float shine;

vec3 lanes(vec3 c) { return (floor(c * 255.0 + 0.5) - 128.0) / 127.0; }

void main() {
    vec3 n = lanes(texture2D(normals, v_coords).rgb);
    vec4 c = texture2D(tex, v_coords + n.xy * refraction);
#if defined(NO_ALPHA)
    c.a = 1.0;
#endif
    c.rgb = clamp(c.rgb + n.z * shine * c.a, 0.0, c.a);
    gl_FragColor = c * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0) {
        gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
    }
#endif
}
"#;

pub(crate) fn compile_water(
    renderer: &mut GlesRenderer,
) -> Result<(GlesTexProgram, GlesTexProgram, GlesTexProgram), GlesError> {
    let with_pack = |src: &str| src.replace("//_PACK_", WATER_PACK);
    let step = renderer.compile_custom_texture_shader(
        with_pack(WATER_STEP_FRAG),
        &[
            UniformName::new("size", UniformType::_2f),
            UniformName::new("damping", UniformType::_1f),
            UniformName::new("c2", UniformType::_1f),
            UniformName::new("seed", UniformType::_1f),
            UniformName::new("seg0", UniformType::_4f),
            UniformName::new("seg1", UniformType::_4f),
            UniformName::new("seg2", UniformType::_4f),
            UniformName::new("seg3", UniformType::_4f),
            UniformName::new("radius", UniformType::_4f),
            UniformName::new("depth", UniformType::_4f),
        ],
    )?;
    let normals = renderer.compile_custom_texture_shader(
        with_pack(WATER_NORMALS_FRAG),
        &[
            UniformName::new("size", UniformType::_2f),
            UniformName::new("gain", UniformType::_1f),
        ],
    )?;
    let draw = renderer.compile_custom_texture_shader(
        WATER_DRAW_FRAG,
        &[
            UniformName::new("normals", UniformType::_1i),
            UniformName::new("refraction", UniformType::_2f),
            UniformName::new("shine", UniformType::_1f),
        ],
    )?;
    Ok((step, normals, draw))
}

/// The screen through a colour filter: each texel's straight colour through
/// the affine map whose rows are `row_r`, `row_g`, `row_b` (fourth column the
/// offset), clamped, alpha kept; in linear light while `linear_light` is 1
/// (`reprisekit::colorfilter::Matrix::apply`).
const COLOR_FRAG: &str = r#"#version 100
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
uniform vec4 row_r;
uniform vec4 row_g;
uniform vec4 row_b;
uniform float linear_light;

vec3 to_linear(vec3 c) {
    return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(0.04045, c));
}

vec3 to_srgb(vec3 c) {
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

void main() {
    vec4 c = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    c.a = 1.0;
#endif
    vec3 rgb = c.a > 0.0 ? c.rgb / c.a : vec3(0.0);
    if (linear_light == 1.0) {
        rgb = to_linear(rgb);
    }
    vec4 v = vec4(rgb, 1.0);
    vec3 o = clamp(vec3(dot(row_r, v), dot(row_g, v), dot(row_b, v)), 0.0, 1.0);
    if (linear_light == 1.0) {
        o = to_srgb(o);
    }
    gl_FragColor = vec4(o * c.a, c.a) * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0) {
        gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
    }
#endif
}
"#;

pub(crate) fn compile_color(renderer: &mut GlesRenderer) -> Result<GlesTexProgram, GlesError> {
    renderer.compile_custom_texture_shader(
        COLOR_FRAG,
        &[
            UniformName::new("row_r", UniformType::_4f),
            UniformName::new("row_g", UniformType::_4f),
            UniformName::new("row_b", UniformType::_4f),
            UniformName::new("linear_light", UniformType::_1f),
        ],
    )
}

pub(crate) fn compile_flip(renderer: &mut GlesRenderer) -> Result<GlesTexProgram, GlesError> {
    renderer.compile_custom_texture_shader(
        FLIP_FRAG,
        &[
            UniformName::new("dst", UniformType::_4f),
            UniformName::new("hx", UniformType::_3f),
            UniformName::new("hy", UniformType::_3f),
            UniformName::new("hw", UniformType::_3f),
            UniformName::new("tex_size", UniformType::_2f),
            UniformName::new("glass", UniformType::_1f),
            UniformName::new("rim", UniformType::_1f),
        ],
    )
}

#[derive(Clone, Debug)]
pub(crate) struct Programs {
    pub blur: GlesTexProgram,
    pub glass: GlesTexProgram,
}

impl Programs {
    pub fn compile(renderer: &mut GlesRenderer) -> Result<Programs, GlesError> {
        let blur = renderer.compile_custom_texture_shader(
            BLUR_FRAG,
            &[
                UniformName::new("dir", UniformType::_2f),
                UniformName::new("sigma", UniformType::_1f),
                UniformName::new("pairs", UniformType::_1f),
            ],
        )?;
        let glass = renderer.compile_custom_texture_shader(
            glass_frag(),
            &[
                UniformName::new("cap", UniformType::_4f),
                UniformName::new("size", UniformType::_2f),
                UniformName::new("radii", UniformType::_4f),
                UniformName::new("colour", UniformType::_3f),
                UniformName::new("lift", UniformType::_2f),
                UniformName::new("material", UniformType::_1f),
                UniformName::new("lens", UniformType::_4f),
                UniformName::new("lens_box", UniformType::_4f),
            ],
        )?;
        Ok(Programs { blur, glass })
    }
}
