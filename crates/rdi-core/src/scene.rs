use crate::{AnimationCurve, DesktopError, IconAnimationSpec, IconFrame, IconRenderPlan, OverlayRenderOptions, Point};

#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub seconds: f64,
    pub pixels: Vec<u8>,
}

pub trait SceneRenderer {
    fn render(&mut self, frame: &[IconFrame], seconds: f64) -> Result<CapturedFrame, DesktopError>;
}

pub struct RenderSession {
    pub(crate) tx: crossbeam_channel::Sender<RenderRequest>,
    pub(crate) worker: std::thread::ThreadId,
    pub duration: f64,
}

pub(crate) enum RenderRequest {
    Frame(f64, crossbeam_channel::Sender<Result<CapturedFrame, DesktopError>>),
    Close(crossbeam_channel::Sender<()>),
}

impl RenderSession {
    pub fn render_at(&self, seconds: f64) -> Result<CapturedFrame, DesktopError> {
        if std::thread::current().id() == self.worker {
            return Err(DesktopError::BackendUnavailable("render calls cannot run on the worker".into()));
        }
        let (reply, response) = crossbeam_channel::bounded(1);
        self.tx.send(RenderRequest::Frame(seconds, reply)).map_err(|_| DesktopError::BackendUnavailable("render session closed".into()))?;
        response.recv().map_err(|_| DesktopError::WorkerCrashed("render worker stopped".into()))?
    }

    pub fn close(&self) -> Result<(), DesktopError> {
        if std::thread::current().id() == self.worker {
            return Err(DesktopError::BackendUnavailable("close cannot run on the worker".into()));
        }
        let (reply, response) = crossbeam_channel::bounded(1);
        if self.tx.send(RenderRequest::Close(reply)).is_ok() { let _ = response.recv(); }
        Ok(())
    }
}

