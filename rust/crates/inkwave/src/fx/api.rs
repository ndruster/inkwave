//! Task 51 batch B — the spawn API surface, ported from `src/fx/fx.js`
//! L1241-2204, plus the frame surface (L2206-2261). Every numeric literal is
//! anchored to its JS line (`// ≡ fx.js L###`) and the `rand()` consumption
//! order is preserved exactly (JS evaluates call arguments left-to-right).

use glam::Vec3;

use super::particles::{
    ColorIn, F_NOCOL, F_PAINT, F_QUIET, F_RING, Fx, G_BUBBLE, G_SOFT, G_STAR, MarkOpts, OP_RING,
    P_CLOUD, P_DUST, P_FEATHER, P_GHOST, P_MIST, PillarOpts, R_BLOT, R_DANGER, R_DISC, R_DUST,
    R_RIPPLE, R_TARGET, R_THIN, R_WAVE, SpriteInit, TAU, UP, WHITE, basis, clamp, col_lerp,
    dust_col, feather_col, foam_col, th_normalize, water_col,
};

// ---------------------------------------------------------------------------
// option bundles (≡ the JS `opts = EMPTY` defaulted objects)
// ---------------------------------------------------------------------------

/// `burst(pos, normal, color, opts)` options ≡ L1241-1268.
#[derive(Default, Clone, Copy, Debug)]
pub struct BurstOpts {
    /// ≡ `opts.count ?? 12` (L1243)
    pub count: Option<f32>,
    /// ≡ `opts.speed ?? 4` (L1244)
    pub speed: Option<f32>,
    /// ≡ `opts.size ?? 0.1` (L1244)
    pub size: Option<f32>,
    /// ≡ `opts.spread ?? 0.9` (L1244)
    pub spread: Option<f32>,
    /// ≡ `opts.gravity ?? 1` (L1244)
    pub gravity: Option<f32>,
    /// ≡ `opts.paint ? F_PAINT : 0` (L1245)
    pub paint: bool,
    /// ≡ `opts.sheet !== false` — default true (L1259)
    pub sheet: Option<bool>,
    /// ≡ `opts.ring === true` (L1264)
    pub ring: bool,
    /// ≡ `opts.mist === true` (L1265)
    pub mist: bool,
}

/// `drop(pos, vel, color, opts)` options ≡ L1271-1274.
#[derive(Default, Clone, Copy, Debug)]
pub struct DropOpts {
    /// ≡ `opts.size ?? 0.1`
    pub size: Option<f32>,
    /// ≡ `opts.life ?? 1.2`
    pub life: Option<f32>,
    /// ≡ `opts.gravity ?? 1`
    pub gravity: Option<f32>,
    /// ≡ `opts.stretch ?? 1`
    pub stretch: Option<f32>,
    pub paint: bool,
    pub ring: bool,
    pub quiet: bool,
    /// ≡ `opts.noCollide`
    pub no_collide: bool,
}

/// `ring(pos, normal, color, opts)` options ≡ L1278-1289.
#[derive(Default, Clone, Copy, Debug)]
pub struct RingOpts {
    /// ≡ `opts.radius ?? 1.5`
    pub radius: Option<f32>,
    /// ≡ `opts.life ?? 0.35`
    pub life: Option<f32>,
    /// ≡ `opts.style ?? R_WAVE`
    pub style: Option<f32>,
    /// ≡ `opts.alpha ?? 0.95`
    pub alpha: Option<f32>,
    /// ≡ `opts.thickness ?? 1`
    pub thickness: Option<f32>,
    /// ≡ `opts.snap !== false` — default true (L1282)
    pub snap: Option<bool>,
}

/// `rain(..., opts)` options ≡ L1466-1484.
#[derive(Default, Clone, Copy, Debug)]
pub struct RainOpts {
    pub paint: bool,
    /// ≡ `opts.cloud !== false` — default true (L1475)
    pub cloud: Option<bool>,
}

/// `muzzle(..., kind)` ≡ L1391 `'shooter' | 'blaster' | 'charger'`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuzzleKind {
    Shooter,
    Blaster,
    Charger,
}

/// `stats()` payload ≡ L2252-2254.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxStats {
    pub drops: usize,
    pub puffs: usize,
    pub glows: usize,
    pub rings: usize,
    pub shells: usize,
    pub beams: usize,
    pub checks: i32,
    pub sched: usize,
    pub cap_drops: usize,
    pub cap_puffs: usize,
    pub cap_glows: usize,
    pub cap_rings: usize,
}

/// `setLighting(env.getSkyColors())` input ≡ L2240-2249. The JS object
/// accepts `{sunDir, sun|sunColor, sunIntensity, zenith|sky, horizon, ground}`.
#[derive(Default, Clone, Copy, Debug)]
pub struct LightInput {
    pub sun_dir: Option<Vec3>,
    /// ≡ `o.sunColor || o.sun` (L2243)
    pub sun_color: Option<Vec3>,
    /// ≡ `o.sunIntensity ?? 3` (L2244)
    pub sun_intensity: Option<f32>,
    /// ≡ `o.sky || o.zenith` (L2245)
    pub sky: Option<Vec3>,
    pub horizon: Option<Vec3>,
    pub ground: Option<Vec3>,
}

// ---------------------------------------------------------------------------
// contract API (≡ fx.js L1241-1484)
// ---------------------------------------------------------------------------

impl Fx {
    /// `burst` ≡ L1241-1269.
    pub fn burst(&mut self, pos: Vec3, normal: Option<Vec3>, color: ColorIn, opts: BurstOpts) {
        let col = self.color(color);
        // ≡ L1243 Math.max(1, Math.round((opts.count ?? 12) * q))
        let count = (opts.count.unwrap_or(12.0) * self.q).round().max(1.0) as usize;
        let speed = opts.speed.unwrap_or(4.0); // ≡ L1244
        let size = opts.size.unwrap_or(0.1);
        let spread = opts.spread.unwrap_or(0.9);
        let grav = opts.gravity.unwrap_or(1.0);
        let paint = if opts.paint { F_PAINT } else { 0 }; // ≡ L1245
        // ≡ L1246: normal || UP, degenerate → UP, normalize
        let mut nrm = normal.unwrap_or(UP);
        if nrm.length_squared() < 1e-6 {
            nrm = UP;
        }
        let nrm = nrm.normalize_or_zero();
        // ≡ L1247
        let p0 = pos + nrm * 0.04;
        for _ in 0..count {
            // ≡ L1249 coneDir(_v3, Math.max(0.05, spread) * Math.PI * 0.5)
            let v = self.cone_dir(nrm, spread.max(0.05) * std::f32::consts::FRAC_PI_2);
            let sp = speed * (0.5 + self.rand() * 0.8); // ≡ L1250
            let sz = size * (0.35 + self.rand() * 0.6); // ≡ L1251
            let life = 1.6 + self.rand() * 0.6; // ≡ L1252
            self.spawn_drop(p0, v * sp, col, sz, life, grav, 1.3, paint);
            if self.rand() < 0.45 {
                // satellite ≡ L1253-1256
                let s2 = sp * (0.7 + self.rand() * 0.5);
                let sx = v.x * s2 + (self.rand() - 0.5) * 0.8;
                let sy = v.y * s2 + (self.rand() - 0.5) * 0.8;
                let sz2 = v.z * s2 + (self.rand() - 0.5) * 0.8;
                self.spawn_drop(p0, Vec3::new(sx, sy, sz2), col, sz * 0.4, 1.2, grav, 1.3, 0);
            }
        }
        // ≡ L1258 splash = size * (1.6 + 0.5 * Math.sqrt(count))
        let splash = size * (1.6 + 0.5 * (count as f32).sqrt());
        if count >= 4 && opts.sheet != Some(false) && self.near(pos, 24.0) {
            // ≡ L1260-1261
            let sp = pos + nrm * (splash * 0.12);
            self.shell(
                sp,
                col,
                splash * 0.35,
                splash * 1.1,
                0.07,
                0.17 + splash * 0.12,
                0.95,
                0.0,
                0.0,
                0.2,
                Some(nrm),
                1.0,
            );
        }
        // ≡ L1263 (col null → no decal ring without the ripple hook)
        self.ripple(pos, 0.0035 + size * 0.03, 0.1, 1.0, 0.5, nrm, None);
        // ≡ L1264
        if opts.ring {
            self.ring_raw(
                pos,
                nrm,
                col,
                0.22 + size * 3.2,
                0.26,
                R_WAVE,
                0.9,
                1.1,
                Vec3::ZERO,
            );
        }
        if opts.mist && count >= 6 {
            // ≡ L1265-1268
            let cb = col_lerp(col, WHITE, 0.3);
            self.sprite(
                true,
                SpriteInit::new(p0, nrm * 1.2, cb, size * 2.0, size * 5.0, 0.35, 0.3).drag(5.0),
            );
        }
    }

    /// `drop` ≡ L1271-1274.
    pub fn drop(&mut self, pos: Vec3, vel: Vec3, color: ColorIn, opts: DropOpts) {
        let col = self.color(color);
        let flags = (if opts.paint { F_PAINT } else { 0 })
            | (if opts.ring { F_RING } else { 0 })
            | (if opts.quiet { F_QUIET } else { 0 })
            | (if opts.no_collide { F_NOCOL } else { 0 });
        self.spawn_drop(
            pos,
            vel,
            col,
            opts.size.unwrap_or(0.1),
            opts.life.unwrap_or(1.2),
            opts.gravity.unwrap_or(1.0),
            opts.stretch.unwrap_or(1.0),
            flags,
        );
    }

    /// `ring` ≡ L1276-1289.
    pub fn ring(&mut self, pos: Vec3, normal: Option<Vec3>, color: ColorIn, opts: RingOpts) {
        let col = self.color(color);
        let n = normal.unwrap_or(UP); // ≡ L1280
        let mut p = pos;
        // ≡ L1282-1287: snap onto the surface behind `pos` along the normal
        if opts.snap != Some(false)
            && let Some(cb) = self.collider.as_mut()
        {
            let from = pos + n * 0.25;
            let to = pos - n * 1.4;
            let h = (cb)(from, to);
            let Some((hp, _)) = h else { return };
            p = hp + n * 0.02;
        }
        self.ring_raw(
            p,
            n,
            col,
            opts.radius.unwrap_or(1.5),
            opts.life.unwrap_or(0.35),
            opts.style.unwrap_or(R_WAVE),
            opts.alpha.unwrap_or(0.95),
            opts.thickness.unwrap_or(1.0),
            Vec3::ZERO,
        );
    }

    /// `explosion` ≡ L1295-1317.
    pub fn explosion(&mut self, pos: Vec3, color: ColorIn, radius: Option<f32>) {
        let col = self.color(color);
        let q = self.q;
        let r = radius.unwrap_or(3.0);
        let rv = r.min(2.4); // ≡ L1298 Rv = Math.min(R, 2.4)
        let paint = if self.paint_effects { F_PAINT } else { 0 }; // ≡ L1299
        let spd = (r / 3.0).sqrt(); // ≡ L1300
        // ≡ L1301-1302 hot flash
        self.sprite(
            false,
            SpriteInit::new(pos, Vec3::ZERO, col * 4.5, rv * 0.4, rv * 0.85, 0.12, 1.0)
                .kind(G_SOFT + 6.0)
                .fade_in(0.0),
        );
        // ≡ L1303-1304 soft core
        self.sprite(
            false,
            SpriteInit::new(pos, Vec3::ZERO, col * 1.15, rv * 0.9, rv * 1.6, 0.17, 0.5)
                .kind(G_SOFT)
                .fade_in(0.0),
        );
        // ≡ L1305 the ink sheet
        self.shell(
            pos,
            col,
            rv * 0.2,
            rv * 0.78,
            0.09,
            0.34,
            1.0,
            0.0,
            0.0,
            0.2,
            None,
            0.0,
        );
        // ≡ L1306 _burstDrops(pos, col, Rv, spd, paint, 26, 6, 18)
        self.burst_drops(pos, col, rv, spd, paint, 26.0, 6.0, 18.0);
        // ≡ L1307-1309 ground wave + ripple
        if let Some((gp, gn)) = self.probe_down(pos, r + 1.5) {
            let amp = 0.011 + 0.0028 * r;
            let wl = 0.24 + 0.03 * r;
            let sp = 2.1 + 0.4 * r;
            self.ripple(gp, amp, wl, sp, 1.05, gn, Some(col));
        }
        // ≡ L1310-1316 mist puffs
        let cm = col_lerp(col, WHITE, 0.42);
        let m = (4.0 * q).round() as usize;
        for _ in 0..m {
            let mut v = self.rand_sphere();
            v.y = v.y.abs() * 0.5;
            let d = r * (0.2 + self.rand() * 0.4);
            let vy = 0.5 + self.rand() * 0.6;
            let s0 = rv * (0.25 + self.rand() * 0.15);
            let s1 = rv * (0.55 + self.rand() * 0.25);
            let life = 0.55 + self.rand() * 0.3;
            self.sprite(
                true,
                SpriteInit::new(
                    pos + Vec3::new(v.x * d, v.y * d * 0.6, v.z * d),
                    Vec3::new(v.x * 1.8, vy, v.z * 1.8),
                    cm,
                    s0,
                    s1,
                    life,
                    0.12,
                )
                .drag(2.8)
                .buoy(0.2),
            );
        }
    }

    /// `_burstDrops` ≡ L1319-1339: ligaments / gloops / fine spray.
    #[allow(clippy::too_many_arguments)] // flat arg list mirrors the JS helper
    pub(crate) fn burst_drops(
        &mut self,
        pos: Vec3,
        col: Vec3,
        rv: f32,
        spd: f32,
        paint: u32,
        nl: f32,
        ng: f32,
        nf: f32,
    ) {
        let q = self.q;
        // ≡ L1321-1327 ligaments
        let n = (nl * q).round() as usize;
        for _ in 0..n {
            let mut v = self.rand_sphere();
            v.y = v.y.abs() * 0.85 + 0.12;
            v = v.normalize_or_zero();
            let sp = (6.0 + self.rand() * 7.0) * spd;
            let o = rv * 0.3;
            let sz = 0.03 + self.rand() * 0.03;
            let fl = if self.rand() < 0.3 { paint } else { 0 };
            self.spawn_drop(pos + v * o, v * sp, col, sz, 1.6, 1.0, 1.5, fl);
        }
        // ≡ L1328-1332 heavy gloops
        let n = (ng * q).round() as usize;
        for _ in 0..n {
            let a = self.rand() * TAU;
            let sp = (2.2 + self.rand() * 2.6) * spd;
            let vy = 3.2 + self.rand() * 3.0;
            let sz = 0.07 + self.rand() * 0.035;
            self.spawn_drop(
                Vec3::new(pos.x, pos.y + 0.2, pos.z),
                Vec3::new(a.cos() * sp, vy, a.sin() * sp),
                col,
                sz,
                2.2,
                1.0,
                1.0,
                paint,
            );
        }
        // ≡ L1333-1338 fine spray
        let n = (nf * q).round() as usize;
        for _ in 0..n {
            let mut v = self.rand_sphere();
            v.y = v.y.abs() * 0.7 + 0.2;
            v = v.normalize_or_zero();
            let sp = (4.0 + self.rand() * 6.0) * spd;
            let sz = 0.011 + self.rand() * 0.016;
            self.spawn_drop(pos, v * sp, col, sz, 1.0, 1.0, 1.3, 0);
        }
    }

    /// `splatted` ≡ L1342-1356.
    pub fn splatted(&mut self, pos: Vec3, color: ColorIn) {
        let col = self.color(color);
        let q = self.q;
        let paint = if self.paint_effects { F_PAINT } else { 0 };
        // ≡ L1346-1347
        self.sprite(
            false,
            SpriteInit::new(pos, Vec3::ZERO, col * 3.6, 0.8, 1.6, 0.14, 1.0)
                .kind(G_SOFT + 5.0)
                .fade_in(0.0),
        );
        // ≡ L1348
        self.shell(
            pos, col, 0.28, 0.9, 0.09, 0.34, 1.0, 0.0, 0.0, 0.2, None, 0.0,
        );
        // ≡ L1349 _burstDrops(pos, col, 1.2, 1, paint, 30, 7, 22)
        self.burst_drops(pos, col, 1.2, 1.0, paint, 30.0, 7.0, 22.0);
        // ≡ L1350-1354
        let cm = col_lerp(col, WHITE, 0.45);
        for _ in 0..((3.0 * q).round() as usize) {
            let v = self.rand_sphere();
            let vy = 0.8 + self.rand() * 0.5;
            let s1 = 1.0 + self.rand() * 0.3;
            let life = 0.6 + self.rand() * 0.3;
            self.sprite(
                true,
                SpriteInit::new(
                    pos + Vec3::new(v.x * 0.35, v.y * 0.3, v.z * 0.35),
                    Vec3::new(v.x * 1.4, vy, v.z * 1.4),
                    cm,
                    0.4,
                    s1,
                    life,
                    0.14,
                )
                .drag(2.0)
                .buoy(0.5),
            );
        }
        // ≡ L1355
        if let Some((gp, gn)) = self.probe_down(pos, 3.0) {
            self.ripple(gp, 0.014, 0.26, 2.4, 1.1, gn, Some(col));
        }
    }

