use rand::{Rng, RngExt};
use rdi_core::{IconId, IconSnapshot, MonitorInfo, Point, Rect};
use std::error::Error;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::LVM_GETITEMSPACING;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
};
use windows::core::{BOOL, w};

pub fn grid_spacing() -> Result<Point, Box<dyn Error>> {
    unsafe extern "system" fn visit(window: HWND, context: LPARAM) -> BOOL {
        // SAFETY: EnumWindows is synchronous; context points to the live output slot.
        let result = unsafe { &mut *(context.0 as *mut Option<Point>) };
        if result.is_some() {
            return true.into();
        }
        // SAFETY: class strings are static and searches pass no cross-process pointers.
        unsafe {
            if let Ok(view) = FindWindowExW(Some(window), None, w!("SHELLDLL_DefView"), None) {
                if let Ok(list) = FindWindowExW(Some(view), None, w!("SysListView32"), None) {
                    let mut packed = 0usize;
                    if SendMessageTimeoutW(
                        list,
                        LVM_GETITEMSPACING,
                        WPARAM(0),
                        LPARAM(0),
                        SMTO_ABORTIFHUNG,
                        1000,
                        Some(&mut packed),
                    )
                    .0 != 0
                    {
                        let width = (packed & 0xffff) as i32;
                        let height = ((packed >> 16) & 0xffff) as i32;
                        if width > 0 && height > 0 {
                            *result = Some(Point::new(width, height));
                        }
                    }
                }
            }
        }
        true.into()
    }
    let mut result: Option<Point> = None;
    // SAFETY: the callback only accesses this output during synchronous enumeration.
    unsafe {
        EnumWindows(
            Some(visit),
            LPARAM((&mut result as *mut Option<Point>) as isize),
        )?;
    }
    result.ok_or_else(|| "Could not read the live desktop icon grid spacing".into())
}

pub struct Grid {
    work: Rect,
    spacing: Point,
    anchor: Point,
    occupied: Vec<Point>,
}

impl Grid {
    pub fn new(work: Rect, spacing: Point, sources: Vec<Point>) -> Self {
        let anchor = Point::new(
            sources
                .iter()
                .map(|point| point.x)
                .min()
                .unwrap_or(work.left),
            sources
                .iter()
                .map(|point| point.y)
                .min()
                .unwrap_or(work.top),
        );
        Self {
            work,
            spacing,
            anchor,
            occupied: sources,
        }
    }

    pub fn target(&self, source: Point, columns: i32, rows: i32) -> Option<Point> {
        if self.spacing.x <= 0
            || self.spacing.y <= 0
            || !(3..=5).contains(&columns)
            || !(1..=3).contains(&rows.unsigned_abs())
            || !self.work.contains(source)
        {
            return None;
        }
        let column = ((i64::from(source.x) - i64::from(self.anchor.x)) as f64
            / f64::from(self.spacing.x))
        .round() as i64;
        let row = ((i64::from(source.y) - i64::from(self.anchor.y)) as f64
            / f64::from(self.spacing.y))
        .round() as i64;
        let target = Point::new(
            i32::try_from(
                i64::from(self.anchor.x)
                    + (column + i64::from(columns)) * i64::from(self.spacing.x),
            )
            .ok()?,
            i32::try_from(
                i64::from(self.anchor.y) + (row + i64::from(rows)) * i64::from(self.spacing.y),
            )
            .ok()?,
        );
        if !self.work.contains(target)
            || i64::from(target.x) + i64::from(self.spacing.x) > i64::from(self.work.right)
            || i64::from(target.y) + i64::from(self.spacing.y) > i64::from(self.work.bottom)
            || self.occupied.iter().any(|other| {
                (i64::from(other.x) - i64::from(target.x)).abs() < i64::from(self.spacing.x)
                    && (i64::from(other.y) - i64::from(target.y)).abs() < i64::from(self.spacing.y)
            })
        {
            return None;
        }
        Some(target)
    }

