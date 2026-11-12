//! The opening animation: three meshing gears zoom in on the near-black background, a light
//! sweep passes, the wordmark appears letter by letter, the family line fades in, and the app
//! icons fly into their places in the grid while the window fades up beneath. About three
//! seconds, drawn with the painter (no video, no image files besides the app icons).
//!
//! The timeline is a pure function of time, [`intro_frame`], so it is tested without a
//! window; [`pintar`] only draws a frame of it.

use crate::texto::tx;
use std::f32::consts::TAU;

use eframe::egui::{self, Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use utsim_ui_tema::familia::{self, cores};

/// Length of the animation, seconds.
pub const DURACAO: f32 = 3.2;

/// The wordmark.
pub const MARCA: &str = "UTSIM";

/// Everything a frame of the animation needs, from [`intro_frame`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntroState {
    /// 0 at the start, 1 at the end.
    pub progress: f32,
    /// Opacity of the black cover over the window (1 hides it, 0 shows it).
    pub cover: f32,
    /// Opacity of the real window under the cover (fades up at the end).
    pub ui_alpha: f32,
    /// Gears: rotation (radians, of the big one), scale and opacity.
    pub gear_angle: f32,
    pub gear_scale: f32,
    pub gear_alpha: f32,
    /// Light sweep: horizontal position from 0 (left of the logo) to 1 (right), if passing.
    pub sweep: Option<f32>,
    /// Letters of the wordmark shown, fractional (the last one is fading in).
    pub letters: f32,
    /// Opacity of the wordmark as a whole (fades out at the end).
    pub mark_alpha: f32,
    /// Opacity of "FAMÍLIA DE APPS".
    pub tagline_alpha: f32,
    /// Flight of the app icons: 0 gathered under the logo, 1 in their grid places.
    pub icons: f32,
    /// Opacity of the flying icons.
    pub icons_alpha: f32,
    /// The animation is over (the window is fully shown).
    pub done: bool,
}

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// 0 before `a`, 1 after `b`, linear between.
fn span(t: f32, a: f32, b: f32) -> f32 {
    clamp01((t - a) / (b - a))
}

pub fn ease_out_cubic(x: f32) -> f32 {
    1.0 - (1.0 - clamp01(x)).powi(3)
}

pub fn ease_out_expo(x: f32) -> f32 {
    let x = clamp01(x);
    if x >= 1.0 {
        1.0
    } else {
        1.0 - 2f32.powf(-10.0 * x)
    }
}

pub fn ease_in_out_cubic(x: f32) -> f32 {
    let x = clamp01(x);
    if x < 0.5 {
        4.0 * x * x * x
    } else {
        1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
    }
}

/// The frame at `t` seconds since the start. Pure; `t >= DURACAO` is the final frame.
pub fn intro_frame(t: f32) -> IntroState {
    let t = t.max(0.0);
    if t >= DURACAO {
        return IntroState {
            progress: 1.0,
            cover: 0.0,
            ui_alpha: 1.0,
            gear_angle: gear_angle(DURACAO),
            gear_scale: 1.12,
            gear_alpha: 0.0,
            sweep: None,
            letters: MARCA.len() as f32,
            mark_alpha: 0.0,
            tagline_alpha: 0.0,
            icons: 1.0,
            icons_alpha: 0.0,
            done: true,
        };
    }
    let fade_out = ease_in_out_cubic(span(t, 2.1, 2.6));
    let sweep_t = span(t, 1.05, 1.75);
    IntroState {
        progress: t / DURACAO,
        cover: 1.0 - ease_in_out_cubic(span(t, 2.35, DURACAO)),
        ui_alpha: ease_out_cubic(span(t, 2.2, DURACAO)),
        gear_angle: gear_angle(t),
        gear_scale: 0.82 + 0.18 * ease_out_expo(span(t, 0.0, 0.9)) + 0.12 * fade_out,
        gear_alpha: ease_out_cubic(span(t, 0.0, 0.5)) * (1.0 - fade_out),
        sweep: (sweep_t > 0.0 && sweep_t < 1.0).then(|| ease_in_out_cubic(sweep_t)),
        letters: MARCA.len() as f32 * span(t, 0.55, 1.45),
        mark_alpha: 1.0 - fade_out,
        tagline_alpha: ease_out_cubic(span(t, 1.45, 1.95)) * (1.0 - fade_out),
        icons: span(t, 2.3, 3.1),
        icons_alpha: ease_out_cubic(span(t, 1.75, 2.15)) * (1.0 - span(t, 3.1, DURACAO)),
        done: false,
    }
}

/// Rotation slows down as the logo settles (ease-out on a constant turn).
fn gear_angle(t: f32) -> f32 {
    let x = clamp01(t / DURACAO);
    TAU * 0.42 * (1.0 - (1.0 - x).powi(2))
}