    /// `wake` ≡ L1361-1378.
    pub fn wake(
        &mut self,
        pos: Vec3,
        dir: Vec3,
        color: ColorIn,
        speed: Option<f32>,
        normal: Option<Vec3>,
    ) {
        if !self.near(pos, 45.0) {
            return; // ≡ L1362
        }
        let col = self.color(color);
        let speed = speed.unwrap_or(8.0);
        let nrm = normal.unwrap_or(UP);
        let k = (speed / 11.8).min(1.0); // ≡ L1364
        let (rx, rz) = (dir.z, -dir.x); // ≡ L1365
        // ≡ L1366 n = Math.floor((0.5 + 3*k*k) * q + rand())
        let mut n = ((0.5 + 3.0 * k * k) * self.q + self.rand()).floor() as i64;
        while n > 0 {
            n -= 1;
            let back = speed * (0.06 + self.rand() * 0.1); // ≡ L1368
            let up = 1.5 + self.rand() * 1.3 + 1.5 * k; // ≡ L1369
            let side = (self.rand() - 0.5) * (0.7 + 0.9 * k); // ≡ L1370
            // ≡ L1371
            let v = Vec3::new(
                -dir.x * back + nrm.x * up + rx * side,
                -dir.y * back + nrm.y * up,
                -dir.z * back + nrm.z * up + rz * side,
            );
            let sz = 0.016 + self.rand() * 0.018 + 0.012 * k; // ≡ L1372
            self.spawn_drop(
                Vec3::new(pos.x - dir.x * 0.34, pos.y + 0.05, pos.z - dir.z * 0.34),
                v,
                col,
                sz,
                0.6,
                1.0,
                1.7,
                0,
            );
        }
        // ≡ L1374-1377 bubble glow
        if self.rand() < 0.28 * self.q + 0.08 {
            let cb = col_lerp(col, WHITE, 0.5) * 1.1;
            let px = pos.x - dir.x * (0.35 + self.rand() * 0.6) + (self.rand() - 0.5) * 0.3;
            let pz = pos.z - dir.z * (0.35 + self.rand() * 0.6) + (self.rand() - 0.5) * 0.3;
            let s0 = 0.045 + self.rand() * 0.04;
            let life = 0.3 + self.rand() * 0.15;
            self.sprite(
                false,
                SpriteInit::new(
                    Vec3::new(px, pos.y + 0.05, pz),
                    Vec3::new(0.0, 0.22, 0.0),
                    cb,
                    s0,
                    0.09,
                    life,
                    0.7,
                )
                .drag(1.0)
                .kind(G_BUBBLE + 1.0)
                .fade_in(0.03)
                .spin(0.0),
            );
        }
    }

    /// `swimCarve` ≡ L1381-1389.
    pub fn swim_carve(&mut self, pos: Vec3, dir: Vec3, out: Vec3, color: ColorIn, k: Option<f32>) {
        if !self.near(pos, 32.0) {
            return; // ≡ L1382
        }
        let col = self.color(color);
        let k = k.unwrap_or(1.0);
        // ≡ L1384 Math.max(1, Math.round((1 + 2.5*k) * q))
        let mut n = ((1.0 + 2.5 * k) * self.q).round().max(1.0) as usize;
        while n > 0 {
            n -= 1;
            // ≡ L1386 o, up, f (three rands)
            let o = 1.6 + self.rand() * 1.8 * k;
            let up = 1.8 + self.rand() * 1.6 * k;
            let f = (self.rand() - 0.3) * 1.2;
            let sz = 0.018 + self.rand() * 0.022; // ≡ L1387
            self.spawn_drop(
                Vec3::new(pos.x + out.x * 0.18, pos.y + 0.05, pos.z + out.z * 0.18),
                Vec3::new(out.x * o + dir.x * f, up, out.z * o + dir.z * f),
                col,
                sz,
                0.65,
                1.0,
                1.6,
                0,
            );
        }
    }
}

impl Fx {
    /// `muzzle` ≡ L1391-1435.
    pub fn muzzle(&mut self, pos: Vec3, dir: Vec3, color: ColorIn, kind: MuzzleKind) {
        let col = self.color(color);
        // ≡ L1393 _v3.copy(dir).normalize()
        let d = dir.normalize_or_zero();
        match kind {
            MuzzleKind::Blaster => {
                // ≡ L1395-1396
                self.sprite(
                    false,
                    SpriteInit::new(pos + d * 0.1, d * 2.0, col * 4.5, 0.35, 0.7, 0.09, 1.0)
                        .kind(G_SOFT + 4.0)
                        .fade_in(0.0),
                );
                // ≡ L1397-1398 travelling ring
                let cb = col_lerp(col, WHITE, 0.3);
                self.ring_raw(pos + d * 0.25, d, cb, 0.55, 0.16, R_THIN, 0.9, 1.2, d * 3.0);
                // ≡ L1399-1404
                let n = (9.0 * self.q).round().max(3.0) as usize;
                for _ in 0..n {
                    let v = self.cone_dir(d, 0.4);
                    let sp = 6.0 + self.rand() * 6.0;
                    let sz = 0.03 + self.rand() * 0.035;
                    let life = 0.4 + self.rand() * 0.2;
                    self.spawn_drop(pos, v * sp, col, sz, life, 0.8, 1.3, 0);
                }
                // ≡ L1405-1406 smoke puffs
                let cb2 = col_lerp(col, WHITE, 0.45);
                for i in 0..2usize {
                    self.sprite(
                        true,
                        SpriteInit::new(
                            pos + d * 0.3,
                            Vec3::new(
                                d.x * (2.0 + i as f32 * 1.5),
                                d.y * 2.0 + 0.4,
                                d.z * (2.0 + i as f32 * 1.5),
                            ),
                            cb2,
                            0.2,
                            0.7,
                            0.4,
                            0.32,
                        )
                        .drag(5.0),
                    );
                }
            }
            MuzzleKind::Charger => {
                // ≡ L1410-1411 star flash (drag 0, buoy 0, fadeIn 0, spin 0)
                let cb = col_lerp(col, WHITE, 0.4) * 3.5;
                self.sprite(
                    false,
                    SpriteInit::new(pos, Vec3::ZERO, cb, 0.7, 0.4, 0.14, 1.0)
                        .drag(0.0)
                        .buoy(0.0)
                        .kind(G_STAR + 3.0)
                        .fade_in(0.0)
                        .spin(0.0),
                );
                // ≡ L1412-1413
                let cb2 = col_lerp(col, WHITE, 0.3);
                self.ring_raw(
                    pos + d * 0.15,
                    d,
                    cb2,
                    0.38,
                    0.14,
                    R_THIN,
                    0.9,
                    1.0,
                    d * 2.0,
                );
                // ≡ L1414-1419
                let n = (5.0 * self.q).round().max(2.0) as usize;
                for _ in 0..n {
                    let v = self.cone_dir(d, 0.14);
                    let sp = 10.0 + self.rand() * 8.0;
                    let sz = 0.018 + self.rand() * 0.02;
                    self.spawn_drop(pos, v * sp, col, sz, 0.25, 0.4, 1.8, F_QUIET);
                }
            }
            MuzzleKind::Shooter => {
                // ≡ L1423-1424
                self.sprite(
                    false,
                    SpriteInit::new(pos + d * 0.05, d * 2.0, col * 4.0, 0.2, 0.38, 0.07, 1.0)
                        .kind(G_SOFT + 4.0)
                        .fade_in(0.0),
                );
                // ≡ L1425-1426
                self.sprite(
                    false,
                    SpriteInit::new(pos, Vec3::ZERO, col * 1.2, 0.4, 0.6, 0.1, 0.8)
                        .kind(G_SOFT)
                        .fade_in(0.0),
                );
                // ≡ L1427-1432
                let n = (5.0 * self.q).round().max(2.0) as usize;
                for _ in 0..n {
                    let v = self.cone_dir(d, 0.28);
                    let sp = 6.0 + self.rand() * 7.0;
                    let sz = 0.022 + self.rand() * 0.028;
                    let life = 0.28 + self.rand() * 0.12;
                    self.spawn_drop(pos, v * sp, col, sz, life, 0.6, 1.4, 0);
                }
                // ≡ L1433-1434
                let cb = col_lerp(col, WHITE, 0.4);
                self.sprite(
                    true,
                    SpriteInit::new(
                        pos + d * 0.15,
                        Vec3::new(d.x * 2.5, d.y * 2.5 + 0.3, d.z * 2.5),
                        cb,
                        0.12,
                        0.38,
                        0.28,
                        0.3,
                    )
                    .drag(6.0),
                );
            }
        }
    }

