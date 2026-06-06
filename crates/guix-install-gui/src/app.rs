//! The iced application: renders the current prompt + progress rail, and
//! relays the user's answers back to the worker thread.

use std::sync::mpsc::Sender as SyncSender;
use std::sync::{Arc, Mutex};

use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::futures::stream;
use iced::keyboard::key::Named;
use iced::keyboard::{Event as KeyEvent, Key};
use iced::widget::operation::{RelativeOffset, focus, focus_next, focus_previous, snap_to};
use iced::widget::{
    Space, button, column, container, progress_bar, row, scrollable, svg, text, text_editor,
    text_input,
};
use iced::{Alignment, Element, Fill, Font, Length, Subscription, Task, Theme};

use guix_install_core::steps::StepId;

use crate::bridge::{PromptRequest, PromptResponse, RailEntry, UiEvent};
use crate::styles;

const ICON_SVG: &[u8] = include_bytes!("../assets/icon.svg");
const INPUT_ID: &str = "prompt-input";
const SELECT_ID: &str = "prompt-select";
/// Cap the content column on big screens so forms don't stretch to 4K width.
const CONTENT_MAX_WIDTH: f32 = 1000.0;
/// Above this many options, show a type-to-filter field over the list.
const SELECT_FILTER_THRESHOLD: usize = 12;

/// Receiver shared with the subscription, taken once on first poll.
type EventSlot = Arc<Mutex<Option<UnboundedReceiver<UiEvent>>>>;

#[derive(Debug, Clone)]
pub enum Message {
    Event(UiEvent),
    /// A select option was chosen (records highlight before submit).
    Highlight(usize),
    /// Pick a select option and submit it immediately (Summary action buttons).
    SubmitIndex(usize),
    InputChanged(String),
    /// Toggle plaintext visibility of the active password field.
    ToggleReveal,
    /// Type-to-filter text changed on a long select.
    SelectFilterChanged(String),
    Confirm(bool),
    /// A `text_editor` edit (in-app system.scm editor).
    EditAction(text_editor::Action),
    /// Editor Save: commit the buffer back to the worker.
    EditSave,
    /// Dismiss the welcome screen and enter the interview.
    Welcome,
    /// Move focus to the next/previous focusable widget (Tab / Shift-Tab).
    FocusNext,
    FocusPrev,
    /// Raw key press, interpreted in `update` against the active prompt.
    Key(Named),
    Next,
    Back,
    /// Failure screen: quit the app.
    Abort,
    /// Failure screen: re-run the install (resume skips completed phases).
    Retry,
    /// Complete screen: reboot the machine.
    Reboot,
    /// Complete screen: power off the machine.
    Shutdown,
    /// GUI-side timer tick that advances the busy spinner during network waits.
    SpinnerTick,
    /// Keyboard step: scratch "type to test your layout" text changed.
    KbdTestChanged(String),
}

/// The active prompt plus its in-progress local edit state.
enum Active {
    None,
    Select {
        prompt: String,
        options: Vec<String>,
        selected: usize,
        filter: String,
    },
    Input {
        prompt: String,
        value: String,
    },
    Password {
        prompt: String,
        value: String,
        revealed: bool,
    },
    Confirm {
        prompt: String,
        default: bool,
    },
    Edit {
        title: String,
        content: text_editor::Content,
    },
}

/// The 8 exec phases as a fixed checklist, in execution order.
const PHASE_LABELS: [&str; 8] = [
    "Partition",
    "Format",
    "Mount",
    "Swap",
    "Config",
    "Authorize",
    "guix pull",
    "Install",
];

/// Tracks the exec pipeline once the interview is done and `install_phase`
/// events start arriving.
#[derive(Default)]
struct Install {
    /// 1-based number of the phase currently running; `0` before any starts.
    current: u8,
    /// Highest 1-based phase reported done.
    done_through: u8,
    /// Overall weighted percent (0.0..=1.0) from the core's `progress` calls.
    pct: f32,
    /// Last `progress` message line.
    status: String,
    /// Compact live detail from the in-flight guix op.
    detail: String,
}

/// Failure-screen payload.
#[derive(Clone)]
struct Failure {
    summary: String,
    detail: String,
}

pub struct State {
    to_worker: SyncSender<PromptResponse>,
    /// Wakes the worker to re-run the install after a failure.
    retry: SyncSender<()>,
    events: EventSlot,
    rail: Vec<RailEntry>,
    current_step: StepId,
    active: Active,
    log: Vec<String>,
    /// Info/warn lines accumulated since the Summary step began; rendered as
    /// the Summary body instead of vanishing into the rolling log.
    summary: Vec<SummaryLine>,
    /// Most recent warn/error to show inline under the active prompt.
    feedback: Option<Feedback>,
    finished: bool,
    /// True once the worker is running a real install (not interview-only).
    dry_run: bool,
    /// Set once exec begins (first `install_phase`); drives the Install screen.
    install: Option<Install>,
    /// Set when a phase fails; drives the failure screen.
    failure: Option<Failure>,
    /// True until the user dismisses the branded welcome screen.
    welcome: bool,
    /// Trimmed tagline pulled from `/etc/issue` at boot (best-effort).
    tagline: String,
    /// Advances on each `SpinnerTick` while the idle "Working…" view is shown.
    spinner_frame: usize,
    /// Scratch text for the Keyboard step's "type to test your layout" field.
    kbd_test: String,
}

