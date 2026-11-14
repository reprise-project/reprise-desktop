// MS-RDPEGFX 3.3.8.3.2 views (3.3.8.3.3 with AUX_V2) as RGBA texels of four luma bytes or two
// U,V pairs. Every value of encode/colour.rs's integer maths stays below 2^24, so floats repeat it.
precision highp float;
precision highp int;

uniform highp sampler2D src;
// The picture's size; the views past it repeat its last column and row.
uniform ivec2 size;
// Where an unscaled picture starts in the frame.
uniform ivec2 origin;
// colour.rs's matrix rows in 16.16 fixed point, and the luma of black.
uniform vec3 ky;
uniform vec3 ku;
uniform vec3 kv;
uniform float y_offset;
// Where clients read a v1 auxiliary view; elsewhere it repeats the main view.
uniform ivec4 read_rect;
// The views' width.
uniform int view_width;

layout(location = 0) out vec4 out0;
layout(location = 1) out vec4 out1;
layout(location = 2) out vec4 out2;

vec3 rgb(ivec2 at) {
#ifdef SCALED
    // A frame of another size, sampled linearly where the picture's pixel centres fall on it.
    vec2 centre = (vec2(clamp(at, ivec2(0), size - 1)) + 0.5) / vec2(size);
    return floor(texture(src, centre).rgb * 255.0 + 0.5);
#else
    return floor(texelFetch(src, origin + clamp(at, ivec2(0), size - 1), 0).rgb * 255.0 + 0.5);
#endif
}

float row(float offset, vec3 k, vec3 c) {
    return clamp(offset + floor((dot(k, c) + 32768.0) / 65536.0), 0.0, 255.0);
}

float luma(vec3 c) {
    return row(y_offset, ky, c);
}

vec2 uv(vec3 c) {
    return vec2(row(128.0, ku, c), row(128.0, kv, c));
}

bool readable(ivec2 at) {
    return all(greaterThanEqual(at, read_rect.xy)) && all(lessThan(at, read_rect.zw));
}

#ifdef AUX_V2
// The left half holds the odd columns' U, the right half their V.
float aux_luma(ivec2 at, float main_luma) {
    int halfw = view_width / 2;
    bool is_v = at.x >= halfw;
    vec2 c = uv(rgb(ivec2(2 * (is_v ? at.x - halfw : at.x) + 1, at.y)));
    return is_v ? c.y : c.x;
}

// Chroma sample `cx` of chroma row `row`: the odd row's U at two even columns in the first
// quarter, V in the second.
vec2 aux_chroma(int cx, int row) {
    int quarter = view_width / 4;
    bool is_v = cx >= quarter;
    ivec2 p = ivec2(4 * (is_v ? cx - quarter : cx), 2 * row + 1);
    vec2 a = uv(rgb(p));
    vec2 b = uv(rgb(p + ivec2(2, 0)));
    return is_v ? vec2(a.y, b.y) : vec2(a.x, b.x);
}
#else
// Rows 0 to 7 of each macroblock row hold its odd U rows, rows 8 to 15 its odd V rows.
float aux_luma(ivec2 at, float main_luma) {
    if (!readable(at)) {
        return main_luma;
    }
    int within = at.y % 16;
    vec2 c = uv(rgb(ivec2(at.x, at.y - within + 2 * (within % 8) + 1)));
    return within < 8 ? c.x : c.y;
}
#endif

void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
#if PASS == 0
    // Both views' luma.
    ivec2 p = ivec2(at.x * 4, at.y);
    vec4 m = vec4(luma(rgb(p)), luma(rgb(p + ivec2(1, 0))), luma(rgb(p + ivec2(2, 0))),
        luma(rgb(p + ivec2(3, 0))));
    vec4 a = vec4(aux_luma(p, m.x), aux_luma(p + ivec2(1, 0), m.y),
        aux_luma(p + ivec2(2, 0), m.z), aux_luma(p + ivec2(3, 0), m.w));
    out0 = m / 255.0;
    out1 = a / 255.0;
#elif PASS == 1
    // Both views' chroma for two 2x2 blocks, and a flag where both have one colour. The client's
    // reverse filter undoes the main view's rounded average; a v1 aux view keeps the top right.
    vec4 m;
    vec4 x;
    bool same = true;
    for (int i = 0; i < 2; i++) {
        ivec2 p = ivec2(at.x * 4 + i * 2, at.y * 2);
        vec2 a = uv(rgb(p));
        vec2 b = uv(rgb(p + ivec2(1, 0)));
        vec2 c = uv(rgb(p + ivec2(0, 1)));
        vec2 d = uv(rgb(p + ivec2(1, 1)));
        vec2 average = floor((a + b + c + d + 2.0) / 4.0);
#ifdef AUX_V2
        vec2 kept = aux_chroma(at.x * 2 + i, at.y);
#else
        vec2 kept = readable(p) ? b : average;
#endif
        same = same && a == b && a == c && a == d;
        if (i == 0) {
            m.xy = average;
            x.xy = kept;
        } else {
            m.zw = average;
            x.zw = kept;
        }
    }
    out0 = m / 255.0;
    out1 = x / 255.0;
    out2 = vec4(same ? 1.0 : 0.0, 0.0, 0.0, 1.0);
#elif PASS == 2
    // A flag per macroblock from pass 1's texture, where a macroblock is 4 texels wide, 8 tall.
    bool same = true;
    for (int y = 0; y < 8; y++) {
        for (int x = 0; x < 4; x++) {
            same = same && texelFetch(src, ivec2(at.x * 4 + x, at.y * 8 + y), 0).r > 0.5;
        }
    }
    out0 = vec4(same ? 1.0 : 0.0, 0.0, 0.0, 1.0);
#endif
}