    pub fn reserve(&mut self, source: Point, random: &mut impl Rng) -> Option<Point> {
        let mut candidates = Vec::new();
        for columns in 3..=5 {
            for rows in [-3, -2, -1, 1, 2, 3] {
                if let Some(target) = self.target(source, columns, rows) {
                    candidates.push(target);
                }
            }
        }
        if candidates.is_empty() {
            return None;
        }
        let target = candidates[random.random_range(0..candidates.len())];
        self.occupied.push(target);
        Some(target)
    }
}

pub struct Move {
    pub id: IconId,
    pub origin: Point,
    pub target: Point,
}

pub fn plan_moves(
    icons: &[IconSnapshot],
    monitors: &[MonitorInfo],
    spacing: Point,
) -> Vec<Result<Move, &'static str>> {
    let mut grids: Vec<_> = monitors
        .iter()
        .map(|monitor| {
            Grid::new(
                monitor.work_area,
                spacing,
                icons
                    .iter()
                    .filter(|icon| monitor.bounds.contains(icon.position))
                    .map(|icon| icon.position)
                    .collect(),
            )
        })
        .collect();
    let mut random = rand::rng();
    icons
        .iter()
        .map(|icon| {
            let monitor = monitors
                .iter()
                .position(|monitor| monitor.bounds.contains(icon.position))
                .ok_or("no containing monitor")?;
            let target = grids[monitor]
                .reserve(icon.position, &mut random)
                .ok_or("no free grid destination")?;
            Ok(Move {
                id: icon.id.clone(),
                origin: icon.position,
                target,
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::Move;
    use rdi_core::{
        Curve, CurveError, Duration, Effect, IconAnimationSpec, Keyframe, KeyframeInterp,
        ShaderPipeline, ShaderProgram,
    };
    use std::time::Duration as StdDuration;

    pub(crate) fn curves() -> Result<(Curve, Curve), CurveError> {
        Ok((
            Curve::keyframes(
                vec![
                    Keyframe::new(0.0, 0.0),
                    Keyframe::new(0.15, 0.0),
                    Keyframe::new(0.85, 1.0),
                    Keyframe::new(1.0, 1.0),
                ],
                KeyframeInterp::SmoothStep,
            )?,
            Curve::keyframes(
                vec![
                    Keyframe::new(0.0, 0.0),
                    Keyframe::new(0.15, 1.0),
                    Keyframe::new(0.85, 1.0),
                    Keyframe::new(1.0, 0.0),
                ],
                KeyframeInterp::SmoothStep,
            )?,
        ))
    }

    pub(crate) fn animation_duration(pipeline: Option<ShaderPipeline>) -> StdDuration {
        StdDuration::from_secs(
            if matches!(pipeline, Some(ShaderPipeline::Particles | ShaderPipeline::Procedural)) {
                4
            } else {
                2
            },
        )
    }

    pub(crate) fn build_specs(
        moves: &[Move],
        reverse: bool,
        program: Option<&ShaderProgram>,
        movement: &Curve,
        envelope: &Curve,
    ) -> Vec<IconAnimationSpec> {
        moves
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let mut spec = IconAnimationSpec::new(
                    entry.id.clone(),
                    if reverse { entry.origin } else { entry.target },
                    Duration::fixed(animation_duration(program.map(|program| program.pipeline))),
                    movement.clone(),
                );
                spec.effect = program.map(|program| Effect {
                    shader: program.clone(),
                    params: if program.pipeline != ShaderPipeline::Sprite {
                        program.default_params()
                    } else {
                        [12.0, 4.0, 6.0, 30.0]
                    },
                    padding_px: 24,
                    envelope: if program.pipeline == ShaderPipeline::Procedural {
                        Curve::keyframes(
                            vec![Keyframe::new(0.0, 1.0), Keyframe::new(1.0, 1.0)],
                            KeyframeInterp::Linear,
                        )
                        .expect("valid constant envelope")
                    } else {
                        envelope.clone()
                    },
                    seed: index as f32,
                });
                spec
            })
            .collect()
    }
}