impl Drop for RenderSession {
    fn drop(&mut self) {
        let (reply, _) = crossbeam_channel::bounded(1);
        let _ = self.tx.send(RenderRequest::Close(reply));
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub dpi_scale: f32,
    pub icon_size: u32,
}

impl Canvas {
    pub fn validate(&self) -> Result<(), DesktopError> {
        if self.width == 0 || self.height == 0 || self.width > 8192 || self.height > 8192
            || u64::from(self.width) * u64::from(self.height) > 33_554_432
            || !self.dpi_scale.is_finite() || !(0.5..=4.0).contains(&self.dpi_scale)
            || !(16..=256).contains(&self.icon_size)
        {
            return Err(DesktopError::InvalidEffect("canvas requires dimensions 1..8192, at most 33554432 pixels, DPI scale 0.5..4 and icon size 16..256 DIP".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct SceneIcon {
    pub origin: Point,
    pub animation: IconAnimationSpec,
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub canvas: Canvas,
    pub icons: Vec<SceneIcon>,
    pub render_options: OverlayRenderOptions,
}

pub(crate) fn position_at(origin: Point, target: Point, horizontal: &crate::Curve, vertical: &crate::Curve, progress: f32) -> Point {
    Point::new(
        (origin.x as f32 + horizontal.eval(progress) * (target.x as f32 - origin.x as f32)).round() as i32,
        (origin.y as f32 + vertical.eval(progress) * (target.y as f32 - origin.y as f32)).round() as i32,
    )
}

pub(crate) fn progress_at(duration: f64, position: f64, elapsed: f64) -> f32 {
    if duration == 0.0 { return if position > 0.0 { 1.0 } else { 0.0 }; }
    (elapsed / duration).clamp(0.0, 1.0) as f32
}

pub(crate) struct EvaluatedScene {
    pub scene: Scene,
    pub duration: f64,
    durations: Vec<f64>,
}

impl EvaluatedScene {
    pub fn new(scene: Scene) -> Result<Self, DesktopError> {
        scene.canvas.validate()?;
        let mut ids = std::collections::HashSet::new();
        let mut durations = Vec::with_capacity(scene.icons.len());
        for icon in &scene.icons {
            if !ids.insert(icon.animation.id.clone()) {
                return Err(DesktopError::InvalidEffect("duplicate scene icon id".into()));
            }
            if let Some(effect) = &icon.animation.effect { effect.validate()?; }
            durations.push(icon.animation.duration.resolve(icon.origin, icon.animation.target)?.as_secs_f64());
        }
        let duration = durations.iter().copied().fold(0.0, f64::max);
        Ok(Self { scene, duration, durations })
    }

    pub fn plans(&self) -> Vec<IconRenderPlan> {
        self.scene.icons.iter().map(|icon| {
            let mut plan = IconRenderPlan::placeholder(icon.animation.id.clone(), icon.origin, icon.animation.target);
            plan.effect = icon.animation.effect.clone();
            plan
        }).collect()
    }

    pub fn sample(&self, seconds: f64) -> Result<Vec<IconFrame>, DesktopError> {
        if !seconds.is_finite() || seconds < 0.0 || seconds > self.duration {
            return Err(DesktopError::InvalidDuration("scene time must be finite and within its duration".into()));
        }
        Ok(self.scene.icons.iter().zip(&self.durations).map(|(icon, duration)| {
            let progress = progress_at(*duration, seconds, seconds);
            let spec = &icon.animation;
            let position = if progress <= 0.0 { icon.origin } else if progress >= 1.0 { spec.target }
                else { position_at(icon.origin, spec.target, &spec.curve_x, &spec.curve_y, progress) };
            IconFrame { id: spec.id.clone(), position, progress, elapsed_seconds: seconds as f32 }
        }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ReadOnlyBackend;
    struct TestRenderer;

    impl SceneRenderer for TestRenderer {
        fn render(&mut self, frame: &[IconFrame], seconds: f64) -> Result<CapturedFrame, DesktopError> {
            Ok(CapturedFrame { width: 1, height: 1, seconds, pixels: vec![frame[0].position.x as u8, 0, 0, 255] })
        }
    }

    impl crate::DesktopBackend for ReadOnlyBackend {
        fn list_icons(&mut self) -> Result<Vec<crate::IconSnapshot>, DesktopError> {
            Ok(vec![crate::IconSnapshot::new("icon".into(), "Icon", None, false, Point::new(999, 999))])
        }
        fn get_flags(&mut self) -> Result<u32, DesktopError> { Ok(42) }
        fn list_monitors(&mut self) -> Result<Vec<crate::MonitorInfo>, DesktopError> { Ok(vec![]) }
        fn apply_flags(&mut self, _: u32, _: u32) -> Result<(), DesktopError> { panic!("scene wrote flags") }
        fn set_positions(&mut self, _: &[(crate::IconId, Point)]) -> Result<Vec<crate::IconId>, DesktopError> { panic!("scene moved icons") }
        fn begin_overlay_session(&mut self, _: &[IconRenderPlan], _: OverlayRenderOptions) -> Result<(), DesktopError> { panic!("scene opened overlay") }
        fn commit_overlay_frame(&mut self, _: &[(crate::IconId, Point)]) -> Result<(), DesktopError> { panic!("scene committed overlay") }
        fn finalize_overlay_session(&mut self, _: &[(crate::IconId, Point)]) -> Result<crate::FinalCommitOutcome, DesktopError> { panic!("scene finalized overlay") }
        fn prepare_scene_renderer(&mut self, _: Canvas, _: &[IconRenderPlan], _: OverlayRenderOptions) -> Result<Box<dyn SceneRenderer>, DesktopError> { Ok(Box::new(TestRenderer)) }
    }

    fn sample_scene() -> Scene {
        Scene { canvas: Canvas { width: 640, height: 480, dpi_scale: 1.0, icon_size: 48 },
            icons: vec![SceneIcon { origin: Point::new(20, 30), animation: IconAnimationSpec::new("icon".into(), Point::new(220, 30),
                crate::Duration::fixed(std::time::Duration::from_secs(2)), crate::Curve::linear()) }],
            render_options: OverlayRenderOptions::default() }
    }

    #[test]
    fn scene_session_is_read_only_and_releases_on_close_drop_shutdown() {
        let controller = crate::DesktopController::new(ReadOnlyBackend).unwrap();
        let scene = sample_scene();
        let session = controller.prepare_scene(scene.clone()).unwrap();
        assert_eq!(session.render_at(1.0).unwrap().pixels[0], 120);
        assert_eq!(controller.get_flags().unwrap(), 42);
        assert!(matches!(controller.prepare_scene(scene.clone()), Err(DesktopError::AnimationBusy)));
        assert!(matches!(controller.set_positions(vec![]), Err(DesktopError::AnimationBusy)));
        assert!(session.render_at(f64::NAN).is_err());
        assert!(session.render_at(3.0).is_err());
        session.close().unwrap();
        session.close().unwrap();
        assert!(session.render_at(0.0).is_err());
        drop(controller.prepare_scene(scene.clone()).unwrap());
        let reopened = controller.prepare_scene(scene).unwrap();
        assert_eq!(reopened.render_at(0.0).unwrap().pixels[0], 20);
        controller.shutdown();
        assert!(reopened.render_at(0.0).is_err());
    }

    #[test]
    fn canvas_accepts_8k_with_bounded_allocations() {
        let mut canvas = Canvas { width: 7680, height: 4320, dpi_scale: 1.0, icon_size: 96 };
        assert!(canvas.validate().is_ok());
        canvas.width = 8192;
        canvas.height = 8192;
        assert!(canvas.validate().is_err());
    }

    #[test]
    fn scene_rejects_duplicate_ids_and_invalid_canvas() {
        let mut scene = sample_scene();
        scene.icons.push(scene.icons[0].clone());
        assert!(EvaluatedScene::new(scene).is_err());
        let mut scene = sample_scene();
        scene.canvas.width = u32::MAX;
        assert!(EvaluatedScene::new(scene).is_err());
    }

    #[test]
    fn scene_samples_explicit_origins_and_rewinds() {
        let icons = [1.0, 2.0].into_iter().enumerate().map(|(index, seconds)| SceneIcon {
            origin: Point::new(20, 30),
            animation: IconAnimationSpec::new(format!("icon{index}").into(), Point::new(220, 30),
                crate::Duration::fixed(std::time::Duration::from_secs_f64(seconds)), crate::Curve::linear()),
        }).collect();
        let scene = EvaluatedScene::new(Scene {
            canvas: Canvas { width: 640, height: 480, dpi_scale: 1.0, icon_size: 48 },
            icons, render_options: OverlayRenderOptions::default(),
        }).unwrap();
        assert_eq!(scene.duration, 2.0);
        let first = scene.sample(0.5).unwrap();
        assert_eq!(first[0].position, Point::new(120, 30));
        assert_eq!(first[1].position, Point::new(70, 30));
        scene.sample(2.0).unwrap();
        for (before, after) in first.iter().zip(scene.sample(0.5).unwrap()) {
            assert_eq!(before.position, after.position);
            assert_eq!(before.progress, after.progress);
            assert_eq!(before.elapsed_seconds, after.elapsed_seconds);
        }
        assert!(scene.sample(f64::NAN).is_err());
        assert!(scene.sample(2.1).is_err());
    }
}