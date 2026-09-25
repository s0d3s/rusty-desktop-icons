use std::collections::{HashMap, HashSet};

use crate::{DesktopError, IconAnimationSpec, IconSnapshot, MonitorInfo, Point, Rect};

/// A read-only snapshot in physical virtual-screen pixels. Gaps between monitors
/// are included in `bounds`, but are never usable grid cells.
#[derive(Clone, Debug, PartialEq)]
pub struct DesktopInfo {
    pub bounds: Rect,
    pub monitors: Vec<MonitorInfo>,
    pub grids: Vec<IconGrid>,
}

/// One monitor's icon grid. `origin` is the first usable icon-position anchor,
/// not the bitmap or label corner. Cell size includes icon/label spacing.
/// An unavailable origin has zero rows/columns and cannot be used for snapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconGrid {
    pub monitor_id: String,
    pub work_area: Rect,
    pub icon_size: Point,
    pub cell_size: Point,
    pub origin: Option<Point>,
    pub origin_inferred: bool,
    pub columns: u32,
    pub rows: u32,
}

impl IconGrid {
    fn footprint(&self) -> Point {
        Point::new(self.cell_size.x.max(self.icon_size.x), self.cell_size.y.max(self.icon_size.y))
    }

    pub fn new(
        monitor_id: String,
        work_area: Rect,
        icon_size: Point,
        cell_size: Point,
        origin: Option<Point>,
        origin_inferred: bool,
    ) -> Result<Self, DesktopError> {
        if cell_size.x <= 0 || cell_size.y <= 0 || icon_size.x <= 0 || icon_size.y <= 0
            || work_area.right <= work_area.left || work_area.bottom <= work_area.top
        {
            return Err(DesktopError::InvalidGrid("non-positive desktop geometry".into()));
        }
        let (columns, rows) = if let Some(origin) = origin {
            if !work_area.contains(origin) {
                return Err(DesktopError::InvalidGrid("grid origin outside work area".into()));
            }
            (
                ((i64::from(work_area.right) - i64::from(origin.x)) / i64::from(cell_size.x)) as u32,
                ((i64::from(work_area.bottom) - i64::from(origin.y)) / i64::from(cell_size.y)) as u32,
            )
        } else {
            (0, 0)
        };
        Ok(Self { monitor_id, work_area, icon_size, cell_size, origin, origin_inferred, columns, rows })
    }

    /// Infer the unique most common two-axis grid phase. Empty or tied samples
    /// return None; never substitute the work-area corner for an unknown phase.
    pub fn infer_origin(work: Rect, spacing: Point, positions: &[Point]) -> Option<Point> {
        if spacing.x <= 0 || spacing.y <= 0 { return None; }
        let mut counts = HashMap::new();
        for position in positions.iter().filter(|position| work.contains(**position)) {
            let phase = (
                (i64::from(position.x) - i64::from(work.left)).rem_euclid(i64::from(spacing.x)),
                (i64::from(position.y) - i64::from(work.top)).rem_euclid(i64::from(spacing.y)),
            );
            *counts.entry(phase).or_insert(0usize) += 1;
        }
        let maximum = counts.values().copied().max()?;
        let mut winners = counts.into_iter().filter(|(_, count)| *count == maximum);
        let ((horizontal, vertical), _) = winners.next()?;
        if winners.next().is_some() { return None; }
        Some(Point::new(
            i32::try_from(i64::from(work.left) + horizontal).ok()?,
            i32::try_from(i64::from(work.top) + vertical).ok()?,
        ))
    }
}

fn overlaps(left: Point, left_size: Point, right: Point, right_size: Point) -> bool {
    (i64::from(left.x) - i64::from(right.x)).abs() < i64::from(left_size.x.max(right_size.x))
        && (i64::from(left.y) - i64::from(right.y)).abs() < i64::from(left_size.y.max(right_size.y))
}

fn distance_to_rect(point: Point, bounds: Rect) -> i128 {
    let horizontal = i128::from(point.x) - i128::from(point.x.clamp(bounds.left, bounds.right - 1));
    let vertical = i128::from(point.y) - i128::from(point.y.clamp(bounds.top, bounds.bottom - 1));
    horizontal * horizontal + vertical * vertical
}

