//! M6 Task 71 — the fullscreen post-process PASS BODY (grade + screenfx
//! composite). M5 shipped the data planes ([`super::post_process`] and
//! [`super::composite`]) but explicitly did not wire a GPU pass (D-T65-3 /
//! D-T64-2). This module is the first render-app code in the port and binds
//! those extracted values to a real Bevy 0.19.1 fullscreen pass.
//!
//! API reference (read at implementation time): the official Bevy **v0.19.1**
//! `examples/shader_advanced/custom_post_processing.rs`:
//! - `ExtractComponentPlugin` + `UniformComponentPlugin` extract a main-world
//!   component into a render-world uniform buffer;
//! - a `RenderStartup` system builds the pipeline;
//! - a `Core3d` system in `Core3dSystems::PostProcess` issues
//!   `ViewTarget::post_process_write()` and draws the fullscreen triangle.
//!
//! Deviations (D-M6-1..4, registered in M6 REVIEW_CHECKLIST): two upstream
//! ShaderPasses folded into one pass; GTAO/SMAA bodies still unwired; only
//! vignette/hurt/flash/sat are on the GPU path; target format fixed to
//! `Rgba8UnormSrgb`. All GPU *visuals* remain a user-machine review item — the
//! sandbox only anchors the uniform values and the pass gate (double track).

use std::sync::OnceLock;

use bevy::{
    asset::{embedded_asset, load_embedded_asset},
    core_pipeline::{Core3dSystems, FullscreenShader, schedule::Core3d},
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        extract_component::{
            ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
            UniformComponentPlugin,
        },
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        view::ViewTarget,
    },
};

use super::composite::PassActivity;
use super::post_process::{effective_combo, grade_defaults};
use crate::environment::{EnvParams, Grade};

static POST_SHADER: OnceLock<Handle<Shader>> = OnceLock::new();

/// The per-camera uniform carrying the grade + composite terms. Every member
/// is a `Vec4` so the struct is 16-byte aligned on both WebGPU and WebGL2
/// (the official example's `#[cfg(webgl2)] _webgl2_padding` is unnecessary
/// when all fields are vec4). Mirrors `post_shader.wgsl::PostFxUniforms`.
#[derive(Component, Clone, Copy, ExtractComponent, ShaderType)]
pub struct PostFxUniforms {
    /// x=sat y=vib z=contrast w=lift.
    pub grade_a: Vec4,
    /// x=exposure y=vignette z=hurt w=flash.
    pub grade_b: Vec4,
    /// x=hurt_color.r y=.g z=.b w=aspect.
    pub grade_c: Vec4,
    /// x=shadow_tint.r y=.g z=.b w=pass_enabled (0 = identity).
    pub grade_d: Vec4,
    /// x=high_tint.r y=.g z=.b w=composite_hurt.
    pub grade_e: Vec4,
    /// x=composite_flash y=composite_sat z=desat w=reserved.
    pub grade_f: Vec4,
}

impl Default for PostFxUniforms {
    fn default() -> Self {
        // Identity grade defaults from the extracted doc; pass off so a camera
        // without a sync tick is a no-op.
        let g = grade_defaults();
        Self {
            grade_a: Vec4::new(g.sat as f32, g.vib as f32, g.contrast as f32, g.lift as f32),
            grade_b: Vec4::new(
                g.exposure as f32,
                g.vignette as f32,
                g.hurt as f32,
                g.flash as f32,
            ),
            grade_c: Vec4::new(
                g.hurt_color[0] as f32,
                g.hurt_color[1] as f32,
                g.hurt_color[2] as f32,
                g.aspect as f32,
            ),
            grade_d: Vec4::new(
                g.shadow_tint[0] as f32,
                g.shadow_tint[1] as f32,
                g.shadow_tint[2] as f32,
                0.0,
            ),
            grade_e: Vec4::new(
                g.high_tint[0] as f32,
                g.high_tint[1] as f32,
                g.high_tint[2] as f32,
                0.0,
            ),
            grade_f: Vec4::ZERO,
        }
    }
}

