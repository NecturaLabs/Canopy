//! Animation helpers for the kit: eased booleans and values, springs, entrances and a sliding
//! rectangle. Durations, curves and spring constants come from `ui::motion`. With reduced motion
//! every helper returns its end state at once. Nothing here repaints unless a value is moving.

use egui::{Context, Id, Rect, pos2};

use crate::ui::motion::{self, Spring};

/// Eases towards `on` over `in_s` when turning on and `out_s` when turning off, along
/// `motion::EASE_OUT`; 0 to 1. Zero durations (and reduced motion) switch at once.
pub fn ease_bool(ctx: &Context, id: Id, on: bool, in_s: f32, out_s: f32) -> f32 {
    let seconds = motion::time(if on { in_s } else { out_s });
    ctx.animate_bool_with_time_and_easing(id, on, seconds, |t| motion::EASE_OUT.at(t))
}

/// [`ease_bool`] with one duration both ways.
#[expect(dead_code, reason = "used by the component phase")]
pub fn ease(ctx: &Context, id: Id, on: bool, seconds: f32) -> f32 {
    ease_bool(ctx, id, on, seconds, seconds)
}

/// Eases a number towards `target` over `seconds` (instant with reduced motion).
#[expect(dead_code, reason = "used by the component phase")]
pub fn ease_value(ctx: &Context, id: Id, target: f32, seconds: f32) -> f32 {
    ctx.animate_value_with_time(id, target, motion::time(seconds))
}

#[derive(Debug, Clone, Copy)]
struct SpringState {
    x: f32,
    v: f32,
    at: f64,
}

/// One exact step of a damped spring (any damping ratio) from displacement `d = x - target` and
/// velocity `v` over `dt` seconds. Returns the new displacement and velocity.
pub fn spring_step(d: f32, v: f32, s: Spring, dt: f32) -> (f32, f32) {
    let (w, z) = (s.omega, s.zeta);
    if dt <= 0.0 {
        return (d, v);
    }
    if (z - 1.0).abs() < 1e-4 {
        let e = (-w * dt).exp();
        let c = v + w * d;
        let x = (d + c * dt) * e;
        let vel = (c - w * (d + c * dt)) * e;
        (x, vel)
    } else if z < 1.0 {
        let wd = w * (1.0 - z * z).sqrt();
        let e = (-z * w * dt).exp();
        let (sin, cos) = (wd * dt).sin_cos();
        let b = (v + z * w * d) / wd;
        let x = e * (d * cos + b * sin);
        let vel = e * (-z * w * (d * cos + b * sin) + (-d * wd * sin + b * wd * cos));
        (x, vel)
    } else {
        let r = (z * z - 1.0).sqrt();
        let r1 = -w * (z - r);
        let r2 = -w * (z + r);
        let c2 = (v - r1 * d) / (r2 - r1);
        let c1 = d - c2;
        let (e1, e2) = ((r1 * dt).exp(), (r2 * dt).exp());
        (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
    }
}

/// A spring towards `target`, its state kept per `id`. The first call returns `target`; reduced
/// motion always does. Frame-rate independent (the exact solution over the real elapsed time),
/// repaints only while moving and snaps once within 0.01 and slower than 0.1 per second.
pub fn spring(ctx: &Context, id: Id, target: f32, s: Spring) -> f32 {
    if motion::reduced() {
        ctx.data_mut(|d| d.remove::<SpringState>(id));
        return target;
    }
    let now = ctx.input(|i| i.time);
    let mut moving = false;
    let x = ctx.data_mut(|d| {
        let st = d.get_temp_mut_or_insert_with(id, || SpringState {
            x: target,
            v: 0.0,
            at: now,
        });
        let dt = ((now - st.at) as f32).clamp(0.0, 1.0 / 20.0);
        let (dx, v) = spring_step(st.x - target, st.v, s, dt);
        moving = dx.abs() > 0.01 || v.abs() > 0.1;
        *st = if moving {
            SpringState {
                x: target + dx,
                v,
                at: now,
            }
        } else {
            SpringState {
                x: target,
                v: 0.0,
                at: now,
            }
        };
        st.x
    });
    if moving {
        ctx.request_repaint();
    }
    x
}

/// Progress of an entrance from the first frame `open` was true, eased along `EASE_OUT`; 0 while
/// closed, 1 at once with reduced motion.
#[cfg_attr(not(test), expect(dead_code, reason = "used by the component phase"))]
pub fn entrance(ctx: &Context, id: Id, open: bool, seconds: f32) -> f32 {
    let key = id.with("kit-opened-at");
    if !open {
        ctx.data_mut(|d| d.remove::<f64>(key));
        return 0.0;
    }
    let seconds = motion::time(seconds);
    if seconds <= 0.0 {
        return 1.0;
    }
    let now = ctx.input(|i| i.time);
    let opened = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || now));
    let t = ((now - opened) as f32 / seconds).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    motion::EASE_OUT.at(t)
}

