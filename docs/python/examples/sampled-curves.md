# Bake and Inspect a Motion Curve

This complete example does not access the desktop. It compares a smooth sampled
trajectory with a hold/travel/settle curve and constructs independent X/Y motion.
See [curves and durations](../../concepts/curves.md) for their shared contract.

```python
import rusty_desktop_icons as rdi

def minimum_jerk(time: float) -> float:
    return time**3 * (10.0 - 15.0 * time + 6.0 * time**2)

smooth = rdi.Curve.from_function(minimum_jerk, samples=128)
held = rdi.Curve.keyframes(
    [(0.0, 0.0), (0.15, 0.0), (0.85, 1.0), (1.0, 1.0)],
    interp=rdi.KeyframeInterp.SmoothStep,
)
for progress in (0.0, 0.15, 0.5, 0.85, 1.0):
    print(progress, smooth.eval(progress), held.eval(progress))

def make_spec(identity: str, target: tuple[int, int]) -> rdi.IconAnimationSpec:
    return rdi.IconAnimationSpec(
        identity, target, rdi.Duration.distance_clamped(500, 0.3, 1.5),
        smooth, curve_y=held,
    )
```

Use a real ID from `controller.list_icons()` when calling `make_spec` and pass
the result to the [batch recipe](../guides/basic-usage.md). No sampler runs on
the animation worker; all Python function evaluation happens during construction.
For motion-dependent sampling, `Curve.from_motion_function` supplies origin,
target, duration and your parameter dictionary to a `MotionContext`.