/// Pure uniform construction: grade values come from the extracted `Grade`
/// (theme overlay already applied by `EnvState`), pass enable from the tier's
/// `effective_combo` gate, composite terms from the live `PassActivity`. Split
/// out of the Bevy system so headless unit tests can anchor every field
/// (TR-71.1). No hand-copied numbers: `grade_defaults()` is the sole source.
#[must_use]
pub fn build_uniforms(
    grade: &Grade,
    params: &EnvParams,
    activity: &PassActivity,
) -> PostFxUniforms {
    let mut u = PostFxUniforms {
        grade_a: Vec4::new(grade.sat, grade.vib, grade.contrast, grade.lift),
        grade_b: Vec4::new(grade.exposure, grade.vignette, grade.hurt, grade.flash),
        grade_c: Vec4::new(
            grade.hurt_color[0],
            grade.hurt_color[1],
            grade.hurt_color[2],
            grade.aspect,
        ),
        grade_d: Vec4::new(
            grade.shadow_tint[0],
            grade.shadow_tint[1],
            grade.shadow_tint[2],
            0.0,
        ),
        grade_e: Vec4::new(
            grade.high_tint[0],
            grade.high_tint[1],
            grade.high_tint[2],
            0.0,
        ),
        grade_f: Vec4::ZERO,
    };

    // Upstream GradeShader's ShaderPass is UNCONDITIONAL inside the composer;
    // the whole chain is gated only by `post_enabled` (renderer.js L216). The
    // screenfx extraPass has its own `any` gate, but here it is folded into the
    // SAME draw as additive terms that are neutral when idle (sat=1,
    // desat=hurt=flash=0), so it must NOT gate the grade pass — otherwise an
    // idle high-tier frame would lose grade entirely (review P0-2).
    // `effective_combo("low")` is all-off even under an opt-in override, so
    // the low-tier保底 is preserved by `combo_on` alone.
    let combo = effective_combo(params.tier, params.post_enabled);
    let combo_on = combo.msaa > 0 || combo.bloom || combo.gtao || combo.smaa;
    let enabled = params.post_enabled && combo_on;
    u.grade_d.w = if enabled { 1.0 } else { 0.0 };

    // Composite terms from the live screenfx state (≡ the `SfxVars` that feed
    // the JS ShaderPass uniforms):
    //   uSat   = s.sat + s.satPop * 0.28   (screenfx.js L836; s.sat = 1 + delta)
    //   uDesat = s.desat                   (screenfx.js L835)
    //   hurt / flash are the heartbeat / whiteout magnitudes.
    // Idle ⇒ (1, 0, 0, 0) : neutral, so grade runs alone with no composite effect.
    u.grade_e.w = activity.hurt as f32;
    let composite_sat = 1.0 + activity.sat_delta + activity.sat_pop * 0.28;
    u.grade_f = Vec4::new(
        activity.flash as f32,
        composite_sat as f32,
        activity.desat as f32,
        0.0,
    );
    u
}

/// Main-world system: sync the MainCamera's uniform component each frame from
/// the extracted/ theme-resolved grade and the live composite state.
pub fn post_fx_uniform_sync(
    env: Res<crate::environment::EnvState>,
    overlay: Res<super::composite::CompositeOverlay>,
    mut cameras: Query<&mut PostFxUniforms, With<crate::world::MainCamera>>,
) {
    let built = build_uniforms(&env.params.grade, &env.params, &overlay.activity);
    for mut u in &mut cameras {
        *u = built;
    }
}

/// Registers extraction + the render-app pipeline/node. Headless builds have
/// no `RenderApp`, so `get_sub_app_mut` returns `None` and no GPU code runs —
/// the component stays a plain data holder there.
pub struct PostFxPlugin;

impl Plugin for PostFxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "post_shader.wgsl");
        let handle: Handle<Shader> = load_embedded_asset!(app, "post_shader.wgsl");
        let _ = POST_SHADER.set(handle);
        app.add_plugins((
            ExtractComponentPlugin::<PostFxUniforms>::default(),
            UniformComponentPlugin::<PostFxUniforms>::default(),
        ));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.add_systems(RenderStartup, init_post_fx_pipeline);
        render_app.add_systems(Core3d, post_fx_node.in_set(Core3dSystems::PostProcess));
    }
}

#[derive(Resource)]
struct PostFxPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    pipeline_id: CachedRenderPipelineId,
}

