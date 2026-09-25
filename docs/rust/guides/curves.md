# Author Motion Curves

These examples evaluate curves without opening a controller or modifying the
desktop. Definitions, timing rules and validation limits live under
[curves and durations](../../concepts/curves.md).

## Hold, Travel and Settle

```rust
use rdi_core::{AnimationCurve, Curve, Keyframe, KeyframeInterp};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let curve = Curve::keyframes(vec![
        Keyframe::new(0.0, 0.0),
        Keyframe::new(0.15, 0.0),
        Keyframe::new(0.85, 1.0),
        Keyframe::new(1.0, 1.0),
    ], KeyframeInterp::SmoothStep)?;
    for progress in [0.0, 0.15, 0.5, 0.85, 1.0] {
        println!("{progress:.2}: {:.3}", curve.eval(progress));
    }
    Ok(())
}
```

This is useful when labels should remain readable before and after travel.
Attach the curve with `IconAnimationSpec::new`; use `with_axes` for different
X/Y timing. A movement curve does not automatically become an effect envelope.

## Bake a Custom Function

```rust
use rdi_core::{AnimationCurve, Curve, KeyframeInterp};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let minimum_jerk = Curve::from_curve_fn(
        |time| time.powi(3) * (10.0 - 15.0 * time + 6.0 * time * time),
        128, KeyframeInterp::Linear,
    )?;
    assert!((minimum_jerk.eval(0.5) - 0.5).abs() < 0.001);
    Ok(())
}
```

For per-icon curves derived from distance or direction, use `from_motion_fn`
with a `MotionContext`. For consistent travel speed across unequal distances,
choose `Duration::Distance` or `Duration::DistanceClamped` rather than changing
the sampler. The [native reference](../index.md) lists their exact fields.

Next: [run a batch](animation-lifecycle.md) or use a curve as an
[effect envelope](shaders.md).