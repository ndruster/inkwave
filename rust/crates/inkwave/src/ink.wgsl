// Task 11 - ink presentation extension for StandardMaterial.
//
// Samples the ink atlas (UV1 = per-face atlas rect, see `InkAtlas` in the sim)
// and blends team ink over the base surface colour before lighting. Mirrors
// upstream `src/world/inkShading.js`: atlas RGBA = (team share, wetness, tone,
// coverage), premultiplied by coverage; the shader un-premultiplies R/A to
// recover the team share.
//
// Binding numbers avoid StandardMaterial's own (0 = uniform, 1/2 = base colour
// texture/sampler, ...); the extension bind group is merged into the material
// group (3) by `ExtendedMaterial`.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}

@group(3) @binding(20)
var ink_atlas: texture_2d<f32>;
@group(3) @binding(21)
var ink_sampler: sampler;

// Default team palettes, config.js TEAM_PALETTES[0] (tangerine / cobalt),
// converted from sRGB to linear.
const TEAM_A: vec3<f32> = vec3<f32>(1.0, 0.254, 0.0061); // #ff8a14
const TEAM_B: vec3<f32> = vec3<f32>(0.0284, 0.105, 1.0); // #2f5bff

@fragment
fn fragment(vertex_output: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var in = vertex_output;
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // ---- ink blend (before lighting, so the ink is lit like the surface) ----
    // Uniform control flow: the atlas sample runs unconditionally (WGSL
    // forbids texture samples inside non-uniform branches).
    let ink = textureSample(ink_atlas, ink_sampler, in.uv_b);
    let cov = select(0.0, clamp(ink.a, 0.0, 1.0), ink.a > 0.002);
    if (cov > 0.0) {
        // premultiplied share -> team mix; interior texels (a ~ 1) are exact
        let share = clamp(ink.r / max(ink.a, 1e-3), 0.0, 1.0);
        var ink_rgb = mix(TEAM_A, TEAM_B, share);
        // per-splat tone jitter (atlas B = hsh(seed * 1.73)): brightness plus a
        // small warm/cool shift, mirroring inkShading.js's hue wobble.
        let wob = (ink.b - 0.5) * 0.22;
        ink_rgb = ink_rgb * (1.0 + wob) + vec3<f32>(wob * 0.05, 0.0, -wob * 0.05);
        // wet sheen (from simple): wet ink is glossier than the dry surface.
        let wet = clamp(ink.g / max(ink.a, 1e-3), 0.0, 1.0);
        pbr_input.material.perceptual_roughness =
            mix(pbr_input.material.perceptual_roughness,
                mix(0.45, 0.22, wet), cov);
        pbr_input.material.base_color = vec4<f32>(
            mix(pbr_input.material.base_color.rgb, ink_rgb, cov),
            pbr_input.material.base_color.a,
        );
    }

    // ---- standard forward PBR tail (mirrors bevy_pbr's pbr.wgsl) ----
    pbr_input.material.base_color =
        alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    if ((pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u) {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