/// The animation of one launch: started at a time, maybe skipped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Intro {
    pub start: f64,
    pub skipped: bool,
}

impl Intro {
    pub fn new(start: f64) -> Self {
        Self {
            start,
            skipped: false,
        }
    }

    /// The frame at absolute time `now`; a skipped animation is at its end.
    pub fn frame(&self, now: f64) -> IntroState {
        if self.skipped {
            intro_frame(DURACAO)
        } else {
            intro_frame((now - self.start) as f32)
        }
    }

    pub fn skip(&mut self) {
        self.skipped = true;
    }
}

/// A flying icon: its texture and the place it lands in.
pub struct Voo {
    pub texture: Option<egui::TextureId>,
    pub initials: String,
    pub accent: Color32,
    pub target: Rect,
}

/// The outline of a gear (star-shaped around its centre).
fn gear_outline(center: Pos2, radius: f32, teeth: usize, angle: f32) -> Vec<Pos2> {
    let inner = radius * 0.84;
    let mut pts = Vec::with_capacity(teeth * 4);
    let step = TAU / teeth as f32;
    for k in 0..teeth {
        let a = angle + k as f32 * step;
        // Root, flank up, top, flank down: trapezoid teeth.
        for (frac, r) in [(0.0, inner), (0.18, radius), (0.42, radius), (0.6, inner)] {
            let b = a + frac * step;
            pts.push(center + vec2(b.cos(), b.sin()) * r);
        }
    }
    pts
}

/// One gear: metal body (lighter at the top-left), rim, spokes and hub.
fn paint_gear(p: &Painter, center: Pos2, radius: f32, teeth: usize, angle: f32, alpha: f32) {
    let outline = gear_outline(center, radius, teeth, angle);
    let mut mesh = Mesh::default();
    let shade = |pos: Pos2| {
        let d = (pos - center) / radius;
        let light = 0.5 - 0.28 * (d.x + d.y) * 0.5;
        let v = (150.0 + 90.0 * light) as u8;
        Color32::from_gray(v).gamma_multiply(alpha)
    };
    mesh.colored_vertex(center, Color32::from_gray(210).gamma_multiply(alpha));
    for q in &outline {
        mesh.colored_vertex(*q, shade(*q));
    }
    let n = outline.len() as u32;
    for k in 0..n {
        mesh.add_triangle(0, 1 + k, 1 + (k + 1) % n);
    }
    p.add(Shape::mesh(mesh));
    let dark = Color32::from_gray(40).gamma_multiply(alpha);
    p.add(Shape::closed_line(outline, Stroke::new(1.2, dark)));
    // Web with five windows between the spokes.
    let web = radius * 0.66;
    p.circle_filled(center, web, Color32::from_gray(70).gamma_multiply(alpha));
    for k in 0..5 {
        let a = angle * 1.0 + k as f32 * TAU / 5.0 + TAU / 10.0;
        let c = center + vec2(a.cos(), a.sin()) * web * 0.58;
        p.circle_filled(c, web * 0.3, cores::FUNDO.gamma_multiply(alpha));
    }
    p.circle_stroke(
        center,
        web,
        Stroke::new(1.5, Color32::from_gray(200).gamma_multiply(alpha)),
    );
    // Hub.
    p.circle_filled(
        center,
        radius * 0.24,
        Color32::from_gray(225).gamma_multiply(alpha),
    );
    p.circle_stroke(center, radius * 0.24, Stroke::new(1.0, dark));
    p.circle_filled(
        center,
        radius * 0.1,
        Color32::from_gray(60).gamma_multiply(alpha),
    );
}