    /// `spawnFlash` ≡ L1437-1464.
    pub fn spawn_flash(&mut self, pos: Vec3, color: ColorIn) {
        let col = self.color(color);
        let q = self.q;
        // ≡ L1440-1441 light column
        self.beam(pos, col * 1.25, 1.15, 10.0, 1.0);
        // ≡ L1442-1443
        self.sprite(
            false,
            SpriteInit::new(
                Vec3::new(pos.x, pos.y + 0.8, pos.z),
                Vec3::ZERO,
                col * 4.0,
                2.2,
                3.8,
                0.3,
                1.0,
            )
            .kind(G_SOFT + 5.0)
            .fade_in(0.0),
        );
        // ≡ L1444-1445 wave ring now + a thin one in 0.12 s
        self.ring_raw(pos, UP, col, 2.8, 0.6, R_WAVE, 0.95, 1.3, Vec3::ZERO);
        self.after(0.12, OP_RING, pos, UP, col, 3.6, 0.6, R_THIN);
        // ≡ L1446
        self.ripple(pos, 0.012, 0.24, 2.6, 1.1, UP, Some(col));
        // ≡ L1447-1453 droplets
        let n = (40.0 * q).round() as usize;
        for _ in 0..n {
            let a = self.rand() * TAU;
            let r = self.rand() * 0.7;
            let sp = 7.0 + self.rand() * 9.0;
            let hx = a.cos() * (0.6 + self.rand() * 1.8);
            let hz = a.sin() * (0.6 + self.rand() * 1.8);
            let sz = 0.05 + self.rand() * 0.07;
            self.spawn_drop(
                Vec3::new(pos.x + a.cos() * r, pos.y + 0.1, pos.z + a.sin() * r),
                Vec3::new(hx, sp, hz),
                col,
                sz,
                1.6,
                1.0,
                1.2,
                0,
            );
        }
        // ≡ L1454-1458 puffs
        let cb = col_lerp(col, WHITE, 0.3);
        for _ in 0..((8.0 * q).round() as usize) {
            let a = self.rand() * TAU;
            let life = 0.7 + self.rand() * 0.4;
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(pos.x + a.cos() * 0.8, pos.y + 0.3, pos.z + a.sin() * 0.8),
                    Vec3::new(a.cos() * 2.5, 0.8, a.sin() * 2.5),
                    cb,
                    0.5,
                    1.3,
                    life,
                    0.45,
                )
                .drag(3.0)
                .buoy(0.4),
            );
        }
        // ≡ L1459-1463 six star glints
        let cs = col_lerp(col, WHITE, 0.55) * 2.4;
        for k in 0..6usize {
            let a = (k as f32 / 6.0) * TAU + self.rand() * 0.5;
            let r = 0.9 + self.rand() * 0.8;
            let py = pos.y + 0.5 + self.rand() * 2.5;
            let vy = 1.2 + self.rand();
            let life = 0.5 + self.rand() * 0.3;
            self.sprite(
                false,
                SpriteInit::new(
                    Vec3::new(pos.x + a.cos() * r, py, pos.z + a.sin() * r),
                    Vec3::new(0.0, vy, 0.0),
                    cs,
                    0.35,
                    0.18,
                    life,
                    1.0,
                )
                .drag(1.0)
                .kind(G_STAR + 2.0)
                .fade_in(0.05)
                .spin(1.5),
            );
        }
    }

    /// `rain` ≡ L1466-1484.
    pub fn rain(
        &mut self,
        pos: Vec3,
        radius: Option<f32>,
        color: ColorIn,
        dt: Option<f32>,
        opts: RainOpts,
    ) {
        let col = self.color(color);
        let radius = radius.unwrap_or(3.4);
        let dt = dt.unwrap_or(self.dt);
        let area = radius * radius; // ≡ L1468
        // ≡ L1469 n = Math.floor(area * 8.5 * q * dt + rand())
        let mut n = (area * 8.5 * self.q * dt + self.rand()).floor() as i64;
        while n > 0 {
            n -= 1;
            let a = self.rand() * TAU;
            let r = self.rand().sqrt() * radius * 0.92; // ≡ L1471
            let sp = 15.0 + self.rand() * 5.0; // ≡ L1472
            let py = pos.y - 0.25 - self.rand() * 0.4; // ≡ L1473
            let sz = 0.03 + self.rand() * 0.018;
            self.spawn_drop(
                Vec3::new(pos.x + a.cos() * r, py, pos.z + a.sin() * r),
                Vec3::new(0.4, -sp, 0.15),
                col,
                sz,
                1.4,
                1.0,
                3.2,
                (if opts.paint { F_PAINT } else { 0 }) | F_RING,
            );
        }
        if opts.cloud != Some(false) {
            // ≡ L1476 c = Math.floor(dt * (14 + radius*3.5) + rand())
            let mut c = (dt * (14.0 + radius * 3.5) + self.rand()).floor() as i64;
            let cb = col_lerp(col, WHITE, 0.1); // ≡ L1477
            while c > 0 {
                c -= 1;
                let a = self.rand() * TAU;
                let r = self.rand().sqrt() * radius * 0.9; // ≡ L1479
                let up = (1.0 - r / radius) * 0.6; // ≡ L1480
                let py = pos.y + up + (self.rand() - 0.4) * 0.5;
                let vx = (self.rand() - 0.5) * 0.3;
                let vz = (self.rand() - 0.5) * 0.3;
                let s1 = radius * (0.4 + self.rand() * 0.14);
                let life = 1.6 + self.rand() * 0.6;
                self.sprite(
                    true,
                    SpriteInit::new(
                        Vec3::new(pos.x + a.cos() * r, py, pos.z + a.sin() * r),
                        Vec3::new(vx, 0.04, vz),
                        cb,
                        radius * 0.3,
                        s1,
                        life,
                        1.0,
                    )
                    .drag(1.0)
                    .kind(P_CLOUD)
                    .fade_in(0.3)
                    .spin(0.25),
                );
            }
        }
    }

    /// `footstep` ≡ L1489-1523. surface: 0 dry, 1 own ink, 2 enemy ink.
    pub fn footstep(
        &mut self,
        pos: Vec3,
        color: ColorIn,
        surface: i32,
        dir: Option<Vec3>,
        speed: Option<f32>,
    ) {
        let col = self.color(color);
        let speed = speed.unwrap_or(5.0);
        let (dx, dz) = match dir {
            Some(d) => (d.x, d.z),
            None => (0.0, 0.0),
        };
        if surface == 0 {
            // ≡ L1493 dry ground needs real speed to kick dust
            if speed < 3.2 {
                return;
            }
            // ≡ L1494-1495 dust puff (six rands in argument order)
            let vx = -dx * 0.6 + (self.rand() - 0.5) * 0.3;
            let vy = 0.3 + self.rand() * 0.2;
            let vz = -dz * 0.6 + (self.rand() - 0.5) * 0.3;
            let s0 = 0.1 + self.rand() * 0.05;
            let s1 = 0.34 + self.rand() * 0.14;
            let life = 0.5 + self.rand() * 0.2;
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(pos.x - dx * 0.1, pos.y + 0.08, pos.z - dz * 0.1),
                    Vec3::new(vx, vy, vz),
                    dust_col(),
                    s0,
                    s1,
                    life,
                    0.42,
                )
                .drag(3.2)
                .buoy(0.15)
                .kind(P_DUST)
                .fade_in(0.04),
            );
            // ≡ L1496-1497
            let g = Vec3::new(-dx * 0.7, 0.75, -dz * 0.7).normalize_or_zero();
            self.grains(pos, g, 2.0, 1.5);
            return;
        }
        let k = clamp(speed / 7.0, 0.3, 1.2); // ≡ L1500
        let g = Vec3::new(-dx * 0.55, 1.0, -dz * 0.55).normalize_or_zero(); // ≡ L1501
        let vp = Vec3::new(pos.x, pos.y + 0.01, pos.z); // ≡ L1502
        if surface == 1 {
            // ≡ L1504-1509 squelch droplets
            let n = ((2.0 + 2.5 * k) * self.q).round().max(1.0) as usize;
            for _ in 0..n {
                let v = self.cone_dir(g, 0.75);
                let sp = 1.4 + self.rand() * 1.5 * k;
                let sz = 0.018 + self.rand() * 0.022;
                self.spawn_drop(
                    Vec3::new(pos.x, pos.y + 0.03, pos.z),
                    v * sp,
                    col,
                    sz,
                    0.6,
                    1.0,
                    1.2,
                    0,
                );
            }
            // ≡ L1510-1511
            let cb = col_lerp(col, WHITE, 0.2);
            self.ripple(vp, 0.0035 + 0.002 * k, 0.1, 0.9, 0.55, UP, Some(cb));
        } else {
            // ≡ L1513-1518 enemy ink spits back
            let n = (3.0 * self.q).round().max(1.0) as usize;
            for _ in 0..n {
                let v = self.cone_dir(g, 0.6);
                let sp = 0.9 + self.rand() * 0.9;
                let sz = 0.026 + self.rand() * 0.022;
                self.spawn_drop(
                    Vec3::new(pos.x, pos.y + 0.03, pos.z),
                    v * sp,
                    col,
                    sz,
                    0.6,
                    1.3,
                    0.8,
                    0,
                );
            }
            // ≡ L1519-1522
            self.ripple(vp, 0.003, 0.08, 0.55, 0.5, UP, Some(col));
            self.enemy_ink_sizzle(pos, ColorIn::Rgb(col));
        }
    }

    /// `land` ≡ L1525-1546.
    pub fn land(&mut self, pos: Vec3, color: ColorIn, surface: i32, speed: Option<f32>) {
        let col = self.color(color);
        let speed = speed.unwrap_or(8.0);
        let k = clamp((speed - 3.0) / 12.0, 0.0, 1.0); // ≡ L1528
        let vp = Vec3::new(pos.x, pos.y + 0.01, pos.z); // ≡ L1529
        if surface == 0 {
            // ≡ L1530-1536 dry land: dust ring + grains, no decals
            let dust = dust_col();
            self.ring_raw(
                vp,
                UP,
                dust,
                0.8 + 1.1 * k,
                0.5,
                R_DUST,
                0.62,
                1.0,
                Vec3::ZERO,
            );
            self.dust_ring(pos, dust, 4.0 + 6.0 * k, 1.4 + 1.6 * k);
            self.grains(pos, UP, 4.0 + 6.0 * k, 1.6 + k * 1.5);
            if k > 0.5 {
                self.ring_raw(vp, UP, dust, 1.3 + k, 0.3, R_THIN, 0.55, 1.0, Vec3::ZERO);
            }
            return;
        }
        // ≡ L1539-1545 in ink: crown, sheet, ripple
        self.crown(
            pos,
            UP,
            col,
            8.0 + 16.0 * k,
            2.4 + 2.2 * k,
            0.04 + 0.018 * k,
            0,
        );
        if k > 0.25 {
            self.shell(
                Vec3::new(pos.x, pos.y + 0.05, pos.z),
                col,
                0.15,
                0.4 + 0.35 * k,
                0.07,
                0.22,
                0.95,
                0.0,
                0.0,
                0.2,
                Some(UP),
                1.0,
            );
        }
        let cb = col_lerp(col, WHITE, 0.2);
        self.ripple(vp, 0.007 + 0.006 * k, 0.16, 1.6 + k, 0.8, UP, Some(cb));
    }

    /// `jumpOff` ≡ L1548-1568.
    pub fn jump_off(&mut self, pos: Vec3, color: ColorIn, surface: i32, swim: bool) {
        let col = self.color(color);
        let vp = Vec3::new(pos.x, pos.y + 0.01, pos.z); // ≡ L1551
        if swim || surface == 1 {
            // ≡ L1552-1558 ink push-off
            let n = (8.0 * self.q).round().max(2.0) as usize;
            for _ in 0..n {
                let v = self.cone_dir(UP, 0.5);
                let sp = 3.0 + self.rand() * 2.5;
                let px = pos.x + (self.rand() - 0.5) * 0.3;
                let pz = pos.z + (self.rand() - 0.5) * 0.3;
                let sz = 0.02 + self.rand() * 0.025;
                self.spawn_drop(
                    Vec3::new(px, pos.y + 0.05, pz),
                    v * sp,
                    col,
                    sz,
                    0.9,
                    1.0,
                    1.3,
                    0,
                );
            }
            // ≡ L1559-1560
            let cb = col_lerp(col, WHITE, 0.2);
            self.ripple(vp, 0.006, 0.13, 1.3, 0.7, UP, Some(cb));
        } else if surface == 2 {
            // ≡ L1561 reuse the enemy-ink footstep
            self.footstep(pos, ColorIn::Rgb(col), 2, None, Some(5.0));
        } else {
            // ≡ L1562-1567 dry jump: two dust puffs + grains + a small ring
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(pos.x, pos.y + 0.08, pos.z),
                    Vec3::new(0.3, 0.2, 0.0),
                    dust_col(),
                    0.1,
                    0.36,
                    0.45,
                    0.24,
                )
                .drag(3.5)
                .buoy(0.2)
                .kind(P_DUST)
                .fade_in(0.03)
                .spin(1.0),
            );
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(pos.x, pos.y + 0.08, pos.z),
                    Vec3::new(-0.3, 0.2, 0.0),
                    dust_col(),
                    0.1,
                    0.34,
                    0.45,
                    0.22,
                )
                .drag(3.5)
                .buoy(0.2)
                .kind(P_DUST)
                .fade_in(0.03)
                .spin(1.0),
            );
            self.grains(pos, UP, 3.0, 1.4);
            self.ring_raw(
                vp,
                UP,
                dust_col(),
                0.45,
                0.35,
                R_DUST,
                0.35,
                1.0,
                Vec3::ZERO,
            );
        }
    }

    /// `formPop` ≡ L1571-1585.
    pub fn form_pop(&mut self, pos: Vec3, color: ColorIn, to_squid: Option<bool>, in_ink: bool) {
        let to_squid = to_squid.unwrap_or(true);
        let col = self.color(color);
        // ≡ L1573 pop height: squid form pops low, humanoid pops high
        let y = pos.y + if to_squid { 0.35 } else { 0.55 };
        let cb = col_lerp(col, WHITE, 0.35);
        // ≡ L1574-1575
        self.sprite(
            true,
            SpriteInit::new(
                Vec3::new(pos.x, y, pos.z),
                Vec3::new(0.0, if to_squid { -0.2 } else { 0.6 }, 0.0),
                cb,
                0.22,
                0.62,
                0.26,
                if in_ink { 0.25 } else { 0.38 },
            )
            .drag(5.0),
        );
        // ≡ L1576-1578 camera-facing shockwave
        let vp = Vec3::new(pos.x, y, pos.z);
        let tc = self.to_cam(vp);
        let cb2 = col_lerp(col, WHITE, 0.4);
        self.ring_raw(vp, tc, cb2, 0.5, 0.16, R_THIN, 0.7, 1.0, Vec3::ZERO);
        // ≡ L1579-1583 even droplet ring
        let n = (5.0 * self.q).round().max(2.0) as usize;
        for i in 0..n {
            let a = (i as f32 / n as f32) * TAU + self.rand() * 0.6;
            let sp = 1.4 + self.rand() * 1.2;
            let vy = 1.2 + self.rand() * 1.6;
            let sz = 0.022 + self.rand() * 0.02;
            self.spawn_drop(
                Vec3::new(pos.x, y, pos.z),
                Vec3::new(a.cos() * sp, vy, a.sin() * sp),
                col,
                sz,
                0.6,
                1.0,
                1.0,
                0,
            );
        }
    }

    /// `dive` ≡ L1588-1596.
    pub fn dive(&mut self, pos: Vec3, color: ColorIn, speed: Option<f32>) {
        let col = self.color(color);
        let speed = speed.unwrap_or(4.0);
        let vp = Vec3::new(pos.x, pos.y + 0.01, pos.z); // ≡ L1590
        // ≡ L1591 crown
        self.crown(
            vp,
            UP,
            col,
            10.0 + speed * 0.6,
            2.2 + 2.5f32.min(speed * 0.18),
            0.036,
            0,
        );
        // ≡ L1592 sheet
        if speed > 3.0 {
            self.shell(
                Vec3::new(pos.x, pos.y + 0.04, pos.z),
                col,
                0.12,
                0.34 + 0.25f32.min(speed * 0.02),
                0.06,
                0.2,
                0.95,
                0.0,
                0.0,
                0.2,
                Some(UP),
                1.0,
            );
        }
        // ≡ L1593-1594
        let cb = col_lerp(col, WHITE, 0.2);
        self.ripple(
            vp,
            0.008 + 0.006f32.min(speed * 0.0006),
            0.15,
            1.5,
            0.85,
            UP,
            Some(cb),
        );
        // ≡ L1595
        self.bubbles(pos, ColorIn::Rgb(col), Some(3.0));
    }

    /// `emerge` ≡ L1597-1608. (`speed` is part of the JS signature but the
    /// body never reads it — kept for API parity.)
    pub fn emerge(&mut self, pos: Vec3, color: ColorIn, speed: Option<f32>) {
        let _ = speed;
        let col = self.color(color);
        let vp = Vec3::new(pos.x, pos.y + 0.01, pos.z);
        // ≡ L1600-1605
        let n = (8.0 * self.q).round().max(2.0) as usize;
        for _ in 0..n {
            let v = self.cone_dir(UP, 0.55);
            let sp = 2.5 + self.rand() * 2.0;
            let px = pos.x + (self.rand() - 0.5) * 0.25;
            let pz = pos.z + (self.rand() - 0.5) * 0.25;
            let sz = 0.022 + self.rand() * 0.026;
            self.spawn_drop(
                Vec3::new(px, pos.y + 0.08, pz),
                v * sp,
                col,
                sz,
                0.9,
                1.0,
                1.3,
                0,
            );
        }
        // ≡ L1606-1607
        let cb = col_lerp(col, WHITE, 0.2);
        self.ripple(vp, 0.007, 0.14, 1.3, 0.75, UP, Some(cb));
    }

    /// `bubbles` ≡ L1609-1617.
    pub fn bubbles(&mut self, pos: Vec3, color: ColorIn, n: Option<f32>) {
        let col = self.color(color);
        let cb = col_lerp(col, WHITE, 0.55) * 1.1; // ≡ L1610
        let c = (n.unwrap_or(3.0) * self.q).round().max(1.0) as usize; // ≡ L1611
        for _ in 0..c {
            // ≡ L1612-1615 (eight rands in argument order)
            let px = pos.x + (self.rand() - 0.5) * 0.5;
            let py = pos.y + 0.05 + self.rand() * 0.05;
            let pz = pos.z + (self.rand() - 0.5) * 0.5;
            let vy = 0.3 + self.rand() * 0.3;
            let s0 = 0.04 + self.rand() * 0.04;
            let s1 = 0.08 + self.rand() * 0.04;
            let life = 0.3 + self.rand() * 0.25;
            let fi = 0.04 + self.rand() * 0.1;
            self.sprite(
                false,
                SpriteInit::new(
                    Vec3::new(px, py, pz),
                    Vec3::new(0.0, vy, 0.0),
                    cb,
                    s0,
                    s1,
                    life,
                    0.8,
                )
                .drag(1.0)
                .kind(G_BUBBLE + 1.0)
                .fade_in(fi)
                .spin(0.0),
            );
        }
    }

    /// `climbDrip` ≡ L1620-1627.
    pub fn climb_drip(&mut self, pos: Vec3, normal: Vec3, color: ColorIn) {
        let col = self.color(color);
        // ≡ L1622-1623: args evaluate left-to-right — six rands for the
        // position/velocity components, then the size rand, then the seed
        // rand inside `_spawnDrop`.
        let rx1 = self.rand();
        let rx2 = self.rand();
        let rx3 = self.rand();
        let rx4 = self.rand();
        let rx5 = self.rand();
        let rx6 = self.rand();
        let size = 0.024 + self.rand() * 0.022;
        let p = Vec3::new(
            pos.x + normal.x * 0.06 + (rx1 - 0.5) * 0.2,
            pos.y + (rx2 - 0.5) * 0.2,
            pos.z + normal.z * 0.06 + (rx3 - 0.5) * 0.2,
        );
        let v = Vec3::new(
            normal.x * 0.25 + (rx4 - 0.5) * 0.2,
            -0.4 - rx5 * 0.6,
            normal.z * 0.25 + (rx6 - 0.5) * 0.2,
        );
        self.spawn_drop(p, v, col, size, 1.4, 0.5, 1.3, 0);
        let v2 = pos + normal * 0.03; // ≡ L1624
        let col_b = col_lerp(col, WHITE, 0.2); // ≡ L1625
        if self.rand() < 0.5 {
            // ≡ L1626
            self.ripple(v2, 0.004, 0.12, 0.9, 0.55, normal, Some(col_b));
        }
    }

    /// `climbPop` ≡ L1629-1640.
    pub fn climb_pop(&mut self, pos: Vec3, dir: Vec3, color: ColorIn) {
        let col = self.color(color);
        // ≡ L1631
        let axis = th_normalize(Vec3::new(dir.x * 0.5, 1.0, dir.z * 0.5));
        let n = (10.0 * self.q).round().max(3.0) as usize; // ≡ L1632
        for _ in 0..n {
            let v = self.cone_dir(axis, 0.55); // ≡ L1634
            let sp = 2.5 + self.rand() * 2.5; // ≡ L1635
            let size = 0.03 + self.rand() * 0.03;
            self.spawn_drop(
                Vec3::new(pos.x, pos.y + 0.1, pos.z),
                v * sp,
                col,
                size,
                1.0,
                1.0,
                1.0,
                0,
            );
        }
        let col_b = col_lerp(col, WHITE, 0.35); // ≡ L1638
        self.sprite(
            true,
            SpriteInit::new(
                Vec3::new(pos.x, pos.y + 0.3, pos.z),
                Vec3::new(dir.x, 1.2, dir.z),
                col_b,
                0.15,
                0.5,
                0.35,
                0.3,
            )
            .drag(4.0), // ≡ L1639
        );
    }

    /// `hurtDrip` ≡ L1642-1645.
    pub fn hurt_drip(&mut self, pos: Vec3, color: ColorIn, size: Option<f32>) {
        let size = size.unwrap_or(0.035);
        let col = self.color(color);
        // ≡ L1644: vx, vy, vz rands then the size rand (seed last in
        // `_spawnDrop`).
        let r1 = self.rand();
        let r2 = self.rand();
        let r3 = self.rand();
        let sz = size * (0.7 + self.rand() * 0.6);
        self.spawn_drop(
            pos,
            Vec3::new((r1 - 0.5) * 0.5, -0.2 - r2 * 0.4, (r3 - 0.5) * 0.5),
            col,
            sz,
            1.6,
            1.0,
            1.3,
            0,
        );
    }

    /// `hitSplash` ≡ L1649-1670.
    pub fn hit_splash(
        &mut self,
        pos: Vec3,
        dir: Vec3,
        color: ColorIn,
        amount: Option<f32>,
        killed: bool,
    ) {
        let col = self.color(color);
        let amount = amount.unwrap_or(30.0);
        let k = clamp(amount / 60.0, 0.3, 1.5); // ≡ L1651
        // ≡ L1652
        let mut v3 = dir;
        v3.y *= 0.5;
        if v3.length_squared() < 1e-6 {
            v3 = Vec3::new(0.0, 0.0, 1.0);
        }
        let v3 = th_normalize(v3);
        let v4 = -v3; // ≡ L1653
        let v2 = pos + v4 * 0.28; // ≡ L1654
        // ≡ L1655 (JS args stop at `wobble = 0.18`; axis/crown keep defaults)
        self.shell(
            v2,
            col,
            0.08,
            0.26 + 0.12 * k,
            0.06,
            0.2,
            0.95,
            0.0,
            0.0,
            0.18,
            None,
            0.0,
        );
        let n = ((6.0 + 8.0 * k) * self.q).round().max(3.0) as usize; // ≡ L1656
        for i in 0..n {
            let back = i % 3 == 0; // ≡ L1658
            let mut v1 = self.cone_dir(if back { v4 } else { v3 }, if back { 1.0 } else { 0.65 });
            v1.y += 0.3; // ≡ L1660
            let sp = if back {
                1.8 + self.rand() * 1.8
            } else {
                3.0 + self.rand() * 3.2
            }; // ≡ L1661
            let size = 0.02 + self.rand() * 0.03 * k;
            self.spawn_drop(v2, v1 * sp, col, size, 1.1, 1.0, 1.4, 0);
        }
        // ≡ L1664-1665 flash glow (drag 0, buoy 0, kind G_SOFT+2, fadeIn 0)
        let col_b = col * 2.6;
        self.sprite(
            false,
            SpriteInit::new(v2, Vec3::ZERO, col_b, 0.25, 0.45 + 0.15 * k, 0.07, 1.0)
                .drag(0.0)
                .buoy(0.0)
                .kind(G_SOFT + 2.0)
                .fade_in(0.0),
        );
        if !killed {
            // ≡ L1667-1668 tear puff (drag 4)
            let col_b = col_lerp(col, WHITE, 0.35);
            self.sprite(
                true,
                SpriteInit::new(
                    v2,
                    Vec3::new(v3.x * 1.5, 0.3, v3.z * 1.5),
                    col_b,
                    0.18,
                    0.45 + 0.15 * k,
                    0.28,
                    0.22,
                )
                .drag(4.0),
            );
        }
    }

    /// `mist` ≡ L1672-1676.
    pub fn mist(
        &mut self,
        pos: Vec3,
        vel: Option<Vec3>,
        color: ColorIn,
        size: Option<f32>,
        alpha: Option<f32>,
    ) {
        let size = size.unwrap_or(0.2);
        let alpha = alpha.unwrap_or(0.25);
        let col = self.color(color);
        let col_b = col_lerp(col, WHITE, 0.4); // ≡ L1674
        let life = 0.3 + self.rand() * 0.12; // ≡ L1675
        self.sprite(
            true,
            SpriteInit::new(
                pos,
                vel.unwrap_or(Vec3::ZERO),
                col_b,
                size * 0.45,
                size * 1.4,
                life,
                alpha,
            )
            .drag(4.5)
            .buoy(0.2),
        );
    }

    /// `shotTrail` ≡ L1678-1684.
    pub fn shot_trail(&mut self, pos: Vec3, vel: Vec3, color: ColorIn, big: bool) {
        let col = self.color(color);
        let col_b = col_lerp(col, WHITE, 0.42); // ≡ L1680
        let s = if big { 0.3 } else { 0.14 }; // ≡ L1681
        self.sprite(
            true,
            SpriteInit::new(
                pos,
                Vec3::new(vel.x * 0.06, vel.y * 0.06 + 0.1, vel.z * 0.06),
                col_b,
                s * 0.5,
                s * 1.5,
                if big { 0.4 } else { 0.26 },
                if big { 0.3 } else { 0.2 },
            )
            .drag(5.0)
            .buoy(0.1), // ≡ L1682
        );
        if self.rand() < if big { 0.8 } else { 0.35 } {
            // ≡ L1683: vx jitter rand, vz jitter rand, size rand, seed rand.
            let r1 = self.rand();
            let r2 = self.rand();
            let size = if big {
                0.03 + self.rand() * 0.02
            } else {
                0.016 + self.rand() * 0.012
            };
            self.spawn_drop(
                pos,
                Vec3::new(
                    vel.x * 0.25 + r1 - 0.5,
                    vel.y * 0.25,
                    vel.z * 0.25 + r2 - 0.5,
                ),
                col,
                size,
                0.8,
                1.0,
                1.4,
                F_QUIET,
            );
        }
    }

    /// `dryFire` ≡ L1686-1688.
    pub fn dry_fire(&mut self, pos: Vec3, dir: Vec3) {
        self.sprite(
            true,
            SpriteInit::new(
                pos,
                Vec3::new(dir.x * 0.8, dir.y * 0.8 + 0.3, dir.z * 0.8),
                dust_col(),
                0.05,
                0.22,
                0.35,
                0.35,
            )
            .drag(5.0)
            .buoy(0.3)
            .kind(P_DUST)
            .fade_in(0.02)
            .spin(1.0), // ≡ L1687
        );
    }

    /// `chargeGlow` ≡ L1691-1704.
    pub fn charge_glow(&mut self, pos: Vec3, color: ColorIn, k: f32) {
        let col = self.color(color);
        let flick = 0.85 + 0.15 * (self.time * 40.0).sin(); // ≡ L1693
        let size = (0.07 + 0.2 * k) * flick; // ≡ L1695
        let col_b = col_lerp(col, WHITE, 0.2 + 0.4 * k) * (1.5 + 2.5 * k); // ≡ L1694
        self.sprite(
            false,
            SpriteInit::new(
                pos,
                Vec3::ZERO,
                col_b,
                size,
                size,
                (self.dt * 1.6).max(0.03),
                1.0,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_SOFT + 1.0 + 3.0 * k)
            .fade_in(0.0)
            .spin(0.0),
        );
        // ≡ L1697 gathering sparks
        let mut n = ((10.0 + 36.0 * k) * self.dt * self.q.max(0.5) + self.rand()).floor() as i64;
        let col_b = col_lerp(col, WHITE, 0.5) * 2.2; // ≡ L1698
        while n > 0 {
            n -= 1;
            let v = self.rand_sphere(); // ≡ L1700
            let r = 0.3 + self.rand() * 0.25; // ≡ L1701
            self.sprite(
                false,
                SpriteInit::new(pos + v * r, -v * (r / 0.17), col_b, 0.05, 0.02, 0.17, 1.0)
                    .drag(0.0)
                    .buoy(0.0)
                    .kind(G_SOFT + 1.0)
                    .fade_in(0.03)
                    .spin(0.0), // ≡ L1702
            );
        }
    }

    /// `chargeFull` ≡ L1705-1717.
    pub fn charge_full(&mut self, pos: Vec3, color: ColorIn) {
        let col = self.color(color);
        // ≡ L1707-1708 star flash (drag 0, buoy 0, kind G_STAR+3, fadeIn 0,
        // spin 0.6)
        let col_b = col_lerp(col, WHITE, 0.5) * 3.2;
        self.sprite(
            false,
            SpriteInit::new(pos, Vec3::ZERO, col_b, 1.0, 0.5, 0.32, 1.0)
                .drag(0.0)
                .buoy(0.0)
                .kind(G_STAR + 3.0)
                .fade_in(0.0)
                .spin(0.6),
        );
        let v4 = self.to_cam(pos); // ≡ L1709
        let col_b = col_lerp(col, WHITE, 0.35); // ≡ L1710
        self.ring_raw(pos, v4, col_b, 0.6, 0.2, R_THIN, 0.9, 1.0, Vec3::ZERO); // ≡ L1711
        let col_b = col_lerp(col, WHITE, 0.5) * 2.0; // ≡ L1712
        for _ in 0..8 {
            let v = self.rand_sphere(); // ≡ L1714
            self.sprite(
                false,
                SpriteInit::new(pos, v * 2.2, col_b, 0.12, 0.05, 0.3, 1.0)
                    .drag(3.0)
                    .buoy(0.0)
                    .kind(G_STAR + 1.0)
                    .fade_in(0.0)
                    .spin(2.0), // ≡ L1715
            );
        }
    }

    /// `laserDot` ≡ L1719-1726 (immediate).
    pub fn laser_dot(&mut self, pos: Vec3, normal: Vec3, color: ColorIn, k: Option<f32>) {
        let k = k.unwrap_or(0.5);
        let col = self.color(color);
        let pulse = 0.8 + 0.2 * (self.time * 22.0).sin(); // ≡ L1721
        let col_b = col_lerp(col, WHITE, 0.35) * (2.0 + 2.0 * k); // ≡ L1722
        let v2 = pos + normal * 0.03; // ≡ L1723
        let size = (0.08 + 0.1 * k) * pulse;
        self.sprite(
            false,
            SpriteInit::new(
                v2,
                Vec3::ZERO,
                col_b,
                size,
                size,
                (self.dt * 1.6).max(0.03),
                1.0,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_STAR + 1.0 + 2.0 * k)
            .fade_in(0.0)
            .spin(0.0), // ≡ L1724
        );
        self.mark(
            v2,
            Some(normal),
            ColorIn::Rgb(col),
            0.12 + 0.12 * k,
            MarkOpts {
                style: Some(R_BLOT),
                alpha: Some(0.5),
                t: Some(0.0),
                thick: Some(1.0),
                seed: Some(0.3),
            },
        ); // ≡ L1725
    }

    /// `beamImpact` ≡ L1727-1743.
    pub fn beam_impact(&mut self, pos: Vec3, normal: Vec3, color: ColorIn, charge: Option<f32>) {
        let charge = charge.unwrap_or(1.0);
        let col = self.color(color);
        let c = clamp(charge, 0.0, 1.0); // ≡ L1729
        // ≡ L1730-1731 soft burst glow (spin keeps the default 1.2)
        let col_b = col * 4.0;
        self.sprite(
            false,
            SpriteInit::new(
                pos,
                Vec3::ZERO,
                col_b,
                0.4 + 0.5 * c,
                0.9 + 0.6 * c,
                0.12,
                1.0,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_SOFT + 4.0)
            .fade_in(0.0),
        );
        // ≡ L1732-1733 star core (spin 1)
        let col_b = col_lerp(col, WHITE, 0.5) * 2.6;
        self.sprite(
            false,
            SpriteInit::new(pos, Vec3::ZERO, col_b, 0.9, 0.4, 0.22, 1.0)
                .drag(0.0)
                .buoy(0.0)
                .kind(G_STAR + 3.0)
                .fade_in(0.0)
                .spin(1.0),
        );
        let v2 = pos + normal * 0.02; // ≡ L1734
        self.ring_raw(
            v2,
            normal,
            col,
            0.55 + 0.6 * c,
            0.3,
            R_WAVE,
            0.95,
            1.0,
            Vec3::ZERO,
        ); // ≡ L1735
        self.ring_raw(v2, normal, col, 0.9 + c, 0.2, R_THIN, 0.8, 1.0, Vec3::ZERO); // ≡ L1736
        let n = ((8.0 + 10.0 * c) * self.q).round().max(3.0) as usize; // ≡ L1737
        for _ in 0..n {
            let v = self.cone_dir(normal, 0.95); // ≡ L1739
            let sp = 3.0 + self.rand() * 3.5; // ≡ L1740
            let size = 0.03 + self.rand() * 0.04;
            self.spawn_drop(v2, v * sp, col, size, 1.1, 1.0, 1.0, 0);
        }
    }

    /// `beamTrail` ≡ L1745-1759.
    pub fn beam_trail(&mut self, from: Vec3, to: Vec3, color: ColorIn, charge: Option<f32>) {
        let charge = charge.unwrap_or(1.0);
        let col = self.color(color);
        let mut v3 = to - from; // ≡ L1747
        let len = v3.length();
        if len < 0.5 {
            return; // ≡ L1749
        }
        v3 /= len; // ≡ L1750
        let step = 0.75;
        let n = (len / step).floor().min(40.0) as i64; // ≡ L1751
        let col_b = col_lerp(col, WHITE, 0.45); // ≡ L1752
        for i in 1..=n {
            let d = i as f32 * step - self.rand() * 0.3; // ≡ L1754
            let p = from + v3 * d; // ≡ L1755
            if i % 2 == 0 || self.q >= 1.0 {
                // ≡ L1756: vx rand, vz rand, life rand
                let r1 = self.rand();
                let r2 = self.rand();
                let life = 0.3 + self.rand() * 0.2;
                self.sprite(
                    true,
                    SpriteInit::new(
                        p,
                        Vec3::new((r1 - 0.5) * 0.3, 0.15, (r2 - 0.5) * 0.3),
                        col_b,
                        0.06,
                        0.2 + 0.1 * charge,
                        life,
                        0.28,
                    )
                    .drag(4.0)
                    .buoy(0.1),
                );
            }
            if self.rand() < 0.45 * self.q {
                // ≡ L1757: vx rand, vy rand, vz rand, size rand, seed rand
                let r1 = self.rand();
                let r2 = self.rand();
                let r3 = self.rand();
                let size = 0.016 + self.rand() * 0.018;
                self.spawn_drop(
                    p,
                    Vec3::new((r1 - 0.5) * 0.6, -0.5 - r2, (r3 - 0.5) * 0.6),
                    col,
                    size,
                    0.9,
                    1.0,
                    1.4,
                    F_QUIET,
                );
            }
        }
    }

    /// `rollerSpray` ≡ L1762-1777 (call per frame; k = speed / roll speed).
    pub fn roller_spray(
        &mut self,
        pos: Vec3,
        fwd: Vec3,
        width: f32,
        color: ColorIn,
        k: Option<f32>,
    ) {
        let k = k.unwrap_or(1.0);
        let col = self.color(color);
        let (rx, rz) = (fwd.z, -fwd.x); // ≡ L1764
        let mut n = ((30.0 * k) * self.q * self.dt + self.rand()).floor() as i64; // ≡ L1765
        while n > 0 {
            n -= 1;
            let o = (self.rand() - 0.5) * width; // ≡ L1767
            let sp = 1.6 + self.rand() * 2.2; // ≡ L1768
            let vy = 1.4 + self.rand() * 1.8;
            let size = 0.025 + self.rand() * 0.03;
            let flags = if self.rand() < 0.3 { F_PAINT } else { 0 };
            self.spawn_drop(
                Vec3::new(
                    pos.x + rx * o + fwd.x * 0.05,
                    pos.y + 0.1,
                    pos.z + rz * o + fwd.z * 0.05,
                ),
                Vec3::new(
                    fwd.x * sp * k + rx * o.signum() * 0.7,
                    vy,
                    fwd.z * sp * k + rz * o.signum() * 0.7,
                ),
                col,
                size,
                1.0,
                1.0,
                1.0,
                flags,
            );
        }
        if self.rand() < 8.0 * self.dt * k {
            // ≡ L1772-1776 corner puff
            let s = if self.rand() < 0.5 { -1.0 } else { 1.0 };
            let col_b = col_lerp(col, WHITE, 0.4);
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(
                        pos.x + rx * s * width * 0.5,
                        pos.y + 0.15,
                        pos.z + rz * s * width * 0.5,
                    ),
                    Vec3::new(rx * s * 0.8 + fwd.x, 0.5, rz * s * 0.8 + fwd.z),
                    col_b,
                    0.1,
                    0.35,
                    0.3,
                    0.28,
                )
                .drag(4.0),
            );
        }
    }

    /// `flickCurtain` ≡ L1779-1797.
    pub fn flick_curtain(&mut self, pos: Vec3, dir: Vec3, color: ColorIn, spread_deg: Option<f32>) {
        let spread_deg = spread_deg.unwrap_or(50.0);
        let col = self.color(color);
        let paint = if self.paint_effects { F_PAINT } else { 0 }; // ≡ L1781
        let yaw = dir.x.atan2(dir.z); // ≡ L1782
        let spread = spread_deg * std::f32::consts::PI / 180.0;
        let n = (30.0 * self.q).round().max(6.0) as usize; // ≡ L1783
        for i in 0..n {
            let t = (i as f32 / (n as f32 - 1.0)) * 2.0 - 1.0; // ≡ L1785
            let a = yaw + t * spread * 0.55 + (self.rand() - 0.5) * 0.08; // ≡ L1786
            let up = 0.35 + self.rand() * 0.35; // ≡ L1787
            let sp = 6.0 + self.rand() * 5.0 * (1.0 - 0.4 * t.abs());
            let cu = up.cos(); // ≡ L1788
            let sz = 0.03 + self.rand() * 0.04; // ≡ L1789
            let ry = self.rand(); // ≡ L1790 (py jitter)
            self.spawn_drop(
                Vec3::new(
                    pos.x + a.sin() * 0.4,
                    pos.y + (ry - 0.3) * 0.4,
                    pos.z + a.cos() * 0.4,
                ),
                Vec3::new(a.sin() * cu * sp, up.sin() * sp, a.cos() * cu * sp),
                col,
                sz,
                1.4,
                1.0,
                1.2,
                if sz > 0.055 { paint } else { 0 },
            );
        }
        let col_b = col_lerp(col, WHITE, 0.4); // ≡ L1792
        for i in 0..3 {
            let a = yaw + (i as f32 - 1.0) * spread * 0.35; // ≡ L1794
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(pos.x + a.sin() * 0.8, pos.y + 0.2, pos.z + a.cos() * 0.8),
                    Vec3::new(a.sin() * 3.0, 1.0, a.cos() * 3.0),
                    col_b,
                    0.2,
                    0.7,
                    0.4,
                    0.3,
                )
                .drag(4.0), // ≡ L1795
            );
        }
    }

    /// `dangerRing` ≡ L1800 (immediate).
    pub fn danger_ring(
        &mut self,
        pos: Vec3,
        normal: Vec3,
        color: ColorIn,
        radius: Option<f32>,
        k: Option<f32>,
    ) {
        self.mark(
            pos,
            Some(normal),
            color,
            radius.unwrap_or(3.1),
            MarkOpts {
                style: Some(R_DANGER),
                alpha: Some(0.95),
                t: Some(k.unwrap_or(0.0)),
                thick: Some(1.0),
                seed: Some(0.37),
            },
        );
    }

    /// `beepPulse` ≡ L1801-1806.
    pub fn beep_pulse(
        &mut self,
        bomb_pos: Vec3,
        ground_pos: Option<Vec3>,
        normal: Option<Vec3>,
        color: ColorIn,
        radius: Option<f32>,
        k: Option<f32>,
    ) {
        let radius = radius.unwrap_or(3.1);
        let k = k.unwrap_or(0.0);
        let col = self.color(color);
        // ≡ L1803-1804 bomb glow (drag 0, buoy 0, kind G_SOFT+2+3k, fadeIn 0)
        let col_b = col_lerp(col, WHITE, 0.3) * (3.0 + 3.0 * k);
        self.sprite(
            false,
            SpriteInit::new(bomb_pos, Vec3::ZERO, col_b, 0.35, 0.8 + 0.4 * k, 0.14, 1.0)
                .drag(0.0)
                .buoy(0.0)
                .kind(G_SOFT + 2.0 + 3.0 * k)
                .fade_in(0.0),
        );
        if let Some(gp) = ground_pos {
            // ≡ L1805 ground ring (normal || UP)
            let n = normal.unwrap_or(UP);
            self.ring_raw(
                gp,
                n,
                col,
                radius,
                0.32,
                R_THIN,
                0.55 + 0.35 * k,
                1.2,
                Vec3::ZERO,
            );
        }
    }

    /// `bombTrail` ≡ L1807-1811.
    pub fn bomb_trail(&mut self, pos: Vec3, vel: Vec3, color: ColorIn) {
        let col = self.color(color);
        if self.rand() < 0.5 * self.q + 0.2 {
            // ≡ L1809: vx rand, vz rand, size rand, seed rand
            let r1 = self.rand();
            let r2 = self.rand();
            let size = 0.02 + self.rand() * 0.02;
            self.spawn_drop(
                pos,
                Vec3::new(
                    vel.x * 0.1 + (r1 - 0.5) * 0.6,
                    vel.y * 0.1,
                    vel.z * 0.1 + (r2 - 0.5) * 0.6,
                ),
                col,
                size,
                0.8,
                1.0,
                1.2,
                0,
            );
        }
        if self.rand() < 0.35 {
            // ≡ L1810 wisp
            let col_b = col_lerp(col, WHITE, 0.45);
            self.sprite(
                true,
                SpriteInit::new(pos, Vec3::new(0.0, 0.2, 0.0), col_b, 0.06, 0.2, 0.25, 0.22)
                    .drag(4.0),
            );
        }
    }

    /// `bounceSplash` ≡ L1812-1817.
    pub fn bounce_splash(&mut self, pos: Vec3, normal: Vec3, color: ColorIn) {
        let col = self.color(color);
        self.crown(pos, normal, col, 6.0, 2.0, 0.03, 0); // ≡ L1814
        let v2 = pos + normal * 0.01; // ≡ L1815
        self.ripple(v2, 0.006, 0.12, 1.2, 0.6, normal, Some(col)); // ≡ L1816
    }
}

