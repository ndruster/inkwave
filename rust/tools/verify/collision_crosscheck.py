#!/usr/bin/env python3
"""Independent reimplementation of the INKWAVE collision world, used to
cross-validate `inkwave_sim::collision` (Task 4 TR-4.1/4.2).

It reads ONLY the extracted layout JSON (never the Rust code) and rebuilds:
  * block frames for box/obox/ramp  (ported by hand from src/world/level.js)
  * the 4 m XZ spatial hash
  * face construction + hidden-face culling (so face ids are comparable)
  * groundHeight and raycast (slab intersection), ported from level.js and
    src/game/physics.js

Usage: collision_crosscheck.py <collision_dump.json>
  (the JSON is produced by `cargo run --example collision_dump -p inkwave_sim`)

Exit 0 iff every Rust record matches this script within 1e-3 m / exact ids.
"""

import json
import math
import sys

# --------------------------------------------------------------------- vectors

def vadd(a, b): return [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
def vsub(a, b): return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
def vmul(a, s): return [a[0] * s, a[1] * s, a[2] * s]
def vdot(a, b): return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
def vcross(a, b):
    return [a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0]]
def vlen(a): return math.sqrt(vdot(a, a))
def vnorm(a):
    l = vlen(a)
    return [a[0] / l, a[1] / l, a[2] / l] if l > 1e-12 else [0.0, 0.0, 0.0]
def vmin(a, b): return [min(a[0], b[0]), min(a[1], b[1]), min(a[2], b[2])]
def vmax(a, b): return [max(a[0], b[0]), max(a[1], b[1]), max(a[2], b[2])]

UP = [0.0, 1.0, 0.0]
HASH_CELL = 4.0

# ---------------------------------------------------------------- block build

class Block:
    def __init__(self, d):
        kind = d["kind"]
        c = d
        if kind == "box":
            mn, mx = d["min"], d["max"]
            self.center = [(mn[0] + mx[0]) / 2, (mn[1] + mx[1]) / 2, (mn[2] + mx[2]) / 2]
            self.half = [(mx[0] - mn[0]) / 2, (mx[1] - mn[1]) / 2, (mx[2] - mn[2]) / 2]
            self.axes = [[1, 0, 0], [0, 1, 0], [0, 0, 1]]
        elif kind == "obox":
            a = math.radians(d["rotY"])
            cc, ss = math.cos(a), math.sin(a)
            self.center = list(d["center"])
            s = d["size"]
            self.half = [s[0] / 2, s[1] / 2, s[2] / 2]
            self.axes = [[cc, 0.0, -ss], [0.0, 1.0, 0.0], [ss, 0.0, cc]]
        else:  # ramp
            low, high = list(d["low"]), list(d["high"])
            sv = vsub(high, low)
            length = vlen(sv)
            s = vmul(sv, 1.0 / length)
            flat = vnorm([s[0], 0.0, s[2]])
            side = vnorm(vcross(UP, flat))
            n = vnorm(vcross(s, side))
            if n[1] < 0.0:
                n = vmul(n, -1.0)
                side = vmul(side, -1.0)
            cos_t = n[1]
            rise = high[1] - low[1]
            thick = d["thickness"] if d.get("thin") else max(d["thickness"], rise * cos_t + 0.35)
            ext = 0.6
            a0 = vsub(low, vmul(s, ext))
            top_mid = vmul(vadd(a0, high), 0.5)
            self.center = vsub(top_mid, vmul(n, thick / 2.0))
            self.half = [d["width"] / 2.0, thick / 2.0, (length + ext) / 2.0]
            axes = [side, n, s]
            if vdot(vcross(side, n), s) < 0.0:
                axes[0] = vmul(side, -1.0)
            self.axes = axes

        # AABB of 8 corners
        lo = [math.inf] * 3
        hi = [-math.inf] * 3
        for i in range(8):
            p = list(self.center)
            for k in range(3):
                sgn = 1.0 if (i & (1 << k)) else -1.0
                p = vadd(p, vmul(self.axes[k], sgn * self.half[k]))
            lo = vmin(lo, p)
            hi = vmax(hi, p)
        self.aabb_min, self.aabb_max = lo, hi

        self.solid = c.get("solid", True)
        self.grate = bool(c.get("grate"))
        self.rail = bool(c.get("rail"))
        self.hidden = bool(c.get("hidden"))
        self.paint = c.get("paint", True) and not self.grate
        self.roof = bool(c.get("roof"))
        self.perch = bool(c.get("perch"))
        self.no_paint = c.get("noPaint", [])
        self.murals = [(m["id"], m["n"]) for m in c.get("mural", [])]
        self.faces = [-1] * 6