/// Draws the frame `s` over `screen`; `voos` are the icons flying to their grid places.
pub fn pintar(p: &Painter, screen: Rect, s: &IntroState, voos: &[Voo]) {
    if s.done {
        return;
    }
    p.rect_filled(screen, 0.0, cores::FUNDO.gamma_multiply(s.cover));
    let c = screen.center() - vec2(0.0, screen.height() * 0.09);
    let unit = (screen.height() * 0.11).clamp(34.0, 90.0) * s.gear_scale;

    // A faint grid of hairlines drifting slowly (depth), and a few rising sparks.
    let grid_alpha = 0.035 * s.cover;
    if grid_alpha > 0.002 {
        let line = Stroke::new(1.0, Color32::WHITE.gamma_multiply(grid_alpha));
        let drift = s.progress * 28.0;
        let step = 64.0;
        let mut x = screen.left() - step + (drift % step);
        while x < screen.right() {
            p.line_segment([pos2(x, screen.top()), pos2(x, screen.bottom())], line);
            x += step;
        }
        let mut y = screen.top() - step + ((drift * 0.5) % step);
        while y < screen.bottom() {
            p.line_segment([pos2(screen.left(), y), pos2(screen.right(), y)], line);
            y += step;
        }
        for k in 0..28u32 {
            let h = |n: u32| {
                ((k.wrapping_mul(2_654_435_761).wrapping_add(n)) % 10_000) as f32 / 10_000.0
            };
            let x = screen.left() + h(1) * screen.width();
            let speed = 0.25 + h(2) * 0.6;
            let y = screen.bottom() - ((h(3) + s.progress * speed) % 1.0) * screen.height();
            let a = (0.25 + 0.5 * h(4)) * s.cover * s.gear_alpha.max(s.mark_alpha);
            p.circle_filled(
                pos2(x, y),
                0.8 + h(5) * 1.2,
                Color32::from_rgb(120, 190, 255).gamma_multiply(a),
            );
        }
    }

    // Three gears in mesh (the launcher's icon): big one left, two smaller to the right.
    if s.gear_alpha > 0.0 {
        let big = unit;
        let mid = unit * 0.72;
        let small = unit * 0.62;
        let big_c = c + vec2(-unit * 0.42, unit * 0.12);
        let mid_c = big_c + Vec2::angled(-0.55) * (big * 0.93 + mid * 0.93);
        let small_c = big_c + Vec2::angled(0.62) * (big * 0.93 + small * 0.93);
        let a = s.gear_angle;
        paint_gear(
            p,
            small_c,
            small,
            10,
            -a * 16.0 / 10.0 + 0.3,
            s.gear_alpha * 0.9,
        );
        paint_gear(
            p,
            mid_c,
            mid,
            12,
            -a * 16.0 / 12.0 + 0.12,
            s.gear_alpha * 0.95,
        );
        paint_gear(p, big_c, big, 16, a, s.gear_alpha);
    }

    // Wordmark, letter by letter, rising into place.
    let mark_y = c.y + unit * 1.95;
    let size = (unit * 0.62).clamp(26.0, 56.0);
    let spacing = size * 0.38;
    let font = familia::negrito(size);
    let widths: Vec<f32> = MARCA
        .chars()
        .map(|ch| {
            p.layout_no_wrap(ch.to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + spacing * (widths.len() as f32 - 1.0);
    let mut x = c.x - total / 2.0;
    let mut mark_rect = Rect::NOTHING;
    for (k, ch) in MARCA.chars().enumerate() {
        let shown = clamp01(s.letters - k as f32);
        if shown > 0.0 && s.mark_alpha > 0.0 {
            let rise = (1.0 - ease_out_cubic(shown)) * size * 0.35;
            let g = p.layout_no_wrap(
                ch.to_string(),
                font.clone(),
                Color32::WHITE.gamma_multiply(ease_out_cubic(shown) * s.mark_alpha),
            );
            let pos = pos2(x, mark_y - g.size().y / 2.0 + rise);
            mark_rect = mark_rect.union(Rect::from_min_size(pos, g.size()));
            p.galley(pos, g, Color32::WHITE);
        }
        x += widths[k] + spacing;
    }

    // Light sweep: a soft diagonal band crossing the logo and the wordmark.
    if let Some(sw) = s.sweep {
        let area = Rect::from_center_size(
            pos2(c.x, (c.y + mark_y) / 2.0),
            vec2(total.max(unit * 3.2) + unit, mark_y - c.y + unit * 2.6),
        );
        let x = area.left() - unit + sw * (area.width() + unit * 2.0);
        let half = unit * 0.35;
        let slant = area.height() * 0.35;
        let band = |dx: f32, alpha: f32| {
            let mut m = Mesh::default();
            let col = Color32::WHITE.gamma_multiply(alpha * s.mark_alpha.max(s.gear_alpha));
            let clear = Color32::TRANSPARENT;
            let (top, bottom) = (area.top(), area.bottom());
            m.colored_vertex(pos2(x - dx + slant, top), clear);
            m.colored_vertex(pos2(x + slant, top), col);
            m.colored_vertex(pos2(x + dx + slant, top), clear);
            m.colored_vertex(pos2(x - dx - slant, bottom), clear);
            m.colored_vertex(pos2(x - slant, bottom), col);
            m.colored_vertex(pos2(x + dx - slant, bottom), clear);
            m.add_triangle(0, 1, 4);
            m.add_triangle(0, 4, 3);
            m.add_triangle(1, 2, 5);
            m.add_triangle(1, 5, 4);
            m
        };
        p.add(Shape::mesh(band(half, 0.10)));
        p.add(Shape::mesh(band(half * 0.25, 0.22)));
    }

    // Family line.
    if s.tagline_alpha > 0.0 {
        let job = familia::espacado(
            tx("abertura-familia_de_apps"),
            familia::mono((size * 0.3).clamp(11.0, 15.0)),
            cores::TEXTO_FRACO.gamma_multiply(s.tagline_alpha),
            4.0,
        );
        let g = p.layout_job(job);
        let y = if mark_rect.is_positive() {
            mark_rect.bottom()
        } else {
            mark_y + size * 0.5
        };
        p.galley(
            pos2(c.x - g.size().x / 2.0, y + size * 0.45),
            g,
            Color32::WHITE,
        );
    }

    // Icons: gathered in a row under the wordmark, then flying to their grid places.
    if s.icons_alpha > 0.0 && !voos.is_empty() {
        let side = 48.0;
        let gap = 18.0;
        let row_w = voos.len() as f32 * side + (voos.len() as f32 - 1.0) * gap;
        let start_y = mark_y + size * 1.9;
        for (k, v) in voos.iter().enumerate() {
            let from = Rect::from_min_size(
                pos2(c.x - row_w / 2.0 + k as f32 * (side + gap), start_y),
                Vec2::splat(side),
            );
            // A small stagger so they do not move as one block.
            let t = clamp01(s.icons * (1.0 + 0.12 * k as f32) - 0.12 * k as f32);
            let r = lerp_rect(from, v.target, ease_out_cubic(t));
            let alpha = s.icons_alpha;
            match v.texture {
                Some(id) => {
                    p.image(
                        id,
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE.gamma_multiply(alpha),
                    );
                }
                None => {
                    familia::pintar_monograma(p, r, &v.initials, v.accent.gamma_multiply(alpha))
                }
            }
        }
    }
}

fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::from_min_max(a.min.lerp(b.min, t), a.max.lerp(b.max, t))
}

/// Whether the system asks for less motion. Windows: the "animate windows" setting
/// (`MinAnimate` = 0) that the accessibility switch "Animation effects" also turns off;
/// anywhere: `UTSIM_SEM_ANIMACAO=1`. Slow (runs `reg`): call it off the UI thread.
pub fn system_prefers_less_motion() -> bool {
    if std::env::var("UTSIM_SEM_ANIMACAO").is_ok_and(|v| v == "1") {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Control Panel\Desktop\WindowMetrics",
                "/v",
                "MinAnimate",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Ok(o) = out {
            let text = String::from_utf8_lossy(&o.stdout);
            return text
                .lines()
                .find(|l| l.contains("MinAnimate"))
                .and_then(|l| l.split_whitespace().last())
                .is_some_and(|v| v == "0");
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_moves_forward_and_ends_exactly_at_the_end() {
        let mut last = intro_frame(0.0);
        assert!(!last.done);
        assert_eq!(last.cover, 1.0);
        assert_eq!(last.ui_alpha, 0.0);
        assert_eq!(last.letters, 0.0);
        let steps = (DURACAO * 60.0) as usize;
        for k in 1..=steps + 30 {
            let s = intro_frame(k as f32 / 60.0);
            assert!(s.progress >= last.progress, "progresso em {k}");
            assert!(s.letters >= last.letters);
            assert!(s.icons >= last.icons);
            assert!(s.ui_alpha >= last.ui_alpha);
            assert!(s.cover <= last.cover);
            for v in [
                s.cover,
                s.ui_alpha,
                s.gear_alpha,
                s.mark_alpha,
                s.tagline_alpha,
                s.icons_alpha,
            ] {
                assert!((0.0..=1.0).contains(&v), "{s:?}");
            }
            last = s;
        }
        assert!(intro_frame(DURACAO - 0.001).progress < 1.0);
        let end = intro_frame(DURACAO);
        assert!(end.done && end.progress == 1.0 && end.cover == 0.0 && end.ui_alpha == 1.0);
        assert_eq!(end, intro_frame(DURACAO + 10.0));
        assert_eq!(intro_frame(-1.0), intro_frame(0.0));
        // Every letter is shown before the fade out, the sweep passes in the middle.
        assert_eq!(intro_frame(1.5).letters, MARCA.len() as f32);
        assert!(intro_frame(1.4).sweep.is_some());
        assert!(intro_frame(0.5).sweep.is_none());
    }

    #[test]
    fn skipping_jumps_to_the_end() {
        let mut intro = Intro::new(10.0);
        assert!(!intro.frame(10.5).done);
        intro.skip();
        assert_eq!(intro.frame(10.5), intro_frame(DURACAO));
        assert!(intro.frame(10.0).done);
    }

    #[test]
    fn easing_ends() {
        for f in [ease_out_cubic, ease_out_expo, ease_in_out_cubic] {
            assert_eq!(f(0.0), 0.0);
            assert_eq!(f(1.0), 1.0);
            assert!(f(0.5) > 0.0 && f(0.5) < 1.0);
        }
    }
}
