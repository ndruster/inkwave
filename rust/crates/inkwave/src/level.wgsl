// Task 52 batch D (D-T52-4) - full level surface + wet-ink MaterialExtension shader.
//
// Replaces the simplified `ink.wgsl` fragment surface once D2 wires `LevelExt`
// (this batch is shader text only; `ink.wgsl` stays untouched until then).
//
// JS sources ported in full (line anchors inline as `≡ <file> L##`):
//   * src/world/texlib.js       L1173-1326  TEXLIB_GLSL runtime (texlibSample
//     PLAIN/GRID/HEX anti-tiling + Toksvig + detail/macro/specularAA/coverage)
//   * src/world/levelMaterial.js L44-71/L153-233 uniforms+pars, L234-607
//     color_fragment injection (see-through, texlib lookup, modelled detail,
//     murals), L608-613 rough/metal, L614-693 normal block (gel / swim wakes /
//     ripples / relief), L12-24 lamp loop, L694-711 lighting tail + clamp
//   * src/world/inkShading.js   L22-314 the 9 INK_* chunks + uniform table
//
// Declared deviations (spec.md D-T52-7; REVIEW_CHECKLIST registered):
//   ② clearcoat: Bevy StandardMaterial has no clearcoat layer here. The wet
//     coat of INK_LIGHTS (inkShading.js L261-279) is approximated on the single
//     GGX lobe: the Tokuyoshi-Kaplanyan roughness widening is ported verbatim
//     onto perceptual_roughness, the coat lobe becomes a min(rough, sqrt(cr))
//     dip, and specularColor/F0 taming folds into `reflectance`. The
//     clearcoatNormal injection (levelMaterial.js L704-705) is a no-op (the
//     coat shares the perturbed normal, as JS already forces it).
//   ③ faceData/faceFlags/faceTan -> per-batch uniforms (BatchParams, binding
//     42): constant inside a box face, behaviour-equivalent. lamp/lightKind
//     bits are 0 (Task 53/55).
//   ④ lightUv -> paintUv: no lightmap atlas in Rust; the aomap_fragment block
//     (levelMaterial.js L696-703) stays data-inactive (u_ao = 0 default).
//   ⑤ gel/mural: code ported in full, default-inactive (u_gel = -1 sentinel
//     ≡ levelMaterial.js L100 `L.gel ?? -1`; u_mur = -1 ≡ vFaceFlags.z < 0).
//   INK_LIGHTS / INK_LIGHT_MAPS / INK_SHADE / LAMP_LIGHTS operate on three's
//     internal light loop (irradiance / clearcoatRadiance / reflectedLight),
//     which Bevy's forward `apply_pbr_lighting` does not expose. Their math is
//     preserved expression-for-expression but re-anchored onto the reachable
//     hooks: emissive + diffuse modulation (see each block's comment).
//   NUM_DIR_LIGHTS (inkShading.js L132-136): the sun direction is a uniform
//     (u_sun_dir) instead of three's directionalLights[0]; default is the same
//     #else fallback normalize(vec3(-0.41, 0.83, -0.38)).
//   The `if (false)` legacy procedural chain (levelMaterial.js L458-578) is
//     dead under USE_TEXLIB in JS and is not ported. The `#ifndef USE_TEXLIB`
//     corrugation fallbacks (L619-624, L208-217 gel-else) likewise.
//
// ---------------------------------------------------------------- binding contract (D2 AsBindGroup)
// group(3) = the merged material bind group (ExtendedMaterial appends the
// extension after StandardMaterial's own slots; 20/21 follow the InkExt
// precedent in ink_render.rs L39-48).
//   20 texture_2d<f32>       ink_atlas        ≡ uPaint        (levelMaterial L45 / L154)
//   21 sampler               ink_sampler      (linear, no wrap needed: rect-baked)
//   22 texture_2d_array<f32> t_albedo         ≡ tAlbedo       (L92/L204)
//   23 texture_2d_array<f32> t_normal         ≡ tNormal       (L93/L205)
//   24 texture_2d_array<f32> t_orm            ≡ tOrm          (L94/L206)
//   25 sampler               texlib_sampler   (repeat + mip-linear + aniso<=16)
//   26 texture_2d<f32>       mural_atlas      ≡ uMural        (L51/L160; ⑤ default null/white)
//   27 sampler               mural_sampler
//   28 array<vec4<f32>, 50>  uTL              ≡ uTL           (L96/L208)  (layer, 1/scale, mode, sym)
//   29 array<vec4<f32>, 50>  uTLt             ≡ uTLt          (L97/L209)  (tint, nrm, onWall, onTop)
//   30 array<vec4<f32>, 50>  uTLs             ≡ uTLs          (L99/L210)  (stair)
//   31 array<vec4<f32>, 24>  uRip             ≡ uRip          (inkShading L28/L51)
//   32 array<vec4<f32>, 24>  uRipP            ≡ uRipP         (inkShading L29/L52)
//   33 array<vec4<f32>, 48>  uWake            ≡ uWake         (levelMaterial L67/L174)
//   34 array<vec4<f32>, 4>   uWakeB           ≡ uWakeB        (L68/L175)
//   35 array<vec4<f32>, 4>   uSwimH           ≡ uSwimH        (L69/L176)
//   36 array<vec4<f32>, 4>   uSwimF           ≡ uSwimF        (L70/L177)
//   37 array<vec3<f32>, 12>  uLamps           ≡ uLamps        (L60/L163)
//   38 array<vec4<f32>, 12>  uMurA            ≡ uMurA         (L108/L213)
//   39 array<vec4<f32>, 12>  uMurB            ≡ uMurB         (L109/L214)
//   40 array<vec4<f32>, 12>  uMurC            ≡ uMurC vec2[12] (L110/L215; vec4-padded)
//   41 LevelParams  (per-frame material scalars/colours; member names ≡ JS uniforms)
//   42 BatchParams  (per batch, deviation ③)
//
// Mesh vertex semantics (world.rs D2): uv = UV_0 faceUv in world metres
// (shader divides by the 1/scale table), uv_b = UV_1 atlas paint rect (≡
// paintUv), color = per-block tint (≡ three vertexColors), world_position /
// world_normal as usual.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    mesh_view_bindings::view,
}

// ---------------------------------------------------------------- bindings
@group(3) @binding(20)
var ink_atlas: texture_2d<f32>;
@group(3) @binding(21)
var ink_sampler: sampler;

@group(3) @binding(22)
var t_albedo: texture_2d_array<f32>;
@group(3) @binding(23)
var t_normal: texture_2d_array<f32>;
@group(3) @binding(24)
var t_orm: texture_2d_array<f32>;
@group(3) @binding(25)
var texlib_sampler: sampler;

@group(3) @binding(26)
var mural_atlas: texture_2d<f32>;
@group(3) @binding(27)
var mural_sampler: sampler;

@group(3) @binding(28)
var<uniform> uTL: array<vec4<f32>, 50u>;    // ≡ levelMaterial.js L208 (TL_SLOTS = 50, L101)
@group(3) @binding(29)
var<uniform> uTLt: array<vec4<f32>, 50u>;   // ≡ L209
@group(3) @binding(30)
var<uniform> uTLs: array<vec4<f32>, 50u>;   // ≡ L210

@group(3) @binding(31)
var<uniform> uRip: array<vec4<f32>, 24u>;   // ≡ inkShading.js L51 (INK_RIPPLES = 24, L22)
@group(3) @binding(32)
var<uniform> uRipP: array<vec4<f32>, 24u>;  // ≡ inkShading.js L52

@group(3) @binding(33)
var<uniform> uWake: array<vec4<f32>, 48u>;  // ≡ levelMaterial.js L174
@group(3) @binding(34)
var<uniform> uWakeB: array<vec4<f32>, 4u>;  // ≡ L175
@group(3) @binding(35)
var<uniform> uSwimH: array<vec4<f32>, 4u>;  // ≡ L176
@group(3) @binding(36)
var<uniform> uSwimF: array<vec4<f32>, 4u>;  // ≡ L177

@group(3) @binding(37)
var<uniform> uLamps: array<vec3<f32>, 12u>; // ≡ L163 (MAX_LAMPS = 12, L11)
@group(3) @binding(38)
var<uniform> uMurA: array<vec4<f32>, 12u>;  // ≡ L213 (MUR = 12, L106)
@group(3) @binding(39)
var<uniform> uMurB: array<vec4<f32>, 12u>;  // ≡ L214
@group(3) @binding(40)
var<uniform> uMurC: array<vec4<f32>, 12u>;  // ≡ L215 (JS vec2[12]; WGSL uniform array stride forces vec4 padding - D2 packs .xy)

// ≡ levelMaterial.js L44-71 + inkShading.js L24-31 uniform table (per frame).
struct LevelParams {
    u_ink_clock: f32,   // ≡ uInkClock (inkShading L26; default 0)
    u_time: f32,        // ≡ uTime (levelMaterial L49; default 0)
    u_ppm: f32,         // ≡ uPpm (L62; default 20)
    u_atlas_px: f32,    // ≡ uAtlasSize (L61); uTexel = 1/this (L46)
    u_ink_glow: f32,    // ≡ uInkGlow (L50; default 0.06)
    u_gel: f32,         // ≡ uGel (L64; L100 `L.gel ?? -1` -> -1 sentinel = off, ⑤)
    u_tex_size: f32,    // ≡ uTexSize (L95; default 512)
    u_ao: f32,          // ≡ uAO (L58; ④ default 0 = inactive)
    u_lamp_n: i32,      // ≡ uLampN (L60; default 0)
    u_see_on: f32,      // ≡ uSeeOn (L55; default 0)
    u_see_feet: f32,    // ≡ uSeeFeet (L56)
    u_see_a2c: f32,     // ≡ uSeeA2C (L57; JS reads rt.samples - D2 supplies)
    u_grate: f32,       // ≡ #define GRATE (L715 opts.grate) -> runtime flag
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    u_team_a: vec3<f32>,   // ≡ uTeamA '#ff8a14' (L47)
    _pad3: f32,
    u_team_b: vec3<f32>,   // ≡ uTeamB '#2f5bff' (L48)
    _pad4: f32,
    u_lamp_col: vec3<f32>, // ≡ uLampCol LAMP_HEX x LAMP_I x k (L11/L60)
    _pad5: f32,
    u_see_b: vec3<f32>,    // ≡ uSeeB player chest (L54)
    _pad6: f32,
    u_see_a: vec3<f32>,    // ≡ uSeeA camera position (L53; unused by the JS fragment, kept for parity)
    _pad7: f32,
    u_sun_dir: vec3<f32>,  // ≡ directionalLights[0].direction (inkShading L132-136); fallback normalize(-0.41, 0.83, -0.38)
    _pad8: f32,
}
@group(3) @binding(41)
var<uniform> level: LevelParams;