impl Fx {
    /// `spinUp` ≡ L1823-1843 (call every frame; k = spin 0..1).
    pub fn spin_up(
        &mut self,
        pos: Vec3,
        dir: Vec3,
        color: ColorIn,
        k: Option<f32>,
        streaming: bool,
    ) {
        let k = k.unwrap_or(0.5);
        let col = self.color(color);
        let kk = clamp(k, 0.0, 1.0); // ≡ L1825
        // ≡ L1826
        let mut v3 = dir;
        if v3.length_squared() < 1e-6 {
            v3 = Vec3::new(0.0, 0.0, 1.0);
        }
        let v3 = th_normalize(v3);
        let g = (0.045 + 0.1 * kk) * (1.0 + 0.06 * kk * kk * (self.time * 12.0).sin()); // ≡ L1827
        let col_b = col_lerp(col, WHITE, 0.2 + 0.4 * kk) * (1.1 + 3.4 * kk * kk); // ≡ L1828
        self.sprite(
            false,
            SpriteInit::new(
                pos + v3 * 0.03,
                Vec3::ZERO,
                col_b,
                g,
                g,
                (self.dt * 1.6).max(0.03),
                1.0,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_SOFT + 1.0 + 3.0 * kk)
            .fade_in(0.0)
            .spin(0.0), // ≡ L1829
        );
        // ≡ L1830
        let mut n = ((if streaming {
            30.0
        } else {
            3.0 + 38.0 * kk * kk
        }) * self.dt
            * self.q.max(0.5)
            + self.rand())
        .floor() as i64;
        if n <= 0 {
            return; // ≡ L1831
        }
        let (v1, v2) = basis(v3); // ≡ L1832
        while n > 0 {
            n -= 1;
            let a = self.rand() * TAU; // ≡ L1834
            let (c, sn) = (a.cos(), a.sin());
            let radial = v1 * c + v2 * sn; // ≡ L1835
            let tang = v1 * (-sn) + v2 * c; // ≡ L1836
            let back = 0.06 + self.rand() * 0.16; // ≡ L1837
            let sp = (1.2 + 3.6 * kk) * (0.7 + self.rand() * 0.6); // ≡ L1838
            let fw = if streaming {
                2.0 + self.rand() * 2.0
            } else {
                0.2 + self.rand() * 0.5
            };
            let size = 0.011 + self.rand() * 0.013 + 0.008 * kk;
            let flags = if self.rand() < 0.55 { F_QUIET } else { 0 };
            self.spawn_drop(
                pos - v3 * back + radial * 0.065,
                tang * sp + radial * (sp * 0.35) + v3 * fw + Vec3::new(0.0, 0.5, 0.0),
                col,
                size,
                0.75,
                1.0,
                1.5,
                flags,
            );
        }
    }