/// Resolve all destinations atomically in spec order, using nearest available
/// cells on the target's monitor (nearest work area for off-screen targets).
/// Stationary footprints remain reserved; active source cells can be reused.
pub fn resolve_grid_targets(
    specs: &mut [IconAnimationSpec],
    icons: &[IconSnapshot],
    desktop: &DesktopInfo,
) -> Result<(), DesktopError> {
    let mut ids = HashSet::new();
    for spec in specs.iter() {
        if !ids.insert(&spec.id) {
            return Err(DesktopError::InvalidGrid(format!("duplicate animation icon {}", spec.id)));
        }
    }
    let existing: HashSet<_> = icons.iter().map(|icon| &icon.id).collect();
    if specs.iter().all(|spec| !existing.contains(&spec.id)) { return Ok(()); }
    if desktop.grids.is_empty() {
        return Err(DesktopError::InvalidGrid("desktop has no icon grids".into()));
    }
    for grid in &desktop.grids {
        let checked = IconGrid::new(grid.monitor_id.clone(), grid.work_area, grid.icon_size,
            grid.cell_size, grid.origin, grid.origin_inferred)?;
        if checked.columns != grid.columns || checked.rows != grid.rows
            || u64::from(grid.columns) * u64::from(grid.rows) > 1_000_000
        {
            return Err(DesktopError::InvalidGrid("invalid or excessive grid capacity".into()));
        }
    }
    let mut occupied: Vec<_> = icons.iter().filter(|icon| !ids.contains(&icon.id))
        .map(|icon| {
            let size = desktop.grids.iter()
                .min_by_key(|grid| distance_to_rect(icon.position, grid.work_area))
                .unwrap().footprint();
            (icon.position, size)
        }).collect();
    let mut targets = Vec::with_capacity(specs.len());
    for spec in specs.iter() {
        if !existing.contains(&spec.id) {
            targets.push(spec.target);
            continue;
        }
        let grid = desktop.grids.iter()
            .min_by_key(|grid| distance_to_rect(spec.target, grid.work_area)).unwrap();
        let origin = grid.origin.ok_or_else(|| DesktopError::InvalidGrid(
            format!("grid origin unavailable for {}", grid.monitor_id)))?;
        let footprint = grid.footprint();
        let mut best: Option<(i128, Point)> = None;
        for column in 0..grid.columns {
            for row in 0..grid.rows {
                let candidate = Point::new(
                    (i64::from(origin.x) + i64::from(column) * i64::from(grid.cell_size.x)) as i32,
                    (i64::from(origin.y) + i64::from(row) * i64::from(grid.cell_size.y)) as i32,
                );
                if i64::from(candidate.x) + i64::from(footprint.x) > i64::from(grid.work_area.right)
                    || i64::from(candidate.y) + i64::from(footprint.y) > i64::from(grid.work_area.bottom)
                    || occupied.iter().any(|(point, size)| overlaps(candidate, footprint, *point, *size)) {
                    continue;
                }
                let horizontal = i128::from(candidate.x) - i128::from(spec.target.x);
                let vertical = i128::from(candidate.y) - i128::from(spec.target.y);
                let distance = horizontal * horizontal + vertical * vertical;
                if best.is_none_or(|(previous, _)| distance < previous) {
                    best = Some((distance, candidate));
                }
            }
        }
        let target = best.ok_or_else(|| DesktopError::InvalidGrid(
            format!("no free grid cell on {} for {}", grid.monitor_id, spec.id)))?.1;
        occupied.push((target, footprint));
        targets.push(target);
    }
    for (spec, target) in specs.iter_mut().zip(targets) { spec.target = target; }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Curve, Duration};

    fn icon(id: &str, position: Point) -> IconSnapshot {
        IconSnapshot::new(id.into(), id, None, false, position)
    }

    fn spec(id: &str, target: Point) -> IconAnimationSpec {
        IconAnimationSpec::new(id.into(), target, Duration::fixed(std::time::Duration::ZERO), Curve::linear())
    }

    fn desktop() -> DesktopInfo {
        let bounds = Rect::new(-300, -200, 0, 0);
        DesktopInfo { bounds, monitors: vec![], grids: vec![IconGrid::new(
            "left".into(), bounds, Point::new(40, 40), Point::new(100, 100),
            Some(Point::new(-300, -200)), false).unwrap()] }
    }

    #[test]
    fn grid_snaps_conflicts_and_protects_off_grid_stationary_icons() {
        let desktop = desktop();
        let icons = vec![icon("first", Point::new(-300, -200)),
            icon("second", Point::new(-300, -100)), icon("fixed", Point::new(-150, -150))];
        let mut specs = vec![spec("first", Point::new(-120, -110)), spec("second", Point::new(-120, -110))];
        resolve_grid_targets(&mut specs, &icons, &desktop).unwrap();
        assert_eq!(specs[0].target, Point::new(-300, -100));
        assert_eq!(specs[1].target, Point::new(-300, -200));
    }

    #[test]
    fn grid_failure_is_atomic_and_missing_icons_do_not_consume_cells() {
        let mut desktop = desktop();
        desktop.grids[0] = IconGrid::new("tiny".into(), Rect::new(0, 0, 100, 100),
            Point::new(40, 40), Point::new(100, 100), Some(Point::new(0, 0)), false).unwrap();
        let icons = vec![icon("first", Point::new(0, 0)), icon("second", Point::new(10, 10))];
        let mut specs = vec![spec("first", Point::new(12, 12)), spec("second", Point::new(22, 22))];
        assert!(resolve_grid_targets(&mut specs, &icons, &desktop).is_err());
        assert_eq!(specs[0].target, Point::new(12, 12));
        let mut specs = vec![spec("missing", Point::new(12, 12)), spec("first", Point::new(22, 22))];
        resolve_grid_targets(&mut specs, &icons[..1], &desktop).unwrap();
        assert_eq!(specs[1].target, Point::new(0, 0));
    }

    #[test]
    fn grid_allows_swaps_and_clamps_extreme_targets() {
        let icons = vec![icon("first", Point::new(-300, -200)), icon("second", Point::new(-100, -100))];
        let mut specs = vec![spec("first", icons[1].position), spec("second", icons[0].position)];
        resolve_grid_targets(&mut specs, &icons, &desktop()).unwrap();
        assert_eq!(specs[0].target, icons[1].position);
        assert_eq!(specs[1].target, icons[0].position);
        specs[0].target = Point::new(i32::MAX, i32::MIN);
        resolve_grid_targets(&mut specs, &icons, &desktop()).unwrap();
        assert_eq!(specs[0].target, Point::new(-100, -200));
    }

    #[test]
    fn grid_infers_phase_without_requiring_first_row_or_column() {
        let work = Rect::new(-400, -300, 0, 0);
        let spacing = Point::new(100, 100);
        let points = [Point::new(-180, -90), Point::new(-80, -190), Point::new(-65, -70)];
        assert_eq!(IconGrid::infer_origin(work, spacing, &points), Some(Point::new(-380, -290)));
        assert_eq!(IconGrid::infer_origin(work, spacing, &points[1..]), None);
        assert_eq!(IconGrid::infer_origin(work, spacing, &[]), None);
    }

    #[test]
    fn grid_uses_target_monitor_spacing_and_reserves_large_icon_slots() {
        let mut desktop = desktop();
        desktop.grids.push(IconGrid::new("right".into(), Rect::new(0, 0, 600, 600),
            Point::new(240, 240), Point::new(200, 200), Some(Point::new(0, 0)), false).unwrap());
        let icons = vec![icon("first", Point::new(-300, -200)), icon("second", Point::new(-300, -100))];
        let mut specs = vec![spec("first", Point::new(1, 1)), spec("second", Point::new(1, 1))];
        assert!(resolve_grid_targets(&mut specs, &icons, &desktop).is_err());
        desktop.grids[1] = IconGrid::new("right".into(), Rect::new(0, 0, 800, 800),
            Point::new(240, 240), Point::new(200, 200), Some(Point::new(0, 0)), false).unwrap();
        resolve_grid_targets(&mut specs, &icons, &desktop).unwrap();
        assert_eq!(specs[0].target, Point::new(0, 0));
        assert_eq!(specs[1].target, Point::new(0, 400));
    }

    #[test]
    fn grid_rejects_duplicates_unknown_origins_and_invalid_sizes() {
        let icons = vec![icon("first", Point::new(-300, -200))];
        let mut specs = vec![spec("first", icons[0].position), spec("first", icons[0].position)];
        assert!(resolve_grid_targets(&mut specs, &icons, &desktop()).is_err());
        specs.pop();
        let mut desktop = desktop();
        desktop.grids[0].origin = None;
        desktop.grids[0].columns = 0;
        desktop.grids[0].rows = 0;
        assert!(resolve_grid_targets(&mut specs, &icons, &desktop).is_err());
        desktop.grids[0].cell_size.x = 0;
        assert!(resolve_grid_targets(&mut specs, &icons, &desktop).is_err());
    }
}