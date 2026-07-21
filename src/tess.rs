//! CPU tessellation: construction geometry → GPU vertex/index buffers.
//!
//! Adapted from Colliderscope's `scene/sink.rs`. The miter-clamped quad-strip
//! expansion and the energy-conserving hairline rule are kept; hue, palette and
//! the per-vertex draw reveal are gone.
//!
//! Two departures from the original worth knowing about:
//!
//! * Each vertex carries `u` / `arc_len` / `step_id`, so the *fragment* shader
//!   resolves the reveal. A two-vertex chord still gets a pixel-exact pen head.
//! * Vertices stay in **construction space**; the view transform lives in the
//!   vertex shader. Zoom, pan and resize therefore cost nothing, which is what
//!   makes an endlessly growing figure affordable.

use glam::Vec2;

pub const FEATHER: f32 = 1.25;
const MITER_LIMIT: f32 = 2.0;
/// Coincident-sample threshold in construction units. Well below the sample
/// spacing of even the smallest circle an Apollonian gasket produces.
const DEDUP: f32 = 1e-6;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StrokeVertex {
    /// Construction space (Y-up).
    pub pos: [f32; 2],
    /// Construction-space miter vector: unit normal scaled by 1/cos(θ/2).
    /// The view map is a uniform scale plus a Y flip — conformal — so this
    /// stays a correct unit normal after transform, and the miter ratio is
    /// angle-based and therefore scale-invariant.
    pub miter: [f32; 2],
    /// Which side of the centerline this vertex is on, ±1.
    pub side: f32,
    /// Half width in PIXELS, excluding feather. View-independent by design:
    /// line weight should not change when the camera pulls back.
    pub half_width: f32,
    /// Normalized arc position along this stroke, 0..1.
    pub u: f32,
    /// Total arc length in CONSTRUCTION units; the shader multiplies by the
    /// view scale to recover pixels for the pen head and shimmer wavelengths.
    pub arc_len: f32,
    /// Brightness multiplier: hairline energy conservation.
    pub energy: f32,
    /// Index into the per-frame step storage buffer.
    pub step_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PointInstance {
    /// Construction space — transformed in the vertex shader like everything else.
    pub center: [f32; 2],
    pub radius: f32,
    pub level: f32,
    pub seed: f32,
    pub _pad: f32,
}

/// Per-step animation state. Rewritten every frame; 32 bytes each.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StepGpu {
    pub head: f32,
    pub glow: f32,
    pub pen: f32,
    pub seed: f32,
    /// Linear RGB for this stroke's role — figure, scaffold or accent. The pen
    /// glint is added as white in the shader regardless: a specular highlight
    /// is the light's colour, not the body's.
    pub color: [f32; 3],
    pub _pad: f32,
}

/// Construction space (Y-up, unit-ish) → surface pixels (Y-down).
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub center: Vec2,
    pub scale: f32,
}

impl View {
    /// Frame a unit-radius figure with margin to spare for the halo.
    pub fn fit(width: u32, height: u32) -> Self {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        Self { center: Vec2::new(w * 0.5, h * 0.5), scale: w.min(h) * 0.42 }
    }

    /// Frame a figure of the given construction-space radius.
    pub fn fit_radius(width: u32, height: u32, radius: f32) -> Self {
        let mut v = Self::fit(width, height);
        v.scale /= radius.max(1e-6);
        v
    }

}

#[derive(Default)]
pub struct Tess {
    pub verts: Vec<StrokeVertex>,
    pub indices: Vec<u32>,
    /// The view scale this was tessellated for. Arc sample counts are chosen
    /// against it; drifting far from it means it's time to rebuild.
    pub detail_scale: f32,
    px: Vec<Vec2>,
    us: Vec<f32>,
}

impl Tess {
    pub fn clear(&mut self) {
        self.verts.clear();
        self.indices.clear();
    }

    /// True once the camera has drifted far enough from the tessellation scale
    /// that arc faceting would start to show (or we're wasting samples).
    pub fn needs_retess(&self, scale: f32) -> bool {
        self.detail_scale <= 0.0 || scale > self.detail_scale * 2.0
    }

    /// A straight segment. Two vertices — the per-fragment reveal makes that enough.
    pub fn seg(&mut self, a: Vec2, b: Vec2, width: f32, step_id: u32) {
        self.polyline(&[a, b], false, width, step_id);
    }