// ≡ deviation ③: per-vertex faceData/faceFlags/faceTan (level.js L371-380) as
// per-batch constants (box faces are constant within a batch).
struct BatchParams {
    u_pattern: f32,    // ≡ vFaceData.x (pattern slot id)
    u_wet_base: f32,   // ≡ vFaceData.y (ink gate, INK_COLOR L99 `> 0.5`)
    u_flags_x: f32,    // ≡ vFaceFlags.x (wall-cap / stands-on flag)
    u_flags_y: f32,    // ≡ vFaceFlags.y (foot contact flag)
    u_mur: f32,        // ≡ vFaceFlags.z (mural id, < 0 = none; ⑤ default -1)
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    u_fsz: vec2<f32>,  // ≡ vFaceData.zw (face size in metres, fsz0 L201)
    _pad3: f32,
    _pad4: f32,
    u_tan: vec3<f32>,  // ≡ vFaceTan (object space; model matrix is identity)
    _pad5: f32,
}
@group(3) @binding(42)
var<uniform> batch: BatchParams;

// ---------------------------------------------------------------- module state
// ≡ levelMaterial.js L216-233 global scratch (three's chunk chain talks through
// these; WGSL `var<private>` mirrors that, re-initialised per fragment below).
var<private> u_ink_clock: f32;
var<private> u_time: f32;
var<private> u_ppm: f32;
var<private> u_atlas_px: f32;
var<private> u_texel: f32;        // ≡ L46/L155 uniform float uTexel
var<private> u_ink_glow: f32;
var<private> u_gel: f32;
var<private> u_tex_size: f32;
var<private> u_lamp_n: i32;
var<private> u_team_a: vec3<f32>;
var<private> u_team_b: vec3<f32>;
var<private> u_lamp_col: vec3<f32>;
var<private> u_sun_dir: vec3<f32>;
var<private> u_pattern: f32;
var<private> u_wet_base: f32;
var<private> u_fsz: vec2<f32>;
var<private> u_flags_x: f32;
var<private> u_flags_y: f32;
var<private> u_mur: f32;
var<private> u_tan: vec3<f32>;

var<private> g_tex_mod: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);  // ≡ L216
var<private> g_tex_paint: f32 = 1.0;                            // ≡ L217
var<private> g_tex_n: vec3<f32> = vec3<f32>(0.0, 0.0, 1.0);     // ≡ L218
var<private> g_tex_orm: vec4<f32> = vec4<f32>(1.0, 0.8, 0.0, 0.5); // ≡ L219
var<private> g_tex_str: f32 = 0.0;                              // ≡ L220
var<private> g_stair: vec4<f32> = vec4<f32>(0.0, 0.0, 0.0, 0.0); // ≡ L221
var<private> g_tex_keep: f32 = 0.0;                             // ≡ L222
var<private> g_tex_alpha: f32 = 1.0;                            // ≡ L223
var<private> g_ink: f32 = 0.0;                                  // ≡ L224
var<private> g_ink_col: vec3<f32> = vec3<f32>(0.0, 0.0, 0.0);  // ≡ L225
var<private> g_base_rough: f32 = 0.8;                           // ≡ L226
var<private> g_ink_h: f32 = 0.0;                                // ≡ L227 (declared-unused in JS too)
var<private> g_ink_d: vec2<f32> = vec2<f32>(0.0, 0.0);          // ≡ L228
var<private> g_rib: f32 = 0.0;                                  // ≡ L229 (legacy chain only)
var<private> g_fresh: f32 = 0.0;                                // ≡ L230
var<private> g_ink_near: f32 = 0.0;                             // ≡ L231
var<private> g_ink_s: f32 = 0.0;                                // ≡ L232
var<private> g_wake: f32 = 0.0;                                 // ≡ L233
var<private> g_ink_keep: f32 = 0.0;                             // ≡ inkShading L53