class World:
    def __init__(self, defs, bounds):
        self.blocks = [Block(d) for d in defs]
        self.bounds = bounds
        self.has_rails = any(b.rail for b in self.blocks)
        minx, maxx, minz, maxz = bounds
        self.hx0 = minx - 8.0
        self.hz0 = minz - 8.0
        self.hw = math.ceil((maxx - minx + 16.0) / HASH_CELL)
        self.hd = math.ceil((maxz - minz + 16.0) / HASH_CELL)
        self.hash = [[] for _ in range(self.hw * self.hd)]
        for bid, b in enumerate(self.blocks):
            x0 = self._hxi(b.aabb_min[0]); x1 = self._hxi(b.aabb_max[0])
            z0 = self._hzi(b.aabb_min[2]); z1 = self._hzi(b.aabb_max[2])
            for z in range(z0, z1 + 1):
                for x in range(x0, x1 + 1):
                    self.hash[z * self.hw + x].append(bid)
        self.faces = self._build_faces()

    def _hxi(self, x):
        return max(0, min(self.hw - 1, math.floor((x - self.hx0) / HASH_CELL)))
    def _hzi(self, z):
        return max(0, min(self.hd - 1, math.floor((z - self.hz0) / HASH_CELL)))

    def query(self, minx, minz, maxx, maxz):
        out = []
        for z in range(self._hzi(minz), self._hzi(maxz) + 1):
            for x in range(self._hxi(minx), self._hxi(maxx) + 1):
                for bid in self.hash[z * self.hw + x]:
                    if bid not in out:
                        out.append(bid)
        return out

    def point_in_block(self, bid, p, pad=0.0):
        b = self.blocks[bid]
        d = vsub(p, b.center)
        return (abs(vdot(d, b.axes[0])) < b.half[0] + pad
                and abs(vdot(d, b.axes[1])) < b.half[1] + pad
                and abs(vdot(d, b.axes[2])) < b.half[2] + pad)

    def point_inside(self, p, pad, exclude):
        for bid in self.query(p[0] - .01, p[2] - .01, p[0] + .01, p[2] + .01):
            if bid != exclude and self.blocks[bid].solid and self.point_in_block(bid, p, pad):
                return True
        return False

    def ground_height(self, x, z, y_max=50.0, skip_grates=False):
        best = -math.inf
        for bid in self.query(x - .01, z - .01, x + .01, z + .01):
            b = self.blocks[bid]
            if not b.solid or (skip_grates and b.grate):
                continue
            n = b.axes[1]
            if n[1] < 0.5:
                continue
            top = vadd(b.center, vmul(n, b.half[1]))
            y = top[1] - (n[0] * (x - top[0]) + n[2] * (z - top[2])) / n[1]
            if y <= y_max and y > best and self.point_in_block(bid, [x, y - .01, z], .001):
                best = y
        return best

    def raycast(self, origin, direction, max_dist, skip_grates):
        ex = vadd(origin, vmul(direction, max_dist))
        best, best_k, best_sign, best_b = max_dist, 0, 0.0, -1
        for bid in self.query(min(origin[0], ex[0]), min(origin[2], ex[2]),
                              max(origin[0], ex[0]), max(origin[2], ex[2])):
            b = self.blocks[bid]
            if not b.solid or (skip_grates and b.grate):
                continue
            o = vsub(origin, b.center)
            tmin, tmax, kmin, smin = -math.inf, math.inf, 0, 0.0
            miss = False
            for k in range(3):
                ax = b.axes[k]
                ok, dk, h = vdot(o, ax), vdot(direction, ax), b.half[k]
                if abs(dk) < 1e-9:
                    if ok < -h or ok > h:
                        miss = True
                        break
                    continue
                t1, t2 = (-h - ok) / dk, (h - ok) / dk
                s1 = -1.0
                if t1 > t2:
                    t1, t2, s1 = t2, t1, 1.0
                if t1 > tmin:
                    tmin, kmin, smin = t1, k, s1
                tmax = min(tmax, t2)
                if tmin > tmax:
                    miss = True
                    break
            if miss or tmax < 0 or tmin < 0 or tmin > best:
                continue
            best, best_k, best_sign, best_b = tmin, kmin, smin, bid
        if best_b < 0:
            return None
        return {"dist": best, "block": best_b,
                "normal": vmul(self.blocks[best_b].axes[best_k], best_sign),
                "face": self.blocks[best_b].faces[best_k * 2 + (0 if best_sign > 0 else 1)]}

    # ------------------------------------------------------------ face build

    def _build_faces(self):
        faces = []
        for bid, b0 in enumerate(self.blocks):
            if b0.hidden:
                continue
            b = b0
            h = b.half
            for k in range(3):
                for sign in (1.0, -1.0):
                    n = vmul(b.axes[k], sign)
                    if n[1] < -0.5 and b.center[1] - h[1] < 0.5:
                        continue
                    others = [1, 2] if k == 0 else ([0, 2] if k == 1 else [0, 1])
                    if abs(n[1]) < 0.5:
                        vi = others[0] if abs(b.axes[others[0]][1]) > abs(b.axes[others[1]][1]) else others[1]
                        ui = others[1] if others[0] == vi else others[0]
                    else:
                        ui = others[0] if abs(b.axes[others[0]][0]) >= abs(b.axes[others[1]][0]) else others[1]
                        vi = others[1] if others[0] == ui else others[0]
                    vv = b.axes[vi]
                    if abs(n[1]) < 0.5:
                        if vv[1] < 0.0:
                            vv = vmul(vv, -1.0)
                    elif vv[2] < 0.0 and abs(vv[2]) > 0.3:
                        vv = vmul(vv, -1.0)
                    u = vcross(vv, n)
                    su, sv = 2.0 * h[ui], 2.0 * h[vi]
                    origin = vsub(vsub(vadd(b.center, vmul(n, h[k])), vmul(u, su / 2)), vmul(vv, sv / 2))
                    if self._face_hidden(bid, origin, u, vv, n, su, sv):
                        continue
                    fid = len(faces)
                    faces.append({"id": fid, "block": bid, "n": n})
                    b.faces[k * 2 + (0 if sign > 0 else 1)] = fid
        return faces

    def _face_hidden(self, block, origin, u, vv, n, su, sv):
        nu = max(2, math.ceil(su / 1.25))
        nv = max(2, math.ceil(sv / 1.25))
        for j in range(nv + 1):
            for i in range(nu + 1):
                uu = min(max(i / nu * su, 0.05), su - 0.05)
                vvf = min(max(j / nv * sv, 0.05), sv - 0.05)
                p = vadd(vadd(vadd(origin, vmul(u, uu)), vmul(vv, vvf)), vmul(n, 0.03))
                if not self.point_inside(p, 0.0, block):
                    return False
        return True