/// Build the pipeline once at render startup, using Bevy's built-in fullscreen
/// triangle vertex state (≡ the v0.19.1 example's `FullscreenShader`).
fn init_post_fx_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    fullscreen_shader: Res<FullscreenShader>,
    pipeline_cache: Res<PipelineCache>,
) {
    let Some(shader) = POST_SHADER.get().cloned() else {
        return;
    };
    let layout = BindGroupLayoutDescriptor::new(
        "post_fx_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<PostFxUniforms>(true),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    let pipeline_id = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("post_fx_pipeline".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen_shader.to_vertex_state(),
        fragment: Some(FragmentState {
            shader,
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8UnormSrgb,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(PostFxPipeline {
        layout,
        sampler,
        pipeline_id,
    });
}

#[derive(Default)]
struct PostFxBindGroupCache {
    cached: Option<(TextureViewId, BindGroup)>,
}

fn post_fx_node(
    view: ViewQuery<(
        &ViewTarget,
        &PostFxUniforms,
        &DynamicUniformIndex<PostFxUniforms>,
    )>,
    pipeline: Option<Res<PostFxPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<PostFxUniforms>>,
    mut cache: Local<PostFxBindGroupCache>,
    mut ctx: RenderContext,
) {
    let Some(pipeline) = pipeline else { return };
    let (view_target, settings, settings_index) = view.into_inner();

    // Low tier / idle composite: skip the pass entirely so the main texture is
    // not flipped (zero cost, mirrors JS `this.pass.enabled = false`).
    if settings.grade_d.w < 0.5 {
        return;
    }
    let Some(p) = pipeline_cache.get_render_pipeline(pipeline.pipeline_id) else {
        return;
    };
    let Some(binding) = uniforms.uniforms().binding() else {
        return;
    };

    let post_process = view_target.post_process_write();
    let bind_group = match &mut cache.cached {
        Some((id, group)) if post_process.source.id() == *id => group,
        cached => {
            let group = ctx.render_device().create_bind_group(
                "post_fx_bind_group",
                &pipeline_cache.get_bind_group_layout(&pipeline.layout),
                &BindGroupEntries::sequential((
                    post_process.source,
                    &pipeline.sampler,
                    binding.clone(),
                )),
            );
            let (_, group) = cached.insert((post_process.source.id(), group));
            group
        }
    };

    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("post_fx_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post_process.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(p);
    pass.set_bind_group(0, bind_group, &[settings_index.index()]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::env_params;

    const SHADER: &str = include_str!("post_shader.wgsl");

    fn idle() -> PassActivity {
        PassActivity::default()
    }

    #[test]
    fn task71_shader_binding_matches_rust_uniform_struct() {
        // Review P0-1/P1-4: the sandbox has no GPU, so bind the WGSL interface
        // to the Rust struct with a structural anchor. This catches a missing
        // `var<uniform>` declaration or a field-count drift (both would fail
        // at pipeline creation on a real GPU). Full naga compilation is a
        // user-machine item (D-M6-4) — this is a static guard, not a fake pass.
        assert!(
            SHADER.contains("@group(0) @binding(2) var<uniform> fx: PostFxUniforms;"),
            "the uniform binding instance must be declared (P0-1)"
        );
        assert!(
            SHADER.contains(
                "#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput"
            ),
            "fullscreen vertex import must match Bevy 0.19.1"
        );
        assert!(
            SHADER.contains("@fragment\nfn fragment("),
            "fragment entry point"
        );
        // Exactly six vec4 members, in order, matching PostFxUniforms' fields.
        let struct_body = SHADER
            .split("struct PostFxUniforms {")
            .nth(1)
            .expect("uniform struct present")
            .split('}')
            .next()
            .expect("uniform struct closed");
        let members: Vec<&str> = struct_body
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("grade_") && l.contains("vec4"))
            .collect();
        assert_eq!(members.len(), 6, "six vec4 uniform members (WebGL2-safe)");
        for (i, name) in [
            "grade_a", "grade_b", "grade_c", "grade_d", "grade_e", "grade_f",
        ]
        .iter()
        .enumerate()
        {
            assert!(
                members[i].starts_with(name),
                "uniform member {i} must be {name}, got {}",
                members[i]
            );
        }
        // Every grade_* field access in the fragment body goes through `fx.`
        // (all six members, not just grade_a — review re-check observation).
        let frag = SHADER.split("@fragment").nth(1).unwrap_or("");
        for name in [
            "grade_a", "grade_b", "grade_c", "grade_d", "grade_e", "grade_f",
        ] {
            assert!(
                !frag.contains(&format!("{name}.")) || frag.contains(&format!("fx.{name}.")),
                "uniform field {name} must be accessed via the binding instance"
            );
        }
    }

    #[test]
    fn task71_default_is_identity_and_pass_off() {
        let u = PostFxUniforms::default();
        assert_eq!(u.grade_d.w, 0.0, "default camera must be a no-op");
        let g = grade_defaults();
        assert!((u.grade_a.x - g.sat as f32).abs() < 1e-6);
        assert!((u.grade_a.z - g.contrast as f32).abs() < 1e-6);
        assert!((u.grade_b.x - g.exposure as f32).abs() < 1e-6);
        for i in 0..3 {
            assert!((u.grade_c[i] - g.hurt_color[i] as f32).abs() < 1e-6);
        }
    }

    #[test]
    fn task71_low_tier_and_post_off_force_pass_off() {
        let grade = Grade::defaults();
        let mut low = env_params("day", "low");
        low.post_enabled = true; // even the opt-in override cannot revive low
        let u = build_uniforms(&grade, &low, &idle());
        assert_eq!(u.grade_d.w, 0.0, "low tier combo is all-off");

        let mut high = env_params("day", "high");
        high.post_enabled = false; // explicit post disable wins at every tier
        let u2 = build_uniforms(&grade, &high, &idle());
        assert_eq!(u2.grade_d.w, 0.0, "post_enabled=false forces all off");
    }

    #[test]
    fn task71_high_tier_idle_still_enables_grade() {
        // Review P0-2: GradeShader is unconditional inside the composer. An
        // idle screenfx (composite any=false) must NOT disable the grade pass.
        let grade = Grade::defaults();
        let high = env_params("day", "high");
        let u = build_uniforms(&grade, &high, &idle());
        assert_eq!(
            u.grade_d.w, 1.0,
            "grade must run even when composite is idle"
        );
        // Idle composite terms are neutral: sat=1, desat=hurt=flash=0.
        assert!((u.grade_f.y - 1.0).abs() < 1e-6, "idle composite sat = 1");
        assert_eq!(u.grade_f.x, 0.0);
        assert_eq!(u.grade_f.z, 0.0);
        assert_eq!(u.grade_e.w, 0.0);
    }

    #[test]
    fn task71_composite_activity_is_forwarded_neutrally() {
        let grade = Grade::defaults();
        let high = env_params("day", "high");
        let mut active = idle();
        active.hurt = 0.5;
        active.flash = 0.25;
        active.sat_delta = 0.1;
        active.sat_pop = 0.2;
        active.desat = 0.3;
        let u = build_uniforms(&grade, &high, &active);
        assert_eq!(u.grade_d.w, 1.0);
        assert!((u.grade_e.w - 0.5).abs() < 1e-6, "composite hurt forwarded");
        assert!(
            (u.grade_f.x - 0.25).abs() < 1e-6,
            "composite flash forwarded"
        );
        // uSat = s.sat + satPop*0.28 = (1 + sat_delta) + satPop*0.28.
        assert!((u.grade_f.y - (1.0 + 0.1 + 0.2 * 0.28)).abs() < 1e-6);
        assert!((u.grade_f.z - 0.3).abs() < 1e-6, "desat forwarded");
    }

    #[test]
    fn task71_grade_12_uniforms_are_pinned() {
        // Review P2-2: compare all 12 grade uniforms against the extracted doc,
        // not a handful of literals, so a data-plane drift cannot pass unnoticed.
        let g = grade_defaults();
        let grade = Grade::defaults();
        let high = env_params("day", "high");
        let u = build_uniforms(&grade, &high, &idle());
        assert!((f64::from(u.grade_a.x) - g.sat).abs() < 1e-6);
        assert!((f64::from(u.grade_a.y) - g.vib).abs() < 1e-6);
        assert!((f64::from(u.grade_a.z) - g.contrast).abs() < 1e-6);
        assert!((f64::from(u.grade_a.w) - g.lift).abs() < 1e-6);
        assert!((f64::from(u.grade_b.x) - g.exposure).abs() < 1e-6);
        assert!((f64::from(u.grade_b.y) - g.vignette).abs() < 1e-6);
        assert!((f64::from(u.grade_b.z) - g.hurt).abs() < 1e-6);
        assert!((f64::from(u.grade_b.w) - g.flash).abs() < 1e-6);
        for i in 0..3 {
            assert!((f64::from(u.grade_c[i]) - g.hurt_color[i]).abs() < 1e-6);
            assert!((f64::from(u.grade_d[i]) - g.shadow_tint[i]).abs() < 1e-6);
            assert!((f64::from(u.grade_e[i]) - g.high_tint[i]).abs() < 1e-6);
        }
        assert!((f64::from(u.grade_c.w) - g.aspect).abs() < 1e-6);
    }
}