    /// `spinFull` ≡ L1846-1863.
    pub fn spin_full(&mut self, pos: Vec3, dir: Vec3, color: ColorIn) {
        let col = self.color(color);
        // ≡ L1848
        let mut v3 = dir;
        if v3.length_squared() < 1e-6 {
            v3 = Vec3::new(0.0, 0.0, 1.0);
        }
        let v3 = th_normalize(v3);
        // ≡ L1849-1850 hot flash (fadeIn 0, spin default)
        let col_b = col_lerp(col, WHITE, 0.55) * 3.2;
        self.sprite(
            false,
            SpriteInit::new(pos, Vec3::ZERO, col_b, 0.34, 0.12, 0.16, 1.0)
                .drag(0.0)
                .buoy(0.0)
                .kind(G_SOFT + 3.0)
                .fade_in(0.0),
        );
        let col_b = col_lerp(col, WHITE, 0.35); // ≡ L1851
        self.ring_raw(
            pos - v3 * 0.08,
            v3,
            col_b,
            0.46,
            0.2,
            R_THIN,
            0.85,
            0.8,
            Vec3::ZERO,
        ); // ≡ L1852
        let (v1, v2) = basis(v3); // ≡ L1853
        let n = (16.0 * self.q).round().max(6.0) as usize; // ≡ L1854
        for i in 0..n {
            let a = (i as f32 / n as f32) * TAU + self.rand() * 0.3; // ≡ L1856
            let (c, sn) = (a.cos(), a.sin());
            let radial = v1 * c + v2 * sn; // ≡ L1857
            let tang = v1 * (-sn) + v2 * c; // ≡ L1858
            let sp = 3.2 + self.rand() * 1.6; // ≡ L1859
            let size = 0.016 + self.rand() * 0.014;
            let rv = radial * 0.8 + tang * 0.6;
            self.spawn_drop(
                pos - v3 * 0.1 + radial * 0.07,
                rv * sp + Vec3::new(0.0, 0.8, 0.0),
                col,
                size,
                0.8,
                1.0,
                1.5,
                0,
            ); // ≡ L1860-1861
        }
    }

    /// `dodgeSplash` ≡ L1868-1880.
    pub fn dodge_splash(&mut self, pos: Vec3, dir: Vec3, color: ColorIn) {
        let col = self.color(color);
        let v2 = Vec3::new(pos.x, pos.y + 0.03, pos.z); // ≡ L1870
        let v3 = th_normalize(Vec3::new(-dir.x * 0.8, 0.9, -dir.z * 0.8)); // ≡ L1871
        let n = (14.0 * self.q).round().max(4.0) as usize; // ≡ L1872
        for _ in 0..n {
            let v = self.cone_dir(v3, 1.05); // ≡ L1874
            let sp = 2.4 + self.rand() * 2.8; // ≡ L1875
            let size = 0.02 + self.rand() * 0.026;
            let flags = if self.rand() < 0.25 { F_PAINT } else { 0 };
            self.spawn_drop(
                Vec3::new(v2.x, v2.y + 0.05, v2.z),
                v * sp,
                col,
                size,
                1.0,
                1.0,
                1.4,
                flags,
            );
        }
        // ≡ L1878 low crown sheet (axis UP, crown 1)
        self.shell(
            v2,
            col,
            0.12,
            0.5,
            0.06,
            0.2,
            0.95,
            0.0,
            0.0,
            0.2,
            Some(UP),
            1.0,
        );
        self.ripple(v2, 0.008, 0.15, 1.6, 0.8, UP, Some(col)); // ≡ L1879
    }

    /// `dodgeSkid` ≡ L1883-1899 (every frame; k = roll progress 0..1).
    pub fn dodge_skid(&mut self, pos: Vec3, dir: Vec3, color: ColorIn, k: Option<f32>) {
        let k = k.unwrap_or(0.0);
        let col = self.color(color);
        let f = 1.0 - clamp(k, 0.0, 1.0); // ≡ L1885
        let (rx, rz) = (dir.z, -dir.x); // ≡ L1886
        let mut n = ((10.0 + 46.0 * f) * self.dt * self.q.max(0.5) + self.rand()).floor() as i64; // ≡ L1887
        while n > 0 {
            n -= 1;
            let side = (self.rand() - 0.5) * 2.0; // ≡ L1889
            let sp = (3.0 + self.rand() * 3.0) * (0.5 + 0.5 * f);
            let vy = 0.5 + self.rand() * 1.1;
            let size = 0.011 + self.rand() * 0.014;
            self.spawn_drop(
                Vec3::new(
                    pos.x + dir.x * 0.3 + rx * side * 0.25,
                    pos.y + 0.05,
                    pos.z + dir.z * 0.3 + rz * side * 0.25,
                ),
                Vec3::new(
                    dir.x * sp + rx * side * 1.6,
                    vy,
                    dir.z * sp + rz * side * 1.6,
                ),
                col,
                size,
                0.55,
                1.0,
                1.7,
                F_QUIET,
            ); // ≡ L1890-1891
        }
        let mut n = (22.0 * f * self.dt * self.q.max(0.5) + self.rand()).floor() as i64; // ≡ L1893
        while n > 0 {
            n -= 1;
            let mut v = self.rand_sphere(); // ≡ L1895
            v.y = v.y.abs() * 0.8 + 0.3;
            let sp = 1.5 + self.rand() * 2.0; // ≡ L1896
            let size = 0.015 + self.rand() * 0.018;
            self.spawn_drop(
                Vec3::new(
                    pos.x + v.x * 0.25,
                    pos.y + 0.35 + v.y * 0.2,
                    pos.z + v.z * 0.25,
                ),
                Vec3::new(v.x * sp + dir.x * 2.2, v.y * sp, v.z * sp + dir.z * 2.2),
                col,
                size,
                0.9,
                1.0,
                1.3,
                0,
            ); // ≡ L1897
        }
    }

    /// `dodgePlant` ≡ L1901-1912.
    pub fn dodge_plant(&mut self, pos: Vec3, dir: Vec3, color: ColorIn) {
        let col = self.color(color);
        let v3 = th_normalize(Vec3::new(dir.x * 0.7, 1.0, dir.z * 0.7)); // ≡ L1903
        let n = (9.0 * self.q).round().max(3.0) as usize; // ≡ L1904
        for _ in 0..n {
            let v = self.cone_dir(v3, 0.75); // ≡ L1906
            let sp = 1.8 + self.rand() * 2.0; // ≡ L1907
            let size = 0.016 + self.rand() * 0.02;
            self.spawn_drop(
                Vec3::new(pos.x, pos.y + 0.04, pos.z),
                v * sp,
                col,
                size,
                0.8,
                1.0,
                1.3,
                0,
            );
        }
        let v2 = Vec3::new(pos.x, pos.y + 0.01, pos.z); // ≡ L1910
        self.ripple(v2, 0.0065, 0.13, 1.3, 0.7, UP, Some(col)); // ≡ L1911
    }

    /// `sloshTrail` ≡ L1916-1923 (call every ~0.5 m).
    pub fn slosh_trail(&mut self, pos: Vec3, vel: Vec3, color: ColorIn, head: bool) {
        let col = self.color(color);
        // ≡ L1918: the rand is consumed only when `head` is false
        let mut n = if head {
            2
        } else if self.rand() < 0.6 {
            1
        } else {
            0
        };
        while n > 0 {
            n -= 1;
            let r1 = self.rand(); // ≡ L1920 px
            let r2 = self.rand(); // ≡ L1920 pz
            let r3 = self.rand(); // ≡ L1921 vx
            let r4 = self.rand(); // ≡ L1921 vz
            let size = (if head { 0.026 } else { 0.018 }) + self.rand() * 0.016;
            self.spawn_drop(
                Vec3::new(
                    pos.x + (r1 - 0.5) * 0.08,
                    pos.y - 0.04,
                    pos.z + (r2 - 0.5) * 0.08,
                ),
                Vec3::new(
                    vel.x * 0.25 + (r3 - 0.5) * 0.8,
                    vel.y * 0.2 - 0.5,
                    vel.z * 0.25 + (r4 - 0.5) * 0.8,
                ),
                col,
                size,
                1.1,
                1.0,
                1.5,
                0,
            ); // ≡ L1921
        }
    }

    /// `sloshImpact` ≡ L1927-1947.
    pub fn slosh_impact(&mut self, pos: Vec3, normal: Option<Vec3>, dir: Vec3, color: ColorIn) {
        let col = self.color(color);
        let n0 = normal.unwrap_or(UP); // ≡ L1929
        let v2 = pos + n0 * 0.03; // ≡ L1930
        // ≡ L1931 crown sheet along the wave direction
        self.shell(
            v2,
            col,
            0.3,
            1.05,
            0.08,
            0.3,
            0.97,
            0.0,
            0.0,
            0.22,
            Some(n0),
            1.0,
        );
        let (hx, hz) = (dir.x, dir.z); // ≡ L1932
        let hl = hx.hypot(hz);
        let hl = if hl == 0.0 { 1.0 } else { hl };
        let (fx, fz) = (hx / hl, hz / hl); // ≡ L1933
        let (rx, rz) = (fz, -fx);
        let paint = if self.paint_effects { F_PAINT } else { 0 }; // ≡ L1934
        let mut n = ((18.0 * self.q).round().max(6.0)) as i64; // ≡ L1935
        while n > 0 {
            n -= 1;
            let side = (self.rand() - 0.5) * 2.0; // ≡ L1937
            let sp = 3.5 + self.rand() * 3.5;
            let vy = 1.2 + self.rand() * 2.2;
            let size = 0.02 + self.rand() * 0.03;
            let flags = if self.rand() < 0.3 { paint } else { 0 };
            self.spawn_drop(
                Vec3::new(v2.x + rx * side * 0.3, v2.y + 0.05, v2.z + rz * side * 0.3),
                Vec3::new(fx * sp + rx * side * 2.2, vy, fz * sp + rz * side * 2.2),
                col,
                size,
                1.2,
                1.0,
                1.5,
                flags,
            ); // ≡ L1938-1939
        }
        let mut n = (5.0 * self.q).round().max(2.0) as i64; // ≡ L1941
        while n > 0 {
            n -= 1;
            let a = self.rand() * TAU; // ≡ L1943
            let sp = 1.6 + self.rand() * 1.8;
            let vy = 3.0 + self.rand() * 2.2;
            let size = 0.06 + self.rand() * 0.03;
            self.spawn_drop(
                Vec3::new(v2.x, v2.y + 0.1, v2.z),
                Vec3::new(a.cos() * sp + fx * 1.2, vy, a.sin() * sp + fz * 1.2),
                col,
                size,
                1.8,
                1.0,
                1.0,
                paint,
            ); // ≡ L1944
        }
        self.ripple(v2, 0.014, 0.26, 2.3, 1.05, n0, Some(col)); // ≡ L1946
    }

