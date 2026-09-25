mod curves;
mod shaders;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use curves::Track;
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use ratatui_textarea::{CursorMove, TextArea};
use rdi_core::{
    AnimationCurve, AnimationPreset, Duration, Effect, IconAnimationSpec, Point, ShaderPipeline,
    ShaderProgram,
};
use rdi_platform_windows::shader::{self, BuiltinShader};
use rdi_platform_windows::shader_library::{DEFAULT_PIXEL, DEFAULT_VERTEX};
use shaders::{Settings, ShaderDraft};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Off,
    Builtin(BuiltinShader),
    Custom,
}

#[derive(Clone, Debug)]
pub struct SavedShader {
    draft: ShaderDraft,
    program: ShaderProgram,
    settings: Settings,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub choice: Choice,
    pub custom_mode: bool,
    pub tracks: Option<[Track; 2]>,
    pub shader: Option<SavedShader>,
    pub seconds: f64,
}

impl Config {
    pub fn new(selected: Option<BuiltinShader>) -> Self {
        let mut result = Self {
            choice: selected.map_or(Choice::Off, Choice::Builtin),
            custom_mode: false,
            tracks: None,
            shader: None,
            seconds: 2.0,
        };
        result.seconds = result
            .preset()
            .duration
            .resolve(Point::ZERO, Point::ZERO)
            .unwrap()
            .as_secs_f64();
        result
    }

    fn base(&self) -> Option<BuiltinShader> {
        match self.choice {
            Choice::Builtin(shader) => Some(shader),
            Choice::Custom => self.shader.as_ref().map(|shader| shader.draft.base),
            Choice::Off => None,
        }
    }

    fn preset(&self) -> AnimationPreset {
        self.base()
            .map_or_else(AnimationPreset::default, BuiltinShader::default_preset)
    }

    fn track(&self, index: usize) -> Track {
        if self.custom_mode
            && let Some(tracks) = &self.tracks
        {
            return tracks[index].clone();
        }
        let preset = self.preset();
        Track::from_curve(
            if index == 0 {
                &preset.movement
            } else {
                &preset.envelope
            },
            33,
        )
    }

    pub fn program(&self) -> Result<Option<ShaderProgram>, String> {
        match self.choice {
            Choice::Off => Ok(None),
            Choice::Builtin(shader) => shader::compile(shader)
                .map(Some)
                .map_err(|error| error.to_string()),
            Choice::Custom => self
                .shader
                .as_ref()
                .map(|shader| Some(shader.program.clone()))
                .ok_or("No saved custom shader".into()),
        }
    }

    pub fn specs(
        &self,
        moves: &[crate::demo_animation::Move],
        program: Option<&ShaderProgram>,
    ) -> Result<Vec<IconAnimationSpec>, String> {
        let preset = self.preset();
        let (movement, envelope) = if self.custom_mode {
            let tracks = self.tracks.as_ref().ok_or("No custom curves")?;
            (tracks[0].curve(false)?, tracks[1].curve(true)?)
        } else {
            (preset.movement, preset.envelope)
        };
        let duration = std::time::Duration::try_from_secs_f64(self.seconds)
            .map_err(|error| error.to_string())?;
        if duration.is_zero() {
            return Err("Duration must be positive".into());
        }
        moves
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let mut spec = IconAnimationSpec::new(
                    entry.id.clone(),
                    entry.target,
                    Duration::fixed(duration),
                    movement.clone(),
                );
                if let Some(program) = program {
                    let settings = if self.choice == Choice::Custom {
                        self.shader.as_ref().map(|shader| &shader.settings)
                    } else {
                        None
                    };
                    let effect = Effect {
                        shader: program.clone(),
                        params: settings
                            .map_or_else(|| program.default_params(), |settings| settings.params),
                        padding_px: settings.map_or(24, |settings| settings.padding_px),
                        seed: settings.map_or(index as f32, |settings| settings.seed),
                        envelope: envelope.clone(),
                    };
                    effect.validate().map_err(|error| error.to_string())?;
                    spec.effect = Some(effect);
                }
                Ok(spec)
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Main,
    Curves,
    Shader,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Json,
    Time,
    Value,
    Vertex,
    Pixel,
    Settings,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Control {
    Tab(usize),
    Mode(bool),
    Track(usize),
    ShaderMenu,
    Shader(Choice),
    FromMenu,
    From(BuiltinShader),
    Graph,
    Field(Field),
    Save,
    Discard,
    Delete,
    Interp,
    Samples,
    Range,
    Customize,
    Confirm,
    PipelineMenu,
    Pipeline(usize),
    Stage(usize),
    StageSource(bool),
    Compile,
    ResetShader,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Shader,
    From,
    Pipeline,
}

const PRESSED_COLOR: Color = Color::Rgb(155, 125, 65);

pub enum EditorEvent {
    Ignored,
    Consumed,
    Pause,
    Apply {
        config: Config,
        reset_duration: bool,
    },
}

pub struct MainView<'a> {
    pub position: f64,
    pub speed: f64,
    pub destination: f64,
    pub state: &'a str,
    pub summary: &'a str,
    pub visible: bool,
    pub help: bool,
    pub prompt: &'a str,
}

struct Edit {
    field: Field,
    text: TextArea<'static>,
    area: Rect,
    read_only: bool,
}

trait ClipboardAccess {
    fn read(&mut self) -> Result<String, String>;
    fn write(&mut self, text: String) -> Result<(), String>;
}

struct SystemClipboard;

impl ClipboardAccess for SystemClipboard {
    fn read(&mut self) -> Result<String, String> {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .map_err(|error| format!("Clipboard paste failed: {error}"))
    }

    fn write(&mut self, text: String) -> Result<(), String> {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text))
            .map_err(|error| format!("Clipboard copy failed: {error}"))
    }
}

impl Edit {
    fn has_selection(&self) -> bool {
        self.text.selection_range().is_some_and(|(start, end)| start != end)
    }

    fn paste(&mut self, text: &str) -> Result<(), String> {
        if self.read_only {
            return Ok(());
        }
        if self.text.lines().iter().map(String::len).sum::<usize>() + text.len()
            > 2 * 1024 * 1024
        {
            return Err("Text field exceeds 2 MiB".into());
        }
        self.text.insert_str(text.replace("\r\n", "\n").replace('\r', "\n"));
        Ok(())
    }

    fn click(&mut self, position: Position) {
        let mut probe = self.text.clone();
        probe.move_cursor(CursorMove::Top);
        probe.move_cursor(CursorMove::InViewport);
        let origin = probe.screen_cursor();
        let row = origin.row + usize::from(position.y.saturating_sub(self.area.y));
        let target = origin.col + usize::from(position.x.saturating_sub(self.area.x));
        let mut width = 0;
        let mut column = 0;
        if let Some(line) = self.text.lines().get(row) {
            for character in line.chars() {
                let next = if character == '\t' {
                    4 - width % 4
                } else {
                    Span::raw(character.to_string()).width()
                };
                if width + next > target {
                    break;
                }
                width += next;
                column += 1;
            }
        }
        self.text.move_cursor(CursorMove::Jump(
            row.min(u16::MAX as usize) as u16,
            column.min(u16::MAX as usize) as u16,
        ));
    }
}

pub struct Editor {
    pub config: Config,
    tab: Tab,
    track: usize,
    source: BuiltinShader,
    samples: usize,
    range: [f32; 2],
    selected: Vec<usize>,
    pending: Option<Track>,
    creating: bool,
    edit: Option<Edit>,
    shader_draft: ShaderDraft,
    stage: usize,
    menu: Option<Menu>,
    menu_index: usize,
    confirm: bool,
    hits: Vec<(Rect, Control)>,
    pressed: Option<Control>,
    graph: Rect,
    pub message: String,
}

impl Editor {
    pub fn new(selected: Option<BuiltinShader>) -> Self {
        let source = selected.unwrap_or(BuiltinShader::Identity);
        Self {
            config: Config::new(selected),
            tab: Tab::Main,
            track: 0,
            source,
            samples: 33,
            range: [0.0, 1.0],
            selected: Vec::new(),
            pending: None,
            creating: false,
            edit: None,
            shader_draft: ShaderDraft::from_builtin(source),
            stage: 0,
            menu: None,
            menu_index: 0,
            confirm: false,
            hits: Vec::new(),
            pressed: None,
            graph: Rect::default(),
            message: String::new(),
        }
    }

