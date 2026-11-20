// M6 Task 71 — fullscreen grade + screenfx-composite post pass.
// Ported expression-for-expression from src/core/renderer.js L34-60 (GradeShader)
// and the grading-type / hurt / flash terms of src/fx/screenfx.js (InkwaveScreenFX).
//
// The vertex stage is Bevy's built-in fullscreen triangle. One draw covers
// the whole screen, so no per-vertex data is authored here.
//
// D-M6-1: upstream runs GradeShader and the screenfx ShaderPass as two
// separate passes; here they are folded into one fullscreen pass. D-M6-2:
// GTAO/SMAA bodies are not wired. D-M6-3: only grading-type saturation,
// hurt, flash and vignette are on the GPU path; the remaining screenfx
// uniforms stay on the DOM/CSS overlay data plane (Task 64). D-M6-4: the
// fragment target is Rgba8UnormSrgb (Bevy tonemaps before this set).

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> fx: PostFxUniforms;

// 16-byte-aligned: every member is a vec4 (WebGL2-safe). Field order mirrors
// `fx::post_pass::PostFxUniforms` byte-for-byte.
struct PostFxUniforms {
    // x=uSat y=uVib z=uContrast w=uLift
    grade_a: vec4<f32>,
    // x=uExposure y=uVignette z=uHurt(grade) w=uFlash(grade)
    grade_b: vec4<f32>,
    // x=uHurtColor.r y=.g z=.b w=uAspect
    grade_c: vec4<f32>,
    // x=uShadowTint.r y=.g z=.b w=pass_enabled (0 => skip write)
    grade_d: vec4<f32>,
    // x=uHighTint.r y=.g z=.b w=composite_hurt
    grade_e: vec4<f32>,
    // x=composite_flash y=composite_sat z=composite_desat w=reserved
    grade_f: vec4<f32>,
};

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    var color = textureSample(screen_texture, screen_sampler, uv).rgb;

    // pass_enabled == 0 : identity. The render node also short-circuits before
    // post_process_write(), so this branch is only a safety net.
    if (fx.grade_d.w < 0.5) {
        return vec4<f32>(color, 1.0);
    }

    let sat = fx.grade_a.x;
    let vib = fx.grade_a.y;
    let contrast = fx.grade_a.z;
    let lift = fx.grade_a.w;
    let exposure = fx.grade_b.x;
    let vignette = fx.grade_b.y;
    let hurt = fx.grade_b.z;
    let flash = fx.grade_b.w;
    let aspect = fx.grade_c.w;
    let shadow_tint = fx.grade_d.xyz;
    let high_tint = fx.grade_e.xyz;

    // ---- GradeShader (≡ src/core/renderer.js L40-59) ------------------------
    color *= exposure;                                           // L40
    let l = dot(color, LUMA);                                    // L41 (pre-sat luma, used by split tone)
    let mx = max(color.r, max(color.g, color.b));                // L43
    let mn = min(color.r, min(color.g, color.b));                // L43
    let chroma = (mx - mn) / max(mx, 1e-4);                      // L44
    // L45: vibrance raises muted colours only; team ink barely moves.
    color = max(mix(vec3<f32>(l), color, sat + vib * (1.0 - smoothstep(0.1, 0.7, chroma))), vec3<f32>(0.0));
    // L47: log-space contrast around mid grey keeps HDR highlights ordered.
    color = 0.18 * pow(max(color, vec3<f32>(1e-6)) / 0.18, vec3<f32>(contrast)) + lift;
    // L49-50: split tone protects strongly saturated team ink.
    let lt = smoothstep(0.015, 0.55, l);
    color *= mix(vec3<f32>(1.0), mix(shadow_tint, high_tint, lt), 1.0 - 0.85 * smoothstep(0.35, 0.8, chroma));
    // L51-54: corner vignette.
    let q = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let r = length(q);
    let vig = smoothstep(0.55, 1.25, r);
    color *= 1.0 - vignette * vig;
    // L56-59: low-health drains saturation + darkens the rim; flash is additive.
    let lum = dot(color, LUMA);
    color = mix(color, vec3<f32>(lum), hurt * 0.45);
    color *= 1.0 - hurt * 0.25 * smoothstep(0.4, 1.2, r);
    color += flash;

    // ---- screenfx composite terms (≡ src/fx/screenfx.js) --------------------
    // L169: grading-type saturation with desaturation (satPop folded upstream
    // into the value already carried by grade_f.y).
    let l0 = dot(color, LUMA);
    color = max(mix(vec3<f32>(l0), color, fx.grade_f.y * (1.0 - fx.grade_f.z)), vec3<f32>(0.0));
    // L200-203: heartbeat / hurt vignette (heart colour term stays on the DOM
    // overlay plane; here the rim darkening only).
    let comp_hurt = fx.grade_e.w;
    if (comp_hurt > 0.001) {
        let v = smoothstep(0.42, 1.08, r);
        let k = clamp(v * comp_hurt * 0.5, 0.0, 0.9);
        color = mix(color, color * 0.3, k);
    }
    // L325-327: edge-weighted whiteout flash; the centre stays readable.
    let comp_flash = fx.grade_f.x;
    if (comp_flash > 0.001) {
        color = mix(color, vec3<f32>(dot(color, LUMA)), clamp(comp_flash * 0.35, 0.0, 0.5));
        color += vec3<f32>(1.0) * comp_flash * (0.2 + 0.8 * smoothstep(0.12, 0.95, r));
    }

    return vec4<f32>(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