    /// `slamLaunch` ≡ L1950-1964.
    pub fn slam_launch(&mut self, pos: Vec3, color: ColorIn) {
        let col = self.color(color);
        let v2 = Vec3::new(pos.x, pos.y + 0.02, pos.z); // ≡ L1952
        self.ring_raw(v2, UP, col, 1.5, 0.4, R_WAVE, 0.9, 1.1, Vec3::ZERO); // ≡ L1953
        let col_b = col_lerp(col, WHITE, 0.2); // ≡ L1954
        self.ripple(v2, 0.01, 0.2, 1.8, 0.9, UP, Some(col_b)); // ≡ L1955
        let n = (18.0 * self.q).round().max(6.0) as usize; // ≡ L1956
        for _ in 0..n {
            let v = self.cone_dir(UP, 0.45); // ≡ L1958
            let sp = 5.0 + self.rand() * 5.0; // ≡ L1959
            let r1 = self.rand(); // px jitter
            let r2 = self.rand(); // pz jitter
            let size = 0.035 + self.rand() * 0.045;
            self.spawn_drop(
                Vec3::new(
                    pos.x + (r1 - 0.5) * 0.4,
                    pos.y + 0.1,
                    pos.z + (r2 - 0.5) * 0.4,
                ),
                v * sp,
                col,
                size,
                1.4,
                1.0,
                1.3,
                0,
            );
        }
        let col_b = col_lerp(col, WHITE, 0.35); // ≡ L1962
        for _ in 0..3 {
            // ≡ L1963: vx rand, vy rand, vz rand
            let r1 = self.rand();
            let r2 = self.rand();
            let r3 = self.rand();
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(pos.x, pos.y + 0.3, pos.z),
                    Vec3::new((r1 - 0.5) * 2.0, 1.5 + r2, (r3 - 0.5) * 2.0),
                    col_b,
                    0.3,
                    0.9,
                    0.5,
                    0.32,
                )
                .drag(3.0),
            );
        }
    }

    /// `slamCharge` ≡ L1966-1977 (per frame; k 0..1).
    pub fn slam_charge(&mut self, pos: Vec3, color: ColorIn, k: Option<f32>) {
        let k = k.unwrap_or(0.5);
        let col = self.color(color);
        // ≡ L1968-1969 body halo (drag 0, buoy 0, fadeIn 0, spin 0)
        let col_b = col_lerp(col, WHITE, 0.25) * (1.6 + 2.0 * k);
        let size = 0.9 + 0.6 * k;
        self.sprite(
            false,
            SpriteInit::new(
                pos,
                Vec3::ZERO,
                col_b,
                size,
                size,
                (self.dt * 1.6).max(0.03),
                0.8,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_SOFT + 1.0 + 2.0 * k)
            .fade_in(0.0)
            .spin(0.0),
        );
        let mut n = (40.0 * self.dt * self.q.max(0.5) + self.rand()).floor() as i64; // ≡ L1970
        let col_b = col_lerp(col, WHITE, 0.5) * 2.4; // ≡ L1971
        while n > 0 {
            n -= 1;
            let v = self.rand_sphere(); // ≡ L1973
            let r = 1.2 + self.rand() * 0.6; // ≡ L1974
            self.sprite(
                false,
                SpriteInit::new(
                    Vec3::new(pos.x + v.x * r, pos.y + v.y * r * 0.7, pos.z + v.z * r),
                    Vec3::new(-v.x * r / 0.22, -v.y * r * 0.7 / 0.22, -v.z * r / 0.22),
                    col_b,
                    0.14,
                    0.05,
                    0.22,
                    1.0,
                )
                .drag(0.0)
                .buoy(0.0)
                .kind(G_STAR + 1.0)
                .fade_in(0.03)
                .spin(2.0), // ≡ L1975
            );
        }
    }

    /// `slamFall` ≡ L1979-1987 (per frame).
    pub fn slam_fall(&mut self, pos: Vec3, color: ColorIn) {
        let col = self.color(color);
        let col_b = col_lerp(col, WHITE, 0.45); // ≡ L1981
        let mut n = (3.0 * self.q).round().max(1.0) as i64; // ≡ L1982
        while n > 0 {
            n -= 1;
            let a = self.rand() * TAU; // ≡ L1984
            let r = 0.25 + self.rand() * 0.4;
            let py = self.rand(); // ≡ L1985 py
            let vy = -14.0 - self.rand() * 4.0;
            let size = 0.018 + self.rand() * 0.015;
            self.spawn_drop(
                Vec3::new(
                    pos.x + a.cos() * r,
                    pos.y + 0.6 + py * 1.2,
                    pos.z + a.sin() * r,
                ),
                Vec3::new(0.0, vy, 0.0),
                col_b,
                size,
                0.14,
                0.0,
                3.4,
                F_NOCOL | F_QUIET,
            );
        }
    }

    /// `slamWave` ≡ L1989-2007 (extra impact layers on top of `explosion`).
    pub fn slam_wave(&mut self, pos: Vec3, color: ColorIn, radius: Option<f32>) {
        let radius = radius.unwrap_or(5.2);
        let col = self.color(color);
        let r = radius; // ≡ L1991
        // ≡ L1992 probe the ground under the slam point (fallback: pos/UP)
        let (v5, v6) = self
            .probe_down(Vec3::new(pos.x, pos.y + 0.5, pos.z), 2.5)
            .unwrap_or((pos, UP));
        self.ring_raw(v5, v6, col, r * 1.45, 0.6, R_WAVE, 0.95, 1.4, Vec3::ZERO); // ≡ L1993
        self.after(0.1, OP_RING, v5, v6, col, r * 1.85, 0.5, R_THIN); // ≡ L1994
        self.ripple(v5, 0.016, 0.32, 3.2, 1.3, v6, Some(col)); // ≡ L1995
        let sc = col_lerp(dust_col(), col, 0.25); // ≡ L1996
        self.ring_raw(v5, v6, sc, r * 1.1, 0.7, R_DUST, 0.55, 1.0, Vec3::ZERO); // ≡ L1997
        self.dust_ring(v5, sc, 14.0, 6.0); // ≡ L1998
        let paint = if self.paint_effects { F_PAINT } else { 0 }; // ≡ L1999
        let n = (20.0 * self.q).round().max(6.0) as usize; // ≡ L2000
        for i in 0..n {
            let a = (i as f32 / n as f32) * TAU + self.rand() * 0.3; // ≡ L2002
            let sp = 5.0 + self.rand() * 4.0;
            let vy = 4.0 + self.rand() * 3.0;
            let size = 0.1 + self.rand() * 0.06;
            self.spawn_drop(
                Vec3::new(v5.x + a.cos() * 0.6, v5.y + 0.3, v5.z + a.sin() * 0.6),
                Vec3::new(a.cos() * sp, vy, a.sin() * sp),
                col,
                size,
                2.2,
                1.0,
                1.1,
                paint,
            ); // ≡ L2003
        }
        let col_b = col * 1.2; // ≡ L2005
        self.beam(v5, col_b, 1.3, 5.5, 0.6); // ≡ L2006
    }

    /// `stormStart` ≡ L2010-2021.
    pub fn storm_start(&mut self, pos: Vec3, color: ColorIn, radius: Option<f32>) {
        let radius = radius.unwrap_or(3.4);
        let col = self.color(color);
        let col_b = col_lerp(col, WHITE, 0.2); // ≡ L2012
        let n = (10.0 * self.q).round() as i64 + 2; // ≡ L2013
        for _ in 0..n {
            // ≡ L2014-2015: a, r, py, vy, life rands; drag 2.5, buoy 0,
            // kind P_MIST, fadeIn 0.05, spin 0.4
            let a = self.rand() * TAU;
            let r = self.rand() * radius * 0.6;
            let py = self.rand();
            let vy = self.rand();
            let life = 0.7 + self.rand() * 0.4;
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(
                        pos.x + a.cos() * r,
                        pos.y + (py - 0.5) * 0.6,
                        pos.z + a.sin() * r,
                    ),
                    Vec3::new(a.cos() * 3.4, (vy - 0.3) * 1.5, a.sin() * 3.4),
                    col_b,
                    radius * 0.25,
                    radius * 0.6,
                    life,
                    0.4,
                )
                .drag(2.5)
                .kind(P_MIST)
                .fade_in(0.05)
                .spin(0.4),
            );
        }
        // ≡ L2017-2018 core glow (fadeIn 0, spin default)
        let col_b = col * 3.0;
        self.sprite(
            false,
            SpriteInit::new(
                pos,
                Vec3::ZERO,
                col_b,
                radius * 0.5,
                radius * 1.3,
                0.25,
                1.0,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_SOFT + 3.0)
            .fade_in(0.0),
        );
        let v4 = self.to_cam(pos); // ≡ L2019
        self.ring_raw(
            pos,
            v4,
            col,
            radius * 1.4,
            0.3,
            R_THIN,
            0.8,
            1.3,
            Vec3::ZERO,
        ); // ≡ L2020
    }

    /// `rainSheet` ≡ L2023-2031 (call per frame; dt defaults to the last
    /// update's dt).
    pub fn rain_sheet(&mut self, pos: Vec3, radius: f32, color: ColorIn, dt: Option<f32>) {
        let dt = dt.unwrap_or(self.dt);
        let col = self.color(color);
        let col_b = col_lerp(col, WHITE, 0.35); // ≡ L2025
        let mut n = (radius * radius * 3.2 * self.q * dt + self.rand()).floor() as i64; // ≡ L2026
        while n > 0 {
            n -= 1;
            let a = self.rand() * TAU; // ≡ L2028
            let r = self.rand().sqrt() * radius * 0.95;
            let py = self.rand(); // ≡ L2029 py
            let vy = -19.0 - self.rand() * 4.0;
            let size = 0.022 + self.rand() * 0.012;
            self.spawn_drop(
                Vec3::new(
                    pos.x + a.cos() * r,
                    pos.y - 0.4 - py * 0.5,
                    pos.z + a.sin() * r,
                ),
                Vec3::new(0.35, vy, 0.12),
                col_b,
                size,
                1.0,
                1.0,
                5.5,
                F_QUIET,
            );
        }
    }

    /// `stormPuddle` ≡ L2032-2038.
    pub fn storm_puddle(&mut self, pos: Vec3, normal: Option<Vec3>, color: ColorIn) {
        let col = self.color(color);
        let n = normal.unwrap_or(UP); // ≡ L2034 (`normal || UP` twice)
        let v2 = pos + n * 0.01;
        let col_b = col_lerp(col, WHITE, 0.15); // ≡ L2035
        let amp = 0.004 + self.rand() * 0.003; // ≡ L2036
        self.ripple(v2, amp, 0.09, 0.8, 0.5, n, Some(col_b));
        if self.rand() < 0.35 {
            // ≡ L2037
            self.crown(v2, n, col, 2.0, 1.4, 0.02, F_QUIET);
        }
    }

    /// `stormFlash` ≡ L2040-2044.
    pub fn storm_flash(&mut self, pos: Vec3, color: ColorIn, radius: Option<f32>) {
        let radius = radius.unwrap_or(3.4);
        let col = self.color(color);
        let col_b = col_lerp(col, WHITE, 0.5) * 2.5; // ≡ L2042
        // ≡ L2043: px, py, pz rands
        let r1 = self.rand();
        let r2 = self.rand();
        let r3 = self.rand();
        self.sprite(
            false,
            SpriteInit::new(
                Vec3::new(
                    pos.x + (r1 - 0.5) * radius,
                    pos.y + (r2 - 0.3) * 0.8,
                    pos.z + (r3 - 0.5) * radius,
                ),
                Vec3::ZERO,
                col_b,
                radius * 0.4,
                radius * 0.7,
                0.12,
                1.0,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_SOFT + 1.0)
            .fade_in(0.0),
        );
    }
}

impl Fx {
    /// `superJumpCharge` ≡ L2047-2065 (per frame; k 0..1).
    pub fn super_jump_charge(&mut self, pos: Vec3, color: ColorIn, k: Option<f32>) {
        let k = k.unwrap_or(0.5);
        let col = self.color(color);
        let q = self.q.max(0.5); // ≡ L2049
        let mut n = ((24.0 + 30.0 * k) * self.dt * q + self.rand()).floor() as i64; // ≡ L2051
        let col_b = col_lerp(col, WHITE, 0.5) * 2.2; // ≡ L2052
        while n > 0 {
            n -= 1;
            // ≡ L2054: spiral angle uses `_time * 9`
            let a = self.time * 9.0 + self.rand() * TAU;
            let r = 0.75 - 0.35 * k + self.rand() * 0.15;
            let (c, s) = (a.cos(), a.sin());
            let py = self.rand(); // ≡ L2056 py
            self.sprite(
                false,
                SpriteInit::new(
                    Vec3::new(pos.x + c * r, pos.y + 0.1 + py * 0.4, pos.z + s * r),
                    Vec3::new(-s * 2.4, 2.5 + 2.0 * k, c * 2.4),
                    col_b,
                    0.12,
                    0.05,
                    0.4,
                    1.0,
                )
                .drag(1.5)
                .buoy(0.0)
                .kind(G_STAR + 1.0)
                .fade_in(0.03)
                .spin(1.5),
            );
        }
        let mut n = ((14.0 + 24.0 * k) * self.dt * q + self.rand()).floor() as i64; // ≡ L2058
        while n > 0 {
            n -= 1;
            let a = self.rand() * TAU; // ≡ L2060
            let r = 0.5 + self.rand() * 0.3;
            let vy = 2.5 + self.rand() * 3.0 * (0.5 + k);
            let size = 0.025 + self.rand() * 0.025;
            self.spawn_drop(
                Vec3::new(pos.x + a.cos() * r, pos.y + 0.05, pos.z + a.sin() * r),
                Vec3::new(-a.sin() * 1.8, vy, a.cos() * 1.8),
                col,
                size,
                0.7,
                0.8,
                1.3,
                F_QUIET,
            ); // ≡ L2061
        }
        // ≡ L2063-2064 body glow (drag 0, buoy 0, kind G_SOFT+2k, fadeIn 0,
        // spin 0)
        let col_b = col_lerp(col, WHITE, 0.2) * (1.4 + 2.0 * k);
        let size = 0.7 + 0.5 * k;
        self.sprite(
            false,
            SpriteInit::new(
                Vec3::new(pos.x, pos.y + 0.45, pos.z),
                Vec3::ZERO,
                col_b,
                size,
                size,
                (self.dt * 1.6).max(0.03),
                0.7,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_SOFT + 2.0 * k)
            .fade_in(0.0)
            .spin(0.0),
        );
        if self.rand() < 5.0 * self.dt {
            // ≡ L2065
            self.ring_raw(
                Vec3::new(pos.x, pos.y + 0.02, pos.z),
                UP,
                col,
                1.1 + 0.4 * k,
                0.35,
                R_THIN,
                0.7,
                1.0,
                Vec3::ZERO,
            );
        }
    }

