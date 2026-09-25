# Curves and Durations

A curve describes movement progress, not elapsed time or a destination. Combine
it with an origin, target and duration in an [animation specification](animation.md).
For each axis the engine evaluates:

```text
time = clamp(elapsed_seconds / resolved_duration_seconds, 0, 1)
position = origin + curve(time) * (target - origin)
```

The clock uses elapsed time, not accumulated frame deltas. A slow frame skips
ahead rather than extending the requested duration. Progress zero/one yields
the exact origin/target at the animation boundaries; intermediate positions
are rounded to integer pixels.

## Curve

`Curve` is reusable data. Rust's `AnimationCurve` trait exposes evaluation;
`BuiltinEasing`, `KeyframeCurve` and `Keyframe` are its building blocks. Python
uses factory methods on `Curve` and tuple/dictionary keyframes.

| Construction | Suitable use |
| --- | --- |
| Linear | Constant progress, diagnostics, shader-owned transport |
| Ease, quadratic, cubic, sine | Gradual acceleration/deceleration |
| Cubic Bezier | A deliberately tuned response shape |
| Spring, bounce, elastic, overshoot | Bounded expressive movement |
| Keyframes | Holds, pauses, staged arrivals and overshoot |
| Sampled function | Reuse mathematical or externally generated shapes |
| Motion-aware sampled function | Adapt a shape to distance/duration/parameters |

Curve output may overshoot beyond `[0, 1]`; allow space beyond the target.
A plateau holds an intermediate position but consumes time inside the existing
duration. It does not create a second animation phase or pause shader time.

## Keyframes and Interpolation

A keyframe pairs normalized time with a value. Keys must cover valid increasing
times from zero to one and contain finite, valid values. `KeyframeInterp`
selects linear interpolation, step holds, or smoothstep between keys.
Smoothstep eases each segment independently; it is not a global spline.

Use more samples when a function has narrow peaks or rapid oscillations.
Sampling is an approximation: increasing the sample count improves fidelity
but increases the curve data. `Curve.from_function` / `Curve::from_curve_fn`
sample your function **during construction**, never in the animation tick.
The function is invoked for each sample, not just once in total.

`MotionContext` adds origin, target, distance, resolved duration and named
parameters for `from_motion_function` / `from_motion_fn`. It is a build-time
snapshot. Rebuild the curve when those inputs change; it does not react to a
moving window or live desktop query during playback.

## Duration

`Duration` is a policy resolved from the planned motion:

| Policy | Choice |
| --- | --- |
| Fixed | Equal time for all selected icons |
| Distance | Equal nominal speed in pixels per second |
| Distance clamped | Distance-based travel with minimum/maximum time |

Grid resolution happens before distance-based durations are resolved. With a
shader on a stationary icon, choose a nonzero fixed duration; distance alone
can resolve to zero. Curves control the speed *profile*, so a distance-based
policy does not imply constant instantaneous velocity.

X and Y can use different curves under the same clock. An effect's strength
envelope is a separate curve and does not replace either axis curve.

## Put It to Work

- [Rust curve recipes](../rust/guides/curves.md)
- [Python sampled-curve example](../python/examples/sampled-curves.md)
- [Animation options](animation.md) and [shader envelopes](shaders/index.md)