    /// A circular arc swept from `a0` to `a1` (signed; |a1 − a0| = TAU is a full
    /// circle). `u` runs along the sweep, so the reveal *is* the compass swing.
    pub fn arc(&mut self, c: Vec2, r: f32, a0: f32, a1: f32, width: f32, step_id: u32) {
        let sweep = a1 - a0;
        // ~one sample per 4 px of arc at the tessellation scale.
        let r_px = (r * self.detail_scale).abs();
        let segs = ((r_px * sweep.abs()) / 4.0).clamp(8.0, 512.0) as usize;
        let closed = (sweep.abs() - std::f32::consts::TAU).abs() < 1e-4;
        let n = if closed { segs } else { segs + 1 };
        let pts: Vec<Vec2> = (0..n)
            .map(|i| {
                let a = a0 + sweep * (i as f32 / segs as f32);
                c + Vec2::new(a.cos(), a.sin()) * r
            })
            .collect();
        self.polyline(&pts, closed, width, step_id);
    }

    /// Emit a polyline in construction space. `closed` links last back to first.
    pub fn polyline(&mut self, pts: &[Vec2], closed: bool, width: f32, step_id: u32) {
        if pts.len() < 2 {
            return;
        }
        self.px.clear();
        for &q in pts {
            if self.px.last().is_none_or(|l| l.distance_squared(q) > DEDUP * DEDUP) {
                self.px.push(q);
            }
        }
        if closed && self.px.len() > 2 {
            if let (Some(&first), Some(&last)) = (self.px.first(), self.px.last()) {
                if first.distance_squared(last) <= DEDUP * DEDUP {
                    self.px.pop();
                }
            }
        }
        if self.px.len() < 2 {
            return;
        }

        // Cumulative arc length, then normalized. The pen therefore travels at
        // constant *screen* speed once scaled, so a short arc and a long chord
        // draw at the same apparent rate.
        self.us.clear();
        self.us.push(0.0);
        let mut total = 0.0f32;
        for i in 1..self.px.len() {
            total += self.px[i].distance(self.px[i - 1]);
            self.us.push(total);
        }
        let closing = if closed { self.px[0].distance(*self.px.last().unwrap()) } else { 0.0 };
        let full = (total + closing).max(1e-9);
        for u in &mut self.us {
            *u /= full;
        }

        let px = std::mem::take(&mut self.px);
        let us = std::mem::take(&mut self.us);
        self.tessellate(&px, &us, closed, full, width, step_id);
        self.px = px;
        self.us = us;
    }

    fn tessellate(
        &mut self,
        px: &[Vec2],
        us: &[f32],
        closed: bool,
        arc_len: f32,
        width: f32,
        step_id: u32,
    ) {
        // Energy-conserving thin lines: strokes narrower than the feather are
        // drawn at feather width with proportionally reduced brightness. Without
        // this, Metatron's 78 hairline chords over-ink the additive field.
        let half_w_raw = (width * 0.5).max(0.1);
        let energy = (half_w_raw / FEATHER).min(1.0);
        let half_w = half_w_raw.max(FEATHER);
        let n = px.len();

        let seg_normal = |i: usize, j: usize| -> Vec2 {
            let d = (px[j] - px[i]).normalize_or_zero();
            Vec2::new(-d.y, d.x)
        };
        let miter = |n0: Vec2, n1: Vec2| -> Vec2 {
            let sum = n0 + n1;
            if sum.length_squared() < 1e-12 {
                n1
            } else {
                let m = sum.normalize();
                let cos = m.dot(n1).max(1.0 / MITER_LIMIT);
                m / cos
            }
        };

        let base = self.verts.len() as u32;
        let count = if closed { n + 1 } else { n };
        for k in 0..count {
            let i = k % n;
            let m = if closed {
                miter(seg_normal((i + n - 1) % n, i), seg_normal(i, (i + 1) % n))
            } else if i == 0 {
                seg_normal(0, 1)
            } else if i == n - 1 {
                seg_normal(n - 2, n - 1)
            } else {
                miter(seg_normal(i - 1, i), seg_normal(i, i + 1))
            };

            // The wrap vertex of a closed path must read u = 1, not wrap to 0.
            let u = if closed && k == n { 1.0 } else { us[i] };
            let p = px[i];
            for side in [1.0f32, -1.0] {
                self.verts.push(StrokeVertex {
                    pos: [p.x, p.y],
                    miter: [m.x, m.y],
                    side,
                    half_width: half_w,
                    u,
                    arc_len,
                    energy,
                    step_id,
                });
            }
        }

        for s in 0..(count - 1) as u32 {
            let a = base + s * 2;
            self.indices.extend_from_slice(&[a, a + 1, a + 2, a + 1, a + 3, a + 2]);
        }
    }
}