    /// `superJumpLaunch` ≡ L2067-2086.
    pub fn super_jump_launch(&mut self, pos: Vec3, color: ColorIn) {
        let col = self.color(color);
        let col_b = col * 1.3; // ≡ L2069-2070 geyser
        self.beam(pos, col_b, 0.9, 12.0, 0.8);
        let v2 = Vec3::new(pos.x, pos.y + 0.02, pos.z); // ≡ L2071
        self.ring_raw(v2, UP, col, 2.2, 0.45, R_WAVE, 0.95, 1.2, Vec3::ZERO); // ≡ L2072
        self.ring_raw(v2, UP, col, 2.9, 0.3, R_THIN, 0.8, 1.0, Vec3::ZERO); // ≡ L2073
        let n = (30.0 * self.q).round().max(8.0) as usize; // ≡ L2074
        for _ in 0..n {
            let v = self.cone_dir(UP, 0.35); // ≡ L2076
            let sp = 9.0 + self.rand() * 7.0; // ≡ L2077
            let r1 = self.rand(); // px
            let r2 = self.rand(); // pz
            let size = 0.035 + self.rand() * 0.05;
            self.spawn_drop(
                Vec3::new(
                    pos.x + (r1 - 0.5) * 0.5,
                    pos.y + 0.1,
                    pos.z + (r2 - 0.5) * 0.5,
                ),
                v * sp,
                col,
                size,
                1.6,
                1.0,
                1.5,
                0,
            );
        }
        let col_b = col_lerp(col, WHITE, 0.3); // ≡ L2080
        let m = (6.0 * self.q).round() as i64 + 1; // ≡ L2081
        for _ in 0..m {
            let a = self.rand() * TAU; // ≡ L2082
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(pos.x + a.cos() * 0.5, pos.y + 0.2, pos.z + a.sin() * 0.5),
                    Vec3::new(a.cos() * 3.0, 0.8, a.sin() * 3.0),
                    col_b,
                    0.3,
                    0.9,
                    0.6,
                    0.4,
                )
                .drag(3.0), // ≡ L2083
            );
        }
        // ≡ L2085-2086 flash (drag 0, buoy 0, kind G_SOFT+4, fadeIn 0)
        let col_b = col * 4.0;
        self.sprite(
            false,
            SpriteInit::new(
                Vec3::new(pos.x, pos.y + 0.6, pos.z),
                Vec3::ZERO,
                col_b,
                1.2,
                2.4,
                0.22,
                1.0,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_SOFT + 4.0)
            .fade_in(0.0),
        );
    }

    /// `superJumpTrail` ≡ L2088-2094.
    pub fn super_jump_trail(&mut self, pos: Vec3, vel: Vec3, color: ColorIn) {
        let col = self.color(color);
        let mut n = (2.0 * self.q).round().max(1.0) as i64; // ≡ L2090
        while n > 0 {
            n -= 1;
            // ≡ L2091: px, py, pz, vx, vz, size rands (vy has no jitter)
            let r1 = self.rand();
            let r2 = self.rand();
            let r3 = self.rand();
            let r4 = self.rand();
            let r5 = self.rand();
            let size = 0.03 + self.rand() * 0.035;
            self.spawn_drop(
                Vec3::new(
                    pos.x + (r1 - 0.5) * 0.3,
                    pos.y + (r2 - 0.5) * 0.3,
                    pos.z + (r3 - 0.5) * 0.3,
                ),
                Vec3::new(
                    vel.x * 0.15 + r4 - 0.5,
                    vel.y * 0.15,
                    vel.z * 0.15 + r5 - 0.5,
                ),
                col,
                size,
                1.1,
                1.0,
                1.4,
                0,
            );
        }
        // ≡ L2092-2093 trail glow (drag 0, buoy 0, kind G_SOFT+1.5, fadeIn 0)
        let col_b = col_lerp(col, WHITE, 0.2) * 2.2;
        self.sprite(
            false,
            SpriteInit::new(pos, Vec3::ZERO, col_b, 0.55, 0.3, 0.16, 0.9)
                .drag(0.0)
                .buoy(0.0)
                .kind(G_SOFT + 1.5)
                .fade_in(0.0),
        );
        if self.rand() < 0.5 {
            // ≡ L2094 puff (drag 3)
            let col_b = col_lerp(col, WHITE, 0.35);
            self.sprite(
                true,
                SpriteInit::new(pos, Vec3::new(0.0, 0.2, 0.0), col_b, 0.2, 0.55, 0.45, 0.3)
                    .drag(3.0),
            );
        }
    }

    /// `jumpMarker` ≡ L2097-2102 (immediate).
    pub fn jump_marker(&mut self, pos: Vec3, color: ColorIn, t: Option<f32>) {
        let t = t.unwrap_or(0.0);
        let col = self.color(color);
        let v2 = Vec3::new(pos.x, pos.y + 0.03, pos.z); // ≡ L2099
        self.mark(
            v2,
            Some(UP),
            ColorIn::Rgb(col),
            1.5,
            MarkOpts {
                style: Some(R_TARGET),
                alpha: Some(1.0),
                t: Some(t),
                thick: Some(1.0),
                seed: Some(0.5),
            },
        ); // ≡ L2100
        // ≡ L2101-2102 pulsing light pillar
        let col_b = col * 1.5;
        self.pillar(
            v2,
            ColorIn::Rgb(col_b),
            PillarOpts {
                radius: Some(0.5),
                height: Some(9.0),
                alpha: Some(0.55 + 0.15 * (self.time * 6.0).sin()),
                seed: Some(0.4),
            },
        );
    }

    /// `superJumpLand` ≡ L2104-2117.
    pub fn super_jump_land(&mut self, pos: Vec3, color: ColorIn) {
        let col = self.color(color);
        let v2 = Vec3::new(pos.x, pos.y + 0.02, pos.z); // ≡ L2106
        self.ring_raw(v2, UP, col, 2.4, 0.45, R_WAVE, 0.95, 1.2, Vec3::ZERO); // ≡ L2107
        self.ring_raw(v2, UP, col, 3.0, 0.3, R_THIN, 0.8, 1.0, Vec3::ZERO); // ≡ L2108
        self.crown(v2, UP, col, 26.0, 4.2, 0.045, 0); // ≡ L2109
        let v4 = Vec3::new(pos.x, pos.y + 0.1, pos.z); // ≡ L2110
        self.shell(
            v4,
            col,
            0.2,
            0.9,
            0.08,
            0.3,
            0.95,
            0.0,
            0.0,
            0.18,
            Some(UP),
            1.0,
        ); // ≡ L2111
        self.ripple(v2, 0.013, 0.24, 2.4, 1.1, UP, Some(col)); // ≡ L2112
        // ≡ L2113-2114 flash (drag 0, buoy 0, kind G_SOFT+3, fadeIn 0)
        let col_b = col * 3.0;
        self.sprite(
            false,
            SpriteInit::new(
                Vec3::new(pos.x, pos.y + 0.5, pos.z),
                Vec3::ZERO,
                col_b,
                0.8,
                1.8,
                0.18,
                1.0,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_SOFT + 3.0)
            .fade_in(0.0),
        );
        let sc = col_lerp(dust_col(), col, 0.2); // ≡ L2115
        self.dust_ring(v2, sc, 8.0, 3.0); // ≡ L2116
    }

    /// `ghost` ≡ L2120-2123.
    pub fn ghost(&mut self, pos: Vec3, color: ColorIn) {
        let col = self.color(color);
        let col_b = col_lerp(col, WHITE, 0.3); // ≡ L2122
        self.sprite(
            true,
            SpriteInit::new(
                Vec3::new(pos.x, pos.y + 0.35, pos.z),
                Vec3::new(0.0, 0.5, 0.0),
                col_b,
                0.55,
                0.85,
                1.5,
                0.8,
            )
            .drag(1.2)
            .buoy(0.9)
            .kind(P_GHOST)
            .fade_in(0.18)
            .spin(0.0)
            .wobble(0.35)
            .fade_out(0.6), // ≡ L2123
        );
    }

    /// `waterSplash` ≡ L2126-2138 (falling into the sea; fixed WATER/FOAM).
    pub fn water_splash(&mut self, pos: Vec3, size: Option<f32>) {
        let size = size.unwrap_or(1.0);
        let water = water_col();
        let foam = foam_col();
        let v2 = Vec3::new(pos.x, self.water_y + 0.02, pos.z); // ≡ L2127
        self.crown(v2, UP, water, 22.0 * size, 4.0 + 2.0 * size, 0.06, 0); // ≡ L2128
        self.ring_raw(
            v2,
            UP,
            foam,
            1.2 * size,
            0.6,
            R_RIPPLE,
            0.8,
            1.0,
            Vec3::ZERO,
        ); // ≡ L2129
        self.after(0.12, OP_RING, v2, UP, foam, 2.2 * size, 0.9, R_RIPPLE); // ≡ L2130
        self.ring_raw(v2, UP, foam, 0.6 * size, 0.4, R_DISC, 0.7, 1.0, Vec3::ZERO); // ≡ L2131
        let n = (5.0 * self.q).round() as i64 + 1; // ≡ L2132
        for _ in 0..n {
            // ≡ L2132: px, pz, vx, vy, vz, life rands; drag 2, buoy 0.1
            let r1 = self.rand();
            let r2 = self.rand();
            let r3 = self.rand();
            let r4 = self.rand();
            let r5 = self.rand();
            let life = 0.9 + self.rand() * 0.4;
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(pos.x + r1 - 0.5, self.water_y + 0.4, pos.z + r2 - 0.5),
                    Vec3::new((r3 - 0.5) * 1.5, 1.5 + r4, (r5 - 0.5) * 1.5),
                    foam,
                    0.3,
                    1.0,
                    life,
                    0.4,
                )
                .drag(2.0)
                .buoy(0.1),
            );
        }
        let n = (8.0 * self.q).round().max(3.0) as usize; // ≡ L2133
        for _ in 0..n {
            let v = self.cone_dir(UP, 0.25); // ≡ L2135
            let sp = 6.0 + self.rand() * 4.0 * size; // ≡ L2136
            let r1 = self.rand(); // px
            let r2 = self.rand(); // pz
            let sz = 0.05 + self.rand() * 0.05;
            self.spawn_drop(
                Vec3::new(
                    pos.x + (r1 - 0.5) * 0.4,
                    self.water_y + 0.1,
                    pos.z + (r2 - 0.5) * 0.4,
                ),
                v * sp,
                foam,
                sz,
                1.6,
                1.0,
                1.4,
                F_NOCOL,
            ); // ≡ L2137
        }
    }

    /// `waterPlop` ≡ L2141-2144 (something small hitting the sea).
    pub fn water_plop(&mut self, pos: Vec3, size: Option<f32>) {
        let size = size.unwrap_or(0.3);
        let water = water_col();
        let foam = foam_col();
        let v2 = Vec3::new(pos.x, self.water_y + 0.02, pos.z); // ≡ L2142
        self.ring_raw(
            v2,
            UP,
            foam,
            0.35 + size,
            0.5,
            R_RIPPLE,
            0.7,
            1.0,
            Vec3::ZERO,
        ); // ≡ L2143
        self.crown(
            v2,
            UP,
            water,
            4.0 + size * 8.0,
            1.8 + size * 2.0,
            0.03,
            F_NOCOL,
        ); // ≡ L2144
    }

    /// `specialSparkle` ≡ L2148-2156 (call at ~10-14 Hz per character).
    pub fn special_sparkle(&mut self, pos: Vec3, color: ColorIn, height: Option<f32>) {
        let height = height.unwrap_or(1.5);
        let col = self.color(color);
        // ≡ L2150: a, r, y rands
        let a = self.rand() * TAU;
        let r = 0.38 + self.rand() * 0.18;
        let y = 0.2 + self.rand() * height;
        let col_b = col_lerp(col, WHITE, 0.35) * 2.4; // ≡ L2151
        // ≡ L2152: vy rand, size0 rand, life rand; drag 0.5, buoy 0,
        // kind G_STAR+1.5, fadeIn 0.08, spin 3
        let vy = self.rand();
        let s0 = self.rand();
        let life = self.rand();
        self.sprite(
            false,
            SpriteInit::new(
                Vec3::new(pos.x + a.cos() * r, pos.y + y, pos.z + a.sin() * r),
                Vec3::new(-a.sin() * 0.6, 0.45 + vy * 0.3, a.cos() * 0.6),
                col_b,
                0.26 + s0 * 0.12,
                0.07,
                0.5 + life * 0.2,
                1.0,
            )
            .drag(0.5)
            .buoy(0.0)
            .kind(G_STAR + 1.5)
            .fade_in(0.08)
            .spin(3.0),
        );
        if self.rand() < 0.35 {
            // ≡ L2153-2155 companion glow (drag 0.5, buoy 0, kind G_SOFT+1,
            // fadeIn 0.05, spin 0)
            let col_b = col * 2.0;
            let a2 = a + 2.0;
            self.sprite(
                false,
                SpriteInit::new(
                    Vec3::new(
                        pos.x + a2.cos() * r * 0.8,
                        pos.y + y * 0.6,
                        pos.z + a2.sin() * r * 0.8,
                    ),
                    Vec3::new(0.0, 0.8, 0.0),
                    col_b,
                    0.07,
                    0.03,
                    0.6,
                    1.0,
                )
                .drag(0.5)
                .buoy(0.0)
                .kind(G_SOFT + 1.0)
                .fade_in(0.05)
                .spin(0.0),
            );
        }
    }

    /// `enemyInkSizzle` ≡ L2159-2167 (standing in enemy ink).
    pub fn enemy_ink_sizzle(&mut self, pos: Vec3, color: ColorIn) {
        let col = self.color(color);
        let col_b = col_lerp(col, WHITE, 0.35) * 1.3; // ≡ L2161
        let n = 1 + (if self.rand() < 0.5 { 1 } else { 0 }); // ≡ L2162
        for _ in 0..n {
            // ≡ L2164-2165: a, r, vy, size0, size1, life rands; drag 1,
            // buoy 0, kind G_BUBBLE+1, fadeIn 0.05, spin 0
            let a = self.rand() * TAU;
            let r = 0.12 + self.rand() * 0.25;
            let vy = self.rand();
            let s0 = self.rand();
            let s1 = self.rand();
            let life = self.rand();
            self.sprite(
                false,
                SpriteInit::new(
                    Vec3::new(pos.x + a.cos() * r, pos.y + 0.04, pos.z + a.sin() * r),
                    Vec3::new(0.0, 0.22 + vy * 0.2, 0.0),
                    col_b,
                    0.035 + s0 * 0.03,
                    0.07 + s1 * 0.03,
                    0.28 + life * 0.18,
                    0.85,
                )
                .drag(1.0)
                .buoy(0.0)
                .kind(G_BUBBLE + 1.0)
                .fade_in(0.05)
                .spin(0.0),
            );
        }
        if self.rand() < 0.3 {
            // ≡ L2167: px, pz, vx, vy, vz, size rands
            let r1 = self.rand();
            let r2 = self.rand();
            let r3 = self.rand();
            let r4 = self.rand();
            let r5 = self.rand();
            let size = 0.018 + self.rand() * 0.012;
            self.spawn_drop(
                Vec3::new(
                    pos.x + (r1 - 0.5) * 0.3,
                    pos.y + 0.04,
                    pos.z + (r2 - 0.5) * 0.3,
                ),
                Vec3::new((r3 - 0.5) * 0.4, 0.8 + r4 * 0.6, (r5 - 0.5) * 0.4),
                col,
                size,
                0.4,
                1.0,
                1.0,
                F_QUIET,
            );
        }
    }

    /// `seaSpray` ≡ L2173-2193 (a wave slapping the deck edge).
    pub fn sea_spray(&mut self, pos: Vec3, outward: Vec3, strength: Option<f32>) {
        let strength = strength.unwrap_or(1.0);
        let s = clamp(strength, 0.3, 1.6); // ≡ L2174
        let foam = foam_col();
        let (ax, az) = (-outward.z, outward.x); // ≡ L2175 along the edge
        let n = ((16.0 + 20.0 * s) * self.q).round().max(6.0) as usize; // ≡ L2176
        let along0 = (self.rand() - 0.5) * 0.6; // ≡ L2177
        for _ in 0..n {
            // ≡ L2179: along, up, out rands
            let along = along0 + (self.rand() - 0.5) * 2.6 * s;
            let up = (7.5 + self.rand() * 4.5) * (0.75 + 0.3 * s);
            let out = 0.3 + self.rand() * 1.6;
            let py = self.rand(); // ≡ L2180 py
            let jx = self.rand(); // ≡ L2181 vx jitter
            let jz = self.rand(); // vz jitter
            let size = 0.03 + self.rand() * 0.06;
            let flags = F_NOCOL | if self.rand() < 0.3 { 0 } else { F_QUIET };
            self.spawn_drop(
                Vec3::new(
                    pos.x + ax * along + outward.x * 0.12,
                    pos.y + py * 0.25,
                    pos.z + az * along + outward.z * 0.12,
                ),
                Vec3::new(
                    outward.x * out + ax * (jx - 0.5) * 1.4,
                    up,
                    outward.z * out + az * (jz - 0.5) * 1.4,
                ),
                foam,
                size,
                2.2,
                1.0,
                1.25,
                flags,
            );
        }
        // ≡ L2184-2188 foam curtain
        let m = ((5.0 + 4.0 * s) * self.q).round().max(3.0) as usize;
        for _ in 0..m {
            let along = along0 + (self.rand() - 0.5) * 2.8 * s; // ≡ L2186
            let h = self.rand();
            let vy = self.rand(); // ≡ L2188 vy
            let s1 = self.rand(); // size1
            let life = self.rand(); // life
            self.sprite(
                true,
                SpriteInit::new(
                    Vec3::new(
                        pos.x + ax * along + outward.x * (0.25 + h * 0.4),
                        pos.y + 0.8 + h * 1.6 * s,
                        pos.z + az * along + outward.z * (0.25 + h * 0.4),
                    ),
                    Vec3::new(
                        outward.x * 0.7 + 0.25,
                        (2.6 + vy * 2.2) * s,
                        outward.z * 0.7 + 0.1,
                    ),
                    foam,
                    0.4 * s,
                    (1.1 + s1 * 0.7) * s,
                    1.0 + life * 0.6,
                    0.42,
                )
                .drag(2.2)
                .buoy(-1.2)
                .kind(P_MIST)
                .fade_in(0.06)
                .spin(0.4)
                .wobble(0.3),
            );
        }
        let v2 = Vec3::new(
            pos.x + outward.x * 0.8,
            self.water_y + 0.03,
            pos.z + outward.z * 0.8,
        ); // ≡ L2190
        self.ring_raw(
            v2,
            UP,
            foam,
            1.5 * s + 0.5,
            0.9,
            R_RIPPLE,
            0.8,
            1.0,
            Vec3::ZERO,
        ); // ≡ L2191
        self.ring_raw(v2, UP, foam, 0.9 * s, 0.55, R_DISC, 0.5, 1.0, Vec3::ZERO); // ≡ L2192
        self.after(0.18, OP_RING, v2, UP, foam, 2.4 * s + 0.6, 0.9, R_RIPPLE); // ≡ L2193
    }

    /// `feather` ≡ L2195-2196.
    pub fn feather(&mut self, pos: Vec3) {
        let feather = feather_col();
        // ≡ L2196: vx, vz, life rands; drag 1.6, buoy -0.7, kind P_FEATHER,
        // fadeIn 0.5, spin 1.8, wobble 0.9, fadeOut 1.5
        let r1 = self.rand();
        let r2 = self.rand();
        let life = self.rand();
        self.sprite(
            true,
            SpriteInit::new(
                pos,
                Vec3::new((r1 - 0.5) * 0.4, -0.25, (r2 - 0.5) * 0.4),
                feather,
                0.24,
                0.24,
                11.0 + life * 4.0,
                0.97,
            )
            .drag(1.6)
            .buoy(-0.7)
            .kind(P_FEATHER)
            .fade_in(0.5)
            .spin(1.8)
            .wobble(0.9)
            .fade_out(1.5),
        );
    }

    /// `glint` ≡ L2199-2202 (sun glint on wet ink).
    pub fn glint(&mut self, pos: Vec3, color: ColorIn, size: Option<f32>) {
        let size = size.unwrap_or(0.12);
        let col = self.color(color);
        let col_b = col_lerp(col, WHITE, 0.6) * 2.6; // ≡ L2201
        let life = self.rand(); // ≡ L2202
        self.sprite(
            false,
            SpriteInit::new(
                pos,
                Vec3::ZERO,
                col_b,
                size * 0.4,
                size,
                0.35 + life * 0.2,
                1.0,
            )
            .drag(0.0)
            .buoy(0.0)
            .kind(G_STAR + 2.0)
            .fade_in(0.12)
            .spin(0.8),
        );
    }
}

impl Fx {
    /// `update(dt, camera)` ≡ L2206-2230. `camera` is the world position
    /// (deviation ⑤ — the JS side also reads `getWorldDirection` and pushes
    /// view-space uniforms; batch C sets [`Fx::cam_dir`] directly).
    pub fn update(&mut self, dt: f32, camera: Option<Vec3>) {
        // ≡ L2207: `dt = Math.min(dt || 0, 0.05)` (NaN/0 → 0; negatives pass
        // through exactly as in JS).
        let dt = if dt.is_nan() { 0.0 } else { dt }.min(0.05);
        if dt > 0.0 {
            self.dt = dt; // ≡ L2208
        }
        self.time += dt; // ≡ L2209
        self.checks = 0; // ≡ L2210
        // L2211-2213 / L2217-2222: shader uniform time + view-space sun/up +
        // mote camera — renderer state, batch C (deviation ③).
        if let Some(cam) = camera {
            self.cam_pos = cam; // ≡ L2215
        }
        self.run_schedule(dt); // ≡ L2223
        self.update_drops(dt); // ≡ L2224
        self.update_sprites(true, dt); // ≡ L2225 puffs
        self.update_sprites(false, dt); // ≡ L2226 glows
        self.update_rings(dt); // ≡ L2227
        self.update_shells(dt); // ≡ L2228
        self.update_beams(dt); // ≡ L2229
    }

    /// `clear()` ≡ L2232-2236: zero every pool (the mote field is untouched).
    pub fn clear(&mut self) {
        self.drops.clear();
        self.puffs.clear();
        self.glows.clear();
        self.rings.clear();
        self.shells.clear();
        self.beams.clear();
        self.marks.clear();
        self.pillars.clear();
        self.sched.clear();
    }

    /// `setLighting(env.getSkyColors())` ≡ L2240-2249.
    pub fn set_lighting(&mut self, o: LightInput) {
        if let Some(v) = o.sun_dir {
            self.light.sun_dir = th_normalize(v); // ≡ L2242
        }
        // ≡ L2243-2244: `sunCol = (sunColor || sun) * min(1.25, intensity/3)`
        if let Some(sc) = o.sun_color {
            let k = (o.sun_intensity.unwrap_or(3.0) / 3.0).min(1.25);
            self.light.sun_col = sc * k;
        }
        // ≡ L2245-2246: sky (horizon blend only when sky is present)
        if let Some(sky) = o.sky {
            let mut s = sky;
            if let Some(h) = o.horizon {
                s = col_lerp(s, h, 0.45);
            }
            self.light.sky = s;
        }
        if let Some(g) = o.ground {
            self.light.ground = g; // ≡ L2247
        }
        // ≡ L2248: mote color follows the sun color (`sc` re-check)
        if let Some(sc) = o.sun_color {
            self.mote_col = col_lerp(sc, WHITE, 0.35) * 0.9;
        }
    }

    /// `setSunDir(v)` ≡ L2250.
    pub fn set_sun_dir(&mut self, v: Vec3) {
        self.light.sun_dir = th_normalize(v);
    }