#[derive(Clone)]
struct SummaryLine {
    text: String,
    danger: bool,
}

#[derive(Clone)]
struct Feedback {
    text: String,
    danger: bool,
}

impl State {
    pub fn new(
        to_worker: SyncSender<PromptResponse>,
        events: UnboundedReceiver<UiEvent>,
        retry: SyncSender<()>,
        dry_run: bool,
    ) -> (Self, Task<Message>) {
        let state = State {
            to_worker,
            retry,
            events: Arc::new(Mutex::new(Some(events))),
            rail: Vec::new(),
            current_step: StepId::Network,
            active: Active::None,
            log: Vec::new(),
            summary: Vec::new(),
            feedback: None,
            finished: false,
            dry_run,
            install: None,
            failure: None,
            welcome: true,
            tagline: read_tagline(),
            spinner_frame: 0,
            kbd_test: String::new(),
        };
        (state, focus_input())
    }

    pub fn title(&self) -> String {
        "Guix System Installer".to_string()
    }

    pub fn theme(&self) -> Theme {
        Theme::custom(
            "GuixGold".to_string(),
            iced::theme::Palette {
                background: styles::BG,
                text: styles::TEXT,
                primary: styles::PRIMARY,
                success: styles::SUCCESS,
                warning: styles::WARNING,
                danger: styles::DANGER,
            },
        )
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let events = Subscription::run_with(
            SubData {
                slot: self.events.clone(),
            },
            drain,
        )
        .map(Message::Event);

        let keys = iced::keyboard::listen().filter_map(key_message);

        // Drive the busy spinner only while the idle "Working…" view is on
        // screen; it auto-stops the moment a prompt or any other screen takes
        // over. The worker thread blocks on sync network calls, but iced's
        // event loop and this timer run independently, so motion continues.
        let spinner = if self.is_working() {
            iced::time::every(std::time::Duration::from_millis(120)).map(|_| Message::SpinnerTick)
        } else {
            Subscription::none()
        };

        Subscription::batch([events, keys, spinner])
    }

