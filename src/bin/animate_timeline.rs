#[cfg(windows)]
#[path = "../demo_animation.rs"]
mod demo_animation;

#[cfg(windows)]
#[path = "../timeline_editor/mod.rs"]
mod timeline_editor;

#[cfg(windows)]
mod app {
    use crate::timeline_editor::{Config, Editor, EditorEvent, MainView};
    use ratatui::{Terminal, backend::CrosstermBackend};
    use std::error::Error;
    use std::io::{self, IsTerminal};
    #[cfg(test)]
    use std::io::Write;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    use crossterm::{
        cursor,
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
            KeyModifiers, MouseButton, MouseEventKind,
        },
        execute,
        style::ResetColor,
        terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    };
    #[cfg(test)]
    use crossterm::{queue, style::{Color, Print, SetBackgroundColor, SetForegroundColor}, terminal::{Clear, ClearType}};
    use rdi_core::{
        AnimationOptions, DesktopController, FinishReason, PlaybackHandle, PlaybackOutcome,
        TimelineCloseMode, TimelineSession, TimelineState,
    };
    use rdi_platform_windows::{WindowsBackend, shader};
    use windows::Win32::UI::Shell::FWF_AUTOARRANGE;

    type Result<T> = std::result::Result<T, Box<dyn Error>>;
    const MAX_SPEED: f64 = 50.0;
    pub(crate) const DURATION_STEPS: [f64; 26] = [
        0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0,
        1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9, 2.0,
        2.5, 3.0, 3.5, 4.0, 4.5, 5.0,
    ];
    static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);
    const HELP: &str = "animate_timeline [--shader NAME | --no-shader] [--dry-run] [--log-file PATH]\n\
Reserved grid moves at 144 Hz. Starts by playing to 0.5.\n\
When free destinations run out, skipped icons exchange positions on the same monitor.\n\
Default: two-second glitch. Particle vortex: four seconds. Disable Auto arrange icons first.\n\
Detailed help starts hidden; ?: toggle help; : shows help during command entry.\n\
Left/Right: seek 1%; Shift+arrows: 10%; Home/End: seek 0/1.\n\
Space: pause/resume; f/r/m: play to 1/0/0.5; Up/Down or +/-: speed; 1: reset.\n\
h: show/hide REAL desktop icons (overlay unchanged; hidden at startup).\n\
s: shader dropdown; arrows/Enter select, Esc cancels; mouse selection supported.\n\
Switching rebuilds the overlay at the same position; a brief desktop reveal is possible.\n\
Mouse: first slider seeks; second sets duration: 0.1-2s in 0.1s steps, then 2.5-5s in 0.5s steps.\n\
Wheel over duration steps through values. Duration changes speed without pausing (up to 50x).\n\
Tab/Shift+Tab or mouse: Main, Curves, Shader Editor. Built-in changes apply preset duration.\n\
Curves: Movement/Envelope, graph and JSON; Custom values survive shader changes.\n\
Shader Editor: trusted HLSL only; Compile & Apply replaces the single custom shader.\n\
Colon: command entry (play POSITION [SPEED], seek POSITION, speed RATE, pause,\n\
close restore|current|target). q or Ctrl+C: close and restore original positions.\n\
--dry-run only prints destinations, without moving icons or opening the TUI.";

    #[derive(Default)]
    struct Args {
        no_shader: bool,
        shader: Option<shader::BuiltinShader>,
        dry_run: bool,
        help: bool,
        log_file: Option<String>,
    }

    impl Args {
        fn selected_shader(&self) -> Option<shader::BuiltinShader> {
            if self.no_shader {
                None
            } else {
                Some(self.shader.unwrap_or(shader::BuiltinShader::Glitch))
            }
        }

        fn parse(arguments: impl IntoIterator<Item = String>) -> std::result::Result<Self, String> {
            let mut args = Self::default();
            let mut arguments = arguments.into_iter();
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--no-shader" => args.no_shader = true,
                    "--shader" => {
                        let name = arguments.next().ok_or_else(|| {
                            format!("--shader requires one of: {}", shader_names())
                        })?;
                        args.shader = Some(
                            name.parse()
                                .map_err(|error| format!("{error}; choose {}", shader_names()))?,
                        );
                    }
                    "--dry-run" => args.dry_run = true,
                    "--help" | "-h" => args.help = true,
                    "--log-file" => {
                        args.log_file = Some(arguments.next().ok_or("--log-file requires a path")?)
                    }
                    _ => return Err(format!("unknown argument: {argument}; use --help")),
                }
            }
            if args.no_shader && args.shader.is_some() {
                return Err("--shader and --no-shader cannot be combined".into());
            }
            Ok(args)
        }
    }

    fn shader_names() -> String {
        shader::BuiltinShader::ALL
            .iter()
            .map(|shader| shader.name())
            .collect::<Vec<_>>()
            .join(", ")
    }

    #[cfg(test)]
    #[derive(Debug, PartialEq)]
    enum PickerEvent {
        Ignored,
        Consumed,
        Select(Option<shader::BuiltinShader>),
    }

    #[cfg(test)]
    struct ShaderPicker {
        selected: Option<shader::BuiltinShader>,
        highlighted: Option<usize>,
    }

    #[cfg(test)]
    impl ShaderPicker {
        fn choice(index: usize) -> Option<shader::BuiltinShader> {
            index
                .checked_sub(1)
                .map(|index| shader::BuiltinShader::ALL[index])
        }

        fn name(choice: Option<shader::BuiltinShader>) -> &'static str {
            choice.map_or("off", shader::BuiltinShader::name)
        }

        fn dimensions(size: (u16, u16)) -> Option<(u16, usize)> {
            if size.0 < 12 || size.1 < 6 {
                return None;
            }
            let longest = shader::BuiltinShader::ALL
                .iter()
                .map(|shader| shader.name().len())
                .max()
                .unwrap_or(3)
                .max(3);
            Some((
                (longest + 13).min(usize::from(size.0 - 1)) as u16,
                usize::from(size.1 - 4).min(shader::BuiltinShader::ALL.len() + 1),
            ))
        }

        fn first_visible(&self, capacity: usize) -> usize {
            self.highlighted.unwrap_or(0).saturating_sub(capacity - 1)
        }

        fn event(&mut self, event: &Event, size: (u16, u16)) -> PickerEvent {
            let Some((width, capacity)) = Self::dimensions(size) else {
                self.highlighted = None;
                return PickerEvent::Ignored;
            };
            if matches!(event, Event::Resize(..)) {
                self.highlighted = None;
                return PickerEvent::Ignored;
            }
            if let Event::Key(key) = event {
                if key.kind == KeyEventKind::Release {
                    return PickerEvent::Ignored;
                }
                if key.code == KeyCode::Char('q')
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    return PickerEvent::Ignored;
                }
            }
            if let Some(highlighted) = self.highlighted {
                let last = shader::BuiltinShader::ALL.len();
                match event {
                    Event::Key(key) => match key.code {
                        KeyCode::Up => self.highlighted = Some(highlighted.saturating_sub(1)),
                        KeyCode::Down => self.highlighted = Some((highlighted + 1).min(last)),
                        KeyCode::Home => self.highlighted = Some(0),
                        KeyCode::End => self.highlighted = Some(last),
                        KeyCode::Esc => self.highlighted = None,
                        KeyCode::Char('s') if key.kind == KeyEventKind::Press => {
                            self.highlighted = None
                        }
                        KeyCode::Enter if key.kind == KeyEventKind::Press => {
                            self.highlighted = None;
                            return PickerEvent::Select(Self::choice(highlighted));
                        }
                        _ => {}
                    },
                    Event::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            self.highlighted = Some(highlighted.saturating_sub(1))
                        }
                        MouseEventKind::ScrollDown => {
                            self.highlighted = Some((highlighted + 1).min(last))
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            let first = self.first_visible(capacity);
                            self.highlighted = None;
                            if mouse.column < width
                                && mouse.row >= 3
                                && usize::from(mouse.row - 3) < capacity
                            {
                                return PickerEvent::Select(Self::choice(
                                    first + usize::from(mouse.row - 3),
                                ));
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
                return PickerEvent::Consumed;
            }
            let open = match event {
                Event::Key(key) => {
                    key.code == KeyCode::Char('s')
                        && key.kind == KeyEventKind::Press
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                }
                Event::Mouse(mouse) => {
                    mouse.kind == MouseEventKind::Down(MouseButton::Left)
                        && mouse.row == 2
                        && mouse.column < width
                }
                _ => false,
            };
            if open {
                self.highlighted = Some(self.selected.map_or(0, |selected| {
                    shader::BuiltinShader::ALL
                        .iter()
                        .position(|&shader| shader == selected)
                        .unwrap()
                        + 1
                }));
                PickerEvent::Consumed
            } else {
                PickerEvent::Ignored
            }
        }

        fn draw(&self, output: &mut impl Write, size: (u16, u16)) -> io::Result<()> {
            let Some((width, capacity)) = Self::dimensions(size) else {
                return Ok(());
            };
            line(
                output,
                2,
                size.0,
                size.1,
                &format!("[shader: {} v]", Self::name(self.selected)),
            )?;
            if let Some(highlighted) = self.highlighted {
                let first = self.first_visible(capacity);
                for index in first..first + capacity {
                    let choice = Self::choice(index);
                    let text = format!(
                        "{} {} {}",
                        if index == highlighted { ">" } else { " " },
                        if choice == self.selected { "*" } else { " " },
                        Self::name(choice)
                    );
                    let text: String = text.chars().take(usize::from(width)).collect();
                    queue!(
                        output,
                        cursor::MoveTo(0, 3 + (index - first) as u16),
                        SetBackgroundColor(if index == highlighted {
                            Color::DarkCyan
                        } else {
                            Color::Black
                        }),
                        SetForegroundColor(Color::White),
                        Print(format!("{text:<width$}", width = usize::from(width))),
                        ResetColor
                    )?;
                }
            }
            Ok(())
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Action {
        Seek(f64),
        Play(f64, f64),
        Speed(f64),
        Pause,
        Toggle,
        ToggleRealIcons,
        Close(TimelineCloseMode),
    }

    fn number(text: &str, maximum: f64, allow_zero: bool) -> std::result::Result<f64, String> {
        let value: f64 = text.parse().map_err(|_| "Expected a number".to_string())?;
        if !value.is_finite() || value < 0.0 || (!allow_zero && value == 0.0) || value > maximum {
            return Err(if allow_zero {
                "Position must be in [0, 1]"
            } else {
                "Speed must be > 0 and <= 50x"
            }
            .into());
        }
        Ok(value)
    }

    fn command(text: &str, speed: f64) -> std::result::Result<Action, String> {
        let parts: Vec<_> = text.split_whitespace().collect();
        match parts.as_slice() {
            ["play", position] => Ok(Action::Play(number(position, 1.0, true)?, speed)),
            ["play", position, rate] => Ok(Action::Play(number(position, 1.0, true)?, number(rate, MAX_SPEED, false)?)),
            ["seek", position] => Ok(Action::Seek(number(position, 1.0, true)?)),
            ["speed", rate] => Ok(Action::Speed(number(rate, MAX_SPEED, false)?)),
            ["pause"] => Ok(Action::Pause),
            ["close", "restore"] => Ok(Action::Close(TimelineCloseMode::RestoreOrigins)),
            ["close", "current"] => Ok(Action::Close(TimelineCloseMode::LeaveInPlace)),
            ["close", "target"] => Ok(Action::Close(TimelineCloseMode::TeleportToTarget)),
            _ => Err("Use play POSITION [SPEED], seek POSITION, speed RATE, pause, close restore|current|target".into()),
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct Slider {
        left: u16,
        right: u16,
        row: u16,
    }

    impl Slider {
        fn layout(width: u16, height: u16) -> Option<Self> {
            (width >= 12 && height >= 6).then(|| Self {
                left: 2,
                right: width - 4,
                row: 3,
            })
        }

        fn position(self, column: u16) -> f64 {
            f64::from(column.clamp(self.left, self.right) - self.left)
                / f64::from(self.right - self.left)
        }

        #[cfg(test)]
        fn column(self, position: f64) -> u16 {
            self.left
                + (position.clamp(0.0, 1.0) * f64::from(self.right - self.left)).round() as u16
        }

        fn duration_layout(width: u16, height: u16) -> Option<Self> {
            Self::layout(width, height).filter(|_| height >= 9).map(|slider| Self {
                row: 6,
                right: slider.right.min(slider.left + DURATION_STEPS.len() as u16 - 1),
                ..slider
            })
        }

        fn duration_speed(self, column: u16, base: f64) -> f64 {
            let index = (self.position(column) * (DURATION_STEPS.len() - 1) as f64).round() as usize;
            base / DURATION_STEPS[index]
        }

        fn duration_index(seconds: f64) -> usize {
            DURATION_STEPS.iter().enumerate()
                .min_by(|(_, first), (_, second)| {
                    (**first - seconds).abs().total_cmp(&(**second - seconds).abs())
                })
                .map(|(index, _)| index)
                .unwrap_or(0)
        }

        #[cfg(test)]
        fn duration_position(seconds: f64) -> f64 {
            Self::duration_index(seconds) as f64 / (DURATION_STEPS.len() - 1) as f64
        }

        #[cfg(test)]
        fn draw(self, output: &mut impl Write, position: f64, color: Color) -> io::Result<()> {
            let column = self.column(position);
            let track: String = (self.left..=self.right)
                .map(|cell| if cell == column { 'O' } else if cell < column { '=' } else { '-' })
                .collect();
            queue!(output, cursor::MoveTo(self.left - 1, self.row),
                SetForegroundColor(color), Print(format!("[{track}]")), ResetColor)
        }
    }

    #[derive(Default)]
    struct Input {
        command: Option<String>,
        dragging: bool,
        duration_dragging: bool,
        message: String,
        help_open: bool,
    }

    impl Input {
        fn help_visible(&self) -> bool {
            self.help_open || self.command.is_some()
        }

        #[cfg(test)]
        fn picker_event(
            &mut self,
            picker: &mut ShaderPicker,
            event: &Event,
            size: (u16, u16),
        ) -> PickerEvent {
            if self.command.is_some() {
                return PickerEvent::Ignored;
            }
            let result = picker.event(event, size);
            if result != PickerEvent::Ignored {
                self.dragging = false;
                self.duration_dragging = false;
            }
            result
        }

        fn duration_event(&mut self, event: &Event, size: (u16, u16), base: f64, speed: f64) -> Option<Action> {
            let slider = Slider::duration_layout(size.0, size.1);
            if self.command.is_some() || slider.is_none() || matches!(event, Event::Resize(..)) {
                self.duration_dragging = false;
                return None;
            }
            let slider = slider.unwrap();
            match event {
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        self.duration_dragging = mouse.row == slider.row
                            && (slider.left..=slider.right).contains(&mouse.column);
                        if self.duration_dragging {
                            self.dragging = false;
                            return Some(Action::Speed(slider.duration_speed(mouse.column, base)));
                        }
                    }
                    MouseEventKind::Drag(MouseButton::Left) if self.duration_dragging => {
                        return Some(Action::Speed(slider.duration_speed(mouse.column, base)));
                    }
                    MouseEventKind::Up(MouseButton::Left) => self.duration_dragging = false,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                        if !self.dragging && !self.duration_dragging
                            && mouse.row == slider.row
                            && (slider.left..=slider.right).contains(&mouse.column) =>
                    {
                        let index = Slider::duration_index(base / speed);
                        let index = if mouse.kind == MouseEventKind::ScrollUp {
                            index.saturating_sub(1)
                        } else {
                            (index + 1).min(DURATION_STEPS.len() - 1)
                        };
                        return Some(Action::Speed(base / DURATION_STEPS[index]));
                    }
                    _ => {}
                },
                Event::Key(key) if key.kind != KeyEventKind::Release
                    && key.code == KeyCode::Char(':') => self.duration_dragging = false,
                _ => {}
            }
            None
        }

        fn event(
            &mut self,
            event: Event,
            slider: Option<Slider>,
            position: f64,
            speed: f64,
        ) -> Option<Action> {
            match event {
                Event::Resize(..) => self.dragging = false,
                Event::Mouse(mouse) if self.command.is_none() => {
                    let Some(slider) = slider else {
                        self.dragging = false;
                        return None;
                    };
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left)
                            if mouse.row == slider.row
                                && (slider.left..=slider.right).contains(&mouse.column) =>
                        {
                            self.dragging = true;
                            return Some(Action::Seek(slider.position(mouse.column)));
                        }
                        MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                            return Some(Action::Seek(slider.position(mouse.column)));
                        }
                        MouseEventKind::Up(MouseButton::Left) => self.dragging = false,
                        _ => {}
                    }
                }
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        return Some(Action::Close(TimelineCloseMode::RestoreOrigins));
                    }
                    if let Some(buffer) = &mut self.command {
                        match key.code {
                            KeyCode::Esc => self.command = None,
                            KeyCode::Backspace => {
                                buffer.pop();
                            }
                            KeyCode::Enter => {
                                let text = self.command.take().unwrap();
                                match command(&text, speed) {
                                    Ok(action) => return Some(action),
                                    Err(error) => self.message = error,
                                }
                            }
                            KeyCode::Char(character)
                                if character.is_ascii()
                                    && !character.is_control()
                                    && !key
                                        .modifiers
                                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                    && buffer.len() < 160 =>
                            {
                                buffer.push(character)
                            }
                            _ => {}
                        }
                        return None;
                    }
                    let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                        0.1
                    } else {
                        0.01
                    };
                    let repeatable = match key.code {
                        KeyCode::Left => Some(Action::Seek((position - step).max(0.0))),
                        KeyCode::Right => Some(Action::Seek((position + step).min(1.0))),
                        KeyCode::Home => Some(Action::Seek(0.0)),
                        KeyCode::End => Some(Action::Seek(1.0)),
                        KeyCode::Up | KeyCode::Char('+') | KeyCode::Char('=') => {
                            Some(Action::Speed((speed * 2.0).min(MAX_SPEED)))
                        }
                        KeyCode::Down | KeyCode::Char('-') => {
                            Some(Action::Speed((speed / 2.0).max(0.0625).min(speed)))
                        }
                        _ => None,
                    };
                    if repeatable.is_some() {
                        return repeatable;
                    }
                    if key.kind == KeyEventKind::Repeat {
                        return None;
                    }
                    return match key.code {
                        KeyCode::Char('?') => {
                            self.help_open = !self.help_open;
                            None
                        }
                        KeyCode::Char(' ') => Some(Action::Toggle),
                        KeyCode::Char('h') => Some(Action::ToggleRealIcons),
                        KeyCode::Char('f') => Some(Action::Play(1.0, speed)),
                        KeyCode::Char('r') => Some(Action::Play(0.0, speed)),
                        KeyCode::Char('m') => Some(Action::Play(0.5, speed)),
                        KeyCode::Char('1') => Some(Action::Speed(1.0)),
                        KeyCode::Char('q') => {
                            Some(Action::Close(TimelineCloseMode::RestoreOrigins))
                        }
                        KeyCode::Char(':') => {
                            self.command = Some(String::new());
                            self.dragging = false;
                            None
                        }
                        _ => None,
                    };
                }
                _ => {}
            }
            None
        }
    }

    struct Playback {
        handle: Option<PlaybackHandle>,
        destination: f64,
        forward: bool,
    }

    impl Default for Playback {
        fn default() -> Self {
            Self {
                handle: None,
                destination: 0.5,
                forward: true,
            }
        }
    }

    impl Playback {
        fn apply(
            &mut self,
            action: Action,
            timeline: &TimelineSession,
        ) -> Result<Option<TimelineCloseMode>> {
            match action {
                Action::Seek(position) => {
                    timeline.seek(position)?;
                    self.handle = None;
                }
                Action::Speed(speed) => {
                    number(&speed.to_string(), MAX_SPEED, false)?;
                    timeline.set_speed(speed)?;
                }
                Action::Pause => {
                    timeline.pause()?;
                    self.handle = None;
                }
                Action::Play(position, speed) => {
                    number(&speed.to_string(), MAX_SPEED, false)?;
                    self.forward = position >= timeline.position();
                    self.destination = position;
                    self.handle = Some(timeline.play_to(position, speed)?);
                }
                Action::Toggle => {
                    if timeline.state() == TimelineState::Playing {
                        return self.apply(Action::Pause, timeline);
                    }
                    let destination = if (timeline.position() - self.destination).abs() < 1e-9 {
                        if self.forward { 1.0 } else { 0.0 }
                    } else {
                        self.destination
                    };
                    return self.apply(Action::Play(destination, timeline.speed()), timeline);
                }
                Action::Close(mode) => return Ok(Some(mode)),
                Action::ToggleRealIcons => {
                    timeline.set_real_icons_visible(!timeline.real_icons_visible())?;
                }
            }
            Ok(None)
        }

        fn poll(&mut self) -> Result<Option<PlaybackOutcome>> {
            let outcome = self
                .handle
                .as_ref()
                .map(|handle| handle.wait_timeout(Duration::ZERO))
                .transpose()?
                .flatten();
            if outcome.is_some() {
                self.handle = None;
            }
            Ok(outcome)
        }
    }

    struct TerminalGuard;

    impl TerminalGuard {
        fn enter() -> io::Result<Self> {
            terminal::enable_raw_mode()?;
            TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
            let guard = Self;
            execute!(
                io::stdout(),
                EnterAlternateScreen,
                EnableMouseCapture,
                event::EnableBracketedPaste,
                cursor::Hide
            )?;
            Ok(guard)
        }
    }

    fn restore_terminal() {
        if !TERMINAL_ACTIVE.swap(false, Ordering::SeqCst) {
            return;
        }
        let _ = execute!(
            io::stdout(),
            ResetColor,
            cursor::Show,
            DisableMouseCapture,
            event::DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }

    pub fn install_panic_hook() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |information| {
            restore_terminal();
            previous(information);
        }));
    }

    impl Drop for TerminalGuard {
        fn drop(&mut self) {
            restore_terminal();
        }
    }

    #[cfg(test)]
    fn line(
        output: &mut impl Write,
        row: u16,
        width: u16,
        height: u16,
        text: &str,
    ) -> io::Result<()> {
        if row < height.saturating_sub(1) {
            let text: String = text
                .chars()
                .filter(|character| !character.is_control())
                .take(usize::from(width.saturating_sub(1)))
                .collect();
            queue!(output, cursor::MoveTo(0, row), Print(text))?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn colored_line(
        output: &mut impl Write,
        row: u16,
        size: (u16, u16),
        spans: &[(Color, &str)],
    ) -> io::Result<()> {
        let (width, height) = size;
        if width <= 1 || row >= height.saturating_sub(1) {
            return Ok(());
        }
        let mut remaining = usize::from(width - 1);
        queue!(output, cursor::MoveTo(0, row))?;
        for &(color, text) in spans {
            if remaining == 0 {
                break;
            }
            let text: String = text.chars()
                .filter(|character| !character.is_control())
                .take(remaining)
                .collect();
            remaining -= text.chars().count();
            queue!(output, SetForegroundColor(color), Print(text))?;
        }
        queue!(output, ResetColor)
    }

    #[cfg(test)]
    fn draw_help(output: &mut impl Write, size: (u16, u16)) -> io::Result<()> {
        use Color::{Cyan, DarkGrey, Reset, Yellow};

        let rows: &[&[(Color, &str)]] = &[
            &[
                (DarkGrey, "SEEK      "), (Cyan, "Left/Right"), (Reset, " "),
                (Yellow, "1%"), (Reset, "   "), (Cyan, "Shift+arrows"), (Reset, " "),
                (Yellow, "10%"), (Reset, "   "), (Cyan, "Home/End"), (Reset, " "),
                (Yellow, "0/1"),
            ],
            &[
                (DarkGrey, "PLAY      "), (Cyan, "Space"), (Reset, " pause/resume   "),
                (Cyan, "f"), (Reset, " forward   "), (Cyan, "r"),
                (Reset, " rewind   "), (Cyan, "m"), (Reset, " midpoint"),
            ],
            &[
                (DarkGrey, "SPEED     "), (Cyan, "Up/Down"), (Reset, " or "),
                (Cyan, "+/-"), (Reset, " adjust   "), (Cyan, "1"),
                (Reset, " reset   max "), (Yellow, "50x"),
            ],
            &[
                (DarkGrey, "COMMANDS  "), (Cyan, ":"), (Reset, " enter   "),
                (Cyan, "Enter"), (Reset, " run   "), (Cyan, "Esc"), (Reset, " cancel"),
            ],
            &[
                (Reset, "  "), (Cyan, "play"), (Yellow, " POSITION [SPEED]"),
                (DarkGrey, " | "), (Cyan, "seek"), (Yellow, " POSITION"),
                (DarkGrey, " | "), (Cyan, "speed"), (Yellow, " RATE"),
                (DarkGrey, " | "), (Cyan, "pause"),
            ],
            &[
                (Reset, "  "), (Cyan, "close"), (Yellow, " restore|current|target"),
                (DarkGrey, " | "), (Cyan, "q/Ctrl+C"), (Reset, " restore & quit"),
            ],
        ];
        for (index, spans) in rows.iter().enumerate() {
            colored_line(output, 16 + index as u16, size, spans)?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn draw_guidance(
        output: &mut impl Write,
        size: (u16, u16),
        input: &Input,
    ) -> io::Result<()> {
        use Color::{Cyan, DarkGrey, Reset};

        colored_line(output, 11, size, &[(DarkGrey, "QUICK CONTROLS")])?;
        colored_line(output, 12, size, &[
            (DarkGrey, "| "), (Cyan, "Left/Right"), (Reset, " scrub   "),
            (Cyan, "Space"), (Reset, " play/pause"),
        ])?;
        colored_line(output, 13, size, &[
            (DarkGrey, "| "), (Cyan, "Mouse"), (Reset, " drag sliders"),
        ])?;
        if input.command.is_some() {
            colored_line(output, 14, size, &[
                (DarkGrey, "| "), (Cyan, "Esc"), (Reset, " cancel command   "),
                (Cyan, "Enter"), (Reset, " run command"),
            ])?;
        } else {
            colored_line(output, 14, size, &[
                (DarkGrey, "| "), (Cyan, "?"),
                (Reset, if input.help_open { " hide help   " } else { " show help   " }),
                (Cyan, ":"), (Reset, " type command   "),
                (Cyan, "q"), (Reset, " quit"),
            ])?;
        }
        if input.help_visible() {
            draw_help(output, size)?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn draw(
        output: &mut impl Write,
        size: (u16, u16),
        timeline: &TimelineSession,
        playback: &Playback,
        input: &Input,
        summary: &str,
        picker: &ShaderPicker,
    ) -> io::Result<()> {
        let (width, height) = size;
        queue!(
            output,
            cursor::MoveTo(0, 0),
            Clear(ClearType::All),
            SetForegroundColor(Color::Cyan)
        )?;
        line(output, 0, width, height, "ANIMATE TIMELINE")?;
        queue!(output, ResetColor)?;
        line(
            output,
            1,
            width,
            height,
            &format!(
                "{:?}  t={:.4}  speed={:.4}x  to={:.3}",
                timeline.state(),
                timeline.position(),
                timeline.speed(),
                playback.destination
            ),
        )?;
        if let Some(slider) = Slider::layout(width, height) {
            slider.draw(output, timeline.position(), Color::Green)?;
            queue!(
                output,
                cursor::MoveTo(slider.left, 4),
                Print("0"),
                cursor::MoveTo(slider.column(0.5).saturating_sub(1), 4),
                Print("0.5"),
                cursor::MoveTo(slider.right, 4),
                Print("1")
            )?;
        } else {
            line(output, 2, width, height, "Resize terminal; q quits")?;
        }
        if let Some(slider) = Slider::duration_layout(width, height) {
            let base = crate::demo_animation::test_support::animation_duration(
                picker.selected.map(|shader| shader.source().pipeline),
            ).as_secs_f64();
            let seconds = base / timeline.speed();
            let label = if width >= 24 {
                format!("Duration: {seconds:.2}s")
            } else {
                format!("{seconds:.2}s")
            };
            line(output, 5, width, height, &label)?;
            slider.draw(output, Slider::duration_position(seconds), Color::Yellow)?;
            queue!(output,
                cursor::MoveTo(slider.left, 7), Print("0.1s"),
                cursor::MoveTo(slider.right - 1, 7), Print("5s"))?;
        }
        line(output, 9, width, height, summary)?;
        colored_line(
            output,
            10,
            size,
            &[
                (Color::DarkGrey, "DESKTOP   "),
                (Color::Reset, if timeline.real_icons_visible() {
                    "Real icons shown   "
                } else {
                    "Real icons hidden  "
                }),
                (Color::Cyan, "h"),
                (Color::Reset, " toggle"),
            ],
        )?;
        draw_guidance(output, size, input)?;
        let prompt = input
            .command
            .as_ref()
            .map_or_else(|| input.message.clone(), |text| format!(":{text}_"));
        let capacity = usize::from(width.saturating_sub(1));
        let tail: String = prompt
            .chars()
            .rev()
            .take(capacity)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if height > 0 && width > 0 {
            queue!(output, cursor::MoveTo(0, height - 1), Print(tail))?;
        }
        picker.draw(output, size)?;
        output.flush()
    }

    fn push_event(batch: &mut Vec<Event>, next: Event) {
        let drag = |event: &Event| matches!(event, Event::Mouse(mouse) if mouse.kind == MouseEventKind::Drag(MouseButton::Left));
        if drag(&next) && batch.last().is_some_and(drag) {
            batch.pop();
        }
        batch.push(next);
    }

    fn events() -> io::Result<Vec<Event>> {
        let mut batch = Vec::new();
        if event::poll(Duration::from_millis(33))? {
            for _ in 0..64 {
                push_event(&mut batch, event::read()?);
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
        Ok(batch)
    }

    fn interact(
        timeline: &mut TimelineSession,
        interrupted: &AtomicBool,
        summary: &str,
        controller: &DesktopController,
        moves: &[crate::demo_animation::Move],
        selected: Option<shader::BuiltinShader>,
    ) -> Result<TimelineCloseMode> {
        let _terminal = TerminalGuard::enter()?;
        let mut output = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        let mut input = Input::default();
        let mut playback = Playback::default();
        let mut editor = Editor::new(selected);
        let mut active_program = editor.config.program()?;
        playback.apply(Action::Play(0.5, 1.0), timeline)?;
        loop {
            editor.finish_action();
            if interrupted.load(Ordering::Relaxed) {
                return Ok(TimelineCloseMode::RestoreOrigins);
            }
            if timeline.state() == TimelineState::Closed {
                return Err(format!(
                    "Timeline closed unexpectedly: {:?}",
                    timeline.finish_reason()
                )
                .into());
            }
            if let Some(outcome) = playback.poll()? {
                input.message = format!("Playback: {outcome:?}");
            }
            let mut size = terminal::size()?;
            let state = format!("{:?}", timeline.state());
            let prompt = input.command.as_ref().map_or_else(|| input.message.clone(), |text| format!(":{text}_"));
            output.draw(|frame| editor.draw(frame, MainView {
                position: timeline.position(), speed: timeline.speed(), destination: playback.destination,
                state: &state, summary, visible: timeline.real_icons_visible(), help: input.help_visible(), prompt: &prompt,
            }))?;
            for event in events()? {
                if let Event::Resize(width, height) = event {
                    size = (width, height);
                }
                if editor.ctrl_c_exits(&event) {
                    return Ok(TimelineCloseMode::RestoreOrigins);
                }
                if input.command.is_none() && editor.can_quit() && matches!(&event, Event::Key(key) if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q')) {
                    return Ok(TimelineCloseMode::RestoreOrigins);
                }
                let navigation = matches!(&event, Event::Key(key) if matches!(key.code, KeyCode::Tab | KeyCode::BackTab))
                    || matches!(&event, Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left));
                let feedback = (input.command.is_none() || navigation) && editor.begin_action(&event);
                if feedback {
                    output.draw(|frame| editor.draw(frame, MainView {
                        position: timeline.position(), speed: timeline.speed(), destination: playback.destination,
                        state: &state, summary, visible: timeline.real_icons_visible(), help: input.help_visible(), prompt: &prompt,
                    }))?;
                }
                let routed = if input.command.is_some() && !navigation {
                    EditorEvent::Ignored
                } else { editor.event(&event) };
                match routed {
                    EditorEvent::Consumed | EditorEvent::Pause => {
                        if matches!(routed, EditorEvent::Pause) { playback.apply(Action::Pause, timeline)?; }
                        input.dragging = false; input.duration_dragging = false; input.command = None;
                        if feedback { break; }
                        continue;
                    }
                    EditorEvent::Apply { mut config, reset_duration } => {
                        if !editor.is_main() { playback.apply(Action::Pause, timeline)?; }
                        if !reset_duration { config.seconds = editor.config.seconds; }
                        let speed = if reset_duration { 1.0 } else { timeline.speed() };
                        match rebuild_configuration(controller, moves, timeline, &mut playback, &editor.config, active_program.as_ref(), &config, speed) {
                            Ok(program) => { active_program = program; editor.committed(config); }
                            Err(error) => {
                                if timeline.state() == TimelineState::Closed { return Err(error); }
                                editor.error(error);
                            }
                        }
                        input.dragging = false; input.duration_dragging = false; input.command = None;
                        break;
                    }
                    EditorEvent::Ignored => {
                        if matches!(&event, Event::Key(_) | Event::Mouse(_)) { editor.message.clear(); }
                    }
                }
                if !editor.is_main() || size.0 < 42 || size.1 < 24 { continue; }
                let base = editor.config.seconds;
                let action = input.duration_event(&event, size, base, timeline.speed()).or_else(|| input.event(
                    event,
                    Slider::layout(size.0, size.1),
                    timeline.position(),
                    timeline.speed(),
                ));
                if let Some(action) = action {
                    if let Some(mode) = playback.apply(action, timeline)? {
                        return Ok(mode);
                    }
                    input.message.clear();
                }
            }
        }
    }

    fn prepare_configuration(
        controller: &DesktopController,
        moves: &[crate::demo_animation::Move],
        config: &Config,
        program: Option<&rdi_core::ShaderProgram>,
    ) -> Result<rdi_core::PreparedAnimation> {
        Ok(controller.prepare(config.specs(moves, program)?, AnimationOptions { tick_hz: Some(144), ..Default::default() })?)
    }

    fn rebuild_configuration(
        controller: &DesktopController,
        moves: &[crate::demo_animation::Move],
        timeline: &mut TimelineSession,
        playback: &mut Playback,
        previous: &Config,
        previous_program: Option<&rdi_core::ShaderProgram>,
        next: &Config,
        next_speed: f64,
    ) -> Result<Option<rdi_core::ShaderProgram>> {
        let program = next.program()?;
        next.specs(moves, program.as_ref())?;
        number(&next_speed.to_string(), MAX_SPEED, false)?;
        let playing = timeline.state() == TimelineState::Playing;
        timeline.pause()?;
        let position = timeline.position();
        let speed = timeline.speed();
        let visible = timeline.real_icons_visible();
        let (_, expected) = finish(timeline, Ok(TimelineCloseMode::RestoreOrigins))?;
        playback.handle = None;
        verify_positions(controller, &expected)?;
        let open = |config: &Config, program: Option<&rdi_core::ShaderProgram>, speed| -> Result<TimelineSession> {
            let session = prepare_configuration(controller, moves, config, program)?.open_timeline()?;
            let initialized = session.seek(position).and_then(|_| session.set_speed(speed)).and_then(|_| session.set_real_icons_visible(visible));
            if let Err(error) = initialized {
                let (_, expected) = finish(&session, Ok(TimelineCloseMode::RestoreOrigins))?;
                verify_positions(controller, &expected)?;
                return Err(error.into());
            }
            Ok(session)
        };
        match open(next, program.as_ref(), next_speed) {
            Ok(session) => *timeline = session,
            Err(error) => {
                *timeline = open(previous, previous_program, speed).map_err(|restore| format!("Apply failed: {error}; recovery failed: {restore}"))?;
                if playing { playback.apply(Action::Play(playback.destination, speed), timeline)?; }
                return Err(format!("Apply failed; previous configuration restored: {error}").into());
            }
        }
        if playing { playback.apply(Action::Play(playback.destination, next_speed), timeline)?; }
        Ok(program)
    }

    fn plan_moves(
        icons: &[rdi_core::IconSnapshot],
        monitors: &[rdi_core::MonitorInfo],
        spacing: rdi_core::Point,
    ) -> Vec<std::result::Result<crate::demo_animation::Move, &'static str>> {
        let mut moves = crate::demo_animation::plan_moves(icons, monitors, spacing);
        if spacing.x <= 0 || spacing.y <= 0 {
            return moves;
        }
        let overlaps = |first: rdi_core::Point, second: rdi_core::Point| {
            (i64::from(first.x) - i64::from(second.x)).abs() < i64::from(spacing.x)
                && (i64::from(first.y) - i64::from(second.y)).abs() < i64::from(spacing.y)
        };
        for (monitor_index, monitor) in monitors.iter().enumerate() {
            let candidates: Vec<_> = icons
                .iter()
                .enumerate()
                .filter(|(index, icon)| {
                    let origin = icon.position;
                    matches!(moves[*index], Err("no free grid destination"))
                        && monitors.iter().position(|monitor| monitor.contains(origin))
                            == Some(monitor_index)
                        && monitor.work_area.contains(origin)
                        && i64::from(origin.x) + i64::from(spacing.x)
                            <= i64::from(monitor.work_area.right)
                        && i64::from(origin.y) + i64::from(spacing.y)
                            <= i64::from(monitor.work_area.bottom)
                        && !icons.iter().enumerate().any(|(other_index, other)| {
                            other_index != *index && overlaps(origin, other.position)
                        })
                        && !moves.iter().flatten().any(|entry| overlaps(origin, entry.target))
                })
                .map(|(index, _)| index)
                .collect();
            let mut remaining = candidates.as_slice();
            while remaining.len() >= 2 {
                let count = if remaining.len() == 3 { 3 } else { 2 };
                let (group, rest) = remaining.split_at(count);
                for (offset, &index) in group.iter().enumerate() {
                    moves[index] = Ok(crate::demo_animation::Move {
                        id: icons[index].id.clone(),
                        origin: icons[index].position,
                        target: icons[group[(offset + 1) % count]].position,
                    });
                }
                remaining = rest;
            }
        }
        moves
    }

    #[cfg(test)]
    fn prepare_timeline(
        controller: &DesktopController,
        moves: &[crate::demo_animation::Move],
        program: Option<&rdi_core::ShaderProgram>,
    ) -> Result<rdi_core::PreparedAnimation> {
        let (movement, envelope) = crate::demo_animation::test_support::curves()?;
        let specs = crate::demo_animation::test_support::build_specs(moves, false, program, &movement, &envelope);
        Ok(controller.prepare(
            specs,
            AnimationOptions {
                tick_hz: Some(144),
                ..Default::default()
            },
        )?)
    }

    fn verify_positions(
        controller: &DesktopController,
        expected: &[(rdi_core::IconId, rdi_core::Point)],
    ) -> Result<()> {
        let settled = controller.list_icons()?;
        if expected.iter().any(|(id, position)| {
            !settled
                .iter()
                .any(|icon| icon.id == *id && icon.position == *position)
        }) {
            return Err("Shell positions differ from the requested close state; check snapping or external changes".into());
        }
        Ok(())
    }

    #[cfg(test)]
    fn change_shader(
        controller: &DesktopController,
        moves: &[crate::demo_animation::Move],
        timeline: &mut TimelineSession,
        playback: &mut Playback,
        selected: Option<shader::BuiltinShader>,
    ) -> Result<()> {
        let program = selected.map(shader::compile).transpose()?;
        let playing = timeline.state() == TimelineState::Playing;
        timeline.pause()?;
        let position = timeline.position();
        let speed = timeline.speed();
        let visible = timeline.real_icons_visible();
        let (_, expected) = finish(timeline, Ok(TimelineCloseMode::RestoreOrigins))?;
        playback.handle = None;
        verify_positions(controller, &expected)?;
        *timeline = prepare_timeline(controller, moves, program.as_ref())?.open_timeline()?;
        timeline.seek(position)?;
        timeline.set_speed(speed)?;
        timeline.set_real_icons_visible(visible)?;
        if playing {
            playback.apply(Action::Play(playback.destination, speed), timeline)?;
        }
        Ok(())
    }

    fn finish(
        timeline: &TimelineSession,
        result: Result<TimelineCloseMode>,
    ) -> Result<(TimelineCloseMode, Vec<(rdi_core::IconId, rdi_core::Point)>)> {
        let mode = result
            .as_ref()
            .copied()
            .unwrap_or(TimelineCloseMode::RestoreOrigins);
        let pause = if timeline.state() != TimelineState::Closed {
            timeline.pause()
        } else {
            Ok(())
        };
        let expected = timeline
            .snapshot()
            .into_iter()
            .map(|entry| {
                let position = match mode {
                    TimelineCloseMode::RestoreOrigins => entry.origin,
                    TimelineCloseMode::LeaveInPlace => entry.current,
                    TimelineCloseMode::TeleportToTarget => entry.target,
                };
                (entry.id, position)
            })
            .collect();
        let reason = timeline.close(mode)?;
        if let FinishReason::Error(message) = reason {
            return Err(message.into());
        }
        result?;
        pause?;
        if !timeline.missing_icons().is_empty()
            || timeline
                .final_commit()
                .is_none_or(|outcome| !outcome.missing_ids.is_empty())
        {
            return Err(
                "Some icons were missing or the final commit could not be confirmed".into(),
            );
        }
        Ok((mode, expected))
    }

    pub fn run() -> Result<()> {
        let args = Args::parse(std::env::args().skip(1))?;
        if args.help {
            println!("{HELP}\nShaders: {}", shader_names());
            return Ok(());
        }
        if !args.dry_run && (!io::stdin().is_terminal() || !io::stdout().is_terminal()) {
            return Err(
                "An interactive terminal is required; use --dry-run for redirected output".into(),
            );
        }
        if let Some(path) = &args.log_file {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_env("RDI_LOG")
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
                )
                .with_writer(std::sync::Mutex::new(file))
                .try_init()
                .map_err(|error| error.to_string())?;
        }
        let interrupted = Arc::new(AtomicBool::new(false));
        let signal = interrupted.clone();
        ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))?;
        let controller = DesktopController::new(WindowsBackend::new())?;
        if controller.get_flags()? & FWF_AUTOARRANGE.0 as u32 != 0 {
            return Err(
                "Disable desktop View > Auto arrange icons before running this demo".into(),
            );
        }
        let icons = controller.list_icons()?;
        let monitors = controller.list_monitors()?;
        let spacing = crate::demo_animation::grid_spacing()?;
        let mut moves = Vec::new();
        for (icon, entry) in icons.iter().zip(plan_moves(
            &icons, &monitors, spacing,
        )) {
            match entry {
                Ok(entry) => {
                    if args.dry_run {
                        println!(
                            "{}: {:?} -> {:?}",
                            icon.display_name, entry.origin, entry.target
                        );
                    }
                    moves.push(entry);
                }
                Err(reason) => println!("Skipping {}: {reason}", icon.display_name),
            }
        }
        let summary = format!(
            "{} moving, {} skipped | grid {}x{}",
            moves.len(),
            icons.len() - moves.len(),
            spacing.x,
            spacing.y
        );
        let selected = args.selected_shader();
        println!("{summary} | shader {}", selected.map_or("off", shader::BuiltinShader::name));
        if args.dry_run || moves.is_empty() || interrupted.load(Ordering::Relaxed) {
            return Ok(());
        }
        let program = selected.map(shader::compile).transpose()?;
        let prepared = prepare_configuration(&controller, &moves, &Config::new(selected), program.as_ref())?;
        if interrupted.load(Ordering::Relaxed) {
            prepared.cancel();
            return Ok(());
        }
        let mut timeline = prepared.open_timeline()?;
        let result = interact(
            &mut timeline,
            &interrupted,
            &summary,
            &controller,
            &moves,
            selected,
        );
        let (mode, expected) = match finish(&timeline, result) {
            Ok(result) => result,
            Err(error) => {
                controller.shutdown();
                return Err(error);
            }
        };
        let verified = verify_positions(&controller, &expected);
        controller.shutdown();
        verified?;
        println!("Closed: {mode:?}; positions verified.");
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crossterm::event::{KeyEvent, MouseEvent};

        fn key(code: KeyCode) -> Event {
            Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
        }

        #[test]
        fn help_defaults_hidden_and_toggles_without_repeats() {
            let mut input = Input::default();
            assert!(!input.help_visible());
            assert_eq!(input.event(key(KeyCode::Char('?')), None, 0.5, 1.0), None);
            assert!(input.help_visible());
            for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
                input.event(Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('?'), KeyModifiers::SHIFT, kind,
                )), None, 0.5, 1.0);
                assert!(input.help_visible());
            }
            input.event(key(KeyCode::Char('?')), None, 0.5, 1.0);
            assert!(!input.help_visible());
        }

        #[test]
        fn help_command_mode_restores_manual_visibility_on_exit() {
            for help_open in [false, true] {
                for exit in [KeyCode::Esc, KeyCode::Enter] {
                    let mut input = Input { help_open, ..Input::default() };
                    input.event(key(KeyCode::Char(':')), None, 0.5, 1.0);
                    assert!(input.help_visible());
                    input.event(key(KeyCode::Char('?')), None, 0.5, 1.0);
                    assert_eq!(input.command.as_deref(), Some("?"));
                    assert_eq!(input.help_open, help_open);
                    input.event(key(exit), None, 0.5, 1.0);
                    assert!(input.command.is_none());
                    assert_eq!(input.help_visible(), help_open);

                    input.event(key(KeyCode::Char(':')), None, 0.5, 1.0);
                    for character in "pause".chars() {
                        input.event(key(KeyCode::Char(character)), None, 0.5, 1.0);
                    }
                    assert_eq!(input.event(key(KeyCode::Enter), None, 0.5, 1.0),
                        Some(Action::Pause));
                    assert_eq!(input.help_visible(), help_open);
                }
            }
        }

        #[test]
        fn help_notice_is_always_visible_and_commands_reveal_details() {
            let mut input = Input::default();
            for mode in ["hidden", "manual", "command"] {
                input.help_open = mode == "manual";
                input.command = (mode == "command").then(String::new);
                let mut output = Vec::new();
                draw_guidance(&mut output, (80, 24), &input).unwrap();
                let output = String::from_utf8(output).unwrap();
                for text in ["QUICK CONTROLS", "Left/Right", "play/pause", "drag sliders"] {
                    assert!(output.contains(text), "missing notice: {text}");
                }
                assert_eq!(output.contains("POSITION [SPEED]"), mode != "hidden");
                assert_eq!(output.contains("type command"), mode != "command");
                assert_eq!(output.contains("cancel command"), mode == "command");
                assert!(!output.contains(&cursor::MoveTo(0, 23).to_string()));
            }
        }

        #[test]
        fn help_spans_clip_visible_text_and_reset_color() {
            let mut output = Vec::new();
            colored_line(&mut output, 2, (10, 5), &[
                (Color::Cyan, "play"),
                (Color::Yellow, " \nPOSITION"),
                (Color::Reset, " description"),
            ]).unwrap();
            let mut expected = Vec::new();
            queue!(expected, cursor::MoveTo(0, 2),
                SetForegroundColor(Color::Cyan), Print("play"),
                SetForegroundColor(Color::Yellow), Print(" POSI"), ResetColor).unwrap();
            assert_eq!(output, expected);
            for size in [(0, 5), (1, 5), (80, 0), (80, 1), (80, 3)] {
                output.clear();
                colored_line(&mut output, 2, size, &[(Color::Cyan, "hidden")]).unwrap();
                assert!(output.is_empty(), "size={size:?}");
            }
        }

        #[test]
        fn help_separates_commands_arguments_and_meanings() {
            let mut output = Vec::new();
            draw_help(&mut output, (80, 24)).unwrap();
            let output = String::from_utf8(output).unwrap();
            for (color, text) in [
                (Color::DarkGrey, "SEEK      "),
                (Color::Cyan, "Space"),
                (Color::Reset, " pause/resume   "),
                (Color::Cyan, "play"),
                (Color::Yellow, " POSITION [SPEED]"),
                (Color::Yellow, " restore|current|target"),
                (Color::Cyan, "q/Ctrl+C"),
            ] {
                assert!(output.contains(&format!("{}{text}", SetForegroundColor(color))),
                    "missing styled help: {text}");
            }
            assert_eq!(output.matches(&ResetColor.to_string()).count(), 6);
            assert!(output.ends_with(&format!("restore & quit{ResetColor}")));
            let mut short = Vec::new();
            draw_help(&mut short, (80, 18)).unwrap();
            assert_eq!(String::from_utf8(short).unwrap()
                .matches(&ResetColor.to_string()).count(), 1);
        }

        fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
            Event::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
        }

        fn monitor(bounds: rdi_core::Rect) -> rdi_core::MonitorInfo {
            rdi_core::MonitorInfo {
                id: format!("{bounds:?}"),
                name: "test".into(),
                bounds,
                work_area: bounds,
                is_primary: bounds.left == 0,
                scale_factor: 1.0,
            }
        }

        fn icons_at(positions: &[rdi_core::Point]) -> Vec<rdi_core::IconSnapshot> {
            positions.iter().enumerate().map(|(index, &position)| {
                rdi_core::IconSnapshot::new(
                    rdi_core::IconId::from(format!("icon-{index}")),
                    "test", None, false, position,
                )
            }).collect()
        }

        #[test]
        fn full_desktops_exchange_even_and_odd_icon_counts() {
            use rdi_core::{Point, Rect};
            for count in 2..=9 {
                let positions: Vec<_> = (0..count).map(|row| Point::new(0, row * 100)).collect();
                let icons = icons_at(&positions);
                let monitors = [monitor(Rect::new(0, 0, 100, count * 100))];
                let spacing = Point::new(100, 100);
                assert!(crate::demo_animation::plan_moves(&icons, &monitors, spacing)
                    .iter().all(|entry| entry.is_err()));
                let moves = plan_moves(&icons, &monitors, spacing);
                let mut targets = std::collections::HashSet::new();
                for (icon, entry) in icons.iter().zip(moves) {
                    let entry = entry.unwrap();
                    assert_eq!(entry.id, icon.id);
                    assert_eq!(entry.origin, icon.position);
                    assert_ne!(entry.target, entry.origin);
                    assert!(positions.contains(&entry.target));
                    assert!(targets.insert((entry.target.x, entry.target.y)));
                }
            }
        }

        #[test]
        fn exchanges_preserve_free_moves_and_monitor_boundaries() {
            use rdi_core::{Point, Rect};
            let positions = [
                Point::new(0, 0), Point::new(500, 0), Point::new(500, 100),
                Point::new(-100, 0), Point::new(-100, 100), Point::new(-100, 200),
            ];
            let icons = icons_at(&positions);
            let monitors = [
                monitor(Rect::new(0, 0, 600, 200)),
                monitor(Rect::new(-100, 0, 0, 300)),
            ];
            let moves: Vec<_> = plan_moves(&icons, &monitors, Point::new(100, 100))
                .into_iter().map(|entry| entry.unwrap()).collect();
            assert!(!positions.contains(&moves[0].target));
            assert_eq!(moves[0].target.y, 100);
            assert!([300, 400].contains(&moves[0].target.x));
            assert_eq!(moves[1].target, positions[2]);
            assert_eq!(moves[2].target, positions[1]);
            let mut targets = std::collections::HashSet::new();
            for entry in moves {
                let source_monitor = monitors.iter().position(|monitor| monitor.contains(entry.origin));
                let target_monitor = monitors.iter().position(|monitor| monitor.contains(entry.target));
                assert_eq!(source_monitor, target_monitor);
                assert!(targets.insert((entry.target.x, entry.target.y)));
            }
        }

        #[test]
        fn exchanges_skip_singletons_overlaps_and_unusable_positions() {
            use rdi_core::{Point, Rect};
            let mut primary = monitor(Rect::new(0, 0, 100, 800));
            primary.work_area.bottom = 700;
            let monitors = [primary, monitor(Rect::new(-100, 0, 0, 100))];
            let icons = icons_at(&[
                Point::new(0, 0), Point::new(0, 100),
                Point::new(0, 200), Point::new(0, 200),
                Point::new(0, 350), Point::new(0, 400),
                Point::new(0, 650), Point::new(0, 750),
                Point::new(1000, 0), Point::new(-100, 0),
            ]);
            let moves = plan_moves(&icons, &monitors, Point::new(100, 100));
            assert_eq!(moves[0].as_ref().unwrap().target, icons[1].position);
            assert_eq!(moves[1].as_ref().unwrap().target, icons[0].position);
            assert!(moves[2..].iter().all(|entry| entry.is_err()));
            assert_eq!(moves[8].as_ref().err(), Some(&"no containing monitor"));
            for spacing in [Point::new(0, 100), Point::new(100, -1)] {
                assert!(plan_moves(&icons, &monitors, spacing).iter().all(|entry| entry.is_err()));
            }
            assert!(plan_moves(&[], &monitors, Point::new(100, 100)).is_empty());
        }

        #[test]
        fn exchanged_positions_support_timeline_close_modes() {
            use rdi_core::{Point, Rect};
            let icons = icons_at(&[Point::new(0, 0), Point::new(0, 100), Point::new(0, 200)]);
            let monitors = [monitor(Rect::new(0, 0, 100, 300))];
            for mode in [TimelineCloseMode::RestoreOrigins, TimelineCloseMode::LeaveInPlace,
                TimelineCloseMode::TeleportToTarget] {
                let desktop = rdi_core::fake::FakeDesktop::new();
                for icon in &icons {
                    desktop.add_icon(icon.clone());
                }
                let moves: Vec<_> = plan_moves(&icons, &monitors, Point::new(100, 100))
                    .into_iter().map(|entry| entry.unwrap()).collect();
                let controller = DesktopController::new(desktop.clone()).unwrap();
                let timeline = prepare_timeline(&controller, &moves, None).unwrap().open_timeline().unwrap();
                timeline.seek(1.0).unwrap();
                let (_, expected) = finish(&timeline, Ok(mode)).unwrap();
                verify_positions(&controller, &expected).unwrap();
                for entry in &moves {
                    assert_eq!(desktop.position_of(&entry.id), Some(
                        if mode == TimelineCloseMode::RestoreOrigins { entry.origin } else { entry.target }
                    ));
                }
                controller.shutdown();
                assert!(!desktop.overlay_active());
                assert!(desktop.real_icons_visible());
                assert_eq!(desktop.overlay_finalize_log().len(), 1);
            }
        }

        #[test]
        fn shader_picker_navigation_mouse_and_resize() {
            let mut picker = ShaderPicker {
                selected: Some(shader::BuiltinShader::Glitch),
                highlighted: None,
            };
            let size = (80, 24);
            assert_eq!(
                picker.event(&key(KeyCode::Char('s')), size),
                PickerEvent::Consumed
            );
            assert_eq!(
                ShaderPicker::choice(picker.highlighted.unwrap()),
                picker.selected
            );
            assert_eq!(
                picker.event(&key(KeyCode::End), size),
                PickerEvent::Consumed
            );
            assert_eq!(
                picker.event(&key(KeyCode::Enter), size),
                PickerEvent::Select(shader::BuiltinShader::ALL.last().copied())
            );
            assert_eq!(picker.selected, Some(shader::BuiltinShader::Glitch));
            picker.event(&mouse(MouseEventKind::Down(MouseButton::Left), 1, 2), size);
            assert_eq!(
                picker.event(&mouse(MouseEventKind::Down(MouseButton::Left), 1, 3), size),
                PickerEvent::Select(None)
            );
            picker.event(&key(KeyCode::Char('s')), size);
            assert_eq!(
                picker.event(&key(KeyCode::Char(' ')), size),
                PickerEvent::Consumed
            );
            assert_eq!(
                picker.event(&key(KeyCode::Char('q')), size),
                PickerEvent::Ignored
            );
            picker.event(&key(KeyCode::Esc), size);
            assert!(picker.highlighted.is_none());
            picker.event(&key(KeyCode::Char('s')), size);
            assert_eq!(
                picker.event(&mouse(MouseEventKind::Down(MouseButton::Left), 79, 3), size),
                PickerEvent::Consumed
            );
            assert!(picker.highlighted.is_none());
            picker.event(&key(KeyCode::Char('s')), size);
            picker.event(&Event::Resize(12, 6), (12, 6));
            assert!(picker.highlighted.is_none());
            picker.event(&key(KeyCode::Char('s')), (12, 6));
            picker.event(&key(KeyCode::End), (12, 6));
            let mut output = Vec::new();
            picker.draw(&mut output, (12, 6)).unwrap();
            assert!(!output.is_empty());
            assert_eq!(
                picker.event(&key(KeyCode::Char('s')), (1, 1)),
                PickerEvent::Ignored
            );
        }

        #[test]
        fn shader_picker_isolates_commands_dragging_and_repeats() {
            let mut picker = ShaderPicker {
                selected: None,
                highlighted: None,
            };
            let mut input = Input {
                command: Some(String::new()),
                dragging: true,
                duration_dragging: true,
                ..Default::default()
            };
            let size = (80, 24);
            assert_eq!(
                input.picker_event(&mut picker, &key(KeyCode::Char('s')), size),
                PickerEvent::Ignored
            );
            assert!(picker.highlighted.is_none());
            input.event(key(KeyCode::Char('s')), None, 0.5, 1.0);
            assert_eq!(input.command.as_deref(), Some("s"));
            input.command = None;
            assert_eq!(
                input.picker_event(&mut picker, &key(KeyCode::Char('s')), size),
                PickerEvent::Consumed
            );
            assert!(!input.dragging);
            assert!(!input.duration_dragging);
            for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
                picker.event(
                    &Event::Key(KeyEvent::new_with_kind(
                        KeyCode::Enter,
                        KeyModifiers::NONE,
                        kind,
                    )),
                    size,
                );
                assert!(picker.highlighted.is_some());
            }
            assert_eq!(
                picker.event(
                    &Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                    size
                ),
                PickerEvent::Ignored
            );
            picker.event(&key(KeyCode::Home), size);
            picker.event(&key(KeyCode::Up), size);
            assert_eq!(picker.highlighted, Some(0));
            picker.event(&mouse(MouseEventKind::ScrollDown, 1, 3), size);
            assert_eq!(picker.highlighted, Some(1));
            picker.event(&key(KeyCode::End), size);
            picker.event(&key(KeyCode::Down), size);
            assert_eq!(picker.highlighted, Some(shader::BuiltinShader::ALL.len()));
        }

        #[test]
        fn shader_switches_preserve_state_and_final_close_positions() {
            for mode in [
                TimelineCloseMode::RestoreOrigins,
                TimelineCloseMode::LeaveInPlace,
                TimelineCloseMode::TeleportToTarget,
            ] {
                let desktop = rdi_core::fake::FakeDesktop::new();
                let moves = [crate::demo_animation::Move {
                    id: rdi_core::IconId::from("switch"),
                    origin: rdi_core::Point::new(20, 40),
                    target: rdi_core::Point::new(220, 240),
                }];
                desktop.add_icon(rdi_core::IconSnapshot::new(
                    moves[0].id.clone(),
                    "switch",
                    None,
                    false,
                    moves[0].origin,
                ));
                let controller = DesktopController::new(desktop.clone()).unwrap();
                let mut timeline = prepare_timeline(&controller, &moves, None)
                    .unwrap()
                    .open_timeline()
                    .unwrap();
                timeline.seek(0.5).unwrap();
                timeline.set_speed(0.5).unwrap();
                timeline.set_real_icons_visible(true).unwrap();
                let mut playback = Playback::default();
                let choices = shader::BuiltinShader::ALL
                    .iter()
                    .copied()
                    .map(Some)
                    .chain([None]);
                for (index, selected) in choices.enumerate() {
                    change_shader(&controller, &moves, &mut timeline, &mut playback, selected)
                        .unwrap();
                    assert_eq!(timeline.state(), TimelineState::Paused);
                    assert_eq!(timeline.position(), 0.5);
                    assert_eq!(timeline.speed(), 0.5);
                    assert!(timeline.real_icons_visible());
                    assert!(desktop.real_icons_visible());
                    let snapshot = timeline.snapshot();
                    assert_eq!(snapshot[0].origin, moves[0].origin);
                    assert_eq!(snapshot[0].target, moves[0].target);
                    assert_eq!(snapshot[0].current, rdi_core::Point::new(120, 140));
                    let plans = desktop.last_overlay_plans();
                    assert_eq!(
                        plans[0].effect.as_ref().map(|effect| &effect.shader),
                        selected.map(shader::compile).transpose().unwrap().as_ref()
                    );
                    assert_eq!(desktop.overlay_finalize_log().len(), index + 1);
                    assert!(desktop.overlay_active());
                }
                timeline.set_real_icons_visible(false).unwrap();
                playback
                    .apply(Action::Play(0.0, 0.0625), &timeline)
                    .unwrap();
                change_shader(
                    &controller,
                    &moves,
                    &mut timeline,
                    &mut playback,
                    Some(shader::BuiltinShader::Glitch),
                )
                .unwrap();
                assert_eq!(timeline.state(), TimelineState::Playing);
                assert!(timeline.position() <= 0.5);
                assert_eq!(timeline.speed(), 0.0625);
                assert!(!timeline.real_icons_visible());
                assert_eq!(playback.destination, 0.0);
                assert!(!playback.forward);
                assert!(playback.handle.is_some());
                timeline.seek(0.5).unwrap();
                let (_, expected) = finish(&timeline, Ok(mode)).unwrap();
                verify_positions(&controller, &expected).unwrap();
                drop(timeline);
                controller.shutdown();
                assert!(!desktop.overlay_active());
                assert!(desktop.real_icons_visible());
                assert_eq!(
                    desktop.position_of(&moves[0].id),
                    Some(match mode {
                        TimelineCloseMode::RestoreOrigins => moves[0].origin,
                        TimelineCloseMode::LeaveInPlace => rdi_core::Point::new(120, 140),
                        TimelineCloseMode::TeleportToTarget => moves[0].target,
                    })
                );
                assert_eq!(
                    desktop.overlay_finalize_log().len(),
                    shader::BuiltinShader::ALL.len() + 3
                );
            }
        }

        #[test]
        fn shader_switch_verification_failure_leaves_no_overlay() {
            let desktop = rdi_core::fake::FakeDesktop::new();
            let moves = [crate::demo_animation::Move {
                id: rdi_core::IconId::from("switch-error"),
                origin: rdi_core::Point::new(0, 0),
                target: rdi_core::Point::new(100, 100),
            }];
            desktop.add_icon(rdi_core::IconSnapshot::new(
                moves[0].id.clone(),
                "switch-error",
                None,
                false,
                moves[0].origin,
            ));
            let controller = DesktopController::new(desktop.clone()).unwrap();
            let mut timeline = prepare_timeline(&controller, &moves, None)
                .unwrap()
                .open_timeline()
                .unwrap();
            timeline.seek(0.5).unwrap();
            desktop.set_next_list_error(rdi_core::DesktopError::BackendUnavailable(
                "verification failed".into(),
            ));
            let error = change_shader(
                &controller,
                &moves,
                &mut timeline,
                &mut Playback::default(),
                Some(shader::BuiltinShader::Glitch),
            )
            .unwrap_err();
            assert!(error.to_string().contains("verification failed"));
            assert_eq!(timeline.state(), TimelineState::Closed);
            assert!(finish(&timeline, Err(error)).is_err());
            drop(timeline);
            controller.shutdown();
            assert_eq!(desktop.position_of(&moves[0].id), Some(moves[0].origin));
            assert!(!desktop.overlay_active());
            assert!(desktop.real_icons_visible());
            assert_eq!(desktop.overlay_finalize_log().len(), 1);
        }

        #[test]
        fn commands_enforce_speed_limit_and_positions() {
            assert_eq!(command("play 0.5 2", 1.0), Ok(Action::Play(0.5, 2.0)));
            assert_eq!(command("play 0.5 40", 1.0), Ok(Action::Play(0.5, 40.0)));
            for value in ["0", "-1", "50.01", "NaN", "inf"] {
                assert!(command(&format!("speed {value}"), 1.0).is_err());
                assert!(command(&format!("play 0.5 {value}"), 1.0).is_err());
            }
            for value in ["-0.1", "1.1", "NaN", "inf"] {
                assert!(command(&format!("seek {value}"), 1.0).is_err());
            }
            assert_eq!(command("speed 0.125", 1.0), Ok(Action::Speed(0.125)));
            assert!(command("play 0.5 1 extra", 1.0).is_err());
            let args = Args::parse(["--no-shader".into(), "--dry-run".into()]).unwrap();
            assert!(args.no_shader && args.dry_run);
            assert!(Args::parse(["--log-file".into()]).is_err());
            assert!(Args::parse(["--shader".into()]).is_err());
            assert!(Args::parse(["--shader".into(), "unknown".into()]).is_err());
            assert!(
                Args::parse([
                    "--shader".into(),
                    "particle-vortex".into(),
                    "--no-shader".into()
                ])
                .is_err()
            );
            let args = Args::parse(["--shader".into(), "particle-vortex".into()]).unwrap();
            assert_eq!(args.shader, Some(shader::BuiltinShader::ParticleVortex));
            for &builtin in shader::BuiltinShader::ALL {
                let args = Args::parse(["--shader".into(), builtin.name().into()]).unwrap();
                assert_eq!(args.selected_shader(), Some(builtin));
                assert!(shader_names().contains(builtin.name()));
            }
            assert_eq!(
                Args::default().selected_shader(),
                Some(shader::BuiltinShader::Glitch)
            );
            assert_eq!(
                Args::parse(["--no-shader".into()])
                    .unwrap()
                    .selected_shader(),
                None
            );
        }

        #[test]
        fn particle_specs_use_four_seconds_and_valid_defaults() {
            let program = shader::compile(shader::BuiltinShader::ParticleVortex).unwrap();
            let (movement, envelope) = crate::demo_animation::test_support::curves().unwrap();
            let specs = crate::demo_animation::test_support::build_specs(
                &[crate::demo_animation::Move {
                    id: rdi_core::IconId::from("dust"),
                    origin: rdi_core::Point::new(0, 0),
                    target: rdi_core::Point::new(100, 100),
                }],
                false,
                Some(&program),
                &movement,
                &envelope,
            );
            assert_eq!(
                specs[0].duration,
                rdi_core::Duration::fixed(Duration::from_secs(4))
            );
            let effect = specs[0].effect.as_ref().unwrap();
            effect.validate().unwrap();
            assert_eq!(effect.params, [3.0, 1.5, 2.0, 0.65]);
        }

        #[test]
        fn editor_rebuild_preserves_state_and_recovers_failed_open() {
            let desktop = rdi_core::fake::FakeDesktop::new();
            let id = rdi_core::IconId::from("editor");
            desktop.add_icon(rdi_core::IconSnapshot::new(id.clone(), "editor", None, false, rdi_core::Point::ZERO));
            let moves = vec![crate::demo_animation::Move { id: id.clone(), origin: rdi_core::Point::ZERO, target: rdi_core::Point::new(100, 0) }];
            let controller = DesktopController::new(desktop.clone()).unwrap();
            let previous = Config::new(None);
            let mut timeline = prepare_configuration(&controller, &moves, &previous, None).unwrap().open_timeline().unwrap();
            let mut playback = Playback { destination: 0.0, forward: false, ..Default::default() };
            timeline.seek(0.37).unwrap();
            timeline.set_speed(2.0).unwrap();
            timeline.set_real_icons_visible(true).unwrap();
            let mut next = Config::new(Some(shader::BuiltinShader::SilkFlow));
            next.seconds = previous.seconds;
            let program = rebuild_configuration(&controller, &moves, &mut timeline, &mut playback, &previous, None, &next, 2.0).unwrap();
            assert_eq!(timeline.position(), 0.37);
            assert_eq!(timeline.speed(), 2.0);
            assert!(timeline.real_icons_visible());
            assert_eq!(playback.destination, 0.0);
            assert!(!playback.forward);
            assert_eq!(timeline.state(), TimelineState::Paused);
            desktop.set_next_overlay_error(rdi_core::DesktopError::BackendUnavailable("test open failure".into()));
            let error = rebuild_configuration(&controller, &moves, &mut timeline, &mut playback, &next, program.as_ref(), &previous, 1.0).unwrap_err();
            assert!(error.to_string().contains("previous configuration restored"), "{error}");
            assert_eq!(timeline.position(), 0.37);
            assert_eq!(timeline.speed(), 2.0);
            assert!(desktop.overlay_active());
            assert!(timeline.real_icons_visible());
            finish(&timeline, Ok(TimelineCloseMode::RestoreOrigins)).unwrap();
            assert_eq!(desktop.position_of(&id), Some(rdi_core::Point::ZERO));
            controller.shutdown();
        }

        #[test]
        fn duration_slider_mapping_and_small_layouts() {
            assert!(Slider::duration_layout(80, 8).is_none());
            assert!(Slider::duration_layout(11, 24).is_none());
            for width in 12..=80 {
                let slider = Slider::duration_layout(width, 9).unwrap();
                assert_eq!(slider.row, 6);
                assert!(slider.right - slider.left < DURATION_STEPS.len() as u16);
                assert_eq!(slider.duration_speed(0, 4.0), 40.0);
                assert_eq!(slider.duration_speed(0, 5.0), MAX_SPEED);
                assert_eq!(slider.duration_speed(u16::MAX, 4.0), 0.8);
                for column in slider.left..=slider.right {
                    let speed = slider.duration_speed(column, 4.0);
                    assert!((0.8..=MAX_SPEED).contains(&speed));
                    assert_eq!(slider.column(Slider::duration_position(4.0 / speed)), column);
                }
                let mut output = Vec::new();
                slider.draw(&mut output, 0.5, Color::Yellow).unwrap();
                let text = String::from_utf8(output).unwrap();
                assert!(text.contains("\x1b[7;2H"));
                assert!(text.contains('O'));
            }
            assert_eq!(Slider::duration_position(0.001), 0.0);
            assert_eq!(Slider::duration_position(100.0), 1.0);
            let slider = Slider::duration_layout(80, 24).unwrap();
            for pipeline in [None, Some(rdi_core::ShaderPipeline::Sprite), Some(rdi_core::ShaderPipeline::Particles)] {
                let base = crate::demo_animation::test_support::animation_duration(pipeline).as_secs_f64();
                for (index, &seconds) in DURATION_STEPS.iter().enumerate() {
                    let expected = if index < 20 { (index + 1) as f64 / 10.0 } else { 2.0 + (index - 19) as f64 / 2.0 };
                    assert_eq!(seconds, expected);
                    let speed = slider.duration_speed(slider.left + index as u16, base);
                    assert!((base / speed - seconds).abs() < 1e-12);
                    assert!(speed <= MAX_SPEED);
                }
            }
        }

        #[test]
        fn duration_slider_drag_is_exclusive_and_cancelled_by_modal_input() {
            let size = (80, 24);
            let duration = Slider::duration_layout(size.0, size.1).unwrap();
            let position = Slider::layout(size.0, size.1);
            let mut input = Input::default();
            let dispatch = |input: &mut Input, event: Event| {
                input.duration_event(&event, size, 4.0, 1.0)
                    .or_else(|| input.event(event, position, 0.5, 1.0))
            };
            assert_eq!(dispatch(&mut input, mouse(MouseEventKind::Drag(MouseButton::Left), 20, 6)), None);
            assert_eq!(dispatch(&mut input, mouse(MouseEventKind::Down(MouseButton::Left), 2, 3)), Some(Action::Seek(0.0)));
            assert!(matches!(dispatch(&mut input, mouse(MouseEventKind::Drag(MouseButton::Left), 20, 6)), Some(Action::Seek(_))));
            assert_eq!(dispatch(&mut input, mouse(MouseEventKind::Down(MouseButton::Left), duration.left, 6)), Some(Action::Speed(40.0)));
            assert!(!input.dragging);
            assert!(input.duration_dragging);
            assert_eq!(dispatch(&mut input, mouse(MouseEventKind::Drag(MouseButton::Left), 200, 3)), Some(Action::Speed(0.8)));
            assert_eq!(dispatch(&mut input, mouse(MouseEventKind::Drag(MouseButton::Left), 0, 20)), Some(Action::Speed(40.0)));
            for cancel in [
                mouse(MouseEventKind::Up(MouseButton::Left), 20, 6),
                Event::Resize(40, 8),
                key(KeyCode::Char(':')),
            ] {
                dispatch(&mut input, mouse(MouseEventKind::Down(MouseButton::Left), 2, 6));
                dispatch(&mut input, cancel);
                assert!(!input.duration_dragging);
                assert_eq!(dispatch(&mut input, mouse(MouseEventKind::Drag(MouseButton::Left), 20, 6)), None);
            }
            assert_eq!(dispatch(&mut input, mouse(MouseEventKind::Down(MouseButton::Left), 2, 6)), None);
            input.command = None;
            assert_eq!(dispatch(&mut input, mouse(MouseEventKind::Down(MouseButton::Left), 0, 6)), None);
            assert!(!input.duration_dragging);
            input.duration_dragging = true;
            assert_eq!(input.duration_event(&mouse(MouseEventKind::Drag(MouseButton::Left), 20, 6), (80, 8), 4.0, 1.0), None);
            assert!(!input.duration_dragging);
        }

        #[test]
        fn duration_wheel_reaches_every_step_on_narrow_terminals() {
            for width in [12, 80] {
                for base in [2.0, 4.0] {
                    let mut input = Input::default();
                    let mut speed = base / DURATION_STEPS[0];
                    for &seconds in DURATION_STEPS.iter().skip(1) {
                        let action = input.duration_event(
                            &mouse(MouseEventKind::ScrollDown, 2, 6), (width, 9), base, speed,
                        );
                        assert_eq!(action, Some(Action::Speed(base / seconds)));
                        speed = base / seconds;
                    }
                    assert_eq!(input.duration_event(
                        &mouse(MouseEventKind::ScrollDown, 2, 6), (width, 9), base, speed,
                    ), Some(Action::Speed(speed)));
                    for &seconds in DURATION_STEPS.iter().rev().skip(1) {
                        let action = input.duration_event(
                            &mouse(MouseEventKind::ScrollUp, 2, 6), (width, 9), base, speed,
                        );
                        assert_eq!(action, Some(Action::Speed(base / seconds)));
                        speed = base / seconds;
                    }
                    assert_eq!(input.duration_event(
                        &mouse(MouseEventKind::ScrollUp, 2, 6), (width, 9), base, speed,
                    ), Some(Action::Speed(speed)));
                    assert_eq!(input.duration_event(
                        &mouse(MouseEventKind::ScrollUp, 2, 3), (width, 9), base, speed,
                    ), None);
                    for (command, dragging, duration_dragging) in [
                        (Some(String::new()), false, false),
                        (None, true, false),
                        (None, false, true),
                    ] {
                        let mut input = Input { command, dragging, duration_dragging, ..Default::default() };
                        assert_eq!(input.duration_event(
                            &mouse(MouseEventKind::ScrollDown, 2, 6), (width, 9), base, speed,
                        ), None);
                    }
                }
            }
        }

        #[test]
        fn duration_changes_preserve_playback_and_overlay() {
            let desktop = rdi_core::fake::FakeDesktop::new();
            let moves = [crate::demo_animation::Move {
                id: rdi_core::IconId::from("duration"),
                origin: rdi_core::Point::new(0, 0),
                target: rdi_core::Point::new(100, 100),
            }];
            desktop.add_icon(rdi_core::IconSnapshot::new(
                moves[0].id.clone(), "duration", None, false, moves[0].origin,
            ));
            let controller = DesktopController::new(desktop.clone()).unwrap();
            let timeline = prepare_timeline(&controller, &moves, None).unwrap().open_timeline().unwrap();
            let mut playback = Playback::default();
            let slider = Slider::duration_layout(80, 24).unwrap();
            timeline.seek(0.5).unwrap();
            playback.apply(Action::Speed(slider.duration_speed(slider.right, 2.0)), &timeline).unwrap();
            assert_eq!(timeline.position(), 0.5);
            assert_eq!(timeline.state(), TimelineState::Paused);
            for destination in [1.0, 0.0] {
                playback.apply(Action::Play(destination, 0.125), &timeline).unwrap();
                let before = timeline.position();
                playback.apply(Action::Speed(slider.duration_speed(slider.right, 2.0)), &timeline).unwrap();
                assert_eq!(timeline.state(), TimelineState::Playing);
                assert_eq!(timeline.speed(), 0.4);
                assert_eq!(playback.destination, destination);
                assert_eq!(playback.forward, destination == 1.0);
                assert!((timeline.position() - before).abs() < 0.05);
                assert_eq!(playback.handle.as_ref().unwrap().wait_timeout(Duration::ZERO).unwrap(), None);
                timeline.pause().unwrap();
            }
            let picker = ShaderPicker { selected: None, highlighted: None };
            let mut output = Vec::new();
            draw(&mut output, (80, 24), &timeline, &playback, &Input::default(), "summary", &picker).unwrap();
            assert!(String::from_utf8(output).unwrap().contains("Duration: 5.00s"));
            output = Vec::new();
            draw(&mut output, (12, 9), &timeline, &playback, &Input::default(), "summary", &picker).unwrap();
            assert!(String::from_utf8(output).unwrap().contains("5.00s"));
            output = Vec::new();
            draw(&mut output, (80, 8), &timeline, &playback, &Input::default(), "summary", &picker).unwrap();
            assert!(!String::from_utf8(output).unwrap().contains("Duration:"));
            for speed in [20.0, MAX_SPEED] {
                timeline.seek(0.5).unwrap();
                playback.apply(Action::Speed(speed), &timeline).unwrap();
                assert_eq!(timeline.position(), 0.5);
                assert_eq!(timeline.state(), TimelineState::Paused);
                playback.apply(Action::Toggle, &timeline).unwrap();
                assert_eq!(playback.handle.as_ref().unwrap().wait_timeout(Duration::from_secs(2)).unwrap(), Some(PlaybackOutcome::Reached));
                assert_eq!(timeline.position(), 0.0);
                playback.apply(Action::Play(1.0, speed), &timeline).unwrap();
                assert_eq!(playback.handle.as_ref().unwrap().wait_timeout(Duration::from_secs(2)).unwrap(), Some(PlaybackOutcome::Reached));
                assert_eq!(timeline.position(), 1.0);
                playback.destination = 0.0;
                playback.forward = false;
            }
            assert!(desktop.overlay_active());
            assert!(desktop.overlay_finalize_log().is_empty());
            finish(&timeline, Ok(TimelineCloseMode::RestoreOrigins)).unwrap();
            controller.shutdown();
            assert_eq!(desktop.position_of(&moves[0].id), Some(moves[0].origin));
        }

        #[test]
        fn slider_mouse_is_exclusive_clamped_and_resize_safe() {
            assert!(Slider::layout(1, 2).is_none());
            let slider = Slider::layout(80, 24).unwrap();
            assert_eq!(slider.position(slider.left), 0.0);
            assert_eq!(slider.position(slider.right), 1.0);
            let mut input = Input::default();
            assert_eq!(
                input.event(
                    mouse(MouseEventKind::Down(MouseButton::Left), 10, 1),
                    Some(slider),
                    0.0,
                    1.0
                ),
                None
            );
            assert_eq!(
                input.event(
                    mouse(MouseEventKind::Down(MouseButton::Left), slider.left, 3),
                    Some(slider),
                    0.0,
                    1.0
                ),
                Some(Action::Seek(0.0))
            );
            assert_eq!(
                input.event(
                    mouse(MouseEventKind::Drag(MouseButton::Left), 200, 5),
                    Some(slider),
                    0.0,
                    1.0
                ),
                Some(Action::Seek(1.0))
            );
            input.event(Event::Resize(40, 10), Some(slider), 0.0, 1.0);
            assert_eq!(
                input.event(
                    mouse(MouseEventKind::Drag(MouseButton::Left), 20, 3),
                    Some(slider),
                    0.0,
                    1.0
                ),
                None
            );
            assert_eq!(
                input.event(
                    mouse(MouseEventKind::ScrollUp, 20, 3),
                    Some(slider),
                    0.0,
                    1.0
                ),
                None
            );
        }

        #[test]
        fn keys_seek_and_speed_cap_without_repeat_toggle() {
            let mut input = Input::default();
            assert_eq!(
                input.event(key(KeyCode::Right), None, 0.99, 1.0),
                Some(Action::Seek(1.0))
            );
            assert_eq!(
                input.event(key(KeyCode::Left), None, 0.0, 1.0),
                Some(Action::Seek(0.0))
            );
            assert_eq!(
                input.event(key(KeyCode::Char('+')), None, 0.0, 2.0),
                Some(Action::Speed(4.0))
            );
            assert_eq!(
                input.event(key(KeyCode::Up), None, 0.0, 1.0),
                Some(Action::Speed(2.0))
            );
            assert_eq!(
                input.event(key(KeyCode::Down), None, 0.0, 0.01),
                Some(Action::Speed(0.01))
            );
            assert_eq!(
                input.event(
                    Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT)),
                    None,
                    0.5,
                    1.0
                ),
                Some(Action::Seek(0.6))
            );
            assert_eq!(
                input.event(
                    Event::Key(KeyEvent::new_with_kind(
                        KeyCode::Right,
                        KeyModifiers::NONE,
                        KeyEventKind::Release
                    )),
                    None,
                    0.5,
                    1.0
                ),
                None
            );
            assert_eq!(
                input.event(
                    Event::Key(KeyEvent::new_with_kind(
                        KeyCode::Char(' '),
                        KeyModifiers::NONE,
                        KeyEventKind::Repeat
                    )),
                    None,
                    0.0,
                    1.0
                ),
                None
            );
            input.event(key(KeyCode::Char(':')), None, 0.0, 1.0);
            for character in "speed 2".chars() {
                input.event(key(KeyCode::Char(character)), None, 0.0, 1.0);
            }
            assert_eq!(
                input.event(key(KeyCode::Enter), None, 0.0, 1.0),
                Some(Action::Speed(2.0))
            );
        }

        #[test]
        fn drag_coalescing_preserves_keys_and_release_order() {
            let mut batch = Vec::new();
            push_event(
                &mut batch,
                mouse(MouseEventKind::Down(MouseButton::Left), 3, 3),
            );
            push_event(
                &mut batch,
                mouse(MouseEventKind::Drag(MouseButton::Left), 4, 3),
            );
            push_event(
                &mut batch,
                mouse(MouseEventKind::Drag(MouseButton::Left), 5, 3),
            );
            push_event(&mut batch, key(KeyCode::Char('r')));
            push_event(
                &mut batch,
                mouse(MouseEventKind::Drag(MouseButton::Left), 6, 3),
            );
            push_event(
                &mut batch,
                mouse(MouseEventKind::Up(MouseButton::Left), 6, 3),
            );
            assert_eq!(batch.len(), 5);
            assert_eq!(
                batch[1],
                mouse(MouseEventKind::Drag(MouseButton::Left), 5, 3)
            );
            assert_eq!(batch[2], key(KeyCode::Char('r')));
            assert_eq!(batch[4], mouse(MouseEventKind::Up(MouseButton::Left), 6, 3));
        }

        #[test]
        fn close_modes_and_input_errors_finalize_exactly_once() {
            for mode in [
                TimelineCloseMode::RestoreOrigins,
                TimelineCloseMode::LeaveInPlace,
                TimelineCloseMode::TeleportToTarget,
            ] {
                for fail in [false, true] {
                    let desktop = rdi_core::fake::FakeDesktop::new();
                    let id = rdi_core::IconId::from("icon");
                    desktop.add_icon(rdi_core::IconSnapshot::new(
                        id.clone(),
                        "icon",
                        None,
                        false,
                        rdi_core::Point::new(0, 0),
                    ));
                    let spec = rdi_core::IconAnimationSpec::new(
                        id.clone(),
                        rdi_core::Point::new(100, 100),
                        rdi_core::Duration::fixed(Duration::from_secs(1)),
                        rdi_core::Curve::linear(),
                    );
                    let controller = DesktopController::new(desktop.clone()).unwrap();
                    let timeline = controller
                        .prepare(vec![spec], AnimationOptions::default())
                        .unwrap()
                        .open_timeline()
                        .unwrap();
                    timeline.seek(0.5).unwrap();
                    let result = if fail {
                        Err("input failed".into())
                    } else {
                        Ok(mode)
                    };
                    assert_eq!(finish(&timeline, result).is_err(), fail);
                    let coordinate = if fail {
                        0
                    } else {
                        match mode {
                            TimelineCloseMode::RestoreOrigins => 0,
                            TimelineCloseMode::LeaveInPlace => 50,
                            TimelineCloseMode::TeleportToTarget => 100,
                        }
                    };
                    drop(timeline);
                    controller.shutdown();
                    assert_eq!(
                        desktop.position_of(&id),
                        Some(rdi_core::Point::new(coordinate, coordinate))
                    );
                    assert_eq!(desktop.overlay_finalize_log().len(), 1);
                }
            }
        }

        #[test]
        fn h_toggles_only_in_normal_mode_without_repeating() {
            let mut input = Input::default();
            assert_eq!(
                input.event(key(KeyCode::Char('h')), None, 0.5, 1.0),
                Some(Action::ToggleRealIcons)
            );
            for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
                assert_eq!(
                    input.event(
                        Event::Key(KeyEvent::new_with_kind(
                            KeyCode::Char('h'),
                            KeyModifiers::NONE,
                            kind
                        )),
                        None,
                        0.5,
                        1.0
                    ),
                    None
                );
            }
            input.event(key(KeyCode::Char(':')), None, 0.5, 1.0);
            assert_eq!(input.event(key(KeyCode::Char('h')), None, 0.5, 1.0), None);
            assert_eq!(input.command.as_deref(), Some("h"));
        }

        #[test]
        fn fake_startup_scrubbing_and_reverse_keep_overlay_alive() {
            let desktop = rdi_core::fake::FakeDesktop::new();
            desktop.add_icon(rdi_core::IconSnapshot::new(
                rdi_core::IconId::from("icon"),
                "icon",
                None,
                false,
                rdi_core::Point::new(0, 0),
            ));
            let moves = vec![crate::demo_animation::Move {
                id: rdi_core::IconId::from("icon"),
                origin: rdi_core::Point::new(0, 0),
                target: rdi_core::Point::new(500, 300),
            }];
            let (movement, envelope) = crate::demo_animation::test_support::curves().unwrap();
            let mut specs =
                crate::demo_animation::test_support::build_specs(&moves, false, None, &movement, &envelope);
            assert!(specs[0].effect.is_none());
            specs[0].duration = rdi_core::Duration::fixed(Duration::from_millis(80));
            let controller = DesktopController::new(desktop.clone()).unwrap();
            let timeline = controller
                .prepare(specs, AnimationOptions::default())
                .unwrap()
                .open_timeline()
                .unwrap();
            let mut playback = Playback::default();
            playback.apply(Action::Play(0.5, 1.0), &timeline).unwrap();
            assert_eq!(
                playback
                    .handle
                    .as_ref()
                    .unwrap()
                    .wait_timeout(Duration::from_secs(3))
                    .unwrap(),
                Some(PlaybackOutcome::Reached)
            );
            assert_eq!(timeline.position(), 0.5);
            assert_eq!(timeline.state(), TimelineState::Paused);
            assert!(!timeline.real_icons_visible());
            assert!(!desktop.real_icons_visible());
            let snapshot = timeline.snapshot();
            playback.apply(Action::ToggleRealIcons, &timeline).unwrap();
            assert!(timeline.real_icons_visible());
            assert!(desktop.real_icons_visible());
            assert_eq!(timeline.position(), 0.5);
            assert_eq!(timeline.snapshot()[0].current, snapshot[0].current);
            playback.apply(Action::ToggleRealIcons, &timeline).unwrap();
            assert!(!desktop.real_icons_visible());
            playback.apply(Action::Toggle, &timeline).unwrap();
            assert_eq!(playback.destination, 1.0);
            playback.apply(Action::Pause, &timeline).unwrap();
            let paused = timeline.position();
            playback.apply(Action::Toggle, &timeline).unwrap();
            assert_eq!(playback.destination, 1.0);
            assert!(timeline.position() >= paused);
            playback.apply(Action::Seek(1.0), &timeline).unwrap();
            playback.apply(Action::Speed(2.0), &timeline).unwrap();
            assert!(playback.apply(Action::Speed(50.1), &timeline).is_err());
            playback
                .apply(Action::Play(0.0, timeline.speed()), &timeline)
                .unwrap();
            assert_eq!(
                playback
                    .handle
                    .as_ref()
                    .unwrap()
                    .wait_timeout(Duration::from_secs(3))
                    .unwrap(),
                Some(PlaybackOutcome::Reached)
            );
            assert!(desktop.overlay_finalize_log().is_empty());
            timeline.close(TimelineCloseMode::RestoreOrigins).unwrap();
            assert!(desktop.real_icons_visible());
            assert!(timeline.set_real_icons_visible(false).is_err());
            assert_eq!(desktop.position_of(&moves[0].id), Some(moves[0].origin));
        }
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::install_panic_hook();
    app::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("animate_timeline requires Windows.");
    std::process::exit(1);
}