    pub fn is_main(&self) -> bool {
        self.tab == Tab::Main
    }
    pub fn can_quit(&self) -> bool {
        self.edit.is_none() && self.menu.is_none()
    }
    pub fn ctrl_c_exits(&self, event: &Event) -> bool {
        matches!(event, Event::Key(key) if key.kind != KeyEventKind::Release
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C')))
            && !self.edit.as_ref().is_some_and(Edit::has_selection)
    }
    pub fn begin_action(&mut self, event: &Event) -> bool {
        self.pressed = match event {
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);
                if self
                    .edit
                    .as_ref()
                    .is_some_and(|edit| edit.area.contains(position))
                {
                    return false;
                }
                self.hits
                    .iter()
                    .rev()
                    .find(|(area, _)| area.contains(position))
                    .map(|(_, control)| control.clone())
                    .filter(|control| {
                        !matches!(control, Control::Graph | Control::Field(_))
                            && (self.menu.is_none()
                                || matches!(
                                    control,
                                    Control::Shader(_) | Control::From(_) | Control::Pipeline(_)
                                ))
                    })
            }
            Event::Key(key)
                if key.kind == KeyEventKind::Press
                    && key.code == KeyCode::Enter
                    && self.menu.is_some() =>
            {
                self.menu_items()
                    .get(self.menu_index)
                    .map(|(_, control)| control.clone())
            }
            _ => None,
        };
        self.pressed.is_some()
    }
    pub fn finish_action(&mut self) {
        self.pressed = None;
    }
    pub fn error(&mut self, error: impl ToString) {
        self.message = error
            .to_string()
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .take(240)
            .collect();
    }
    pub fn committed(&mut self, config: Config) {
        self.config = config;
        self.pending = None;
        self.creating = false;
        self.edit = None;
        self.selected.clear();
        self.message = "Applied".into();
    }

    fn current(&self) -> Track {
        self.pending
            .clone()
            .unwrap_or_else(|| self.config.track(self.track))
    }
    fn discard(&mut self) {
        self.edit = None;
        self.pending = None;
        self.creating = false;
        self.selected.clear();
        self.confirm = false;
    }

    fn button(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        label: impl Into<String>,
        control: Control,
        active: bool,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let accent = match control {
            Control::Tab(_) => Color::Rgb(180, 130, 255),
            Control::Track(_) | Control::Stage(_) => Color::Rgb(100, 65, 150),
            _ => Color::Cyan,
        };
        let mut style = Style::default()
            .fg(if active {
                if matches!(control, Control::Track(_) | Control::Stage(_)) {
                    Color::White
                } else {
                    Color::Black
                }
            } else {
                accent
            })
            .bg(if active { accent } else { Color::Reset });
        if self.pressed.as_ref() == Some(&control) {
            style = Style::default().fg(Color::White).bg(PRESSED_COLOR);
        }
        let label = label.into();
        let text = if let Some((name, value)) = label.split_once(": ") {
            Line::from(vec![
                Span::styled(
                    format!("{name}: "),
                    Style::default().fg(Color::Rgb(210, 185, 125)),
                ),
                Span::styled(value.to_owned(), style),
            ])
        } else {
            Line::styled(label, style)
        };
        frame.render_widget(Paragraph::new(text), area);
        self.hits.push((area, control));
    }

    fn row(&mut self, frame: &mut Frame, y: u16, buttons: Vec<(String, Control, bool)>) {
        let width = frame.area().width;
        let label = match buttons.first().map(|(_, control, _)| control) {
            Some(Control::Mode(_)) => "Preset: ",
            Some(Control::Track(_)) => "Track: ",
            Some(Control::Stage(_)) => "Stage: ",
            Some(Control::StageSource(_)) => "Source: ",
            _ => "",
        };
        frame.render_widget(
            Paragraph::new(label).style(Style::default().fg(Color::Rgb(210, 185, 125))),
            Rect::new(1, y, label.len() as u16, 1),
        );
        let mut left = 1 + label.len() as u16;
        if matches!(
            buttons.first().map(|(_, control, _)| control),
            Some(Control::Samples)
        ) {
            left = 7;
        }
        for (label, control, active) in buttons {
            let length = (label.chars().count() as u16 + 2).min(width.saturating_sub(left));
            self.button(
                frame,
                Rect::new(left, y, length, 1),
                format!(" {label}"),
                control,
                active,
            );
            left += length + 1;
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, view: MainView<'_>) {
        self.hits.clear();
        let area = frame.area();
        frame.render_widget(Clear, area);
        if area.width < 42 || area.height < 24 {
            frame.render_widget(
                Paragraph::new("Resize to at least 42 x 24\nCtrl+C closes safely"),
                area,
            );
            return;
        }
        self.row(
            frame,
            0,
            vec![
                ("Main".into(), Control::Tab(0), self.tab == Tab::Main),
                ("Curves".into(), Control::Tab(1), self.tab == Tab::Curves),
                (
                    "Shader Editor".into(),
                    Control::Tab(2),
                    self.tab == Tab::Shader,
                ),
            ],
        );
        match self.tab {
            Tab::Main => self.draw_main(frame, &view),
            Tab::Curves => self.draw_curves(frame),
            Tab::Shader => self.draw_shader(frame),
        }
        let status = if view.prompt.starts_with(':') || self.message.is_empty() {
            view.prompt
        } else {
            &self.message
        };
        frame.render_widget(
            Paragraph::new(status.to_string()).style(Style::default().fg(Color::Yellow)),
            Rect::new(0, area.height - 1, area.width, 1),
        );
        if self.menu.is_some() {
            self.draw_menu(frame);
        }
    }

    fn draw_main(&mut self, frame: &mut Frame, view: &MainView<'_>) {
        let area = frame.area();
        let line = |frame: &mut Frame, row, text: String| {
            frame.render_widget(Paragraph::new(text), Rect::new(1, row, area.width - 2, 1))
        };
        line(
            frame,
            1,
            format!(
                "{}  t={:.4}  speed={:.3}x  to={:.3}",
                view.state, view.position, view.speed, view.destination
            ),
        );
        let right = area.width - 4;
        let position = 2 + ((right - 2) as f64 * view.position).round() as u16;
        let track: String = (2..=right)
            .map(|column| {
                if column == position {
                    'O'
                } else if column < position {
                    '='
                } else {
                    '-'
                }
            })
            .collect();
        frame.render_widget(
            Paragraph::new(format!("[{track}]")).style(Style::default().fg(Color::Green)),
            Rect::new(1, 3, area.width - 2, 1),
        );
        line(
            frame,
            4,
            "0                          0.5                          1".into(),
        );
        let seconds = self.config.seconds / view.speed;
        line(frame, 5, format!("Duration: {seconds:.3}s"));
        let steps = crate::app::DURATION_STEPS;
        let index = steps
            .iter()
            .enumerate()
            .min_by(|(_, first), (_, second)| {
                (**first - seconds)
                    .abs()
                    .total_cmp(&(**second - seconds).abs())
            })
            .unwrap()
            .0;
        let track: String = (0..steps.len())
            .map(|step| {
                if step == index {
                    'O'
                } else if step < index {
                    '='
                } else {
                    '-'
                }
            })
            .collect();
        frame.render_widget(
            Paragraph::new(format!("[{track}]")).style(Style::default().fg(Color::Yellow)),
            Rect::new(1, 6, 28, 1),
        );
        line(frame, 7, "0.1s                    5s".into());
        let name = match self.config.choice {
            Choice::Off => "off",
            Choice::Builtin(shader) => shader.name(),
            Choice::Custom => "Custom",
        };
        self.row(
            frame,
            8,
            vec![(format!("Shader: {name} v"), Control::ShaderMenu, false)],
        );
        frame.render_widget(
            Paragraph::new("<--- CLICK ME").style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(28, 8, 13, 1),
        );
        self.row(
            frame,
            9,
            vec![
                (
                    "Default Preset".into(),
                    Control::Mode(false),
                    !self.config.custom_mode,
                ),
                (
                    "Custom".into(),
                    Control::Mode(true),
                    self.config.custom_mode,
                ),
            ],
        );
        let hint = if area.width >= 50 {
            Rect::new(36, 9, 13, 1)
        } else {
            Rect::new(1, 10, 13, 1)
        };
        frame.render_widget(
            Paragraph::new("<--- CLICK ME").style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            hint,
        );
        line(frame, 11, view.summary.into());
        line(
            frame,
            12,
            format!(
                "Real icons {}   h toggle",
                if view.visible { "shown" } else { "hidden" }
            ),
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Left/Right", Style::default().fg(Color::Cyan)),
                Span::raw(" seek  "),
                Span::styled("Space", Style::default().fg(Color::Cyan)),
                Span::raw(" play/pause  ? help  : commands"),
            ])),
            Rect::new(1, 14, area.width - 2, 1),
        );
        if view.help {
            let cyan = Style::default().fg(Color::Cyan);
            let yellow = Style::default().fg(Color::Yellow);
            let help = vec![
                Line::from(vec![
                    Span::styled("f/r/m Home/End", cyan),
                    Span::raw(" end/start/middle/seek"),
                ]),
                Line::from(vec![
                    Span::styled("Up/Down +/- 1", cyan),
                    Span::raw(" speed / reset"),
                ]),
                Line::from(vec![
                    Span::styled("play ", cyan),
                    Span::styled("POSITION [SPEED]", yellow),
                ]),
                Line::from(vec![
                    Span::styled("seek ", cyan),
                    Span::styled("POSITION", yellow),
                    Span::styled("  speed ", cyan),
                    Span::styled("RATE", yellow),
                ]),
                Line::from(vec![
                    Span::styled("pause  close ", cyan),
                    Span::styled("restore|current|target", yellow),
                ]),
                Line::from(vec![
                    Span::styled("Tab/Shift+Tab", cyan),
                    Span::raw(" tabs  "),
                    Span::styled("q", cyan),
                    Span::raw(" close"),
                ]),
            ];
            frame.render_widget(
                Paragraph::new(help).wrap(Wrap { trim: false }),
                Rect::new(1, 15, area.width - 2, area.height.saturating_sub(16)),
            );
        }
    }

    fn draw_curves(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.row(
            frame,
            2,
            vec![
                (
                    "Default Preset".into(),
                    Control::Mode(false),
                    !self.config.custom_mode,
                ),
                (
                    "Custom".into(),
                    Control::Mode(true),
                    self.config.custom_mode,
                ),
            ],
        );
        self.row(
            frame,
            3,
            vec![
                ("Movement".into(), Control::Track(0), self.track == 0),
                ("Envelope".into(), Control::Track(1), self.track == 1),
            ],
        );
        if !self.config.custom_mode {
            self.row(
                frame,
                4,
                vec![
                    (
                        format!("From: {} v", self.source.name()),
                        Control::FromMenu,
                        false,
                    ),
                    (
                        if self.confirm { "Replace?" } else { "Create" }.into(),
                        if self.confirm {
                            Control::Confirm
                        } else {
                            Control::Customize
                        },
                        false,
                    ),
                ],
            );
        } else {
            self.row(
                frame,
                4,
                vec![(
                    format!("Interp: {}", self.current().interp),
                    Control::Interp,
                    false,
                )],
            );
        }
        self.row(
            frame,
            5,
            vec![(
                format!("Approximation samples: {}", self.samples),
                Control::Samples,
                false,
            )],
        );
        let height = 9.min(area.height.saturating_sub(16));
        let graph = Rect::new(1, 8, area.width - 2, height);
        let title = if self.track == 0 {
            " Movement progress "
        } else {
            " Effect strength "
        };
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title_top(Line::styled(
                    title,
                    Style::default().fg(Color::Rgb(210, 185, 125)),
                ))
                .title_bottom(
                    Line::styled(
                        " time(0..1) ",
                        Style::default().fg(Color::Rgb(210, 185, 125)),
                    )
                    .right_aligned(),
                ),
            graph,
        );
        let axis = Rect::new(graph.x, graph.y, 1, graph.height);
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM)
                .border_style(
                    Style::default().fg(if self.pressed == Some(Control::Range) {
                        PRESSED_COLOR
                    } else {
                        Color::Cyan
                    }),
                ),
            axis,
        );
        self.hits.push((axis, Control::Range));
        for (row, value) in [
            (graph.y - 1, self.range[1]),
            (graph.bottom(), self.range[0]),
        ] {
            frame.render_widget(
                Paragraph::new(format!("{value:.1}"))
                    .style(Style::default().fg(Color::Rgb(210, 185, 125))),
                Rect::new(graph.x, row, 5, 1),
            );
        }
        self.graph = Rect::new(graph.x + 1, graph.y + 1, graph.width - 2, graph.height - 2);
        self.hits.push((self.graph, Control::Graph));
        let track = self.current();
        if let Ok(curve) = track.curve(self.track == 1) {
            for column in 0..self.graph.width {
                let time = column as f32 / (self.graph.width - 1).max(1) as f32;
                let (x, y) = self.cell([time, curve.eval(time)]);
                frame.render_widget(
                    Paragraph::new(".").style(Style::default().fg(Color::Green)),
                    Rect::new(x, y, 1, 1),
                );
            }
        }
        for ((x, y), indices) in self.markers(&track) {
            let selected = indices.iter().any(|index| self.selected.contains(index));
            let color = if selected && self.pending.is_some() {
                Color::White
            } else if selected {
                Color::Blue
            } else if indices.len() >= 5 {
                Color::Rgb(255, 150, 40)
            } else if indices.len() >= 2 {
                Color::Yellow
            } else {
                Color::Gray
            };
            frame.render_widget(
                Paragraph::new("o").style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Rect::new(x, y, 1, 1),
            );
        }
        let row = graph.bottom() + 1;
        if let Some(&index) = self.selected.first()
            && let Some(key) = track.keyframes.get(index)
        {
            frame.render_widget(
                Paragraph::new(format!("{} selected", self.selected.len())),
                Rect::new(1, row, 13, 1),
            );
            self.button(
                frame,
                Rect::new(14, row, 12, 1),
                format!("t: {:.3}", key[0]),
                Control::Field(Field::Time),
                false,
            );
            self.button(
                frame,
                Rect::new(27, row, 12, 1),
                format!("v: {:.3}", key[1]),
                Control::Field(Field::Value),
                false,
            );
            self.row(
                frame,
                row + 1,
                vec![("Delete".into(), Control::Delete, false)],
            );
        }
        if self.pending.is_some() {
            self.button(
                frame,
                Rect::new(12, row + 1, 9, 1),
                "Discard",
                Control::Discard,
                false,
            );
            self.button(
                frame,
                Rect::new(23, row + 1, 7, 1),
                "Save",
                Control::Save,
                false,
            );
        }
        let text_area = Rect::new(
            1,
            row + 2,
            area.width - 2,
            area.height.saturating_sub(row + 4),
        );
        self.text(
            frame,
            Field::Json,
            text_area,
            "Keyframes JSON",
            track.json(),
            self.config.custom_mode,
        );
        if self
            .edit
            .as_ref()
            .is_some_and(|edit| edit.field == Field::Json)
        {
            self.button(
                frame,
                Rect::new(area.width - 12, area.height - 2, 10, 1),
                "Save JSON",
                Control::Save,
                false,
            );
        }
        if let Some(edit) = &mut self.edit
            && matches!(edit.field, Field::Time | Field::Value)
        {
            let left = if edit.field == Field::Time { 14 } else { 27 };
            edit.area = Rect::new(left, row, 12, 1);
            frame.render_widget(&edit.text, edit.area);
        }
    }

    fn cell(&self, key: [f32; 2]) -> (u16, u16) {
        let value = ((key[1] - self.range[0]) / (self.range[1] - self.range[0])).clamp(0.0, 1.0);
        (
            self.graph.x
                + (key[0].clamp(0.0, 1.0) * self.graph.width.saturating_sub(1) as f32).round()
                    as u16,
            self.graph.y
                + ((1.0 - value) * self.graph.height.saturating_sub(1) as f32).round() as u16,
        )
    }

    fn markers(&self, track: &Track) -> BTreeMap<(u16, u16), Vec<usize>> {
        let mut result = BTreeMap::new();
        for (index, &key) in track.keyframes.iter().enumerate() {
            result
                .entry(self.cell(key))
                .or_insert_with(Vec::new)
                .push(index);
        }
        result
    }

    fn draw_shader(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.row(
            frame,
            2,
            vec![(
                format!("Pipeline: {:?} v", self.shader_draft.pipeline),
                Control::PipelineMenu,
                false,
            )],
        );
        self.row(
            frame,
            3,
            vec![
                (
                    format!("Copy: {} v", self.source.name()),
                    Control::FromMenu,
                    false,
                ),
                (
                    if self.confirm { "Replace?" } else { "Load" }.into(),
                    if self.confirm {
                        Control::Confirm
                    } else {
                        Control::Customize
                    },
                    false,
                ),
            ],
        );
        self.row(
            frame,
            4,
            vec![
                ("Vertex".into(), Control::Stage(0), self.stage == 0),
                ("Pixel".into(), Control::Stage(1), self.stage == 1),
                ("Settings".into(), Control::Stage(2), self.stage == 2),
            ],
        );
        let default = match self.stage {
            0 => self.shader_draft.default_vertex,
            1 => self.shader_draft.default_pixel,
            _ => false,
        };
        if self.stage < 2 {
            self.row(
                frame,
                5,
                vec![
                    ("Pipeline default".into(), Control::StageSource(true), default),
                    ("Custom HLSL".into(), Control::StageSource(false), !default),
                ],
            );
        }
        frame.render_widget(
            Paragraph::new("Trusted HLSL only; GPU execution is not sandboxed.")
                .style(Style::default().fg(Color::Yellow)),
            Rect::new(1, 6, area.width - 2, 1),
        );
        let (field, text) = match self.stage {
            0 if default => (Field::Vertex, DEFAULT_VERTEX.into()),
            1 if default => (Field::Pixel, DEFAULT_PIXEL.into()),
            0 => (Field::Vertex, self.shader_draft.vertex.clone()),
            1 => (Field::Pixel, self.shader_draft.pixel.clone()),
            _ => (Field::Settings, self.shader_draft.settings.clone()),
        };
        self.text(
            frame,
            field,
            Rect::new(1, 7, area.width - 2, area.height - 10),
            if self.stage == 2 {
                "Parameters / padding / seed / procedural recipe JSON"
            } else if default {
                "Pipeline default (read-only)"
            } else {
                "Custom HLSL"
            },
            text,
            !default,
        );
        self.row(
            frame,
            area.height - 2,
            vec![
                ("Compile & Apply".into(), Control::Compile, false),
                ("Discard".into(), Control::ResetShader, false),
            ],
        );
    }

    fn text(
        &mut self,
        frame: &mut Frame,
        field: Field,
        area: Rect,
        title: &'static str,
        text: String,
        editable: bool,
    ) {
        if let Some(edit) = &mut self.edit
            && edit.field == field
        {
            edit.area = Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                area.height.saturating_sub(2),
            );
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Line::styled(
                        title,
                        Style::default().fg(Color::Rgb(210, 185, 125)),
                    ))
                    .border_style(Style::default().fg(Color::Cyan)),
                area,
            );
            frame.render_widget(&edit.text, edit.area);
        } else {
            let mut paragraph = Paragraph::new(text)
                    .block(Block::default().borders(Borders::ALL).title(Line::styled(
                        title,
                        Style::default().fg(Color::Rgb(210, 185, 125)),
                    )))
                    .style(Style::default().fg(if editable {
                        Color::Gray
                    } else {
                        Color::DarkGray
                    }));
            if !editable {
                paragraph = paragraph.wrap(ratatui::widgets::Wrap { trim: false });
            }
            frame.render_widget(paragraph, area);
        }
        if editable || matches!(field, Field::Vertex | Field::Pixel) {
            self.hits.push((area, Control::Field(field)));
        }
    }

    fn menu_items(&self) -> Vec<(String, Control)> {
        match self.menu {
            Some(Menu::Shader) => std::iter::once(("off".into(), Control::Shader(Choice::Off)))
                .chain(BuiltinShader::ALL.iter().map(|&shader| {
                    (
                        shader.name().into(),
                        Control::Shader(Choice::Builtin(shader)),
                    )
                }))
                .chain(std::iter::once((
                    "Custom".into(),
                    Control::Shader(Choice::Custom),
                )))
                .collect(),
            Some(Menu::From) => BuiltinShader::ALL
                .iter()
                .map(|&shader| (shader.name().into(), Control::From(shader)))
                .collect(),
            Some(Menu::Pipeline) => ["Sprite", "Particles", "Procedural"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| (name.into(), Control::Pipeline(index)))
                .collect(),
            None => Vec::new(),
        }
    }

    fn draw_menu(&mut self, frame: &mut Frame) {
        let items = self.menu_items();
        let area = Rect::new(2, 5, 28.min(frame.area().width - 3), items.len() as u16 + 2);
        frame.render_widget(Clear, area);
        frame.render_widget(Block::default().borders(Borders::ALL), area);
        for (index, (label, control)) in items.into_iter().enumerate() {
            self.button(
                frame,
                Rect::new(area.x + 1, area.y + 1 + index as u16, area.width - 2, 1),
                label,
                control,
                index == self.menu_index,
            );
        }
    }

    fn begin_field(&mut self, field: Field) {
        let read_only = (field == Field::Vertex && self.shader_draft.default_vertex)
            || (field == Field::Pixel && self.shader_draft.default_pixel);
        let text = match field {
            Field::Vertex if read_only => DEFAULT_VERTEX.into(),
            Field::Pixel if read_only => DEFAULT_PIXEL.into(),
            Field::Json => self.current().json(),
            Field::Time | Field::Value => {
                let Some(&index) = self.selected.first() else {
                    return;
                };
                format!(
                    "{:.3}",
                    self.current().keyframes[index][if field == Field::Time { 0 } else { 1 }]
                )
            }
            Field::Vertex => self.shader_draft.vertex.clone(),
            Field::Pixel => self.shader_draft.pixel.clone(),
            Field::Settings => self.shader_draft.settings.clone(),
        };
        let mut text = TextArea::from(text.lines());
        text.set_cursor_line_style(Style::default());
        text.set_cursor_style(Style::default().bg(Color::White).fg(Color::Black));
        self.edit = Some(Edit {
            field,
            text,
            area: Rect::default(),
            read_only,
        });
    }

    fn retain_shader_text(&mut self) {
        self.sync_shader_text();
        self.edit = None;
    }

    fn sync_shader_text(&mut self) {
        if let Some(edit) = &self.edit
            && !edit.read_only
        {
            let text = edit.text.lines().join("\n");
            match edit.field {
                Field::Vertex => self.shader_draft.vertex = text,
                Field::Pixel => self.shader_draft.pixel = text,
                Field::Settings => self.shader_draft.settings = text,
                _ => {}
            }
        }
    }

    fn save_curve(&mut self) -> Result<EditorEvent, String> {
        let track = if let Some(edit) = &self.edit
            && edit.field == Field::Json
        {
            Track::parse(&edit.text.lines().join("\n"), self.track == 1)?
        } else {
            self.current()
        };
        track.curve(self.track == 1)?;
        let mut config = self.config.clone();
        config.tracks.as_mut().ok_or("Create custom curves first")?[self.track] = track;
        Ok(EditorEvent::Apply {
            config,
            reset_duration: false,
        })
    }

    fn numeric_save(&mut self) -> Result<(), String> {
        let edit = self.edit.as_ref().ok_or("No field")?;
        let value: f32 = edit
            .text
            .lines()
            .join("")
            .parse()
            .map_err(|_| "Expected a finite number")?;
        if !value.is_finite() {
            return Err("Expected a finite number".into());
        }
        let axis = if edit.field == Field::Time { 0 } else { 1 };
        let track = self.current();
        if edit.text.lines().join("") == format!("{:.3}", track.keyframes[self.selected[0]][axis]) {
            self.edit = None;
            return Ok(());
        }
        let mut delta = [0.0; 2];
        delta[axis] = value - track.keyframes[self.selected[0]][axis];
        self.pending = Some(track.translated(&self.selected, delta, self.track == 1)?);
        self.edit = None;
        Ok(())
    }

    fn activate(&mut self, control: Control) -> Result<EditorEvent, String> {
        match control {
            Control::Tab(index) => {
                self.discard();
                self.menu = None;
                self.tab = [Tab::Main, Tab::Curves, Tab::Shader][index];
            }
            Control::Track(index) => {
                self.discard();
                self.track = index;
                self.range = [0.0, 1.0];
            }
            Control::ShaderMenu | Control::FromMenu | Control::PipelineMenu => {
                self.menu = Some(match control {
                    Control::ShaderMenu => Menu::Shader,
                    Control::FromMenu => Menu::From,
                    _ => Menu::Pipeline,
                });
                self.menu_index = 0;
            }
            Control::Shader(choice) => {
                self.menu = None;
                if choice == Choice::Custom && self.config.shader.is_none() {
                    self.tab = Tab::Shader;
                    return Ok(EditorEvent::Pause);
                }
                if choice == self.config.choice {
                    return Ok(EditorEvent::Consumed);
                }
                let mut config = self.config.clone();
                config.choice = choice;
                if let Choice::Builtin(shader) = choice {
                    self.source = shader;
                    config.seconds = shader
                        .default_preset()
                        .duration
                        .resolve(Point::ZERO, Point::ZERO)
                        .unwrap()
                        .as_secs_f64();
                }
                return Ok(EditorEvent::Apply {
                    config,
                    reset_duration: matches!(choice, Choice::Builtin(_)),
                });
            }
            Control::Mode(custom) => {
                self.discard();
                let mut config = self.config.clone();
                if custom && config.tracks.is_none() {
                    let preset = config.preset();
                    config.tracks = Some([
                        Track::from_curve(&preset.movement, self.samples),
                        Track::from_curve(&preset.envelope, self.samples),
                    ]);
                    self.tab = Tab::Curves;
                }
                config.custom_mode = custom;
                return Ok(EditorEvent::Apply {
                    config,
                    reset_duration: false,
                });
            }
            Control::From(shader) => {
                self.source = shader;
                self.menu = None;
                self.confirm = false;
            }
            Control::Customize
                if (self.tab == Tab::Curves && self.config.tracks.is_some())
                    || self.tab == Tab::Shader =>
            {
                self.confirm = true;
                return Ok(EditorEvent::Pause);
            }
            Control::Customize | Control::Confirm => {
                self.confirm = false;
                if self.tab == Tab::Shader {
                    self.edit = None;
                    self.shader_draft = ShaderDraft::from_builtin(self.source);
                } else {
                    let preset = self.source.default_preset();
                    let mut config = self.config.clone();
                    config.tracks = Some([
                        Track::from_curve(&preset.movement, self.samples),
                        Track::from_curve(&preset.envelope, self.samples),
                    ]);
                    config.custom_mode = true;
                    return Ok(EditorEvent::Apply {
                        config,
                        reset_duration: false,
                    });
                }
            }
            Control::Samples => {
                self.samples = match self.samples {
                    17 => 33,
                    33 => 65,
                    65 => 129,
                    _ => 17,
                };
            }
            Control::Range => {
                self.range = if self.track == 1 || self.range[1] >= 17.0 {
                    [0.0, 1.0]
                } else if self.range == [0.0, 1.0] {
                    [-1.0, 2.0]
                } else {
                    [self.range[0] * 4.0, 1.0 + (self.range[1] - 1.0) * 4.0]
                };
            }
            Control::Interp if self.config.custom_mode => {
                let mut track = self.current();
                track.interp = match track.interp.as_str() {
                    "linear" => "smooth_step",
                    "smooth_step" => "step",
                    _ => "linear",
                }
                .into();
                self.pending = Some(track);
                return Ok(EditorEvent::Pause);
            }
            Control::Field(field) => {
                self.begin_field(field);
                return Ok(EditorEvent::Pause);
            }
            Control::Save => {
                if self
                    .edit
                    .as_ref()
                    .is_some_and(|edit| matches!(edit.field, Field::Time | Field::Value))
                {
                    self.numeric_save()?;
                }
                return self.save_curve();
            }
            Control::Discard => self.discard(),
            Control::Delete => {
                self.pending = Some(self.current().delete(&self.selected, self.track == 1)?);
                return self.save_curve();
            }
            Control::Stage(stage) => {
                self.retain_shader_text();
                self.stage = stage;
            }
            Control::StageSource(default) if self.stage < 2 => {
                self.retain_shader_text();
                if self.stage == 0 {
                    self.shader_draft.default_vertex = default;
                } else {
                    self.shader_draft.default_pixel = default;
                }
            }
            Control::Pipeline(index) => {
                self.menu = None;
                self.shader_draft.pipeline = [
                    ShaderPipeline::Sprite,
                    ShaderPipeline::Particles,
                    ShaderPipeline::Procedural,
                ][index];
            }
            Control::Compile => {
                self.sync_shader_text();
                let (program, settings) = self.shader_draft.compile()?;
                let mut config = self.config.clone();
                config.shader = Some(SavedShader {
                    draft: self.shader_draft.clone(),
                    program,
                    settings,
                });
                config.choice = Choice::Custom;
                return Ok(EditorEvent::Apply {
                    config,
                    reset_duration: false,
                });
            }
            Control::ResetShader => {
                self.edit = None;
                self.shader_draft = self.config.shader.as_ref().map_or_else(
                    || ShaderDraft::from_builtin(self.source),
                    |shader| shader.draft.clone(),
                );
            }
            _ => {}
        }
        Ok(if self.tab == Tab::Shader {
            EditorEvent::Pause
        } else {
            EditorEvent::Consumed
        })
    }

    pub fn event(&mut self, event: &Event) -> EditorEvent {
        match self.handle(event) {
            Ok(result) => result,
            Err(error) => {
                self.error(error);
                EditorEvent::Consumed
            }
        }
    }

    fn handle(&mut self, event: &Event) -> Result<EditorEvent, String> {
        self.handle_with_clipboard(event, &mut SystemClipboard)
    }

    fn handle_with_clipboard(
        &mut self,
        event: &Event,
        clipboard: &mut impl ClipboardAccess,
    ) -> Result<EditorEvent, String> {
        if let Event::Resize(..) = event {
            self.hits.clear();
            return Ok(EditorEvent::Consumed);
        }
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return Ok(EditorEvent::Consumed);
            }
            if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
                if key.kind != KeyEventKind::Press {
                    return Ok(EditorEvent::Consumed);
                }
                let index = match self.tab {
                    Tab::Main => 0,
                    Tab::Curves => 1,
                    Tab::Shader => 2,
                };
                let backward =
                    key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
                return self.activate(Control::Tab((index + if backward { 2 } else { 1 }) % 3));
            }
        }
        if self.menu.is_some() {
            if let Event::Key(key) = event {
                let count = self.menu_items().len();
                match key.code {
                    KeyCode::Esc => self.menu = None,
                    KeyCode::Up => self.menu_index = (self.menu_index + count - 1) % count,
                    KeyCode::Down => self.menu_index = (self.menu_index + 1) % count,
                    KeyCode::Enter => {
                        return self.activate(self.menu_items()[self.menu_index].1.clone());
                    }
                    _ => {}
                }
                return Ok(EditorEvent::Consumed);
            }
            if !matches!(event, Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left))
            {
                return Ok(EditorEvent::Consumed);
            }
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            let position = Position::new(mouse.column, mouse.row);
            let control = self
                .hits
                .iter()
                .rev()
                .find(|(area, _)| area.contains(position))
                .map(|(_, control)| control.clone());
            if self.menu.is_some()
                && !matches!(
                    control,
                    Some(Control::Shader(_) | Control::From(_) | Control::Pipeline(_))
                )
            {
                self.menu = None;
                return Ok(EditorEvent::Consumed);
            }
            if let Some(edit) = &mut self.edit
                && edit.area.contains(position)
            {
                if !mouse.modifiers.contains(KeyModifiers::SHIFT) {
                    edit.text.cancel_selection();
                } else if !edit.text.is_selecting() {
                    edit.text.start_selection();
                }
                edit.click(position);
                if !edit.text.is_selecting() {
                    edit.text.start_selection();
                }
                return Ok(EditorEvent::Consumed);
            }
            if !matches!(
                control,
                Some(Control::Save | Control::Compile | Control::Stage(_) | Control::StageSource(_))
            ) {
                self.edit = None;
            }
            if control == Some(Control::Graph) && self.config.custom_mode {
                self.pending = None;
                self.creating = false;
                let track = self.current();
                if let Some(indices) = self.markers(&track).get(&(mouse.column, mouse.row)) {
                    self.selected = indices.clone();
                } else {
                    let time = (mouse.column - self.graph.x) as f32
                        / self.graph.width.saturating_sub(1).max(1) as f32;
                    let value = self.range[1]
                        - (mouse.row - self.graph.y) as f32
                            / self.graph.height.saturating_sub(1).max(1) as f32
                            * (self.range[1] - self.range[0]);
                    let (pending, index) = track.insert([time, value], self.track == 1)?;
                    self.pending = Some(pending);
                    self.selected = vec![index];
                    self.creating = true;
                }
                return Ok(EditorEvent::Pause);
            }
            if self.creating
                && !matches!(
                    control,
                    Some(
                        Control::Save
                            | Control::Delete
                            | Control::Field(Field::Time | Field::Value)
                    )
                )
            {
                self.discard();
            }
            if let Some(control) = control {
                if let Control::Field(field) = control {
                    let area = self.hits.iter().find(|(_, control)| *control == Control::Field(field))
                        .map(|(area, _)| *area).unwrap();
                    self.begin_field(field);
                    if let Some(edit) = &mut self.edit {
                        edit.area = area.inner(ratatui::layout::Margin::new(1, 1));
                        edit.click(position);
                        edit.text.start_selection();
                    }
                    return Ok(EditorEvent::Pause);
                }
                return self.activate(control);
            }
            return Ok(if self.tab == Tab::Main {
                EditorEvent::Ignored
            } else {
                EditorEvent::Consumed
            });
        }
        if let Some(edit) = &mut self.edit {
            match event {
                Event::Key(key) if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c' | 'C')) =>
                {
                    if edit.has_selection() {
                        let mut selected = edit.text.clone();
                        selected.copy();
                        clipboard.write(selected.yank_text())?;
                    }
                }
                Event::Key(key) if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('v' | 'V')) =>
                {
                    if !edit.read_only {
                        edit.paste(&clipboard.read()?)?;
                    }
                }
                Event::Key(key) if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('a' | 'A')) =>
                {
                    edit.text.select_all();
                }
                Event::Key(key) if key.code == KeyCode::Esc => {
                    self.edit = None;
                }
                Event::Key(key)
                    if key.code == KeyCode::Enter
                        && matches!(edit.field, Field::Time | Field::Value) =>
                {
                    self.numeric_save()?
                }
                Event::Paste(text) => {
                    edit.paste(text)?;
                }
                Event::Key(key) => {
                    if !edit.read_only || matches!(key.code,
                        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
                        | KeyCode::Home | KeyCode::End | KeyCode::PageUp | KeyCode::PageDown)
                    {
                        edit.text.input(*key);
                    }
                }
                Event::Mouse(mouse) if mouse.kind == MouseEventKind::Drag(MouseButton::Left)
                    && !edit.area.is_empty() =>
                {
                    if !edit.text.is_selecting() {
                        edit.text.start_selection();
                    }
                    edit.click(Position::new(
                        mouse.column.clamp(edit.area.x, edit.area.right() - 1),
                        mouse.row.clamp(edit.area.y, edit.area.bottom() - 1),
                    ));
                }
                Event::Mouse(mouse)
                    if matches!(
                        mouse.kind,
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                    ) =>
                {
                    edit.text.scroll((
                        if mouse.kind == MouseEventKind::ScrollDown {
                            3
                        } else {
                            -3
                        },
                        0,
                    ));
                }
                _ => {}
            }
            return Ok(EditorEvent::Consumed);
        }
        if let Event::Key(key) = event {
            if self.tab == Tab::Main && key.code == KeyCode::Char('s') {
                return self.activate(Control::ShaderMenu);
            }
            if self.tab == Tab::Curves && self.config.custom_mode {
                if key.code == KeyCode::Esc {
                    self.discard();
                    return Ok(EditorEvent::Consumed);
                }
                if key.code == KeyCode::Enter && self.pending.is_some() {
                    return self.save_curve();
                }
                if !self.selected.is_empty() {
                    let fine = if key.modifiers.contains(KeyModifiers::SHIFT) {
                        0.001
                    } else {
                        0.01
                    };
                    let delta = match key.code {
                        KeyCode::Left => Some([-fine, 0.0]),
                        KeyCode::Right => Some([fine, 0.0]),
                        KeyCode::Up => Some([0.0, fine]),
                        KeyCode::Down => Some([0.0, -fine]),
                        _ => None,
                    };
                    if let Some(delta) = delta {
                        self.pending = Some(self.current().translated(
                            &self.selected,
                            delta,
                            self.track == 1,
                        )?);
                        return Ok(EditorEvent::Pause);
                    }
                    if key.code == KeyCode::Delete {
                        return self.activate(Control::Delete);
                    }
                }
            }
        }
        Ok(if self.tab == Tab::Main {
            EditorEvent::Ignored
        } else {
            EditorEvent::Consumed
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use ratatui::{Terminal, backend::TestBackend};

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[derive(Default)]
    struct TestClipboard {
        text: String,
        fail: bool,
        reads: usize,
        writes: usize,
    }

    impl ClipboardAccess for TestClipboard {
        fn read(&mut self) -> Result<String, String> {
            self.reads += 1;
            if self.fail { Err("Clipboard unavailable".into()) } else { Ok(self.text.clone()) }
        }

        fn write(&mut self, text: String) -> Result<(), String> {
            self.writes += 1;
            if self.fail { return Err("Clipboard unavailable".into()); }
            self.text = text;
            Ok(())
        }
    }

    fn control_key(character: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL))
    }

    #[test]
    fn clipboard_selection_and_paste_work_in_every_text_field() {
        for field in [Field::Json, Field::Time, Field::Value, Field::Vertex, Field::Pixel, Field::Settings] {
            let mut editor = Editor::new(None);
            apply(&mut editor, Control::Mode(true));
            editor.selected = vec![0];
            editor.shader_draft.default_vertex = false;
            editor.shader_draft.default_pixel = false;
            editor.begin_field(field);
            editor.edit.as_mut().unwrap().text = TextArea::from(["a\u{e9}\u{1f642}", "second"]);
            let mut clipboard = TestClipboard::default();
            assert!(editor.ctrl_c_exits(&control_key('c')));
            editor.edit.as_mut().unwrap().text.start_selection();
            assert!(editor.ctrl_c_exits(&control_key('c')));
            editor.handle_with_clipboard(&Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT)), &mut clipboard).unwrap();
            assert!(!editor.ctrl_c_exits(&control_key('c')));
            editor.handle_with_clipboard(&control_key('c'), &mut clipboard).unwrap();
            assert_eq!(clipboard.text, "a");
            assert!(editor.edit.as_ref().unwrap().has_selection());
            editor.handle_with_clipboard(&control_key('a'), &mut clipboard).unwrap();
            editor.handle_with_clipboard(&control_key('C'), &mut clipboard).unwrap();
            assert_eq!(clipboard.text, "a\u{e9}\u{1f642}\nsecond");
            assert_eq!(editor.edit.as_ref().unwrap().text.lines(), &["a\u{e9}\u{1f642}", "second"]);
            clipboard.text = "\u{3bb}\r\nnext\rfinal".into();
            editor.handle_with_clipboard(&control_key('v'), &mut clipboard).unwrap();
            assert_eq!(editor.edit.as_ref().unwrap().text.lines(), &["\u{3bb}", "next", "final"]);
            assert!(editor.ctrl_c_exits(&control_key('c')));
            editor.handle_with_clipboard(&control_key('a'), &mut clipboard).unwrap();
            editor.handle_with_clipboard(&Event::Paste("bracketed\r\npaste".into()), &mut clipboard).unwrap();
            assert_eq!(editor.edit.as_ref().unwrap().text.lines(), &["bracketed", "paste"]);
            assert_eq!((clipboard.reads, clipboard.writes), (1, 2));
            assert!(editor.config.shader.is_none());
        }
    }

    #[test]
    fn read_only_defaults_allow_copy_but_never_mutation_or_draft_sync() {
        for (field, stage, source) in [(Field::Vertex, 0, DEFAULT_VERTEX), (Field::Pixel, 1, DEFAULT_PIXEL)] {
            let mut editor = Editor::new(None);
            editor.tab = Tab::Shader;
            editor.stage = stage;
            editor.shader_draft.vertex = "retained vertex".into();
            editor.shader_draft.pixel = "retained pixel".into();
            editor.begin_field(field);
            let mut clipboard = TestClipboard::default();
            editor.handle_with_clipboard(&control_key('a'), &mut clipboard).unwrap();
            assert!(!editor.ctrl_c_exits(&control_key('c')));
            editor.handle_with_clipboard(&control_key('c'), &mut clipboard).unwrap();
            assert_eq!(clipboard.text, source.trim_end());
            let original = editor.edit.as_ref().unwrap().text.lines().to_vec();
            for event in [control_key('v'), control_key('x'), control_key('z'), control_key('k'), control_key('y'),
                key(KeyCode::Char('!')), key(KeyCode::Delete), key(KeyCode::Backspace), key(KeyCode::Enter),
                Event::Paste("bad".into())]
            {
                editor.handle_with_clipboard(&event, &mut clipboard).unwrap();
                assert_eq!(editor.edit.as_ref().unwrap().text.lines(), &original);
            }
            assert_eq!((clipboard.reads, clipboard.writes), (0, 1));
            editor.activate(Control::StageSource(false)).unwrap();
            assert_eq!(editor.shader_draft.vertex, "retained vertex");
            assert_eq!(editor.shader_draft.pixel, "retained pixel");
            editor.begin_field(field);
            assert!(!editor.edit.as_ref().unwrap().read_only);
        }
    }

    #[test]
    fn clipboard_failures_preserve_text_selection_and_exit_routing() {
        let mut editor = Editor::new(None);
        let mut clipboard = TestClipboard { fail: true, ..Default::default() };
        assert!(editor.ctrl_c_exits(&control_key('c')));
        assert!(!editor.ctrl_c_exits(&Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('c'), KeyModifiers::CONTROL, KeyEventKind::Release))));
        editor.begin_field(Field::Settings);
        editor.handle_with_clipboard(&control_key('a'), &mut clipboard).unwrap();
        let original = editor.edit.as_ref().unwrap().text.lines().to_vec();
        for event in [control_key('c'), control_key('v')] {
            assert!(editor.handle_with_clipboard(&event, &mut clipboard).is_err());
            assert_eq!(editor.edit.as_ref().unwrap().text.lines(), &original);
            assert!(!editor.ctrl_c_exits(&control_key('c')));
        }
        assert!(editor.handle_with_clipboard(&Event::Paste("x".repeat(2 * 1024 * 1024 + 1)), &mut clipboard).is_err());
        assert_eq!(editor.edit.as_ref().unwrap().text.lines(), &original);
        editor.handle_with_clipboard(&key(KeyCode::Esc), &mut clipboard).unwrap();
        assert!(editor.ctrl_c_exits(&control_key('c')));
    }

    #[test]
    fn mouse_drag_selects_text_and_plain_click_clears_selection() {
        let mut editor = Editor::new(None);
        editor.tab = Tab::Shader;
        editor.stage = 2;
        editor.shader_draft.settings = "abcdef".into();
        render(&mut editor, 80, 24);
        let mouse = |kind, column, modifiers| Event::Mouse(crossterm::event::MouseEvent {
            kind, column, row: 8, modifiers,
        });
        let mut clipboard = TestClipboard::default();
        editor.handle_with_clipboard(&mouse(MouseEventKind::Down(MouseButton::Left), 3, KeyModifiers::NONE), &mut clipboard).unwrap();
        editor.handle_with_clipboard(&mouse(MouseEventKind::Drag(MouseButton::Left), 6, KeyModifiers::NONE), &mut clipboard).unwrap();
        editor.handle_with_clipboard(&control_key('c'), &mut clipboard).unwrap();
        assert_eq!(clipboard.text, "bcd");
        editor.handle_with_clipboard(&mouse(MouseEventKind::Down(MouseButton::Left), 4, KeyModifiers::NONE), &mut clipboard).unwrap();
        assert!(editor.ctrl_c_exits(&control_key('c')));
        editor.handle_with_clipboard(&mouse(MouseEventKind::Down(MouseButton::Left), 6, KeyModifiers::SHIFT), &mut clipboard).unwrap();
        editor.handle_with_clipboard(&control_key('c'), &mut clipboard).unwrap();
        assert_eq!(clipboard.text, "cd");
    }

    fn render(editor: &mut Editor, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                editor.draw(
                    frame,
                    MainView {
                        position: 0.5,
                        speed: 1.0,
                        destination: 1.0,
                        state: "Paused",
                        summary: "10 icons",
                        visible: false,
                        help: false,
                        prompt: "",
                    },
                )
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn apply(editor: &mut Editor, control: Control) -> bool {
        let EditorEvent::Apply {
            config,
            reset_duration,
        } = editor.activate(control).unwrap()
        else {
            panic!("Expected configuration")
        };
        editor.committed(config);
        reset_duration
    }

    #[test]
    fn layouts_keep_controls_inside_terminal() {
        let mut editor = Editor::new(Some(BuiltinShader::ParticleVortex));
        for (width, height) in [(42, 24), (80, 24), (120, 40)] {
            for tab in [Tab::Main, Tab::Curves, Tab::Shader] {
                editor.tab = tab;
                render(&mut editor, width, height);
                for (area, _) in &editor.hits {
                    assert!(area.right() <= width && area.bottom() < height, "{area:?}");
                }
                if tab == Tab::Curves {
                    assert_eq!(editor.graph.height, 9.min(height - 16) - 2);
                }
            }
        }
        render(&mut editor, 20, 10);
        assert!(editor.hits.is_empty());
    }

    #[test]
    fn curve_axes_colors_and_compact_json() {
        let mut editor = Editor::new(None);
        apply(&mut editor, Control::Mode(true));
        for width in [42, 80, 120] {
            let buffer = render(&mut editor, width, 24);
            let row_text = |row| {
                (0..width)
                    .map(|column| buffer[(column, row)].symbol())
                    .collect::<String>()
            };
            assert!(row_text(2).contains("Preset:"));
            assert!(row_text(3).contains("Track:"));
            assert!(row_text(15).ends_with(" time(0..1) \u{2518} "));
            assert!(row_text(6).trim().is_empty());
            assert_eq!(buffer[(1, 8)].symbol(), "\u{250c}");
            assert_eq!(buffer[(1, 15)].symbol(), "\u{2514}");
            assert_eq!(buffer[(1, 2)].fg, Color::Rgb(210, 185, 125));
            let primary = editor
                .hits
                .iter()
                .find(|(_, control)| *control == Control::Tab(1))
                .unwrap()
                .0;
            let nested = editor
                .hits
                .iter()
                .find(|(_, control)| *control == Control::Track(0))
                .unwrap()
                .0;
            assert_eq!(buffer[(primary.x, primary.y)].bg, Color::Rgb(180, 130, 255));
            assert_eq!(buffer[(nested.x, nested.y)].bg, Color::Rgb(100, 65, 150));
            let axes: Vec<_> = editor
                .hits
                .iter()
                .filter(|(_, control)| *control == Control::Range)
                .collect();
            assert_eq!(axes.len(), 1);
            assert_eq!(
                axes[0].0,
                Rect::new(editor.graph.x - 1, editor.graph.y - 1, 1, 8)
            );
            assert_eq!(buffer[(1, 7)].symbol(), "1");
            assert_eq!(buffer[(1, 16)].symbol(), "0");
        }
        let original = editor.config.tracks.clone();
        let click = Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 10,
            modifiers: KeyModifiers::NONE,
        });
        assert!(matches!(editor.event(&click), EditorEvent::Consumed));
        assert_eq!(editor.range, [-1.0, 2.0]);
        let buffer = render(&mut editor, 42, 24);
        assert_eq!(buffer[(1, 7)].symbol(), "2");
        assert_eq!(buffer[(1, 16)].symbol(), "-");
        assert_eq!(editor.config.tracks, original);
        assert!(editor.pending.is_none());
        editor.begin_field(Field::Json);
        let text = editor.edit.as_ref().unwrap().text.lines().join("\n");
        assert_eq!(text.lines().count(), 6);
        assert!(text.lines().nth(3).unwrap().starts_with("[0.0,0.0], ["));
        assert_eq!(Track::parse(&text, false).unwrap(), editor.current());
        editor.activate(Control::Track(1)).unwrap();
        render(&mut editor, 42, 24);
        editor.event(&click);
        assert_eq!(editor.range, [0.0, 1.0]);
    }

    #[test]
    fn pressed_controls_keep_old_state_until_commit_and_clear_after_failure() {
        let mut editor = Editor::new(None);
        editor.tab = Tab::Curves;
        render(&mut editor, 80, 24);
        let click = |area: Rect| {
            Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            })
        };
        let custom = editor
            .hits
            .iter()
            .find(|(_, control)| *control == Control::Mode(true))
            .unwrap()
            .0;
        let event = click(custom);
        assert!(editor.begin_action(&event));
        let buffer = render(&mut editor, 80, 24);
        assert_eq!(buffer[(custom.x, custom.y)].bg, PRESSED_COLOR);
        assert!(!editor.config.custom_mode);
        let EditorEvent::Apply { config, .. } = editor.event(&event) else {
            panic!("Expected configuration");
        };
        assert!(!editor.config.custom_mode);
        assert!(editor.pressed.is_some());
        editor.committed(config);
        editor.finish_action();
        let buffer = render(&mut editor, 80, 24);
        assert_eq!(buffer[(custom.x, custom.y)].bg, Color::Cyan);
        assert!(editor.config.custom_mode);

        let axis = editor
            .hits
            .iter()
            .find(|(_, control)| *control == Control::Range)
            .unwrap()
            .0;
        assert!(editor.begin_action(&click(axis)));
        assert_eq!(
            render(&mut editor, 80, 24)[(axis.x, axis.y)].fg,
            PRESSED_COLOR
        );
        assert_eq!(editor.range, [0.0, 1.0]);
        editor.event(&click(axis));
        editor.finish_action();
        assert_eq!(
            render(&mut editor, 80, 24)[(axis.x, axis.y)].fg,
            Color::Cyan
        );
        assert_eq!(editor.range, [-1.0, 2.0]);

        editor.tab = Tab::Shader;
        editor.shader_draft.default_pixel = false;
        editor.begin_field(Field::Pixel);
        editor.edit.as_mut().unwrap().text = TextArea::from(["invalid shader"]);
        render(&mut editor, 80, 24);
        let compile = editor
            .hits
            .iter()
            .find(|(_, control)| *control == Control::Compile)
            .unwrap()
            .0;
        assert!(editor.begin_action(&click(compile)));
        assert_eq!(
            render(&mut editor, 80, 24)[(compile.x, compile.y)].bg,
            PRESSED_COLOR
        );
        assert!(matches!(
            editor.event(&click(compile)),
            EditorEvent::Consumed
        ));
        assert!(editor.config.shader.is_none());
        assert!(!editor.message.is_empty());
        assert!(editor.edit.is_some());
        editor.finish_action();
        assert_eq!(
            render(&mut editor, 80, 24)[(compile.x, compile.y)].bg,
            Color::Reset
        );
    }

    #[test]
    fn pressed_menu_choices_exclude_dismissal_clicks_and_text_input() {
        let mut editor = Editor::new(None);
        editor.activate(Control::ShaderMenu).unwrap();
        render(&mut editor, 80, 24);
        let outside = Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert!(!editor.begin_action(&outside));
        let enter = key(KeyCode::Enter);
        assert!(editor.begin_action(&enter));
        assert_eq!(render(&mut editor, 80, 24)[(3, 6)].bg, PRESSED_COLOR);
        editor.event(&enter);
        editor.finish_action();
        assert!(editor.menu.is_none());
        assert!(!editor.begin_action(&key(KeyCode::Char('s'))));
    }

    #[test]
    fn dropdown_blocks_playback_mouse_events() {
        let mut editor = Editor::new(None);
        editor.activate(Control::ShaderMenu).unwrap();
        for kind in [
            MouseEventKind::ScrollDown,
            MouseEventKind::ScrollUp,
            MouseEventKind::Drag(MouseButton::Left),
        ] {
            let event = Event::Mouse(crossterm::event::MouseEvent {
                kind,
                column: 3,
                row: 6,
                modifiers: KeyModifiers::NONE,
            });
            assert!(matches!(editor.event(&event), EditorEvent::Consumed));
            assert!(editor.menu.is_some());
        }
    }

    #[test]
    fn modes_preserve_single_custom_instance_and_duration_policy() {
        let mut editor = Editor::new(Some(BuiltinShader::Glitch));
        assert!(!apply(&mut editor, Control::Mode(true)));
        assert_eq!(editor.tab, Tab::Curves);
        let tracks = editor.config.tracks.clone();
        assert!(apply(
            &mut editor,
            Control::Shader(Choice::Builtin(BuiltinShader::SilkFlow))
        ));
        assert!(editor.config.custom_mode);
        assert_eq!(editor.config.seconds, 5.0);
        assert_eq!(editor.config.tracks, tracks);
        assert!(!apply(&mut editor, Control::Mode(false)));
        assert_eq!(editor.config.tracks, tracks);
        assert!(!apply(&mut editor, Control::Mode(true)));
        assert_eq!(editor.config.tracks, tracks);
        assert!(matches!(
            editor.activate(Control::Shader(Choice::Custom)).unwrap(),
            EditorEvent::Pause
        ));
        assert_eq!(editor.tab, Tab::Shader);
    }

    #[test]
    fn numeric_focus_and_json_save_are_transactional() {
        let mut editor = Editor::new(None);
        apply(&mut editor, Control::Mode(true));
        editor.config.tracks.as_mut().unwrap()[0].keyframes[1][0] = 0.0123456;
        editor.selected = vec![1];
        let original = editor.config.tracks.clone();
        editor.begin_field(Field::Time);
        assert!(matches!(
            editor.event(&key(KeyCode::Enter)),
            EditorEvent::Consumed
        ));
        assert!(editor.pending.is_none());
        assert_eq!(editor.config.tracks, original);
        editor.begin_field(Field::Json);
        editor.edit.as_mut().unwrap().text = TextArea::from(["invalid"]);
        assert!(matches!(editor.activate(Control::Save), Err(_)));
        assert!(editor.edit.is_some());
        assert_eq!(editor.config.tracks, original);
        assert!(matches!(
            editor.event(&key(KeyCode::Enter)),
            EditorEvent::Consumed
        ));
        assert!(editor.edit.is_some());
        editor.event(&key(KeyCode::Esc));
        assert!(editor.edit.is_none());
        assert_eq!(editor.config.tracks, original);
    }

    #[test]
    fn graph_mouse_pending_acceptance_and_outside_discard() {
        let mut editor = Editor::new(None);
        apply(&mut editor, Control::Mode(true));
        editor.config.tracks.as_mut().unwrap()[0] = Track {
            interp: "linear".into(),
            keyframes: vec![[0.0, 0.0], [1.0, 1.0]],
        };
        render(&mut editor, 80, 24);
        let mouse = |column, row| {
            Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
        };
        assert!(matches!(editor.event(&mouse(35, 10)), EditorEvent::Pause));
        assert!(editor.creating);
        assert_eq!(editor.config.track(0).keyframes.len(), 2);
        assert_eq!(editor.current().keyframes.len(), 3);
        editor.event(&mouse(79, 22));
        assert!(editor.pending.is_none());
        editor.event(&mouse(35, 10));
        let EditorEvent::Apply { config, .. } = editor.event(&key(KeyCode::Enter)) else {
            panic!("Expected accepted point")
        };
        editor.committed(config);
        assert_eq!(editor.config.track(0).keyframes.len(), 3);
    }

    #[test]
    fn interpolation_is_a_draft_and_scrolled_mouse_uses_viewport() {
        let mut editor = Editor::new(None);
        apply(&mut editor, Control::Mode(true));
        let original = editor.config.tracks.clone();
        assert!(matches!(
            editor.activate(Control::Interp).unwrap(),
            EditorEvent::Pause
        ));
        assert_eq!(editor.config.tracks, original);
        assert!(editor.pending.is_some());
        render(&mut editor, 42, 24);
        assert!(
            editor
                .hits
                .iter()
                .any(|(_, control)| *control == Control::Save)
        );
        editor.activate(Control::Discard).unwrap();
        editor.begin_field(Field::Json);
        editor.edit.as_mut().unwrap().text = (0..40).map(|index| format!("line {index}")).collect();
        render(&mut editor, 42, 24);
        editor.edit.as_mut().unwrap().text.scroll((15, 0));
        render(&mut editor, 42, 24);
        let edit = editor.edit.as_mut().unwrap();
        edit.click(Position::new(edit.area.x + 2, edit.area.y));
        assert_eq!(edit.text.cursor(), (15, 2));
    }

    #[test]
    fn shader_failure_retains_focus_and_stage_switch_retains_text() {
        let mut editor = Editor::new(None);
        editor.tab = Tab::Shader;
        editor.shader_draft.default_pixel = false;
        editor.begin_field(Field::Pixel);
        editor.edit.as_mut().unwrap().text = TextArea::from(["invalid shader"]);
        assert!(editor.activate(Control::Compile).is_err());
        assert!(editor.edit.is_some());
        assert!(editor.config.shader.is_none());
        editor.activate(Control::Stage(0)).unwrap();
        assert_eq!(editor.shader_draft.pixel, "invalid shader");
        editor.activate(Control::StageSource(false)).unwrap();
        editor.begin_field(Field::Vertex);
        assert!(matches!(
            editor.event(&key(KeyCode::Char('q'))),
            EditorEvent::Consumed
        ));
        assert!(matches!(
            editor.event(&key(KeyCode::Tab)),
            EditorEvent::Consumed
        ));
        assert_eq!(editor.tab, Tab::Main);
    }

    #[test]
    fn shader_source_modes_preserve_drafts_and_defaults_are_read_only() {
        let mut editor = Editor::new(Some(BuiltinShader::Glitch));
        editor.tab = Tab::Shader;
        editor.activate(Control::Stage(1)).unwrap();
        let original_program = editor.config.program().unwrap();
        let original_settings = editor.shader_draft.settings.clone();
        editor.begin_field(Field::Pixel);
        editor.edit.as_mut().unwrap().text = TextArea::from(["invalid custom pixel"]);
        render(&mut editor, 42, 24);
        let default_button = editor.hits.iter()
            .find(|(_, control)| *control == Control::StageSource(true)).unwrap().0;
        let click = |column, row| Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column, row, modifiers: KeyModifiers::NONE,
        });
        assert!(matches!(editor.event(&click(default_button.x, default_button.y)), EditorEvent::Pause));
        assert!(editor.shader_draft.default_pixel);
        assert_eq!(editor.shader_draft.pixel, "invalid custom pixel");
        assert!(editor.edit.is_none());
        assert_eq!(editor.config.program().unwrap(), original_program);
        for width in [42, 80, 120] {
            let buffer = render(&mut editor, width, 24);
            let row_text = |row| (0..width).map(|column| buffer[(column, row)].symbol()).collect::<String>();
            assert!(row_text(5).contains("Source: "));
            assert!(row_text(5).contains("Pipeline default"));
            assert!(row_text(5).contains("Custom HLSL"));
            assert!(row_text(7).contains("read-only"));
            let source_text: String = (8..14).flat_map(|row| (2..width - 2).map(move |column| (column, row)))
                .map(|position| buffer[position].symbol()).collect::<String>()
                .chars().filter(|character| !character.is_whitespace()).collect();
            assert!(source_text.contains("returndefault_pixel(input);"));
            assert!(editor.hits.iter().any(|(_, control)| *control == Control::Field(Field::Pixel)));
            assert!(editor.hits.iter().all(|(area, _)| area.right() <= width));
        }
        editor.event(&click(3, 9));
        editor.begin_field(Field::Pixel);
        editor.event(&Event::Paste("accidental paste".into()));
        editor.event(&key(KeyCode::Char('x')));
        assert!(editor.edit.as_ref().unwrap().read_only);
        assert_eq!(editor.edit.as_ref().unwrap().text.lines().join("\n"), DEFAULT_PIXEL.trim_end());
        assert_eq!(editor.shader_draft.pixel, "invalid custom pixel");
        let EditorEvent::Apply { config, .. } = editor.activate(Control::Compile).unwrap() else {
            panic!("Expected compiled default shader");
        };
        assert_eq!(editor.config.program().unwrap(), original_program);
        editor.committed(config);
        let saved_program = editor.config.program().unwrap();
        assert_eq!(saved_program, Some(shader::compile(BuiltinShader::Identity).unwrap()));
        editor.activate(Control::StageSource(false)).unwrap();
        editor.begin_field(Field::Pixel);
        assert_eq!(editor.edit.as_ref().unwrap().text.lines(), &["invalid custom pixel"]);
        assert!(editor.activate(Control::Compile).is_err());
        assert!(editor.edit.is_some());
        assert_eq!(editor.config.program().unwrap(), saved_program);
        editor.activate(Control::Stage(0)).unwrap();
        assert!(editor.shader_draft.default_vertex);
        editor.activate(Control::StageSource(false)).unwrap();
        editor.begin_field(Field::Vertex);
        editor.edit.as_mut().unwrap().text = TextArea::from(["custom vertex draft"]);
        editor.activate(Control::StageSource(true)).unwrap();
        assert_eq!(editor.shader_draft.vertex, "custom vertex draft");
        assert!(!editor.shader_draft.default_pixel);
        editor.activate(Control::StageSource(false)).unwrap();
        editor.begin_field(Field::Vertex);
        assert_eq!(editor.edit.as_ref().unwrap().text.lines(), &["custom vertex draft"]);
        editor.activate(Control::Stage(2)).unwrap();
        render(&mut editor, 42, 24);
        assert!(!editor.hits.iter().any(|(_, control)| matches!(control, Control::StageSource(_))));
        assert_eq!(editor.shader_draft.settings, original_settings);
        editor.begin_field(Field::Settings);
        assert!(editor.edit.is_some());
    }

    #[test]
    fn collisions_keep_all_keys_and_selection_color() {
        let mut editor = Editor::new(None);
        apply(&mut editor, Control::Mode(true));
        editor.config.tracks.as_mut().unwrap()[0] = Track {
            interp: "linear".into(),
            keyframes: vec![
                [0.0, 0.0],
                [0.499, 0.5],
                [0.5, 0.5],
                [0.501, 0.5],
                [1.0, 1.0],
            ],
        };
        render(&mut editor, 42, 24);
        let groups = editor.markers(&editor.current());
        let (&(column, row), indices) = groups
            .iter()
            .find(|(_, indices)| indices.len() >= 2)
            .unwrap();
        let buffer = render(&mut editor, 42, 24);
        assert_eq!(buffer[(column, row)].fg, Color::Yellow);
        editor.selected = indices.clone();
        let buffer = render(&mut editor, 42, 24);
        assert_eq!(buffer[(column, row)].fg, Color::Blue);
        assert_eq!(editor.current().keyframes.len(), 5);
    }
}