// ================================================================ noise helpers
// ≡ levelMaterial.js L187-199 (h21 / vnoise / fbm / gridLine)
fn h21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + vec2<f32>(45.32, 45.32));
    return fract(q.x * q.y);
}
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (vec2<f32>(3.0, 3.0) - vec2<f32>(2.0, 2.0) * f);
    return mix(mix(h21(i), h21(i + vec2<f32>(1.0, 0.0)), u.x),
               mix(h21(i + vec2<f32>(0.0, 1.0)), h21(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}
fn fbm(p_in: vec2<f32>) -> f32 {
    var a = 0.5;
    var s = 0.0;
    var p = p_in;
    for (var i: i32 = 0; i < 4; i = i + 1) {
        s = s + a * vnoise(p);
        p = p * vec2<f32>(2.03, 2.03) + vec2<f32>(17.1, 17.1);
        a = a * 0.5;
    }
    return s;
}
// anti-aliased grid line: returns 1 on the line  ≡ L195-199
fn grid_line(x: f32, period: f32, width: f32) -> f32 {
    let fx = abs(fract(x / period + 0.5) - 0.5) * period;
    let w = fwidth(x) * 0.75;
    return 1.0 - smoothstep(width - w, width + w, fx);
}

// ================================================================ TEXLIB_GLSL runtime
// ≡ texlib.js L1173-1326 (TEXLIB_GLSL). textureGrad -> textureSampleGrad,
// dFdx/dFdy -> dpdx/dpdy; GLSL column-major mat2/mat3 constructors port 1:1.
const TEXLIB_PLAIN: i32 = 0;   // ≡ L1177
const TEXLIB_GRID: i32 = 1;    // ≡ L1178
const TEXLIB_HEX: i32 = 2;     // ≡ L1179
const TEXLIB_HEX_SCALE: f32 = 0.5;  // ≡ L1181
const TEXLIB_DETAIL_SCALE: f32 = 0.5; // ≡ L1277

struct TexlibSample { albedo: vec4<f32>, normal: vec3<f32>, orm: vec4<f32> }  // ≡ L1183

fn texlib_pcg(v: u32) -> u32 {   // ≡ L1185
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}
fn texlib_hash(p: vec2<i32>) -> u32 {  // ≡ L1186
    return texlib_pcg(u32(p.x) + texlib_pcg(u32(p.y) + 0x9e3779b9u));
}
fn texlib_h01(h: u32) -> f32 {   // ≡ L1187
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

struct TriGrid { w: vec3<f32>, v1: vec2<i32>, v2: vec2<i32>, v3: vec2<i32> }
fn texlib_tri_grid(st_in: vec2<f32>) -> TriGrid {  // ≡ L1189-1198
    var st = st_in * 3.46410162;
    let sk = mat2x2<f32>(1.0, 0.0, -0.57735027, 1.15470054) * st;
    let b = vec2<i32>(floor(sk));
    var t = vec3<f32>(fract(sk), 0.0);
    t.z = 1.0 - t.x - t.y;
    let s = step(0.0, -t.z);
    let s2 = 2.0 * s - 1.0;
    let w = vec3<f32>(-t.z * s2, s - t.y * s2, s - t.x * s2);
    let si = i32(s);
    return TriGrid(w, b + vec2<i32>(si, si), b + vec2<i32>(si, 1 - si), b + vec2<i32>(1 - si, si));
}
fn texlib_hex_centre(v: vec2<i32>) -> vec2<f32> {  // ≡ L1199
    return mat2x2<f32>(1.0, 0.0, 0.5, 0.8660254) * vec2<f32>(f32(v.x), f32(v.y))
         / (3.46410162 * TEXLIB_HEX_SCALE);
}
fn texlib_rot(h: u32) -> mat2x2<f32> {  // ≡ L1200
    let a = texlib_h01(h) * 6.28318531;
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, s, -s, c);
}

struct Tap { a: vec4<f32>, n: vec3<f32>, nl: f32, o: vec4<f32> }
fn texlib_tap(st: vec2<f32>, layer: f32, gx: vec2<f32>, gy: vec2<f32>, M: mat2x2<f32>) -> Tap {  // ≡ L1202-1210
    // WGSL array sampling: (coords: vec2, array_index: i32) instead of GLSL's vec3.
    let a = textureSampleGrad(t_albedo, texlib_sampler, st, i32(layer), M * gx, M * gy);
    let nr = textureSampleGrad(t_normal, texlib_sampler, st, i32(layer), M * gx, M * gy).xyz * vec3<f32>(2.0, 2.0, 2.0) - vec3<f32>(1.0, 1.0, 1.0);
    let o = textureSampleGrad(t_orm, texlib_sampler, st, i32(layer), M * gx, M * gy);
    let nl = length(nr);
    let n = vec3<f32>(transpose(M) * nr.xy, nr.z);
    return Tap(a, n, nl, o);
}

// ≡ texlibSample L1212-1256: PLAIN/GRID/HEX anti-tiling + Toksvig (L1250-1253).
fn texlib_sample(uv: vec2<f32>, layer: f32, mode: i32, sym: i32) -> TexlibSample {
    var gx = dpdx(uv);
    var gy = dpdy(uv);
    // WGSL note: textureSampleGrad has no bias arg here (matches Bevy's own
    // usage); array textures take (coords: vec2, array_index: i32, ddx, ddy).
    // keep the footprint anisotropy within what 16x hardware AF resolves (≤ 12:1)  ≡ L1214-1217
    let lx = dot(gx, gx);
    let ly = dot(gy, gy);
    if (lx > ly * 144.0) {
        gy = select(gy * sqrt(lx / (ly * 144.0)), vec2<f32>(-gx.y, gx.x) / 12.0, ly <= 1e-20);
    } else if (ly > lx * 144.0) {
        gx = select(gx * sqrt(ly / (lx * 144.0)), vec2<f32>(gy.y, -gy.x) / 12.0, lx <= 1e-20);
    }
    var r: TexlibSample;
    var n: vec3<f32>;
    var nl: f32;
    if (mode == TEXLIB_HEX) {
        let tg = texlib_tri_grid(uv * TEXLIB_HEX_SCALE);
        let h1 = texlib_hash(tg.v1);
        let h2 = texlib_hash(tg.v2);
        let h3 = texlib_hash(tg.v3);
        let R1 = texlib_rot(h1);
        let R2 = texlib_rot(h2);
        let R3 = texlib_rot(h3);
        let c1 = texlib_hex_centre(tg.v1);
        let c2 = texlib_hex_centre(tg.v2);
        let c3 = texlib_hex_centre(tg.v3);
        let o1 = vec2<f32>(texlib_h01(texlib_pcg(h1)), texlib_h01(texlib_pcg(h1 + 1u)));
        let o2 = vec2<f32>(texlib_h01(texlib_pcg(h2)), texlib_h01(texlib_pcg(h2 + 1u)));
        let o3 = vec2<f32>(texlib_h01(texlib_pcg(h3)), texlib_h01(texlib_pcg(h3 + 1u)));
        let t1 = texlib_tap(R1 * (uv - c1) + c1 + o1, layer, gx, gy, R1);
        let t2 = texlib_tap(R2 * (uv - c2) + c2 + o2, layer, gx, gy, R2);
        let t3 = texlib_tap(R3 * (uv - c3) + c3 + o3, layer, gx, gy, R3);
        var W = tg.w * tg.w;      // ≡ L1233 w^7
        W = W * W;
        W = W * (tg.w * tg.w * tg.w);
        W = W * mix(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(t1.o.a, t2.o.a, t3.o.a) + vec3<f32>(0.05, 0.05, 0.05), vec3<f32>(0.6, 0.6, 0.6)); // ≡ L1234
        W = W / max(W.x + W.y + W.z, 1e-6);   // ≡ L1235
        r.albedo = W.x * t1.a + W.y * t2.a + W.z * t3.a;
        r.orm = W.x * t1.o + W.y * t2.o + W.z * t3.o;
        n = W.x * t1.n + W.y * t2.n + W.z * t3.n;
        nl = W.x * t1.nl + W.y * t2.nl + W.z * t3.nl;
    } else if (mode == TEXLIB_GRID) {
        // per-repeat dihedral transform masked by sym: bit 1 mirror-u, 2 mirror-v, 4 transpose  ≡ L1241-1245
        let h = texlib_hash(vec2<i32>(floor(uv))) & u32(sym);
        var M = mat2x2<f32>(1.0, 0.0, 0.0, 1.0);
        if ((h & 4u) != 0u) { M = mat2x2<f32>(0.0, 1.0, 1.0, 0.0); }
        if ((h & 1u) != 0u) { M = mat2x2<f32>(-1.0, 0.0, 0.0, 1.0) * M; }
        if ((h & 2u) != 0u) { M = mat2x2<f32>(1.0, 0.0, 0.0, -1.0) * M; }
        let tp = texlib_tap(M * (fract(uv) - vec2<f32>(0.5, 0.5)) + vec2<f32>(0.5, 0.5), layer, gx, gy, M);
        r.albedo = tp.a;
        n = tp.n;
        nl = tp.nl;
        r.orm = tp.o;
    } else {
        let tp = texlib_tap(uv, layer, gx, gy, mat2x2<f32>(1.0, 0.0, 0.0, 1.0));
        r.albedo = tp.a;
        n = tp.n;
        nl = tp.nl;
        r.orm = tp.o;
    }
    // Toksvig: mips average bumps into shorter normals -> widen the lobe instead of sparkling  ≡ L1250-1253
    let v = max((1.0 - nl) / max(nl, 1e-3) - 0.006, 0.0);
    let a = r.orm.g * r.orm.g;
    r.orm.g = sqrt(sqrt(min(a * a + 2.0 * v, 1.0)));
    r.normal = normalize(vec3<f32>(n.xy, max(n.z, 1e-3)));
    return r;
}

// ≡ texlibTangentFrame L1258-1268 (derivative TBN; ported for parity - the
// level path uses the analytic face frame below instead).
fn texlib_tangent_frame(eye_pos: vec3<f32>, surf_normal: vec3<f32>, uv: vec2<f32>) -> mat3x3<f32> {
    let q0 = dpdx(eye_pos);
    let q1 = dpdy(eye_pos);
    let st0 = dpdx(uv);
    let st1 = dpdy(uv);
    let N = surf_normal;
    let q1perp = cross(q1, N);
    let q0perp = cross(N, q0);
    let T = q1perp * st0.x + q0perp * st1.x;
    let B = q1perp * st0.y + q0perp * st1.y;
    let det = max(dot(T, T), dot(B, B));
    let sc = select(1.0 / sqrt(det), 0.0, det == 0.0);  // ≡ GLSL inversesqrt (not a WGSL builtin)
    return mat3x3<f32>(T * sc, B * sc, N);
}
fn texlib_perturb_normal(n_ts: vec3<f32>, T: vec3<f32>, B: vec3<f32>, N: vec3<f32>, strength: f32) -> vec3<f32> {  // ≡ L1269-1271
    return normalize(T * (n_ts.x * strength) + B * (n_ts.y * strength) + N * n_ts.z);
}
fn texlib_perturb_normal_tbn(n_ts: vec3<f32>, tbn: mat3x3<f32>, strength: f32) -> vec3<f32> {  // ≡ L1272-1274
    return texlib_perturb_normal(n_ts, tbn[0], tbn[1], tbn[2], strength);
}

// ≡ texlibDetail L1281-1295: two rotated-scale taps of the 'detail' layer.
fn texlib_detail(s: ptr<function, TexlibSample>, p_metres: vec2<f32>, layer: f32, amount: f32) {
    let u1 = p_metres / TEXLIB_DETAIL_SCALE;
    let gx = dpdx(u1);
    let gy = dpdy(u1);
    if (amount <= 0.0) { return; }
    const R: mat2x2<f32> = mat2x2<f32>(0.8660254, 0.5, -0.5, 0.8660254);
    let u2 = R * u1 * 0.61 + vec2<f32>(0.37, 0.71);
    let hx = R * gx * 0.61;
    let hy = R * gy * 0.61;
    let a1 = textureSampleGrad(t_albedo, texlib_sampler, u1, i32(layer), gx, gy);
    let a2 = textureSampleGrad(t_albedo, texlib_sampler, u2, i32(layer), hx, hy);
    let n1 = textureSampleGrad(t_normal, texlib_sampler, u1, i32(layer), gx, gy).xyz * vec3<f32>(2.0, 2.0, 2.0) - vec3<f32>(1.0, 1.0, 1.0);
    let n2 = textureSampleGrad(t_normal, texlib_sampler, u2, i32(layer), hx, hy).xyz * vec3<f32>(2.0, 2.0, 2.0) - vec3<f32>(1.0, 1.0, 1.0);
    let o1 = textureSampleGrad(t_orm, texlib_sampler, u1, i32(layer), gx, gy);
    let o2 = textureSampleGrad(t_orm, texlib_sampler, u2, i32(layer), hx, hy);
    (*s).albedo.rgb = (*s).albedo.rgb * vec3<f32>(mix(1.0, a1.r + a2.r, amount));
    let dn = (n1.xy + transpose(R) * n2.xy) * 0.7071;
    (*s).normal = normalize(vec3<f32>((*s).normal.xy + dn * amount, (*s).normal.z));
    (*s).orm.g = clamp((*s).orm.g + (o1.g + o2.g - 1.0) * amount, 0.02, 1.0);
}

// ≡ texlibSpecularAA L1299-1304 (Tokuyoshi-Kaplanyan screen-space lobe widening).
fn texlib_specular_aa(n: vec3<f32>, roughness: f32) -> f32 {
    let du = dpdx(n);
    let dv = dpdy(n);
    let kernel = min(2.0 * 0.15 * (dot(du, du) + dot(dv, dv)), 0.2);
    let a = roughness * roughness;
    return sqrt(sqrt(clamp(a * a + kernel, 0.0, 1.0)));
}

// ≡ texlib_vn L1305-1311 + texlibMacro L1312-1315 (world-scale tone drift).
fn texlib_vn(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (vec2<f32>(3.0, 3.0) - vec2<f32>(2.0, 2.0) * f);
    let c = vec2<i32>(i);
    let a = texlib_h01(texlib_hash(c));
    let b = texlib_h01(texlib_hash(c + vec2<i32>(1, 0)));
    let d = texlib_h01(texlib_hash(c + vec2<i32>(0, 1)));
    let e = texlib_h01(texlib_hash(c + vec2<i32>(1, 1)));
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
fn texlib_macro(p: vec2<f32>) -> f32 {
    let n = 0.62 * texlib_vn(p * vec2<f32>(0.085, 0.085) + vec2<f32>(3.7, 3.7))
        + 0.38 * texlib_vn(p * vec2<f32>(0.29, 0.29) + vec2<f32>(11.3, 11.3));
    return 0.94 + 0.12 * n;
}

// ≡ texlibCoverage L1316-1324 (coverage-preserving grate fade).
fn texlib_coverage(a: f32, uv: vec2<f32>, tex_size: f32) -> f32 {
    let dx = dpdx(uv) * tex_size;
    let dy = dpdy(uv) * tex_size;
    let lod = 0.5 * log2(max(max(dot(dx, dx), dot(dy, dy)), 1e-8));
    let sharp = clamp((a - 0.5) / max(fwidth(a), 1e-4) + 0.5, 0.0, 1.0);
    let farv = clamp(a * (1.0 + 1.6 * max(lod, 0.0)), 0.0, 1.0);
    return mix(sharp, farv, smoothstep(-0.5, 0.8, lod));
}

// ================================================================ INK_PARS
// ≡ inkShading.js L49-94 (paint lookups + cubic B-spline reconstruction +
// Mikkelsen ink perturb).
var<private> g_pdx: vec2<f32> = vec2<f32>(0.0, 0.0);   // ≡ L56 gPdx
var<private> g_pdy: vec2<f32> = vec2<f32>(0.0, 0.0);   // ≡ L56 gPdy

fn paint_at(uv: vec2<f32>) -> vec4<f32> {  // ≡ paintAt L57 (textureGrad -> textureSampleGrad)
    return textureSampleGrad(ink_atlas, ink_sampler, uv, g_pdx, g_pdy);
}

struct Bspline { w: vec4<f32>, dw: vec4<f32> }
fn ink_bspl(t: f32) -> Bspline {  // ≡ inkBspl L60-64
    let t2 = t * t;
    let t3 = t2 * t;
    let it = 1.0 - t;
    let w = vec4<f32>(it * it * it, 3.0 * t3 - 6.0 * t2 + 4.0, -3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0, t3) * (1.0 / 6.0);
    let dw = vec4<f32>(-it * it, 3.0 * t2 - 4.0 * t, -3.0 * t2 + 2.0 * t + 1.0, t2) * 0.5;
    return Bspline(w, dw);
}
fn ink_tap(st: vec2<f32>) -> vec4<f32> {  // ≡ inkTap L65 (textureLod -> textureSampleLevel)
    return textureSampleLevel(ink_atlas, ink_sampler, (st + vec2<f32>(0.5, 0.5)) / u_atlas_px, 0.0);
}
fn paint_cubic(uv: vec2<f32>, gX: ptr<function, vec4<f32>>, gY: ptr<function, vec4<f32>>) -> vec4<f32> {  // ≡ paintCubic L66-82
    let st = uv * u_atlas_px - vec2<f32>(0.5, 0.5);
    let i = floor(st);
    let f = st - i;
    let bx = ink_bspl(f.x);
    let by = ink_bspl(f.y);
    let wx = bx.w; let dwx = bx.dw;
    let wy = by.w; let dwy = by.dw;
    let gx2 = vec2<f32>(wx.x + wx.y, wx.z + wx.w);
    let gy2 = vec2<f32>(wy.x + wy.y, wy.z + wy.w);
    let ox = vec2<f32>(-1.0 + wx.y / gx2.x, 1.0 + wx.w / gx2.y);
    let oy = vec2<f32>(-1.0 + wy.y / gy2.x, 1.0 + wy.w / gy2.y);
    let dgx = vec2<f32>(dwx.x + dwx.y, dwx.z + dwx.w);
    let dgy = vec2<f32>(dwy.x + dwy.y, dwy.z + dwy.w);
    let dox = vec2<f32>(-1.0 + dwx.y / dgx.x, 1.0 + dwx.w / dgx.y);
    let doy = vec2<f32>(-1.0 + dwy.y / dgy.x, 1.0 + dwy.w / dgy.y);
    let v = gx2.x * (gy2.x * ink_tap(i + vec2<f32>(ox.x, oy.x)) + gy2.y * ink_tap(i + vec2<f32>(ox.x, oy.y)))
          + gx2.y * (gy2.x * ink_tap(i + vec2<f32>(ox.y, oy.x)) + gy2.y * ink_tap(i + vec2<f32>(ox.y, oy.y)));
    *gX = dgx.x * (gy2.x * ink_tap(i + vec2<f32>(dox.x, oy.x)) + gy2.y * ink_tap(i + vec2<f32>(dox.x, oy.y)))
        + dgx.y * (gy2.x * ink_tap(i + vec2<f32>(dox.y, oy.x)) + gy2.y * ink_tap(i + vec2<f32>(dox.y, oy.y)));
    *gY = dgy.x * (gx2.x * ink_tap(i + vec2<f32>(ox.x, doy.x)) + gx2.y * ink_tap(i + vec2<f32>(ox.y, doy.x)))
        + dgy.y * (gx2.x * ink_tap(i + vec2<f32>(ox.x, doy.y)) + gx2.y * ink_tap(i + vec2<f32>(ox.y, doy.y)));
    return v;
}
// ≡ perturbInk L85-93 (unnormalised Mikkelsen bump; ported for parity - the
// level path uses the analytic slope below instead).
fn perturb_ink(surf_pos: vec3<f32>, surf_norm: vec3<f32>, d_hdxy: vec2<f32>, face_direction: f32) -> vec3<f32> {
    let vSigmaX = dpdx(surf_pos);
    let vSigmaY = dpdy(surf_pos);
    let R1 = cross(vSigmaY, surf_norm);
    let R2 = cross(surf_norm, vSigmaX);
    let fDet = dot(vSigmaX, R1) * face_direction;
    let vGrad = sign(fDet) * (d_hdxy.x * R1 + d_hdxy.y * R2);
    return normalize(abs(fDet) * surf_norm - vGrad);
}

// ================================================================ fragment
@fragment
fn fragment(vertex_output: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var in = vertex_output;
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // ---- mirror the JS uniform names into module state (③ batch constants) ----
    u_ink_clock = level.u_ink_clock;
    u_time = level.u_time;
    u_ppm = level.u_ppm;
    u_atlas_px = level.u_atlas_px;
    u_texel = 1.0 / max(u_atlas_px, 1.0);      // ≡ uTexel (levelMaterial L46)
    u_ink_glow = level.u_ink_glow;
    u_gel = level.u_gel;
    u_tex_size = level.u_tex_size;
    u_lamp_n = level.u_lamp_n;
    u_team_a = level.u_team_a;
    u_team_b = level.u_team_b;
    u_lamp_col = level.u_lamp_col;
    u_sun_dir = level.u_sun_dir;
    u_pattern = batch.u_pattern;
    u_wet_base = batch.u_wet_base;
    u_fsz = batch.u_fsz;
    u_flags_x = batch.u_flags_x;
    u_flags_y = batch.u_flags_y;
    u_mur = batch.u_mur;
    u_tan = batch.u_tan;
    // per-fragment reset of the ≡ L216-233 scratch
    g_tex_mod = vec3<f32>(1.0, 1.0, 1.0);
    g_tex_paint = 1.0;
    g_tex_n = vec3<f32>(0.0, 0.0, 1.0);
    g_tex_orm = vec4<f32>(1.0, 0.8, 0.0, 0.5);
    g_tex_str = 0.0;
    g_stair = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    g_tex_keep = 0.0;
    g_tex_alpha = 1.0;
    g_ink = 0.0;
    g_ink_col = vec3<f32>(0.0, 0.0, 0.0);
    g_base_rough = 0.8;
    g_ink_h = 0.0;
    g_ink_d = vec2<f32>(0.0, 0.0);
    g_rib = 0.0;
    g_fresh = 0.0;
    g_ink_near = 0.0;
    g_ink_s = 0.0;
    g_wake = 0.0;
    g_ink_keep = 0.0;
    g_pdx = vec2<f32>(0.0, 0.0);
    g_pdy = vec2<f32>(0.0, 0.0);

    // ---- shared frame data (≡ levelMaterial.js varyings L137-143/L178-185) ----
    let wpos = in.world_position.xyz;          // ≡ vWPos
    let wnorm = normalize(pbr_input.world_normal); // ≡ vWNorm (double-sided aware)
    let fu = in.uv;                            // ≡ vFaceUv (world metres, D2)
    let paint_uv = in.uv_b;                    // ≡ vPaintUv (atlas rect; ④ also replaces lightUv)
    let fsz = u_fsz;                           // ≡ fsz0(vFaceData) L201/L287
    let face_tan = u_tan;                      // ≡ vFaceTan
    let view_pos = (view.view_from_world * in.world_position).xyz;
    let v_view_position = -view_pos;           // ≡ three vViewPosition

    // ============================================================ color_fragment
    // ≡ levelMaterial.js L234-607.
    // ---- see-through window (L236-272) ----
    var out_alpha: f32 = 1.0;                  // ≡ diffuseColor.a = 1.0 (L241)
    if (level.u_see_on > 0.001) {
        let Bv = (view.view_from_world * vec4<f32>(level.u_see_b, 1.0)).xyz;
        let dB = -Bv.z;
        let dP = v_view_position.z;
        if (dB > 0.6 && dP < dB - 0.2) {
            let bodyH = level.u_see_b.y - level.u_see_feet;
            let Fv = (view.view_from_world * vec4<f32>(level.u_see_b.x, level.u_see_feet + 0.05, level.u_see_b.z, 1.0)).xyz;
            let Hv = (view.view_from_world * vec4<f32>(level.u_see_b.x, level.u_see_feet + bodyH * 1.42 + 0.06, level.u_see_b.z, 1.0)).xyz;
            let p = -v_view_position.xy / dP;
            let f = Fv.xy / max(-Fv.z, 0.05);
            let h = Hv.xy / max(-Hv.z, 0.05);
            let fh = h - f;
            let t = clamp(dot(p - f, fh) / max(dot(fh, fh), 1e-8), 0.0, 1.0);
            let sd = length(p - (f + fh * t)) * dB;
            let R = mix(0.3, 0.46, clamp(bodyH, 0.0, 1.0));
            var k = 1.0 - smoothstep(R, R + 0.42, sd);
            k = k * (1.0 - smoothstep(dB - 0.75, dB - 0.2, dP));  // ≡ L255 smoothstep with descending edges
            let feetTop = level.u_see_feet + 0.3;
            if (wnorm.y > 0.6 && wpos.y < feetTop) { k = 0.0; }
            if (abs(wnorm.y) < 0.6 && wpos.y + max(fsz.y - fu.y, 0.0) * 0.9 < feetTop) { k = 0.0; }
            k = k * level.u_see_on;
            if (k > 0.002) {
                let qc = vec2<u32>(u32(in.position.x), u32(in.position.y)) & vec2<u32>(7u, 7u);  // ≡ ivec2(gl_FragCoord.xy) & 7 (L262)
                let xy = qc.x ^ qc.y;
                let bay = (f32(((xy & 1u) << 5u) | ((qc.x & 1u) << 4u) | ((xy & 2u) << 2u) | ((qc.x & 2u) << 1u) | ((xy & 4u) >> 1u) | ((qc.x & 4u) >> 2u)) + 0.5) / 64.0;  // ≡ L264
                let a = 1.0 - k;
                if (level.u_see_a2c > 0.5) {
                    out_alpha = clamp(a + (bay - 0.5) * 0.25 * smoothstep(0.0, 0.2, a) * (1.0 - smoothstep(0.8, 1.0, a)), 0.0, 1.0);  // ≡ L269 descending-edge smoothstep
                } else if (k > bay * 0.98 + 0.01) {
                    discard;
                }
            }
        }
    }

    var base = pbr_input.material.base_color.rgb;  // ≡ vec3 base = diffuseColor.rgb (L288; vertexColors x material colour)
    var rough = 0.82;                              // ≡ L289
    let big = fbm(wpos.xz * 0.18 + wpos.y * 0.1);  // ≡ L290
    let fine = vnoise(fu * vec2<f32>(9.0, 9.0));   // ≡ L291

    // ---- GRATE bar cutting (≡ L275-285; JS #define -> u_grate runtime flag) ----
    if (level.u_grate > 0.5) {
        let gw = fwidth(fu.x) + 1e-4;
        let bx = 1.0 - smoothstep(0.02 - gw, 0.02 + gw, abs(fract(fu.x / 0.07 + 0.5) - 0.5) * 0.07);
        let by = 1.0 - smoothstep(0.01 - gw, 0.01 + gw, abs(fract(fu.y / 0.2 + 0.5) - 0.5) * 0.2);
        let edge = 1.0 - smoothstep(0.05, 0.06, min(min(fu.x, fsz.x - fu.x), min(fu.y, fsz.y - fu.y)));
        var solid = max(max(bx, by), edge);
        if (abs(wnorm.y) < 0.5) { solid = 1.0; }
        if (solid < 0.5) { discard; }
    }

    // ---- texlib surface (≡ L292-332 USE_TEXLIB block) ----
    {
        var pid = i32(u_pattern + 0.5);
        let vertical = abs(wnorm.y) < 0.5;
        // per-slot remaps: vertical -> uTLt.z, top -> uTLt.w  ≡ L295-297
        if (vertical) {
            pid = i32(uTLt[u32(pid)].z + 0.5);
        } else if (wnorm.y > 0.5) {
            pid = i32(uTLt[u32(pid)].w + 0.5);
        }
        let tl = uTL[u32(pid)];
        let tt = uTLt[u32(pid)];
        var tuv = fu * tl.y;                       // ≡ L301
        let sp = uTLs[u32(pid)];                   // ≡ L302
        var tA = vec2<f32>(1.0, 0.0);              // ≡ L303
        var tD = vec2<f32>(0.0, 1.0);              // ≡ L303
        if (sp.x > 0.0 && wnorm.y > 0.5) {         // stair/ramp frame, phase-locked  ≡ L304-318
            let alongU = abs(face_tan.y) > 0.02;
            var sg = select(sign(wnorm.z * face_tan.x - wnorm.x * face_tan.z), sign(face_tan.y), alongU);
            var fa = select(fu.y, fu.x, alongU);
            var fcr = select(fu.x, fu.y, alongU);
            var La = select(fsz.y, fsz.x, alongU);
            let Wd = select(fsz.x, fsz.y, alongU);
            var dTop = select(fa, La - fa, sg > 0.0);
            if (dTop > La - 0.3) { sg = -sg; dTop = La - dTop; }   // ≡ L310
            let Lv = max(La - 0.6, sp.x);                          // ≡ L311
            let Ps = Lv / max(1.0, floor(Lv / sp.x + 0.5));        // ≡ L312
            tuv = vec2<f32>(fcr * tl.y, dTop / (Ps * sp.y));       // ≡ L313
            tA = select(vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), alongU);
            tD = select(vec2<f32>(0.0, -sg), vec2<f32>(-sg, 0.0), alongU);
            g_stair = vec4<f32>(fcr, Wd, sp.w, dTop / Ps);         // ≡ L316
            g_tex_keep = sp.z;                                     // ≡ L317
        }
        var ts = texlib_sample(tuv, tl.x, i32(tl.z), i32(tl.w));   // ≡ L319
        // back into the face's (u, v) frame  ≡ L320
        ts.normal = vec3<f32>(ts.normal.x * tA + ts.normal.y * tD, ts.normal.z);
        // tint modes 0/1/2  ≡ L321-322
        if (tt.x > 1.5) {
            base = ts.albedo.rgb + pbr_input.material.base_color.rgb * ts.albedo.a * 1.25;
        } else if (tt.x > 0.5) {
            base = pbr_input.material.base_color.rgb * ts.albedo.rgb * 1.25;
        } else {
            base = ts.albedo.rgb;
        }
        base = base * texlib_macro(wpos.xz + wpos.y * 0.7);        // ≡ L323
        base = base * mix(1.0, ts.orm.r, 0.85);                    // ≡ L324 cavity occlusion
        if (tt.x > 1.5) {
            g_tex_mod = vec3<f32>(ts.albedo.a * 1.25, ts.albedo.a * 1.25, ts.albedo.a * 1.25) * mix(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(ts.orm.r, ts.orm.r, ts.orm.r), vec3<f32>(0.85, 0.85, 0.85));
        } else if (tt.x > 0.5) {
            g_tex_mod = ts.albedo.rgb * 1.25 * mix(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(ts.orm.r, ts.orm.r, ts.orm.r), vec3<f32>(0.85, 0.85, 0.85));
        } else {
            g_tex_mod = mix(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(ts.orm.r, ts.orm.r, ts.orm.r), vec3<f32>(0.85, 0.85, 0.85));
        }
        g_tex_paint = select(1.0, clamp(ts.albedo.a * 1.45, 0.0, 1.0), tt.x > 1.5);  // ≡ L326
        rough = ts.orm.g;                                          // ≡ L327
        g_tex_n = ts.normal; g_tex_orm = ts.orm; g_tex_str = tt.y; g_tex_alpha = ts.albedo.a;  // ≡ L328
        if (level.u_grate > 0.5) {                                 // ≡ L329-331 GRATE coverage discard
            if (texlib_coverage(ts.albedo.a, fu * tl.y, u_tex_size) < 0.5 && !vertical) { discard; }
        }
    }

    // ---- pattern overlays (≡ L333-358) ----
    if (u_pattern < 1.5 && u_pattern >= 0.5) {
        // court markings over the pavers  ≡ L335-342
        let rr = length(wpos.xz);
        let wr = fwidth(rr);
        var mark = 1.0 - smoothstep(0.09 - wr, 0.09 + wr, abs(rr - 9.5));
        let wz = fwidth(wpos.z);
        mark = max(mark, (1.0 - smoothstep(0.07 - wz, 0.07 + wz, abs(wpos.z))) * step(9.5, abs(wpos.x)));
        mark = mark * (0.75 + 0.25 * step(0.35, vnoise(wpos.xz * vec2<f32>(6.0, 6.0))));
        if (wnorm.y > 0.5) {
            base = mix(base, vec3<f32>(0.97, 0.96, 0.92), mark * 0.9);
        }
    } else if (u_pattern > 3.5 && u_pattern < 4.5) {              // ≡ L343-345
        let edge = 1.0 - smoothstep(0.16, 0.2, min(fu.x, fsz.x - fu.x));
        if (wnorm.y > 0.5) {
            base = mix(base, vec3<f32>(0.93, 0.72, 0.12), edge * 0.95);
            rough = mix(rough, 0.6, edge);
        }
    } else if (u_pattern > 7.5 && u_pattern < 8.5) {              // ≡ L346-350
        let e = min(min(fu.x, fsz.x - fu.x), min(fu.y, fsz.y - fu.y));
        let band = 1.0 - smoothstep(0.34, 0.36, e);
        let chev = step(0.5, fract((fu.x + fu.y) * 1.6));
        if (wnorm.y > 0.5) {
            base = mix(base, mix(vec3<f32>(0.95, 0.76, 0.1), vec3<f32>(0.12, 0.12, 0.12), chev), band * step(0.05, e));
        }
    } else if (u_pattern > 8.5 && u_pattern < 9.5) {              // planter grass bed  ≡ L351-358
        let e = min(min(fu.x, fsz.x - fu.x), min(fu.y, fsz.y - fu.y));
        let bed = smoothstep(0.24, 0.27, e) * step(0.5, wnorm.y);
        let grass = vec3<f32>(0.32, 0.55, 0.22) * (0.8 + 0.35 * fbm(fu * vec2<f32>(6.0, 6.0)));
        base = mix(base, grass, bed);
        rough = mix(rough, 0.95, bed);
        g_tex_str = g_tex_str * (1.0 - bed);
    }

    // ---- modelled detail (≡ L359-457, shader-only) ----
    {
        let pid = i32(u_pattern + 0.5);
        let wp = wpos.xz;
        let ef = min(min(fu.x, fsz.x - fu.x), min(fu.y, fsz.y - fu.y));
        let isTop = wnorm.y > 0.6;
        let isWall = abs(wnorm.y) < 0.5;
        // floors: grime / sun-bleach / traffic polish  ≡ L367-373
        if (isTop && (pid == 1 || pid == 16 || pid == 10 || pid == 2 || pid == 8 || pid == 17 || pid == 19 || pid == 21)) {
            let m1 = vnoise(wp * vec2<f32>(0.09, 0.09) + vec2<f32>(5.3, 5.3));
            let m2 = vnoise(wp * vec2<f32>(0.31, 0.31) + vec2<f32>(1.7, 1.7));
            let m3 = vnoise(wp * vec2<f32>(1.3, 1.3));
            let grime = smoothstep(0.52, 0.86, m1 * 0.65 + m2 * 0.35);
            base = base * (1.0 - 0.075 * grime * (0.7 + 0.3 * m3));
            base = base * mix(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(1.025, 1.005, 0.965), 1.0 - smoothstep(0.12, 0.42, m1));  // ≡ L371 descending-edge smoothstep
            rough = mix(rough, rough * 0.72, smoothstep(0.62, 0.9, m2) * 0.7);
        }
        // stone coping on exposed tops + drip groove  ≡ L375-391
        if (pid == 0 || pid == 2 || pid == 3 || pid == 13 || pid == 15 || pid == 16 || pid == 23) {
            var cop = 0.0;
            var groove = 0.0;
            let cw = mix(0.2, 0.26, step(3.0, min(fsz.x, fsz.y)));
            if (isTop) {
                cop = select(1.0 - smoothstep(cw - 0.008, cw + 0.008, ef), 1.0, min(fsz.x, fsz.y) < 1.15);
                groove = (1.0 - smoothstep(0.004, 0.012, abs(ef - cw))) * step(1.15, min(fsz.x, fsz.y));
            } else if (isWall && u_flags_x > 0.5) {
                let dt = fsz.y - fu.y;
                cop = 1.0 - smoothstep(0.1, 0.108, dt);
                groove = 1.0 - smoothstep(0.005, 0.016, abs(dt - 0.118));
            }
            let stone = vec3<f32>(0.9, 0.885, 0.845) * (0.95 + 0.07 * vnoise(fu * vec2<f32>(9.0, 9.0)) + 0.03 * vnoise(fu * vec2<f32>(41.0, 41.0)));
            base = mix(base, stone * texlib_macro(wp + wpos.y * 0.7), cop);
            rough = mix(rough, 0.72, cop);
            g_tex_str = g_tex_str * (1.0 - 0.8 * cop);
            base = base * (1.0 - 0.28 * groove);
        }
        // ramps: anti-slip grooves across the slope  ≡ L393-400
        if (pid == 4 && isTop) {
            let Tt = normalize(face_tan - wnorm * dot(face_tan, wnorm));
            let Bb = cross(wnorm, Tt);
            let up2 = vec2<f32>(Tt.y, Bb.y);
            let along = dot(fu, up2 / max(length(up2), 1e-4));
            let g = grid_line(along, 0.32, 0.016);
            base = base * (1.0 - 0.2 * g);
            rough = mix(rough, 0.95, g);
        }
        // containers: rust streaks  ≡ L402-410
        if (pid == 5 && isWall) {
            let st = vnoise(vec2<f32>(dot(wp, vec2<f32>(0.707, 0.707)) * 5.5 + wpos.x * 0.8, wpos.y * 0.35));
            let fromTop = smoothstep(fsz.y * 0.25, fsz.y, fu.y);
            var rust = smoothstep(0.6, 0.92, st) * (0.35 + 0.65 * fromTop);
            rust = max(rust, (1.0 - smoothstep(0.05, 0.3, ef)) * smoothstep(0.45, 0.8, vnoise(fu * vec2<f32>(3.0, 3.0))) * 0.8);
            base = mix(base, vec3<f32>(0.42, 0.2, 0.1) * (0.8 + 0.4 * vnoise(fu * vec2<f32>(11.0, 11.0))), rust * 0.42);
            rough = mix(rough, 0.85, rust);
            base = base * mix(0.84, 1.0, smoothstep(0.0, 0.7, fu.y));
        }
        // stairs/ramps contact shadow along stringers  ≡ L413-421
        if (g_stair.z > 0.5) {
            let e = min(g_stair.x, g_stair.y - g_stair.x);
            let kind = g_stair.z;
            let sw = select(select(select(0.12, 0.14, kind < 3.5), 0.3, kind < 2.5), 0.16, kind < 1.5);
            base = base * (1.0 - select(0.26, 0.2, kind >= 2.5) * (1.0 - smoothstep(0.0, sw, e)));
            let mid = 1.0 - smoothstep(0.18, 0.42, abs(g_stair.x / max(g_stair.y, 0.01) - 0.5));
            if (kind > 1.5 && kind < 2.5) {
                base = base * (1.0 + 0.05 * mid);
                rough = mix(rough, rough * 0.78, mid);
            }
            if (kind < 1.5) {
                rough = mix(rough, rough * 0.85, mid * (1.0 - g_tex_paint));
            }
        }
        // render / cladding rain-washed streaks  ≡ L423-429
        if ((pid == 23 || pid == 22) && isWall) {
            let fromTop = fsz.y - fu.y;
            let st = 0.6 * vnoise(vec2<f32>(fu.x * 1.9, fromTop * 0.3 + 7.1)) + 0.4 * vnoise(vec2<f32>(fu.x * 6.3, fromTop * 0.8 + 2.3));
            let streak = smoothstep(0.5, 0.85, st) * exp(-fromTop * 0.75);
            base = base * (1.0 - 0.2 * streak);
            rough = mix(rough, 0.9, streak * 0.6);
        }
        // boatyard slot drain + oil drips + wheel track  ≡ L431-456
        if (pid == 21 && isTop) {
            let q = select(wp, -wp, wp.y > 0.0);
            let d = abs(q - vec2<f32>(17.5, -18.4)) - vec2<f32>(4.4, 7.0);
            let sd = length(max(d, vec2<f32>(0.0, 0.0))) + min(max(d.x, d.y), 0.0);
            let e = abs(sd);
            let aw = fwidth(e) + 1e-4;
            let chan = 1.0 - smoothstep(0.08 - aw, 0.08 + aw, e);
            let lip = (1.0 - smoothstep(0.105 - aw, 0.105 + aw, e)) * (1.0 - chan);
            let run = select(q.x, q.y, d.x > d.y);
            let fw = fwidth(run) + 1e-5;
            let slot = smoothstep(0.3 - fw / 0.025, 0.3 + fw / 0.025, abs(fract(run / 0.025) - 0.5) * 2.0);
            let bar = mix(0.4, slot, clamp(0.025 / (fw * 4.0) - 0.5, 0.0, 1.0));
            base = mix(base, mix(vec3<f32>(0.025, 0.025, 0.025), vec3<f32>(0.34, 0.35, 0.36), bar), chan);
            base = mix(base, vec3<f32>(0.4, 0.41, 0.42) * (0.9 + 0.2 * fine), lip * 0.85);
            rough = mix(rough, 0.48, max(chan * bar, lip));
            g_tex_orm.b = mix(g_tex_orm.b, 0.75, max(chan * bar, lip));
            g_tex_str = g_tex_str * (1.0 - max(chan, lip));
            base = base * mix(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(0.94, 0.9, 0.87), step(sd, -0.105) * 0.8);
            let oilW = smoothstep(0.68, 0.88, 0.65 * vnoise(q * vec2<f32>(0.9, 0.9) + vec2<f32>(13.1, 13.1)) + 0.35 * vnoise(q * vec2<f32>(3.1, 3.1) + vec2<f32>(2.7, 2.7)));
            base = base * (1.0 - 0.26 * oilW * (1.0 - chan));
            rough = mix(rough, 0.62, oilW * 0.5);
            let tx = q.x - (12.15 + 0.25 * sin(q.y * 0.37));
            let track = (1.0 - smoothstep(0.1, 0.17, min(abs(tx - 0.85), abs(tx + 0.85)))) * step(q.y, -7.6) * (0.5 + 0.5 * vnoise(q * vec2<f32>(1.3, 0.35)));
            base = base * (1.0 - 0.1 * track);
            rough = mix(rough, rough * 0.85, track);
        }
    }

    // ---- murals / signage (≡ L579-593; ⑤ u_mur = -1 default = inactive) ----
    {
        // unconditional sample (WGSL: no implicit-gradient texture in
        // non-uniform flow); the gate below selects the result.
        let mi = i32(u_mur + 0.5);
        let mA = uMurA[u32(max(mi, 0))];
        let mB = uMurB[u32(max(mi, 0))];
        let mC = uMurC[u32(max(mi, 0))];
        let my = select(fu.y / fsz.y, (fu.y - mB.z) / mB.w, mB.w > 0.0);
        let mx = select(clamp((fu.x - mB.x) / mB.y, 0.0005, 0.9995), fu.x / (-mB.y * fsz.y), mB.y < 0.0);
        let muv = vec2<f32>(mA.x + mA.y * mx, mA.z + mA.w * clamp(my, 0.004, 0.996));
        let mc0 = textureSample(mural_atlas, mural_sampler, muv);
        let mc = vec4<f32>(mc0.rgb * mix(vec3<f32>(1.0, 1.0, 1.0), g_tex_mod, mC.x),
                           mc0.a * mix(1.0, g_tex_paint, mC.y));
        if (u_mur > -0.5) {
            base = mix(base, mc.rgb * (0.92 + 0.1 * big), mc.a * 0.96);
        }
    }

    // ---- contact darkening at the foot of walls (≡ L598-601) ----
    if (u_flags_x > 0.5 && u_flags_y > 0.5) {
        base = base * mix(0.55, 1.0, smoothstep(0.0, 0.85, fu.y));
    }
    g_base_rough = rough;  // ≡ L602

    // ============================================================ INK_COLOR
    // ≡ inkShading.js L98-186 (wet ink colour + height field).
    if (u_wet_base > 0.5) {
        let pdx = dpdx(paint_uv);
        let pdy = dpdy(paint_uv);
        let texFoot = max(length(pdx), length(pdy)) / u_texel;
        let gk = min(1.0, 8.0 / max(texFoot, 1e-4));
        g_pdx = pdx * gk;   // ≡ L56 gradients clamped to <= 8 texels
        g_pdy = pdy * gk;
        var pnt = paint_at(paint_uv);
        let near = 1.0 - smoothstep(0.9, 2.4, texFoot);
        var gX = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        var gY = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        let l2 = textureSampleLevel(ink_atlas, ink_sampler, paint_uv, 2.0);
        let seam2 = l2.r / max(l2.a, 1e-3);
        if (near > 0.0 && l2.a > 0.002 && (l2.a < 0.998 || pnt.a < 0.998 || (seam2 > 0.006 && seam2 < 0.994))) {
            var cX = vec4<f32>(0.0, 0.0, 0.0, 0.0);
            var cY = vec4<f32>(0.0, 0.0, 0.0, 0.0);
            let cub = paint_cubic(paint_uv, &cX, &cY);
            pnt = mix(pnt, cub, near);
            gX = cX * near;
            gY = cY * near;
        }
        g_ink_near = near;
        let amt = pnt.a;
        let fw = fwidth(amt);
        let w = clamp(fw * 0.8, 0.008, 0.25);
        g_ink = smoothstep(0.5 - w, 0.5 + w, amt);
        let ia = 1.0 / max(amt, 0.02);
        let tRaw = clamp(pnt.r * ia, 0.0, 1.0);
        let wT = clamp(fwidth(tRaw) * 0.75, 0.012, 0.2);
        let tm = smoothstep(0.5 - wT, 0.5 + wT, tRaw);
        let wet = clamp(pnt.g * ia, 0.0, 1.0);
        let fresh = smoothstep(0.3, 0.97, wet);
        g_fresh = fresh * g_ink;
        let tone = clamp(pnt.b * ia, 0.0, 1.0);
        let wallF = abs(wnorm.y) < 0.5;
        let Tf = normalize(face_tan - wnorm * dot(face_tan, wnorm));
        let Bf = cross(wnorm, Tf);
        // sun direction: u_sun_dir uniform (deviation vs three directionalLights[0])
        let sunW = normalize(u_sun_dir);
        let sunF = vec2<f32>(dot(sunW, Tf), dot(sunW, Bf));
        let s = clamp((amt - 0.5) * 1.7, 0.0, 1.0);
        let hs = 1.0 - (1.0 - s) * (1.0 - s);
        g_ink_s = hs;
        let thick = select(1.0, 0.6, wallF) * (1.0 + 0.5 * fresh);
        g_ink_keep = select(0.0, 0.32 * (1.0 - 0.5 * fresh), wallF);
        let gradA = vec2<f32>(gX.a, gY.a);
        g_ink_d = gradA * 2.0 * (1.0 - s) * 1.7 * 1.9 * g_ink * thick;
        // team seam lip (≡ L148-163)
        let gT = (vec2<f32>(gX.r, gY.r) - tRaw * gradA) * ia;
        let gTl = length(gT);
        var seam = 0.0;
        var seamShadow = 0.0;
        if (gTl > 0.03 && amt > 0.6) {
            let sn = gT / gTl;
            let x = (tRaw - 0.5) / gTl;
            let e1 = exp(-x * x / 1.7);
            let e2 = exp(-x * x / 0.8);
            seam = e1 * smoothstep(0.03, 0.14, gTl) * near;
            let gW = (vec2<f32>(gX.g, gY.g) - wet * gradA) * ia;
            let newer = clamp(dot(gW, sn) / gTl * 1.6, -1.0, 1.0);
            let dh = newer * 2.4 * e1 / 2.3 + 0.9 * 2.0 * x / 0.8 * e2 * (1.0 - 0.6 * abs(newer));
            g_ink_d = g_ink_d + sn * dh * seam * g_ink * thick;
            seamShadow = seam * (0.45 * max(0.0, -x * newer) * e1 + 0.55 * e2 * (1.0 - 0.7 * abs(newer)));
        }
        let team = mix(u_team_a, u_team_b, tm);
        var inkCol = team * (0.93 + 0.13 * tone);
        let lip = (1.0 - hs) * near;
        inkCol = mix(inkCol, inkCol * 1.16 + team * 0.05, lip * 0.35);
        inkCol = inkCol * (1.0 - 0.05 * hs);
        inkCol = inkCol * (1.0 - 0.3 * seamShadow * g_ink);
        inkCol = inkCol * (1.0 + 0.07 * fresh * fresh);
        g_ink_col = inkCol;
        let halo = smoothstep(0.06, 0.5, amt) * (1.0 - g_ink) * near;
        if (!wallF) {
            let outN = -gradA / max(length(gradA), 1e-4);
            let away = clamp(0.55 - 0.9 * dot(outN, normalize(sunF + vec2<f32>(1e-4, 1e-4))) * length(sunF), 0.25, 1.3);
            let h2 = halo * smoothstep(0.4, 0.85, wnorm.y) * away;
            base = base * mix(vec3<f32>(1.0, 1.0, 1.0), team * 0.45 + vec3<f32>(0.3, 0.3, 0.3), h2 * 0.5);
        } else {
            base = base * mix(vec3<f32>(1.0, 1.0, 1.0), team * 0.35 + vec3<f32>(0.5, 0.5, 0.5), halo * 0.55);
        }
        base = mix(base, g_ink_col, g_ink);
    }
    // ≡ L606 diffuseColor.rgb = base
    pbr_input.material.base_color = vec4<f32>(base, out_alpha);

    // ============================================================ roughness / metalness
    // ≡ INK_ROUGH (inkShading L189-190) at roughnessmap_fragment; three's
    // roughnessFactor is perceptual -> Bevy perceptual_roughness 1:1.
    pbr_input.material.perceptual_roughness = mix(g_base_rough, mix(0.26, 0.14, g_fresh), g_ink);
    // ≡ levelMaterial L610-613 at metalnessmap_fragment
    pbr_input.material.metallic = g_tex_orm.b * (1.0 - g_ink);

    // ============================================================ normal block
    // ≡ levelMaterial L614-693 (normal_fragment_maps injection).
    var slope = g_ink_d;
    // ---- INK_GEL (inkShading L194-218) ----
    if (g_ink > 0.01) {
        var gp = fu * select(vec2<f32>(0.75, 0.75), vec2<f32>(0.9, 0.42), abs(wnorm.y) < 0.5);
        let fr2 = g_fresh * g_fresh;
        gp = gp + vec2<f32>(u_time * 0.011, -u_time * 0.007)
           + fr2 * 0.022 * vec2<f32>(sin(u_time * 2.3), cos(u_time * 1.9));
        if (u_gel >= 0.0) {   // ≡ L203; ⑤ default -1 = inactive
            let gx = dpdx(gp);
            let gy = dpdy(gp);
            let gn = textureSampleGrad(t_normal, texlib_sampler, gp, i32(u_gel), gx, gy).xyz * vec3<f32>(2.0, 2.0, 2.0) - vec3<f32>(1.0, 1.0, 1.0);
            slope = slope + -gn.xy / max(gn.z, 0.3) * (0.95 - 0.5 * g_fresh) * g_ink * g_ink_s;
        }
    }
    // (the JS `#ifndef USE_TEXLIB` gel fallback L209-217 and corrugation
    //  analytic slope L619-624 are dead under USE_TEXLIB and not ported)
    let T = normalize(face_tan - wnorm * dot(face_tan, wnorm));  // ≡ L625
    let Bt = cross(wnorm, T);                                     // ≡ L626
    // ---- swim wakes (≡ L627-682) ----
    if (g_ink > 0.01) {
        var wS = vec3<f32>(0.0, 0.0, 0.0);
        var wG = 0.0;
        for (var s: i32 = 0; s < 4; s = s + 1) {
            let Bd = uWakeB[u32(s)];
            if (Bd.w <= 0.0 || distance(wpos, Bd.xyz) > Bd.w) { continue; }
            for (var i: i32 = 0; i < 11; i = i + 1) {
                let A0 = uWake[u32(s * 12 + i)];
                let A1 = uWake[u32(s * 12 + i + 1)];
                if (A0.w < -1.0 || A1.w < -1.0) { continue; }
                let ab = A1.xyz - A0.xyz;
                let t = clamp(dot(wpos - A0.xyz, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
                let age = u_time - mix(A0.w, A1.w, t);
                if (age <= 0.0 || age > 1.15) { continue; }
                var D = wpos - (A0.xyz + ab * t);
                D = D - wnorm * dot(D, wnorm);
                let d = length(D);
                let r = 0.1 + age * 0.95;
                let w = 0.075 + age * 0.085;
                let x = d - r;
                let env = exp(-x * x / (w * w));
                if (env < 0.003) { continue; }
                var fade = 1.0 - age / 1.15;
                fade = fade * fade * smoothstep(0.0, 0.05, age);
                let amp = 0.0135 * fade;
                let dh = amp * env * (-2.0 * x / (w * w) * cos(x * 25.0) - 25.0 * sin(x * 25.0));
                wS = wS + D * (dh / max(d, 1e-4));
                wG = wG + env * fade * 0.6;
            }
            let H = uSwimH[u32(s)];
            let F = uSwimF[u32(s)];
            if (H.w > 0.002) {
                var D2 = wpos - H.xyz;
                D2 = D2 - wnorm * dot(D2, wnorm);
                var Fp = F.xyz - wnorm * dot(F.xyz, wnorm);
                let fl = length(Fp);
                Fp = select(Fp / fl, T, fl <= 1e-3);
                let al = dot(D2, Fp);
                let Cv = D2 - Fp * al;
                let ac = length(Cv);
                let Cn = Cv / max(ac, 1e-4);
                let am = al + 0.1;
                let La = 0.3 + 0.2 * F.w;
                let Lc = 0.19;
                let hm = 0.05 * H.w * exp(-(am * am) / (La * La) - (ac * ac) / (Lc * Lc));
                let ab2 = al - 0.3;
                let hb = 0.022 * H.w * F.w * exp(-(ab2 * ab2) / 0.012 - (ac * ac) / 0.07);
                wS = wS + Fp * (-2.0 * am / (La * La) * hm - 2.0 * ab2 / 0.012 * hb)
                        + Cn * (-2.0 * ac / (Lc * Lc) * hm - 2.0 * ac / 0.07 * hb);
                wG = wG + hm * 16.0 + hb * 20.0;
            }
        }
        let wl = length(wS);
        if (wl > 0.9) { wS = wS * (0.9 / wl); }
        let wFar = 1.0 - smoothstep(0.05, 0.16, length(fwidth(wpos)));
        slope = slope + vec2<f32>(dot(wS, T), dot(wS, Bt)) * g_ink * wFar;
        g_wake = clamp(wG, 0.0, 1.0) * g_ink * wFar;
    }
    // ---- INK_SLOPE ripples (inkShading L221-253) ----
    if (g_ink > 0.01) {
        var rS = vec3<f32>(0.0, 0.0, 0.0);
        var rG = 0.0;
        for (var i: i32 = 0; i < 24; i = i + 1) {   // ≡ INK_RIPPLES = 24 (inkShading L22)
            let Pr = uRip[u32(i)];
            let Qr = uRipP[u32(i)];
            let age = u_ink_clock - Pr.w;
            if (age <= 0.0 || age >= Qr.w) { continue; }
            var D = wpos - Pr.xyz;
            let off = dot(D, wnorm);
            if (abs(off) > 0.4) { continue; }
            D = D - wnorm * off;
            let front = Qr.z * age;
            let d = length(D);
            let x = d - front;
            let sg = Qr.y * 0.8;
            if (x > sg * 3.0 || x < -Qr.y * 4.0) { continue; }
            let env = exp(-x * x / (sg * sg));
            let k = 6.2831853 / Qr.y;
            var fade = 1.0 - age / Qr.w;
            fade = fade * fade * smoothstep(0.0, 0.03, age);
            let amp = Qr.x * fade / (1.0 + d * 1.6);
            let dh = amp * env * (-2.0 * x / (sg * sg) * cos(k * x) - k * sin(k * x));
            rS = rS + D * (dh / max(d, 1e-4));
            rG = rG + env * fade * min(Qr.x * 90.0, 1.0);
        }
        let rl = length(rS);
        if (rl > 0.75) { rS = rS * (0.75 / rl); }
        let rFar = 1.0 - smoothstep(0.05, 0.18, length(fwidth(wpos)));
        slope = slope + vec2<f32>(dot(rS, T), dot(rS, Bt)) * g_ink * rFar;
        g_wake = max(g_wake, clamp(rG * 0.5, 0.0, 1.0) * g_ink * rFar);
    }
    // ---- relief + ink slopes -> final normal (≡ L684-692) ----
    var nBase = wnorm;
    let corrug = select(0.0, 0.55, u_pattern > 4.5 && u_pattern < 5.5);  // ≡ L688
    let keep = max(corrug, g_tex_keep);
    nBase = texlib_perturb_normal(g_tex_n, T, Bt, wnorm,
        g_tex_str * (1.0 - g_ink * (1.0 - max(keep, g_ink_keep))));
    let wn = normalize(nBase - slope.x * T - slope.y * Bt);
    // JS transforms wn into view space (L692); Bevy's PbrInput.N is world space.
    pbr_input.N = wn;
    // ≡ clearcoatNormal = normal (levelMaterial L704-705): the coat shares the
    // perturbed normal - no-op here (deviation ②, no clearcoat layer).

    // ============================================================ lamp pools
    // ≡ LAMP_LIGHTS (levelMaterial.js L12-24). three's RE_Direct has no Bevy
    // forward hook (deviation, REVIEW_CHECKLIST): the distance window, w^4
    // falloff and inverse-square colour are ported verbatim; the BRDF is
    // approximated as a Lambert pool + an extra wet-coat glint on fresh ink,
    // folded into `emissive` before lighting.
    var lamp_emissive = vec3<f32>(0.0, 0.0, 0.0);
    for (var i: i32 = 0; i < u_lamp_n; i = i + 1) {
        let lw = uLamps[u32(i)] - wpos;
        let d2 = dot(lw, lw);
        if (d2 > 100.0) { continue; }          // ≡ LAMP_R^2 = 100 (L11/L16)
        let w = clamp(1.0 - d2 * d2 / 10000.0, 0.0, 1.0);  // ≡ L18 (LAMP_R^4)
        let rad = u_lamp_col * (w * w / max(d2, 0.35));    // ≡ L21
        let ndl = max(dot(wn, normalize(lw)), 0.0);
        lamp_emissive = lamp_emissive + base * rad * (ndl / 3.14159265) * (1.0 + 2.0 * g_ink * g_fresh);
    }

    // ============================================================ INK_LIGHTS
    // ≡ inkShading.js L261-279. Deviation ②: no clearcoat layer - the coat is
    // folded into the single GGX lobe + reflectance.
    {
        let cdu = dpdx(wn);
        let cdv = dpdy(wn);
        let kern = min(0.3 * (dot(cdu, cdu) + dot(cdv, cdv)), 0.18);   // ≡ L265-266
        let cr = mix(mix(0.06, 0.028, g_fresh), 0.022, g_wake);        // ≡ L267 clearcoatRoughness
        var pr = pbr_input.material.perceptual_roughness;
        // ≡ L270 material.roughness widening, verbatim on the perceptual value
        pr = mix(pr, min(sqrt(sqrt(pow(pr, 4.0) + kern)), 1.0), g_ink);
        // ≡ L268-269 material.clearcoat = gInk / clearcoatRoughness: the smooth
        // coat lobe dips the roughness toward sqrt(cr) (linear->perceptual).
        pr = mix(pr, min(pr, sqrt(cr)), g_ink);
        pbr_input.material.perceptual_roughness = pr;
        // ≡ L272-273 specularColor *= 1 - 0.7 gInk -> Bevy reflectance (F0
        // proxy; three's default specular 0.5 == Bevy's default reflectance 0.5).
        pbr_input.material.reflectance = pbr_input.material.reflectance * (1.0 - 0.7 * g_ink);
        // L274-278 (specularF90 / clearcoatF0 / clearcoatF90 grazing Fresnel
        // taming) have no single-layer equivalent; folded into the modulation
        // above - registered REVIEW_CHECKLIST.
    }

    // ============================================================ INK_LIGHT_MAPS
    // ≡ inkShading.js L282-300. three's irradiance / iblIrradiance /
    // clearcoatRadiance are internal to apply_pbr_lighting (deviation,
    // REVIEW_CHECKLIST). The hue-preserving ambient (L284-289) and the tinted,
    // boosted wet reflection (L291-299) are approximated as a grazing
    // Fresnel-weighted emissive sheen carrying the same coefficients.
    var sheen = vec3<f32>(0.0, 0.0, 0.0);
    if (g_ink > 0.0) {
        const LW = vec3<f32>(0.2126, 0.7152, 0.0722);   // ≡ L285
        let ndv = max(dot(wn, pbr_input.V), 0.0);
        let fres = pow(1.0 - ndv, 3.0);
        let tint = g_ink_col / max(dot(g_ink_col, LW), 1e-3);          // ≡ L296
        let boost = 1.0 + g_ink * (0.25 + 0.3 * g_fresh) + g_wake * 0.7; // ≡ L298
        sheen = g_ink_col * min(tint, vec3<f32>(2.5, 2.5, 2.5)) * (0.6 * g_ink * fres) * boost;
    }

    // ============================================================ INK_EMISSIVE
    // ≡ inkShading.js L257-258 (totalEmissiveRadiance += ...)
    let ink_emissive = g_ink_col * g_ink * u_ink_glow * (0.75 + 0.35 * g_ink_s);

    // ---- aomap_fragment (levelMaterial L696-703): ④ no lightmap atlas in
    // Rust; u_ao stays 0 and the reflectedLight modulation is deferred
    // (Task 55). pbr_input.diffuse_occlusion/specular_occlusion are the
    // future anchors. ----

    pbr_input.material.emissive = vec4<f32>(
        pbr_input.material.emissive.rgb + ink_emissive + lamp_emissive + sheen,
        pbr_input.material.emissive.a,
    );

    // ============================================================ standard tail
    pbr_input.material.base_color =
        alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    if ((pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u) {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }

    // ============================================================ INK_SHADE
    // ≡ inkShading.js L303-314. reflectedLight.directDiffuse/indirectDiffuse
    // are internal to apply_pbr_lighting (deviation, REVIEW_CHECKLIST): the
    // shade term is proxied from the lit result's luminance and the "rich"
    // same-luma deeper-chroma fill is added post-lighting with the same
    // 0.5 x shade / (1 + 0.6 x shade) / gInk mixing weights.
    if (g_ink > 0.0) {
        const LW = vec3<f32>(0.2126, 0.7152, 0.0722);   // ≡ L305
        let luma = dot(out.color.rgb, LW);
        let shade = 1.0 - smoothstep(0.1, 0.6, luma);
        let c2 = g_ink_col * g_ink_col;
        let rich = c2 * (dot(g_ink_col, LW) / max(dot(c2, LW), 1e-4));  // ≡ L309
        let idRich = rich * (luma / max(dot(g_ink_col, LW), 1e-4));     // ≡ L311
        let idNew = mix(vec3<f32>(luma, luma, luma), idRich, 0.5 * shade) * (1.0 + 0.6 * shade); // ≡ L312
        out.color = vec4<f32>(out.color.rgb + (idNew - vec3<f32>(luma, luma, luma)) * g_ink, out.color.a);  // ≡ L313
    }

    // ≡ levelMaterial L710: outgoingLight = min(outgoingLight, vec3(5.0))
    out.color = vec4<f32>(min(out.color.rgb, vec3<f32>(5.0, 5.0, 5.0)), out.color.a);

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