# --------------------------------------------------------------------- worlds

def common():
    return {"color": "#dddddd", "pattern": 0, "paint": True, "solid": True}

SYNTH = [
    {"kind": "box", "min": [-10, -1, -10], "max": [-4, 0, 10], **common()},
    {"kind": "ramp", "low": [-3, 0, -4], "high": [-3, 2, 4], "width": 2,
     "thickness": 0.3, "thin": True, **common()},
    {"kind": "ramp", "low": [3, 0, -4], "high": [3, 2, 4], "width": 2,
     "thickness": 0.3, "thin": True, **common()},
]
SYNTH_BOUNDS = (-10, 10, -10, 10)

def r4(v):
    if v is None or (isinstance(v, float) and not math.isfinite(v)):
        return None
    x = round(v, 4)
    return 0.0 if x == 0 else x

def expected_records(tide_path):
    synth = World(SYNTH, SYNTH_BOUNDS)
    doc = json.load(open(tide_path))
    tide = World(doc["primitives"],
                 (doc["stage"]["bounds"]["minX"], doc["stage"]["bounds"]["maxX"],
                  doc["stage"]["bounds"]["minZ"], doc["stage"]["bounds"]["maxZ"]))
    recs = []
    recs.append({"k": "struct", "world": "tide", "blocks": len(tide.blocks),
                 "faces": len(tide.faces), "has_rails": tide.has_rails})
    for bid, b in enumerate(tide.blocks):
        for k in range(3):
            for si, sign in ((0, 1.0), (1, -1.0)):
                if b.faces[k * 2 + si] >= 0:
                    n = vmul(b.axes[k], sign)
                    recs.append({"k": "face", "block": bid, "ax": k,
                                 "sign": 1 if sign > 0 else -1,
                                 "nx": r4(n[0]), "ny": r4(n[1]), "nz": r4(n[2])})
    for x in (-3.0, 3.0):
        for z in (-3.0, -1.5, 0.0, 1.5, 3.0):
            y = synth.ground_height(x, z)
            recs.append({"k": "gh", "world": "synth", "x": r4(x), "z": r4(z),
                         "y": r4(y) if math.isfinite(y) else None})
    y = synth.ground_height(-7.0, 0.0)
    recs.append({"k": "gh", "world": "synth", "x": r4(-7.0), "z": r4(0.0), "y": r4(y)})
    h = synth.raycast([-3, 3, 0], [0, -1, 0], 6.0, False)
    recs.append({"k": "ray", "world": "synth", "ox": -3.0, "oy": 3.0, "oz": 0.0,
                 "dx": 0.0, "dy": -1.0, "dz": 0.0, "hit": True, "dist": r4(h["dist"]),
                 "nx": r4(h["normal"][0]), "ny": r4(h["normal"][1]), "nz": r4(h["normal"][2]),
                 "block": h["block"], "face": h["face"]})
    for z in (-40, -30, -20, -10, -5, 0, 5, 10, 20, 30, 40):
        for x in (-20, -10, -5, -2, 0, 2, 5, 10, 20):
            y = tide.ground_height(float(x), float(z))
            recs.append({"k": "gh", "world": "tide", "x": r4(float(x)), "z": r4(float(z)),
                         "y": r4(y) if math.isfinite(y) else None})
    for pad in doc["stage"]["spawnPads"]:
        y = tide.ground_height(pad[0], pad[2])
        recs.append({"k": "gh", "world": "spawn", "x": r4(pad[0]), "z": r4(pad[2]),
                     "y": r4(y) if math.isfinite(y) else None})
    rays = [
        (-25, 1, 0, 1, 0, 0), (25, 1, 0, -1, 0, 0), (-25, 4, 0, 1, 0, 0),
        (-25, 1, -20, 1, 0, 0), (-25, 1, 20, 1, 0, 0),
        (0, 1, 45, 0, 0, -1), (0, 1, -45, 0, 0, 1),
        (0, 6, -41.8, 0, -1, 0), (0, 6, 41.8, 0, -1, 0),
        (10, 3, 10, -0.7071, 0, -0.7071),
        (-10, 3, -10, 0.7071, 0, 0.7071),
        (20, 2, -30, -0.6, 0.8, 0),
    ]
    for (ox, oy, oz, dx, dy, dz) in rays:
        d = vnorm([dx, dy, dz])
        h = tide.raycast([ox, oy, oz], d, 60.0, True)
        rec = {"k": "ray", "world": "tide", "ox": r4(float(ox)), "oy": r4(float(oy)),
               "oz": r4(float(oz)), "dx": r4(d[0]), "dy": r4(d[1]), "dz": r4(d[2])}
        if h is None:
            rec.update({"hit": False, "dist": None, "nx": None, "ny": None,
                        "nz": None, "block": -1, "face": -1})
        else:
            rec.update({"hit": True, "dist": r4(h["dist"]),
                        "nx": r4(h["normal"][0]), "ny": r4(h["normal"][1]),
                        "nz": r4(h["normal"][2]), "block": h["block"], "face": h["face"]})
        recs.append(rec)
    return recs

