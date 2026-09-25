//! Demo CLI for `rdi-core`.
//!
//! Prints tabulated curve values so you can eyeball the shape of every
//! built-in easing plus a couple of procedural curves.

use rdi_core::{procedural, AnimationCurve, Curve, KeyframeInterp, MotionContext, Point};

fn table(name: &str, curve: &dyn AnimationCurve) {
    println!("\n{name}");
    println!("  t  |  v");
    println!("-----+-------");
    for i in 0..=10 {
        let t = i as f32 / 10.0;
        println!(" {t:.1} | {:+.4}", curve.eval(t));
    }
}

fn main() {
    println!("rusty-desktop-icons — rdi-core demo");

    table("Linear", &Curve::linear());
    table("EaseInOut (quadratic)", &Curve::ease_in_out());
    table("CubicInOut", &Curve::cubic_in_out());
    table("SineInOut", &Curve::sine_in_out());

    // CSS-style parametric Bézier: material design "standard" curve.
    let bezier = Curve::cubic_bezier(0.4, 0.0, 0.2, 1.0).expect("valid bezier");
    table("CubicBezier(0.4, 0.0, 0.2, 1.0)", &bezier);

    // Explicit keyframe curve.
    let keys = Curve::keyframes(
        vec![
            rdi_core::Keyframe::new(0.0, 0.0),
            rdi_core::Keyframe::new(0.3, 0.9),
            rdi_core::Keyframe::new(0.7, 0.4),
            rdi_core::Keyframe::new(1.0, 1.0),
        ],
        KeyframeInterp::SmoothStep,
    )
    .expect("valid keyframe curve");
    table("Keyframe(smoothstep)", &keys);

    // Function-generated curve — sampled once at build time.
    let sampled = Curve::from_curve_fn(
        |t| 1.0 - (1.0 - t).powi(3), // cubic-out via a closure
        32,
        KeyframeInterp::Linear,
    )
    .expect("valid sampled curve");
    table("from_curve_fn(cubic-out, 32 samples)", &sampled);

    // Motion-aware curve: parabolic drop scaled by user param 'g'.
    let ctx = MotionContext::new(
        Point::new(0, 0),
        Point::new(0, 400),
        std::time::Duration::from_secs(1),
    )
    .with_param("g", 9.81);
    let gravity = Curve::from_motion_fn(
        |ctx, t| {
            let g = ctx.param("g").unwrap_or(1.0) as f32;
            (t + 0.5 * g * t * t) / (1.0 + 0.5 * g)
        },
        &ctx,
        64,
        KeyframeInterp::Linear,
    )
    .expect("valid motion curve");
    table(
        &format!("from_motion_fn(gravity, g={:.2}, 64 samples)", ctx.param("g").unwrap()),
        &gravity,
    );

    // Procedural helpers.
    table("procedural::spring(0.5, 180.0, 1.0)", &procedural::spring(0.5, 180.0, 1.0).unwrap());
    table("procedural::bounce(3, 0.5)", &procedural::bounce(3, 0.5).unwrap());
    table("procedural::overshoot(1.7)", &procedural::overshoot(1.7).unwrap());
}