    /// True exactly when `view_content` falls through to the idle `Active::None`
    /// "Working…" arm — mirrors every early-return guard in `view_content` plus
    /// the `Active::None if self.finished` ("Done") split.
    fn is_working(&self) -> bool {
        let welcome = self.welcome && self.install.is_none() && self.failure.is_none();
        let editing = matches!(self.active, Active::Edit { .. });
        let summary = self.current_step == StepId::Summary && self.has_summary_prompt();
        !welcome
            && !editing
            && self.failure.is_none()
            && self.install.is_none()
            && !summary
            && matches!(self.active, Active::None)
            && !self.finished
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Event(event) => self.handle_event(event),
            Message::Highlight(i) => {
                if let Active::Select {
                    selected, options, ..
                } = &mut self.active
                    && i < options.len()
                {
                    *selected = i;
                }
                self.scroll_to_highlight()
            }
            Message::SubmitIndex(i) => {
                if let Active::Select {
                    selected, options, ..
                } = &mut self.active
                    && i < options.len()
                {
                    *selected = i;
                }
                self.submit()
            }
            Message::InputChanged(s) => {
                match &mut self.active {
                    Active::Input { value, .. } | Active::Password { value, .. } => *value = s,
                    _ => {}
                }
                Task::none()
            }
            Message::ToggleReveal => {
                if let Active::Password { revealed, .. } = &mut self.active {
                    *revealed = !*revealed;
                }
                Task::none()
            }
            Message::SelectFilterChanged(s) => {
                if let Active::Select {
                    options,
                    selected,
                    filter,
                    ..
                } = &mut self.active
                {
                    *filter = s;
                    let matches = filtered_indices(options, filter);
                    if !matches.contains(selected)
                        && let Some(&first) = matches.first()
                    {
                        *selected = first;
                    }
                    return self.scroll_to_highlight();
                }
                Task::none()
            }
            Message::Confirm(value) => {
                if matches!(self.active, Active::Confirm { .. }) {
                    self.active = Active::None;
                    let _ = self.to_worker.send(PromptResponse::Bool(value));
                }
                Task::none()
            }
            Message::EditAction(action) => {
                if let Active::Edit { content, .. } = &mut self.active {
                    content.perform(action);
                }
                Task::none()
            }
            Message::EditSave => {
                if let Active::Edit { content, .. } = &self.active {
                    let edited = content.text();
                    self.active = Active::None;
                    let _ = self.to_worker.send(PromptResponse::Edited(Some(edited)));
                }
                Task::none()
            }
            Message::Welcome => {
                self.welcome = false;
                Task::none()
            }
            Message::FocusNext => focus_next(),
            Message::FocusPrev => focus_previous(),
            Message::Key(named) => match named {
                Named::Enter if self.welcome => {
                    self.welcome = false;
                    Task::none()
                }
                Named::Enter if self.finished && self.install.is_some() => {
                    reboot();
                    Task::none()
                }
                // The editor owns Enter (newline); never auto-submit there.
                _ if matches!(self.active, Active::Edit { .. }) => Task::none(),
                Named::ArrowUp if matches!(self.active, Active::Select { .. }) => {
                    self.move_highlight(-1)
                }
                Named::ArrowDown if matches!(self.active, Active::Select { .. }) => {
                    self.move_highlight(1)
                }
                Named::Enter if matches!(self.active, Active::Select { .. }) => self.submit(),
                Named::Enter if matches!(self.active, Active::Confirm { .. }) => self.submit(),
                _ => Task::none(),
            },
            Message::Next => self.submit(),
            Message::Back => {
                if self.welcome {
                    return Task::none();
                }
                match &self.active {
                    Active::Edit { .. } => {
                        self.active = Active::None;
                        let _ = self.to_worker.send(PromptResponse::Edited(None));
                    }
                    Active::None => {}
                    _ => {
                        self.active = Active::None;
                        let _ = self.to_worker.send(PromptResponse::Cancelled);
                    }
                }
                Task::none()
            }
            Message::Abort => iced::exit(),
            Message::Retry => {
                // Wake the parked worker; it re-enters the resume path, which
                // skips completed phases. Clear the failure screen so the next
                // phase events redraw the Install checklist.
                self.failure = None;
                self.install = None;
                let _ = self.retry.send(());
                Task::none()
            }
            Message::Reboot => {
                reboot();
                Task::none()
            }
            Message::Shutdown => {
                shutdown();
                Task::none()
            }
            Message::SpinnerTick => {
                self.spinner_frame = self.spinner_frame.wrapping_add(1);
                Task::none()
            }
            Message::KbdTestChanged(s) => {
                self.kbd_test = s;
                Task::none()
            }
        }
    }

    fn move_highlight(&mut self, delta: i32) -> Task<Message> {
        if let Active::Select {
            selected,
            options,
            filter,
            ..
        } = &mut self.active
        {
            let matches = filtered_indices(options, filter);
            if !matches.is_empty() {
                let pos = matches.iter().position(|i| i == selected).unwrap_or(0) as i32;
                let new_pos = (pos + delta).rem_euclid(matches.len() as i32) as usize;
                *selected = matches[new_pos];
            }
            self.scroll_to_highlight()
        } else {
            Task::none()
        }
    }

    /// Submit the active prompt's current value to the worker.
    fn submit(&mut self) -> Task<Message> {
        let response = match &self.active {
            Active::Select { selected, .. } => Some(PromptResponse::Index(*selected)),
            Active::Input { value, .. } | Active::Password { value, .. } => {
                Some(PromptResponse::Text(value.clone()))
            }
            Active::Confirm { default, .. } => Some(PromptResponse::Bool(*default)),
            Active::Edit { .. } | Active::None => None,
        };
        if let Some(r) = response {
            self.active = Active::None;
            let _ = self.to_worker.send(r);
        }
        Task::none()
    }

    fn handle_event(&mut self, event: UiEvent) -> Task<Message> {
        match event {
            UiEvent::Prompt(req) => {
                self.active = match req {
                    PromptRequest::Select {
                        prompt,
                        options,
                        default,
                    } => Active::Select {
                        prompt,
                        options,
                        selected: default,
                        filter: String::new(),
                    },
                    PromptRequest::Input { prompt, default } => Active::Input {
                        prompt,
                        value: default,
                    },
                    PromptRequest::Password { prompt } => Active::Password {
                        prompt,
                        value: String::new(),
                        revealed: false,
                    },
                    PromptRequest::Confirm { prompt, default } => {
                        Active::Confirm { prompt, default }
                    }
                    PromptRequest::Edit { title, initial } => Active::Edit {
                        title,
                        content: text_editor::Content::with_text(&initial),
                    },
                };
                if self.current_step == StepId::Keyboard {
                    self.kbd_test.clear();
                }
                Task::batch([focus_input(), self.scroll_to_highlight()])
            }
            UiEvent::Rail { entries, current } => {
                if current != self.current_step {
                    self.feedback = None;
                    if current == StepId::Summary {
                        self.summary.clear();
                    }
                }
                self.current_step = current;
                self.rail = entries;
                Task::none()
            }
            UiEvent::Info(m) => {
                if self.current_step == StepId::Summary {
                    self.summary.push(SummaryLine {
                        text: m.clone(),
                        danger: false,
                    });
                }
                self.log.push(m);
                Task::none()
            }
            UiEvent::Warn(m) => {
                let danger = m.contains("ALL DATA WILL BE LOST");
                self.feedback = Some(Feedback {
                    text: m.clone(),
                    danger,
                });
                if self.current_step == StepId::Summary {
                    self.summary.push(SummaryLine {
                        text: m.clone(),
                        danger,
                    });
                }
                self.log.push(format!("warning: {m}"));
                Task::none()
            }
            UiEvent::Error(m) => {
                self.feedback = Some(Feedback {
                    text: m.clone(),
                    danger: true,
                });
                self.log.push(format!("error: {m}"));
                Task::none()
            }
            UiEvent::Progress { msg, pct } => {
                if let (Some(inst), Some(p)) = (self.install.as_mut(), pct) {
                    inst.pct = p;
                    inst.status = msg.clone();
                }
                let line = match pct {
                    Some(p) => format!("[{:.0}%] {msg}", p * 100.0),
                    None => format!("... {msg}"),
                };
                self.log.push(line);
                Task::none()
            }
            UiEvent::Phase { num, label } => {
                let inst = self.install.get_or_insert_with(Install::default);
                // A new phase starting means every earlier phase is done.
                if num > inst.current {
                    inst.done_through = num.saturating_sub(1);
                }
                inst.current = num;
                inst.status = label.clone();
                inst.detail.clear();
                self.active = Active::None;
                Task::none()
            }
            UiEvent::GuixDetail(detail) => {
                if let Some(inst) = self.install.as_mut() {
                    inst.detail = detail;
                }
                Task::none()
            }
            UiEvent::Failed { summary, detail } => {
                // Mark the phase that was running as not-done; it failed.
                self.failure = Some(Failure { summary, detail });
                self.active = Active::None;
                Task::none()
            }
            UiEvent::Finished => {
                if let Some(inst) = self.install.as_mut() {
                    inst.done_through = 8;
                    inst.current = 8;
                    inst.pct = 1.0;
                }
                self.finished = true;
                self.active = Active::None;
                Task::none()
            }
        }
    }

    fn scroll_to_highlight(&self) -> Task<Message> {
        if let Active::Select {
            selected,
            options,
            filter,
            ..
        } = &self.active
        {
            let matches = filtered_indices(options, filter);
            if matches.len() > 1 {
                let pos = matches.iter().position(|i| i == selected).unwrap_or(0);
                let y = pos as f32 / (matches.len() - 1) as f32;
                return snap_to(
                    iced::widget::Id::new(SELECT_ID),
                    RelativeOffset { x: 0.0, y },
                );
            }
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        // Cap and center the content column so forms stay readable from
        // 1024×768 up to 4K instead of stretching edge-to-edge.
        let content = container(self.view_content())
            .max_width(CONTENT_MAX_WIDTH)
            .width(Fill)
            .height(Fill)
            .center_x(Fill);
        row![self.view_rail(), content].height(Fill).into()
    }

    fn view_rail(&self) -> Element<'_, Message> {
        let icon = svg(svg::Handle::from_memory(ICON_SVG)).width(32).height(32);
        let brand = row![icon, text("Guix").size(18).font(styles::BOLD)]
            .spacing(10)
            .align_y(Alignment::Center);

        // Once exec begins, the interview steps are all done and an "Install"
        // entry becomes the current rail row.
        let installing = self.install.is_some() || self.failure.is_some();

        let mut items = column![brand, Space::new().height(16)].spacing(2);
        for entry in &self.rail {
            // During exec every interview step is complete.
            let done = installing || entry.done;
            let current = entry.current && !installing;
            let marker = if done {
                "\u{2713}" // ✓
            } else if current {
                "\u{25B8}" // ▸
            } else {
                "  "
            };
            let label = text(format!("{marker} {}", entry.label)).size(14);
            let row_box = container(label)
                .width(Fill)
                .padding([10, 14])
                .style(rail_row(current));
            items = items.push(row_box);
        }

        if installing {
            let marker = "\u{25B8}"; // ▸
            let label = text(format!("{marker} Install")).size(14);
            let row_box = container(label)
                .width(Fill)
                .padding([10, 14])
                .style(rail_row(true));
            items = items.push(row_box);
        }

        container(items.padding(12).width(210))
            .width(210)
            .height(Fill)
            .style(styles::sidebar)
            .into()
    }

    fn view_content(&self) -> Element<'_, Message> {
        if self.welcome && self.install.is_none() && self.failure.is_none() {
            return self.view_welcome();
        }
        if let Active::Edit { title, content } = &self.active {
            return self.view_editor(title, content);
        }
        if let Some(failure) = &self.failure {
            return self.view_failure(failure);
        }
        if self.finished && self.install.is_some() {
            return self.view_complete();
        }
        if let Some(inst) = &self.install {
            return self.view_install(inst);
        }
        if self.current_step == StepId::Summary && self.has_summary_prompt() {
            return self.view_summary();
        }

        let (title, body): (String, Element<'_, Message>) = match &self.active {
            Active::None if self.finished => {
                let msg = if self.dry_run {
                    "Interview complete (dry run — no disk was touched)."
                } else {
                    "Interview complete."
                };
                ("Done".to_string(), text(msg).size(15).into())
            }
            Active::None => {
                const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
                let spinner = FRAMES[self.spinner_frame % FRAMES.len()];
                let status = self.log.last().map(String::as_str).unwrap_or("");
                let body = column![
                    text(format!("{spinner}  Working\u{2026}"))
                        .size(18)
                        .color(styles::PRIMARY),
                    text(status.to_string()).size(13).color(styles::MUTED),
                ]
                .spacing(8)
                .into();
                (String::new(), body)
            }
            Active::Select {
                prompt,
                options,
                selected,
                filter,
            } if self.current_step == StepId::Keyboard => {
                let body = column![
                    self.view_select(options, *selected, filter),
                    Space::new().height(12),
                    text("Type here to check your layout:")
                        .size(13)
                        .color(styles::MUTED),
                    text_input("", &self.kbd_test)
                        .on_input(Message::KbdTestChanged)
                        .padding(8),
                ]
                .spacing(6);
                (prompt.clone(), body.into())
            }
            Active::Select {
                prompt,
                options,
                selected,
                filter,
            } => (prompt.clone(), self.view_select(options, *selected, filter)),
            Active::Input { prompt, value } => (prompt.clone(), self.view_text(value, false)),
            Active::Password {
                prompt,
                value,
                revealed,
            } => {
                let field = text_input("", value)
                    .id(iced::widget::Id::new(INPUT_ID))
                    .secure(!revealed)
                    .on_input(Message::InputChanged)
                    .on_submit(Message::Next)
                    .padding(10)
                    .size(15);
                let label = if *revealed { "Hide" } else { "Show" };
                let toggle = button(text(format!("\u{1f441} {label}")).size(13))
                    .padding([6, 10])
                    .style(styles::btn_ghost)
                    .on_press(Message::ToggleReveal);
                (prompt.clone(), column![field, toggle].spacing(8).into())
            }
            Active::Confirm { prompt, default } => {
                let yes = button(text("Yes"))
                    .padding([8, 20])
                    .style(styles::btn_primary)
                    .on_press(Message::Confirm(true));
                let no = button(text("No"))
                    .padding([8, 20])
                    .style(styles::btn_secondary)
                    .on_press(Message::Confirm(false));
                let hint = text(format!("Default: {}", if *default { "Yes" } else { "No" }))
                    .size(12)
                    .color(styles::MUTED);
                (
                    prompt.clone(),
                    column![row![yes, no].spacing(8), hint].spacing(10).into(),
                )
            }
            // Handled earlier by `view_editor`; unreachable here.
            Active::Edit { title, .. } => (title.clone(), Space::new().into()),
        };

        let header = text(title).size(24).font(styles::BOLD);
        let card = container(body).padding(20).width(Fill).style(styles::card);

        let next_enabled = !matches!(self.active, Active::None);
        let mut next_btn = button(text("Next"))
            .padding([10, 20])
            .style(styles::btn_primary);
        if next_enabled {
            next_btn = next_btn.on_press(Message::Next);
        }
        let back_btn = button(text("Back"))
            .padding([10, 20])
            .style(styles::btn_secondary)
            .on_press(Message::Back);
        let controls =
            row![back_btn, Space::new().width(Fill), next_btn].align_y(Alignment::Center);

        let mut col = column![header, Space::new().height(8), card].height(Fill);
        if let Some(fb) = &self.feedback {
            col = col.push(Space::new().height(8));
            col = col.push(self.view_feedback(fb));
        }
        col = col.push(Space::new().height(16));
        col = col.push(self.view_log());
        col = col.push(Space::new().height(Fill));
        col = col.push(controls);

        container(col).padding(24).width(Fill).height(Fill).into()
    }

    /// Branded welcome shown before the first prompt arrives.
    fn view_welcome(&self) -> Element<'_, Message> {
        let icon = svg(svg::Handle::from_memory(ICON_SVG)).width(64).height(64);
        let title = text("Guix System Installer")
            .size(28)
            .font(styles::BOLD)
            .color(styles::TEXT);
        let tagline = text(self.tagline.clone()).size(15).color(styles::MUTED);
        let hint = text("Press Enter to begin \u{2022} Esc to go back \u{2022} Tab to move focus")
            .size(12)
            .color(styles::MUTED);

        let start = button(text("Get started").size(15))
            .padding([10, 24])
            .style(styles::btn_primary)
            .on_press(Message::Welcome);

        let card = container(
            column![
                icon,
                Space::new().height(16),
                title,
                tagline,
                Space::new().height(20),
                start,
                Space::new().height(12),
                hint,
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        )
        .padding(40)
        .style(styles::card);

        let col = column![Space::new().height(Fill), card, Space::new().height(Fill)]
            .align_x(Alignment::Center);
        container(col)
            .padding(24)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .into()
    }

    /// In-app multi-line editor for the system.scm override.
    fn view_editor<'a>(
        &'a self,
        title: &str,
        content: &'a text_editor::Content,
    ) -> Element<'a, Message> {
        let header = text(title.to_string()).size(24).font(styles::BOLD);

        let editor = text_editor(content)
            .font(Font::MONOSPACE)
            .size(13)
            .padding(12)
            .height(Fill)
            .on_action(Message::EditAction);

        let card = container(editor)
            .padding(20)
            .width(Fill)
            .height(Fill)
            .style(styles::card);

        let cancel = button(text("Cancel"))
            .padding([10, 20])
            .style(styles::btn_secondary)
            .on_press(Message::Back);
        let save = button(text("Save"))
            .padding([10, 20])
            .style(styles::btn_primary)
            .on_press(Message::EditSave);
        let controls = row![cancel, Space::new().width(Fill), save].align_y(Alignment::Center);

        let col = column![
            header,
            Space::new().height(8),
            card,
            Space::new().height(16),
            controls
        ]
        .height(Fill);
        container(col).padding(24).width(Fill).height(Fill).into()
    }

    fn view_feedback(&self, fb: &Feedback) -> Element<'_, Message> {
        let color = if fb.danger {
            styles::DANGER
        } else {
            styles::WARNING
        };
        container(text(fb.text.clone()).size(13).color(color))
            .padding([8, 12])
            .width(Fill)
            .style(styles::card_flat)
            .into()
    }

    fn view_log(&self) -> Element<'_, Message> {
        scrollable(
            column(
                self.log
                    .iter()
                    .rev()
                    .take(6)
                    .rev()
                    .map(|l| text(l.clone()).size(12).color(styles::MUTED).into())
                    .collect::<Vec<_>>(),
            )
            .spacing(2),
        )
        .height(Length::Fixed(110.0))
        .into()
    }

    /// The Install screen: phase checklist + weighted progress bar + the live
    /// detail line for the current guix op.
    fn view_install(&self, inst: &Install) -> Element<'_, Message> {
        let header = text("Installing").size(24).font(styles::BOLD);

        let mut checklist = column![].spacing(6);
        for (i, label) in PHASE_LABELS.iter().enumerate() {
            let num = (i + 1) as u8;
            let (marker, color) = if num <= inst.done_through {
                ("\u{2713}", styles::SUCCESS) // ✓
            } else if num == inst.current {
                ("\u{25B8}", styles::PRIMARY) // ▸
            } else {
                ("\u{2591}", styles::MUTED) // ░
            };
            let text_color = if num <= inst.current {
                styles::TEXT
            } else {
                styles::MUTED
            };
            checklist = checklist.push(
                row![
                    text(marker).size(15).color(color),
                    text(*label).size(15).color(text_color),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }

        let bar = progress_bar(0.0..=1.0, inst.pct).girth(14);
        let pct_label = text(format!("{:.0}%", inst.pct * 100.0))
            .size(13)
            .color(styles::MUTED);

        let mut detail_col = column![checklist, Space::new().height(16), bar, pct_label].spacing(6);

        if !inst.status.trim().is_empty() {
            detail_col = detail_col.push(Space::new().height(6));
            detail_col = detail_col.push(text(inst.status.clone()).size(13).color(styles::TEXT));
        }
        if !inst.detail.trim().is_empty() {
            detail_col = detail_col.push(text(inst.detail.clone()).size(12).color(styles::MUTED));
        }

        let card = container(detail_col)
            .padding(20)
            .width(Fill)
            .style(styles::card);

        let col = column![
            header,
            Space::new().height(8),
            card,
            Space::new().height(16)
        ]
        .push(self.view_log())
        .height(Fill);

        container(col).padding(24).width(Fill).height(Fill).into()
    }

    /// Success screen shown after a real install completes.
    fn view_complete(&self) -> Element<'_, Message> {
        let icon = svg(svg::Handle::from_memory(ICON_SVG)).width(64).height(64);
        let header = text("Installation complete")
            .size(28)
            .font(styles::BOLD)
            .color(styles::SUCCESS);
        let body = text("Guix System has been installed. You can reboot now.")
            .size(16)
            .color(styles::TEXT);
        let hint = text("Remove the install medium first.")
            .size(13)
            .color(styles::MUTED);

        let card = container(
            column![icon, Space::new().height(12), header, body, hint]
                .spacing(10)
                .align_x(Alignment::Center),
        )
        .padding(40)
        .max_width(480)
        .style(styles::card);

        let reboot_btn = button(text("Reboot now").size(15))
            .padding([10, 24])
            .style(styles::btn_primary)
            .on_press(Message::Reboot);
        let shutdown_btn = button(text("Shut down").size(15))
            .padding([10, 24])
            .style(styles::btn_secondary)
            .on_press(Message::Shutdown);
        let actions = row![reboot_btn, shutdown_btn].spacing(12);

        let inner = column![card, actions]
            .align_x(Alignment::Center)
            .spacing(16);

        let col = column![Space::new().height(Fill), inner, Space::new().height(Fill)]
            .align_x(Alignment::Center);
        container(col)
            .padding(24)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .into()
    }

    /// Failure screen: which phase failed + captured output, Abort/Retry.
    fn view_failure(&self, failure: &Failure) -> Element<'_, Message> {
        let header = text("Installation failed")
            .size(26)
            .font(styles::BOLD)
            .color(styles::DANGER);

        let summary = text(failure.summary.clone()).size(15).color(styles::TEXT);

        let detail = scrollable(
            text(failure.detail.clone())
                .size(12)
                .color(styles::MUTED)
                .font(styles::BOLD),
        )
        .height(Fill);

        let card = container(
            column![summary, Space::new().height(12), detail]
                .spacing(4)
                .height(Fill),
        )
        .padding(20)
        .width(Fill)
        .height(Fill)
        .style(styles::card);

        let abort = button(text("Abort"))
            .padding([10, 24])
            .style(styles::btn_danger)
            .on_press(Message::Abort);
        let retry = button(text("Retry"))
            .padding([10, 24])
            .style(styles::btn_primary)
            .on_press(Message::Retry);
        let controls = row![abort, Space::new().width(Fill), retry].align_y(Alignment::Center);

        let col = column![
            header,
            Space::new().height(8),
            card,
            Space::new().height(16),
            controls
        ]
        .height(Fill);

        container(col).padding(24).width(Fill).height(Fill).into()
    }

    fn has_summary_prompt(&self) -> bool {
        matches!(&self.active, Active::Select { options, .. } if is_summary_menu(options))
    }

    fn view_summary(&self) -> Element<'_, Message> {
        let header = text("Summary").size(24).font(styles::BOLD);

        let body = column(
            self.summary
                .iter()
                .filter(|l| !l.text.trim().is_empty())
                .map(|l| {
                    let color = if l.danger {
                        styles::DANGER
                    } else {
                        styles::TEXT
                    };
                    text(l.text.clone()).size(13).color(color).into()
                })
                .collect::<Vec<_>>(),
        )
        .spacing(4);

        let card = container(scrollable(body).height(Fill))
            .padding(20)
            .width(Fill)
            .height(Fill)
            .style(styles::card);

        // Render the Summary `select` options as one-click action buttons.
        let controls: Element<'_, Message> = match &self.active {
            Active::Select { options, .. } => {
                let back = button(text("Back"))
                    .padding([10, 20])
                    .style(styles::btn_secondary)
                    .on_press(Message::Back);
                let mut r = row![back, Space::new().width(Fill)].align_y(Alignment::Center);
                for (i, opt) in options.iter().enumerate() {
                    let style: fn(&Theme, button::Status) -> button::Style =
                        if opt.contains("Proceed") {
                            styles::btn_primary
                        } else if opt.contains("Cancel") {
                            styles::btn_danger
                        } else {
                            styles::btn_secondary
                        };
                    r = r.push(
                        button(text(opt.clone()))
                            .padding([10, 20])
                            .style(style)
                            .on_press(Message::SubmitIndex(i)),
                    );
                    r = r.push(Space::new().width(8));
                }
                r.into()
            }
            _ => Space::new().into(),
        };

        let mut col = column![header, Space::new().height(8), card].height(Fill);
        if let Some(fb) = &self.feedback
            && (!fb.danger || !fb.text.contains("ALL DATA WILL BE LOST"))
        {
            col = col.push(Space::new().height(8));
            col = col.push(self.view_feedback(fb));
        }
        col = col.push(Space::new().height(16));
        col = col.push(controls);

        container(col).padding(24).width(Fill).height(Fill).into()
    }

    fn view_select(
        &self,
        options: &[String],
        selected: usize,
        filter: &str,
    ) -> Element<'_, Message> {
        let mut col = column![].spacing(4);
        for i in filtered_indices(options, filter) {
            col = col.push(
                button(text(options[i].clone()).size(15))
                    .width(Fill)
                    .padding([8, 12])
                    .style(styles::result_row_btn(i == selected))
                    .on_press(Message::Highlight(i)),
            );
        }
        let list = scrollable(col)
            .id(iced::widget::Id::new(SELECT_ID))
            .height(Length::Fixed(320.0));

        if options.len() > SELECT_FILTER_THRESHOLD {
            let field = text_input("Type to filter\u{2026}", filter)
                .id(iced::widget::Id::new(INPUT_ID))
                .on_input(Message::SelectFilterChanged)
                .on_submit(Message::Next)
                .padding(10)
                .size(15);
            column![field, list].spacing(8).into()
        } else {
            list.into()
        }
    }

    fn view_text(&self, value: &str, secure: bool) -> Element<'_, Message> {
        text_input("", value)
            .id(iced::widget::Id::new(INPUT_ID))
            .secure(secure)
            .on_input(Message::InputChanged)
            .on_submit(Message::Next)
            .padding(10)
            .size(15)
            .into()
    }
}

