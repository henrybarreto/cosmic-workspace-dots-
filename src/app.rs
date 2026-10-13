//! Applet UI: one dot per workspace, shaped by the theme.
use cosmic::cosmic_config::{Config, ConfigGet, ConfigSet};
use cosmic::iced::futures::{SinkExt, StreamExt, channel::mpsc};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::stream;
use cosmic::iced::{Alignment, Background, Border, Length, Limits, Subscription, window};
use cosmic::{Element, Task, app, theme, widget};

use crate::wayland::{Snapshot, Watcher};

/// Persisted user settings (cosmic-config, key `dot_percent`).
struct Prefs {
    config: Option<Config>,
    /// Dot size as a percentage of the panel icon size.
    percent: u8,
}

impl Prefs {
    const KEY: &'static str = "dot_percent";
    const DEFAULT: u8 = 40;
    const RANGE: std::ops::RangeInclusive<u8> = 10..=80;

    fn load(app_id: &str) -> Self {
        let config = Config::new(app_id, 1).ok();
        let percent = config
            .as_ref()
            .and_then(|c| c.get::<u8>(Self::KEY).ok())
            .map_or(Self::DEFAULT, |p| {
                p.clamp(*Self::RANGE.start(), *Self::RANGE.end())
            });
        Self { config, percent }
    }

    fn set(&mut self, percent: u8) {
        self.percent = percent;
        if let Some(config) = &self.config {
            let _ = config.set(Self::KEY, percent);
        }
    }
}

pub struct AppModel {
    core: app::Core,
    prefs: Prefs,
    popup: Option<window::Id>,
    groups: Snapshot,
    watcher: Option<Watcher>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Ready(Watcher),
    Workspaces(Snapshot),
    Activate(String),
    TogglePopup,
    PopupClosed(window::Id),
    SetSize(u8),
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "com.github.henry.cosmic-workspace-dots";

    fn init(core: app::Core, (): ()) -> (Self, app::Task<Message>) {
        (
            Self {
                core,
                prefs: Prefs::load(Self::APP_ID),
                popup: None,
                groups: Vec::new(),
                watcher: None,
            },
            Task::none(),
        )
    }

    fn core(&self) -> &app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut app::Core {
        &mut self.core
    }

    fn view(&self) -> Element<'_, Message> {
        let cosmic = theme::active().cosmic().clone();
        let size = f32::from(self.core.applet.suggested_size(false).0)
            * f32::from(self.prefs.percent)
            / 100.0;
        // Theme presets (radius_xl): round = 160, slightly round = 8, square = 2.
        let xl = cosmic.radius_xl()[0];
        let radius = if xl >= 100.0 {
            size / 2.0
        } else {
            size * xl / 32.0
        };
        let accent = cosmic.accent_color();
        let idle = cosmic.on_bg_color();

        let groups = self.groups.iter().map(|group| -> Element<'_, Message> {
            let dots = group.iter().map(|d| -> Element<'_, Message> {
                let color = if d.active { accent } else { idle };
                let alpha = if d.active { 1.0 } else { 0.4 };
                let dot = widget::container(widget::Space::new().width(size).height(size)).class(
                    theme::Container::custom(move |_| cosmic::iced::widget::container::Style {
                        background: Some(Background::Color(cosmic::iced::Color::from_rgba(
                            color.red,
                            color.green,
                            color.blue,
                            alpha,
                        ))),
                        border: Border {
                            radius: radius.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                );
                widget::button::custom(dot)
                    .class(theme::Button::AppletIcon)
                    .padding(2)
                    .on_press(Message::Activate(d.id.clone()))
                    .into()
            });
            widget::row::with_children(dots)
                .spacing(4)
                .align_y(Alignment::Center)
                .into()
        });

        let row = widget::row::with_children(groups)
            .spacing(16) // wider gap between workspace groups (outputs)
            .padding([0, 10])
            .align_y(Alignment::Center);
        let h = f32::from(
            u16::try_from(self.core.applet.suggested_window_size().1.get()).unwrap_or(u16::MAX),
        );
        let centered = widget::container(row)
            .height(Length::Fixed(h))
            .center_y(Length::Fixed(h));
        // Right-click opens the size settings; left-click on a dot switches workspace.
        let area = widget::mouse_area(centered).on_right_press(Message::TogglePopup);
        self.core.applet.autosize_window(area).into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let content = widget::column![
            widget::text(format!("Dot size: {}%", self.prefs.percent)),
            widget::slider(Prefs::RANGE, self.prefs.percent, Message::SetSize),
        ]
        .spacing(8)
        .padding(16);
        self.core.applet.popup_container(content).into()
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::run(|| {
            stream::channel(8, async |mut out: mpsc::Sender<Message>| {
                let (tx, mut rx) = mpsc::unbounded();
                let _ = out.send(Message::Ready(Watcher::spawn(tx))).await;
                while let Some(snapshot) = rx.next().await {
                    if out.send(Message::Workspaces(snapshot)).await.is_err() {
                        break;
                    }
                }
            })
        })
    }

    fn update(&mut self, message: Message) -> Task<cosmic::Action<Message>> {
        match message {
            Message::Ready(watcher) => self.watcher = Some(watcher),
            Message::Workspaces(groups) => self.groups = groups,
            Message::SetSize(percent) => self.prefs.set(percent),
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
            }
            Message::TogglePopup => {
                if let Some(id) = self.popup.take() {
                    return destroy_popup(id);
                }
                let Some(parent) = self.core.main_window_id() else {
                    return Task::none();
                };
                let id = window::Id::unique();
                self.popup = Some(id);
                let mut settings = self
                    .core
                    .applet
                    .get_popup_settings(parent, id, None, None, None);
                settings.positioner.size_limits = Limits::NONE
                    .min_width(220.0)
                    .max_width(320.0)
                    .min_height(80.0)
                    .max_height(160.0);
                return get_popup(settings);
            }
            Message::Activate(id) => {
                if let Some(watcher) = &self.watcher {
                    watcher.activate(id);
                }
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