def main():
    dump_path = sys.argv[1]
    tide_path = sys.argv[2]
    actual = json.load(open(dump_path))
    expected = expected_records(tide_path)
    failures = 0
    def fkey(r):
        return (r["block"], r["ax"], r["sign"], r["nx"], r["ny"], r["nz"])
    af = {fkey(r) for r in actual if r.get("k") == "face"}
    ef = {fkey(r) for r in expected if r.get("k") == "face"}
    for r in sorted(af - ef):
        print("extra face in rust:", r); failures += 1
    for r in sorted(ef - af):
        print("missing face in rust:", r); failures += 1
    actual = [r for r in actual if r.get("k") != "face"]
    expected = [r for r in expected if r.get("k") != "face"]
    if len(actual) != len(expected):
        print(f"record count mismatch: rust={len(actual)} py={len(expected)}")
        return 1
    for i, (a, e) in enumerate(zip(actual, expected)):
        for key, ev in e.items():
            av = a.get(key)
            if isinstance(ev, float):
                if av is None or abs(av - ev) > 1e-3:
                    print(f"[{i}] {a.get('k')}/{a.get('world')} {key}: rust={av} py={ev}")
                    failures += 1
            else:
                if av != ev:
                    print(f"[{i}] {a.get('k')}/{a.get('world')} {key}: rust={av!r} py={ev!r}")
                    failures += 1
    if failures:
        print(f"CROSSCHECK FAILED: {failures} mismatching fields")
        return 1
    print(f"CROSSCHECK OK: {len(expected)} records match (heights/rays/face ids)")
    return 0

if __name__ == "__main__":
    sys.exit(main())