/// Map a raw keyboard event into a navigation message. Tab/Shift-Tab move focus,
/// Escape goes Back, and arrows/Enter are interpreted per active prompt; other
/// keys are dropped so text fields keep owning their keystrokes via `on_input`.
fn key_message(event: KeyEvent) -> Option<Message> {
    match event {
        KeyEvent::KeyPressed {
            key: Key::Named(Named::Tab),
            modifiers,
            ..
        } => Some(if modifiers.shift() {
            Message::FocusPrev
        } else {
            Message::FocusNext
        }),
        KeyEvent::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        } => Some(Message::Back),
        KeyEvent::KeyPressed {
            key: Key::Named(named),
            ..
        } if matches!(named, Named::ArrowUp | Named::ArrowDown | Named::Enter) => {
            Some(Message::Key(named))
        }
        _ => None,
    }
}

/// Best-effort first meaningful line of `/etc/issue` (Panther's `%issue`),
/// with escape codes stripped; falls back to a static tagline.
fn read_tagline() -> String {
    const FALLBACK: &str = "Install Guix System onto this machine.";
    std::fs::read_to_string("/etc/issue")
        .ok()
        .and_then(|s| {
            s.lines()
                .map(|l| {
                    l.split_whitespace()
                        .filter(|w| !w.starts_with('\\') && !w.starts_with('%'))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .map(|l| l.trim().to_string())
                .find(|l| l.len() > 3)
        })
        .unwrap_or_else(|| FALLBACK.to_string())
}

fn is_summary_menu(options: &[String]) -> bool {
    options
        .iter()
        .any(|o| o.contains("Proceed with installation"))
}

/// Rail row style: current step gets the amber-tinted nav highlight.
fn rail_row(active: bool) -> impl Fn(&Theme) -> iced::widget::container::Style {
    move |_theme| {
        if active {
            iced::widget::container::Style {
                background: Some(iced::Background::Color(styles::ACTIVE)),
                text_color: Some(styles::PRIMARY),
                border: iced::Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        } else {
            iced::widget::container::Style {
                text_color: Some(styles::MUTED),
                ..Default::default()
            }
        }
    }
}

fn focus_input() -> Task<Message> {
    focus(iced::widget::Id::new(INPUT_ID))
}

/// Original-vec indices whose option contains `filter` (case-insensitive).
/// An empty filter matches everything.
fn filtered_indices(options: &[String], filter: &str) -> Vec<usize> {
    if filter.is_empty() {
        return (0..options.len()).collect();
    }
    let f = filter.to_lowercase();
    options
        .iter()
        .enumerate()
        .filter(|(_, o)| o.to_lowercase().contains(&f))
        .map(|(i, _)| i)
        .collect()
}

/// Reboot via whichever command the install env actually has on PATH.
fn reboot() {
    use std::process::Command;
    if Command::new("reboot").spawn().is_ok() {
        return;
    }
    if Command::new("/run/current-system/profile/sbin/reboot")
        .spawn()
        .is_ok()
    {
        return;
    }
    let _ = Command::new("shutdown").args(["-r", "now"]).spawn();
}

/// Power off via whichever command the install env actually has on PATH.
fn shutdown() {
    use std::process::Command;
    if Command::new("poweroff").spawn().is_ok() {
        return;
    }
    if Command::new("/run/current-system/profile/sbin/poweroff")
        .spawn()
        .is_ok()
    {
        return;
    }
    let _ = Command::new("shutdown").args(["-h", "now"]).spawn();
}

/// Stable subscription identity. The slot is excluded from the hash so the
/// subscription is created exactly once and never restarts.
struct SubData {
    slot: EventSlot,
}

impl std::hash::Hash for SubData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "guix-install-ui-events".hash(state);
    }
}

fn drain(data: &SubData) -> impl stream::Stream<Item = UiEvent> + use<> {
    use iced::futures::StreamExt;
    // Take the receiver once; thereafter it lives in the unfold state.
    let initial = data.slot.lock().ok().and_then(|mut g| g.take());
    stream::unfold(initial, |receiver| async move {
        let mut rx = receiver?;
        match rx.next().await {
            Some(ev) => Some((ev, Some(rx))),
            None => Some((UiEvent::Finished, None)),
        }
    })
}