/// Progress of something that enters and leaves: rises after `open` turns true, falls after it
/// turns false, and is `None` once fully gone (stop showing it).
#[expect(dead_code, reason = "used by the component phase")]
pub fn phase(ctx: &Context, id: Id, open: bool, enter_s: f32, exit_s: f32) -> Option<f32> {
    let entering = entrance(ctx, id, open, enter_s);
    let presence = ease_bool(ctx, id.with("kit-presence"), open, 0.0, exit_s);
    if open {
        Some(entering)
    } else if presence > 0.0 {
        Some(presence)
    } else {
        None
    }
}

/// A rectangle that slides to `target`: the edge moving towards the target leads
/// (`INDICATOR_LEAD`), the other trails (`INDICATOR_TRAIL`), so it stretches then settles. Paint it
/// with pixel rounding off.
#[expect(dead_code, reason = "used by the component phase")]
pub fn slide_rect(ctx: &Context, id: Id, target: Rect) -> Rect {
    let last: Option<Rect> = ctx.data(|d| d.get_temp(id.with("kit-slide-target")));
    ctx.data_mut(|d| d.insert_temp(id.with("kit-slide-target"), target));
    let moving_right = last.is_none_or(|l| target.center().x >= l.center().x);
    let (left_s, right_s) = if moving_right {
        (motion::INDICATOR_TRAIL, motion::INDICATOR_LEAD)
    } else {
        (motion::INDICATOR_LEAD, motion::INDICATOR_TRAIL)
    };
    let left = spring(ctx, id.with("kit-slide-l"), target.left(), left_s);
    let right = spring(ctx, id.with("kit-slide-r"), target.right(), right_s);
    let top = spring(
        ctx,
        id.with("kit-slide-t"),
        target.top(),
        motion::INDICATOR_LEAD,
    );
    let bottom = spring(
        ctx,
        id.with("kit-slide-b"),
        target.bottom(),
        motion::INDICATOR_LEAD,
    );
    Rect::from_min_max(pos2(left, top), pos2(right.max(left), bottom.max(top)))
}

/// The content scale of a pressed control (`scale` while held, 1 otherwise), on the `PRESS`
/// spring; always 1 with reduced motion.
pub fn press_scale(ctx: &Context, id: Id, held: bool, scale: f32) -> f32 {
    if motion::reduced() {
        return 1.0;
    }
    spring(
        ctx,
        id.with("kit-press-scale"),
        if held { scale } else { 1.0 },
        motion::PRESS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(s: Spring) -> (f32, f32) {
        // From 0 towards 1 at 120 Hz: the peak and the time to settle.
        let (mut d, mut v, mut peak, mut t) = (-1.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
        while t < 3.0 {
            (d, v) = spring_step(d, v, s, 1.0 / 120.0);
            t += 1.0 / 120.0;
            peak = peak.max(1.0 + d);
            if d.abs() < 0.01 && v.abs() < 0.1 {
                break;
            }
        }
        (peak, t)
    }

    #[test]
    fn springs_overshoot_as_specified() {
        let (peak, t) = settle(motion::SWITCH);
        assert!(peak > 1.05 && peak < 1.12, "switch peak {peak}");
        assert!(t < 0.45, "switch settles in {t}");
        let (peak, _) = settle(motion::PRESS);
        assert!(peak <= 1.0 + 1e-4, "press never overshoots: {peak}");
        let (peak, _) = settle(motion::INDICATOR_TRAIL);
        assert!(peak > 1.0 && peak < 1.03, "indicator {peak}");
    }

    #[test]
    fn spring_step_is_frame_rate_independent() {
        let s = motion::SWITCH;
        let (a, _) = spring_step(-1.0, 0.0, s, 0.1);
        let (mut d, mut v) = (-1.0, 0.0);
        for _ in 0..10 {
            (d, v) = spring_step(d, v, s, 0.01);
        }
        assert!((a - d).abs() < 1e-4, "{a} vs {d}");
        let over = Spring {
            omega: 20.0,
            zeta: 1.6,
        };
        let (a, _) = spring_step(-1.0, 0.0, over, 0.2);
        let (mut d, mut v) = (-1.0, 0.0);
        for _ in 0..20 {
            (d, v) = spring_step(d, v, over, 0.01);
        }
        assert!((a - d).abs() < 1e-4);
    }

    #[test]
    fn reduced_motion_lands_at_once() {
        let ctx = Context::default();
        // Other tests install the still appearance too, so this global never flips back.
        motion::set_reduced(true);
        let id = Id::new("t");
        assert_eq!(spring(&ctx, id, 5.0, motion::SWITCH), 5.0);
        assert_eq!(spring(&ctx, id, 9.0, motion::SWITCH), 9.0);
        assert_eq!(entrance(&ctx, id, true, 0.5), 1.0);
        assert_eq!(press_scale(&ctx, id, true, 0.97), 1.0);
    }
}