    /// `stats()` ≡ L2252-2254.
    pub fn stats(&self) -> FxStats {
        FxStats {
            drops: self.drops.len(),
            puffs: self.puffs.len(),
            glows: self.glows.len(),
            rings: self.rings.len(),
            shells: self.shells.len(),
            beams: self.beams.len(),
            checks: self.checks,
            sched: self.sched.len(),
            cap_drops: self.d_cap,
            cap_puffs: self.puff_cap,
            cap_glows: self.glow_cap,
            cap_rings: self.ring_cap,
        }
    }

    /// `triangles()` ≡ L2256-2261 with the geometry constants baked in
    /// (deviation ⑥: 2 tris per droplet/sprite/ring/mote quad, 320 per shell
    /// icosphere, 336 per beam cylinder).
    pub fn triangles(&self) -> usize {
        (self.drops.len() + self.puffs.len() + self.glows.len() + self.rings.len()) * 2
            + self.shells.len() * 320
            + self.beams.len() * 336
            + if self.motes_visible {
                self.mote_count * 2
            } else {
                0
            }
    }
}

#[cfg(test)]
mod tests {
    use super::super::particles::FxOptions;
    use super::*;

    fn fresh() -> Fx {
        Fx::with_seed(FxOptions::default(), 1234)
    }

    /// Every contract API must touch its documented pools.
    #[test]
    fn contract_apis_spawn() {
        let mut fx = fresh();
        let pos = Vec3::new(0.0, 0.5, 0.0);
        fx.burst(pos, Some(UP), ColorIn::Hex(0xff0000), BurstOpts::default());
        assert!(fx.drops.len() >= 12, "burst droplets");
        assert_eq!(fx.shells.len(), 1, "burst sheet");
        let b = fx.drops.len();
        fx.drop(
            pos,
            Vec3::new(0.0, 2.0, 0.0),
            ColorIn::Hex(0xff0000),
            DropOpts::default(),
        );
        assert_eq!(fx.drops.len(), b + 1);
        fx.ring(pos, Some(UP), ColorIn::Hex(0xff0000), RingOpts::default());
        assert_eq!(fx.rings.len(), 1);
        let d = fx.drops.len();
        fx.explosion(pos, ColorIn::Hex(0xff6347), None);
        assert!(fx.drops.len() > d && fx.shells.len() == 2 && fx.glows.len() >= 2);
        fx.splatted(pos, ColorIn::Hex(0xff6347));
        assert_eq!(fx.shells.len(), 3);
        fx.wake(pos, Vec3::X, ColorIn::Hex(0xff6347), None, None);
        assert!(fx.drops.len() > d);
        for kind in [
            MuzzleKind::Shooter,
            MuzzleKind::Blaster,
            MuzzleKind::Charger,
        ] {
            let g = fx.glows.len();
            fx.muzzle(pos, Vec3::X, ColorIn::Hex(0xff6347), kind);
            assert!(fx.glows.len() > g, "muzzle glow for {kind:?}");
        }
        let s = fx.sched.len();
        fx.spawn_flash(pos, ColorIn::Hex(0xff6347));
        assert_eq!(fx.sched.len(), s + 1);
        assert_eq!(fx.sched[fx.sched.len() - 1].delay, 0.12); // ≡ L1445
        fx.rain(
            pos,
            None,
            ColorIn::Css("#808080"),
            Some(1.0),
            RainOpts::default(),
        );
        assert!(!fx.drops.is_empty());
    }

    /// `_near` culling ≡ L723 usage sites.
    #[test]
    fn near_culling() {
        let mut fx = fresh();
        fx.cam_pos = Vec3::new(0.0, 500.0, 0.0); // everything is far away
        fx.wake(Vec3::ZERO, Vec3::X, ColorIn::Rgb(WHITE), None, None); // > 45 m
        assert!(fx.drops.is_empty(), "wake culled (≡ L1362)");
        fx.swim_carve(Vec3::ZERO, Vec3::X, Vec3::Z, ColorIn::Rgb(WHITE), None); // > 32 m
        assert!(fx.puffs.is_empty(), "swimCarve culled (≡ L1382)");
        let opts = BurstOpts {
            count: Some(20.0),
            ..Default::default()
        };
        fx.burst(Vec3::ZERO, None, ColorIn::Rgb(WHITE), opts);
        assert!(fx.shells.is_empty(), "sheet culled at > 24 m (≡ L1259)");
        assert!(!fx.drops.is_empty(), "droplets still spawn when far");
        // close the camera: the sheet appears
        fx.cam_pos = Vec3::ZERO;
        fx.burst(Vec3::ZERO, None, ColorIn::Rgb(WHITE), opts);
        assert_eq!(fx.shells.len(), 1);
    }

    /// The four JS `_after` call sites and their exact delays.
    #[test]
    fn after_timings() {
        let mut fx = fresh();
        fx.spawn_flash(Vec3::ZERO, ColorIn::Rgb(WHITE));
        assert_eq!(fx.sched[0].delay, 0.12); // ≡ L1445
        fx.slam_wave(Vec3::ZERO, ColorIn::Rgb(WHITE), None);
        assert_eq!(fx.sched[1].delay, 0.1); // ≡ L1994
        fx.water_splash(Vec3::ZERO, None);
        assert_eq!(fx.sched[2].delay, 0.12); // ≡ L2130
        fx.sea_spray(Vec3::ZERO, Vec3::X, None);
        assert_eq!(fx.sched[3].delay, 0.18); // ≡ L2193
        // `update` clamps dt to 0.05 (≡ L2207): three frames are needed for
        // the 0.12/0.1/0.12 entries to come due, the 0.18 one stays pending.
        let before = fx.rings.len();
        for _ in 0..3 {
            fx.update(0.05, None);
        }
        assert_eq!(fx.rings.len(), before + 3); // OP_RING entries dispatched
        assert_eq!(fx.sched.len(), 1);
    }

    /// Same seed ⇒ frame-for-frame identical pools (rand-order guard).
    #[test]
    fn determinism_double_run() {
        let mut a = fresh();
        let mut b = fresh();
        for fx in [&mut a, &mut b] {
            let p = Vec3::new(1.0, 0.5, -2.0);
            fx.burst(
                p,
                Some(UP),
                ColorIn::Hex(0x22aa55),
                BurstOpts {
                    count: Some(16.0),
                    ring: true,
                    mist: true,
                    ..Default::default()
                },
            );
            fx.explosion(p, ColorIn::Css("tomato"), Some(4.0));
            fx.muzzle(p, Vec3::X, ColorIn::Hex(0xff0000), MuzzleKind::Charger);
            fx.footstep(p, ColorIn::Hex(0x101010), 2, Some(Vec3::Z), Some(9.0));
            fx.slam_wave(p, ColorIn::Hex(0x101010), Some(6.0));
            fx.spin_up(p, Vec3::Y, ColorIn::Hex(0x101010), Some(1.0), true);
            fx.sea_spray(p, Vec3::X, Some(1.4));
            fx.update(1.0 / 60.0, Some(Vec3::new(3.0, 2.0, 3.0)));
            fx.rain_sheet(p, 5.0, ColorIn::Css("#808080"), Some(0.5));
            fx.super_jump_charge(p, ColorIn::Hex(0x101010), Some(0.8));
            fx.update(0.2, None);
            fx.water_splash(p, Some(1.2));
            fx.enemy_ink_sizzle(p, ColorIn::Hex(0x101010));
            fx.special_sparkle(p, ColorIn::Hex(0xffcc00), None);
            fx.update(0.25, None);
        }
        assert_eq!(a.drops, b.drops);
        assert_eq!(a.puffs, b.puffs);
        assert_eq!(a.glows, b.glows);
        assert_eq!(a.rings, b.rings);
        assert_eq!(a.shells, b.shells);
        assert_eq!(a.beams, b.beams);
        assert_eq!(a.marks, b.marks);
        assert_eq!(a.pillars, b.pillars);
        assert_eq!(a.sched, b.sched);
        assert_eq!(a.time, b.time);
        assert_eq!(a.recycle, b.recycle);
    }

    /// Player-recipe families all spawn something.
    #[test]
    fn player_recipes() {
        let mut fx = fresh();
        let p = Vec3::new(0.0, 0.2, 0.0);
        let c = ColorIn::Hex(0x112233);
        let n0 = fx.drops.len();
        fx.footstep(p, c, 0, Some(Vec3::X), Some(8.0));
        fx.footstep(p, c, 1, Some(Vec3::X), Some(8.0));
        fx.footstep(p, c, 2, Some(Vec3::X), Some(8.0));
        assert!(fx.drops.len() > n0 && !fx.puffs.is_empty());
        fx.land(p, c, 1, Some(10.0));
        fx.jump_off(p, c, 0, false);
        fx.jump_off(p, c, 0, true);
        fx.form_pop(p, c, Some(true), false);
        fx.form_pop(p, c, Some(false), true);
        fx.dive(p, c, Some(6.0));
        fx.emerge(p, c, Some(6.0));
        fx.bubbles(p, c, Some(6.0));
        fx.climb_drip(p, UP, c);
        fx.climb_pop(p, Vec3::X, c);
        fx.hurt_drip(p, c, None);
        fx.hit_splash(p, Vec3::X, c, Some(80.0), false);
        fx.hit_splash(p, Vec3::X, c, Some(80.0), true);
        fx.mist(p, Some(Vec3::Y), c, None, None);
        fx.shot_trail(p, Vec3::new(10.0, 0.0, 0.0), c, true);
        fx.shot_trail(p, Vec3::new(10.0, 0.0, 0.0), c, false);
        assert!(fx.glows.len() >= 3, "bubbles glow pool filled (≡ L1612)");
    }

    /// Weapon / boss recipe families.
    #[test]
    fn weapon_and_boss_recipes() {
        let mut fx = fresh();
        fx.dt = 0.5; // force the dt-scaled emitters to produce particles
        let p = Vec3::new(0.0, 0.5, 0.0);
        let c = ColorIn::Hex(0x992211);
        fx.dry_fire(p, Vec3::X);
        fx.charge_glow(p, c, 1.0);
        fx.charge_full(p, c);
        fx.beam_impact(p, UP, c, Some(1.0));
        fx.beam_trail(p, p + Vec3::new(12.0, 0.0, 0.0), c, Some(1.0));
        assert!(fx.beam_trail_short_noop());
        fx.roller_spray(p, Vec3::X, 2.0, c, Some(1.0));
        fx.flick_curtain(p, Vec3::X, c, None);
        fx.beep_pulse(p, Some(p), Some(UP), c, None, Some(0.5));
        fx.bomb_trail(p, Vec3::new(4.0, -2.0, 0.0), c);
        fx.bounce_splash(p, UP, c);
        fx.spin_up(p, Vec3::Y, c, Some(1.0), false);
        fx.spin_full(p, Vec3::Y, c);
        fx.dodge_splash(p, Vec3::X, c);
        fx.dodge_skid(p, Vec3::X, c, Some(0.3));
        fx.dodge_plant(p, Vec3::X, c);
        fx.slosh_trail(p, Vec3::X, c, true);
        fx.slosh_impact(p, Some(UP), Vec3::X, c);
        fx.slam_launch(p, c);
        fx.slam_charge(p, c, Some(1.0));
        fx.slam_fall(p, c);
        fx.storm_start(p, c, None);
        fx.storm_puddle(p, Some(UP), c);
        fx.storm_flash(p, c, None);
        assert!(!fx.drops.is_empty() && !fx.puffs.is_empty() && !fx.glows.is_empty());
        // no recipe in this family spawns beams (geysers live in slam_wave /
        // super_jump_launch, covered by other tests)
        assert!(fx.rings.len() >= 2 && !fx.shells.is_empty());
    }

    /// Super-jump / water / misc families.
    #[test]
    fn super_water_misc_recipes() {
        let mut fx = fresh();
        fx.dt = 0.5;
        let p = Vec3::new(0.0, 0.2, 0.0);
        let c = ColorIn::Hex(0x2266ff);
        fx.super_jump_launch(p, c);
        assert_eq!(fx.beams.len(), 1, "geyser (≡ L2070)");
        fx.super_jump_trail(p, Vec3::new(2.0, 12.0, 0.0), c);
        fx.super_jump_land(p, c);
        fx.ghost(p, c);
        // ≡ L890: the squid ghost is the one sprite with rot = 0
        let ghost = fx
            .puffs
            .iter()
            .find(|s| s.kind == P_GHOST)
            .expect("ghost puff");
        assert_eq!(ghost.rot, 0.0);
        fx.water_plop(p, None);
        fx.glint(p, c, None);
        assert!(
            fx.glows.iter().any(|s| s.kind == G_STAR + 2.0),
            "glint star"
        );
        fx.feather(p);
        assert!(
            fx.puffs
                .iter()
                .any(|s| s.kind == P_FEATHER && s.life >= 11.0)
        );
        assert!(!fx.drops.is_empty());
    }

    /// Immediate-mode APIs + their pool caps (≡ L695-696).
    #[test]
    fn immediate_apis_and_caps() {
        let mut fx = fresh();
        fx.danger_ring(Vec3::ZERO, UP, ColorIn::Rgb(WHITE), None, Some(0.5));
        assert_eq!(fx.marks.len(), 1);
        assert_eq!(fx.marks[0].style, R_DANGER);
        fx.laser_dot(Vec3::ZERO, UP, ColorIn::Rgb(WHITE), None);
        assert_eq!(fx.marks.len(), 2); // the R_BLOT under the dot
        assert!(!fx.glows.is_empty());
        fx.jump_marker(Vec3::ZERO, ColorIn::Rgb(WHITE), Some(0.3));
        assert_eq!(fx.marks.len(), 3);
        assert_eq!(fx.marks[2].style, R_TARGET);
        assert_eq!(fx.pillars.len(), 1);
        for _ in 0..80 {
            fx.danger_ring(Vec3::ZERO, UP, ColorIn::Rgb(WHITE), None, None);
            fx.jump_marker(Vec3::ZERO, ColorIn::Rgb(WHITE), None);
        }
        assert_eq!(fx.marks.len(), 48, "MARK_CAP");
        assert_eq!(fx.pillars.len(), 8, "PILLAR_CAP");
    }

    /// stats / triangles / clear / set_lighting (≡ L2232-2261).
    #[test]
    fn frame_surface() {
        let mut fx = fresh();
        assert_eq!(fx.triangles(), 300 * 2, "motes only");
        let p = Vec3::new(0.0, 1.0, 0.0);
        fx.drop(p, Vec3::Y, ColorIn::Rgb(WHITE), DropOpts::default());
        fx.sprite(
            true,
            SpriteInit::new(p, Vec3::ZERO, WHITE, 0.1, 0.2, 1.0, 0.5),
        );
        fx.sprite(
            false,
            SpriteInit::new(p, Vec3::ZERO, WHITE, 0.1, 0.2, 1.0, 0.5),
        );
        fx.ring_raw(p, UP, WHITE, 1.0, 0.5, R_WAVE, 0.9, 1.0, Vec3::ZERO);
        fx.shell(
            p, WHITE, 0.1, 0.5, 0.07, 0.3, 0.95, 0.0, 0.0, 0.2, None, 0.0,
        );
        fx.beam(p, WHITE, 1.0, 2.0, 0.5);
        assert_eq!(fx.triangles(), 600 + 4 * 2 + 320 + 336);
        fx.motes_visible = false;
        assert_eq!(fx.triangles(), 4 * 2 + 320 + 336);
        let st = fx.stats();
        assert_eq!(
            st,
            FxStats {
                drops: 1,
                puffs: 1,
                glows: 1,
                rings: 1,
                shells: 1,
                beams: 1,
                checks: 0,
                sched: 0,
                cap_drops: 2600,
                cap_puffs: 520,
                cap_glows: 300,
                cap_rings: 300,
            }
        );
        fx.clear();
        assert!(fx.drops.is_empty() && fx.puffs.is_empty() && fx.sched.is_empty());
        assert_eq!(fx.mote_count, 300, "motes untouched by clear (≡ L2235)");

        fx.set_lighting(LightInput {
            sun_dir: Some(Vec3::new(2.0, 0.0, 0.0)),
            sun_color: Some(Vec3::new(1.0, 1.0, 1.0)),
            sun_intensity: Some(6.0),
            sky: Some(Vec3::new(0.0, 0.0, 1.0)),
            horizon: Some(Vec3::new(1.0, 0.0, 0.0)),
            ground: Some(Vec3::new(0.1, 0.1, 0.1)),
        });
        assert_eq!(fx.light.sun_dir, Vec3::X); // normalized (≡ L2242)
        assert_eq!(fx.light.sun_col, Vec3::splat(1.25)); // min(1.25, 6/3) ≡ L2244
        assert_eq!(fx.light.sky, Vec3::new(0.45, 0.0, 0.55)); // sky→horizon 0.45 ≡ L2246
        assert_eq!(fx.light.ground, Vec3::splat(0.1));
        assert_eq!(fx.mote_col, Vec3::splat(0.9)); // ≡ L2248
        // horizon alone must not touch sky (≡ L2245 nesting)
        fx.set_lighting(LightInput {
            sky: None,
            horizon: Some(Vec3::splat(0.5)),
            ..Default::default()
        });
        assert_eq!(fx.light.sky, Vec3::new(0.45, 0.0, 0.55));
    }

    impl Fx {
        /// `beamTrail` bails out below 0.5 m (≡ L1749).
        fn beam_trail_short_noop(&mut self) -> bool {
            let before = self.puffs.len() + self.drops.len();
            self.beam_trail(
                Vec3::ZERO,
                Vec3::new(0.3, 0.0, 0.0),
                ColorIn::Rgb(WHITE),
                None,
            );
            self.puffs.len() + self.drops.len() == before
        }
    }
}
