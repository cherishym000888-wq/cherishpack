//! iced GUI.
//!
//! 화면: Welcome → HwCheck → Install(진행) → Done / Error
//! 진행 이벤트는 unbounded mpsc 채널로 orchestrator → subscription → update 로 흐른다.

use anyhow::Result;
use iced::{
    executor,
    widget::{button, column, container, progress_bar, row, scrollable, text, Space},
    Alignment, Application, Command, Element, Length, Settings, Subscription, Theme,
};
use std::sync::{Arc, Mutex};

use crate::{
    channel::Channel,
    hwdetect::{self, HwSnapshot},
    orchestrator::{self, Event as OrcEvent, RunOptions},
    paths::AppDirs,
    preset::{self, Preset},
};

pub fn run(dirs: AppDirs) -> Result<()> {
    App::run(Settings {
        id: None,
        window: iced::window::Settings {
            size: iced::Size::new(720.0, 520.0),
            resizable: false,
            ..iced::window::Settings::default()
        },
        flags: dirs,
        fonts: Vec::new(),
        default_font: iced::Font::default(),
        default_text_size: iced::Pixels(14.0),
        antialiasing: true,
    })
    .map_err(|e| anyhow::anyhow!("iced 실행 실패: {e}"))
}

struct App {
    dirs: AppDirs,
    screen: Screen,
    hw: HwSnapshot,
    chosen_preset: Preset,
    progress_done: u64,
    progress_total: Option<u64>,
    progress_label: String,
    substep_label: String,
    substep_idx: usize,
    substep_total: usize,
    log_lines: Vec<String>,
    /// orchestrator → GUI 이벤트 수신기 (subscription이 가져감)
    rx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<OrcEvent>>>>,
}

enum Screen {
    Welcome,
    Installing,
    Done { launched: bool },
    Error(String),
}

#[derive(Debug, Clone)]
pub enum Msg {
    StartInstall,
    PickPreset(Preset),
    Launch,
    Close,
    Orc(OrcEvent),
}

impl Application for App {
    type Executor = executor::Default;
    type Message = Msg;
    type Theme = Theme;
    type Flags = AppDirs;

    fn new(flags: AppDirs) -> (Self, Command<Msg>) {
        let hw = hwdetect::detect();
        let chosen_preset = preset::recommend(&hw);
        (
            Self {
                dirs: flags,
                screen: Screen::Welcome,
                hw,
                chosen_preset,
                progress_done: 0,
                progress_total: None,
                progress_label: String::new(),
                substep_label: String::new(),
                substep_idx: 0,
                substep_total: 0,
                log_lines: Vec::new(),
                rx: Arc::new(Mutex::new(None)),
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        "CherishPack 설치 프로그램".into()
    }

    fn update(&mut self, msg: Msg) -> Command<Msg> {
        match msg {
            Msg::PickPreset(p) => {
                self.chosen_preset = p;
                Command::none()
            }
            Msg::StartInstall => {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<OrcEvent>();
                *self.rx.lock().unwrap() = Some(rx);
                let dirs = self.dirs.clone();
                let opts = RunOptions {
                    channel: Channel::Stable,
                    preset: Some(self.chosen_preset.key().to_string()),
                    auto_launch: false,
                };
                tokio::spawn(async move { orchestrator::run(dirs, opts, tx).await });
                self.screen = Screen::Installing;
                Command::none()
            }
            Msg::Launch => {
                // Prism 실행은 orchestrator의 auto_launch 대신 별도 버튼으로 처리 가능 — 여기선 단순 종료
                iced::window::close(iced::window::Id::MAIN)
            }
            Msg::Close => iced::window::close(iced::window::Id::MAIN),
            Msg::Orc(ev) => {
                self.apply_event(ev);
                Command::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Msg> {
        let rx = Arc::clone(&self.rx);
        iced::subscription::unfold("orc-events", rx, |rx| async move {
            let ev_opt = {
                let mut guard = rx.lock().unwrap();
                match guard.as_mut() {
                    Some(r) => r.try_recv().ok(),
                    None => None,
                }
            };
            match ev_opt {
                Some(ev) => (Msg::Orc(ev), rx),
                None => {
                    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                    // 더미 이벤트 — 이건 메시지를 발생시키므로 안 좋음.
                    // 대신 계속 폴링만 하도록 재귀적으로 다시 sleep.
                    // iced::subscription::unfold는 반드시 (Msg, State)를 리턴해야 하므로
                    // 빈 틱 메시지를 따로 두거나, 아래처럼 아무 일도 안 하는 더미를 리턴.
                    //
                    // 더 나은 방법: subscription::channel 사용. 아래 버전은 stop-gap.
                    (Msg::Orc(OrcEvent::Info(String::new())), rx)
                }
            }
        })
    }

    fn view(&self) -> Element<'_, Msg> {
        match &self.screen {
            Screen::Welcome => self.view_welcome(),
            Screen::Installing => self.view_installing(),
            Screen::Done { launched } => self.view_done(*launched),
            Screen::Error(e) => self.view_error(e),
        }
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

impl App {
    fn apply_event(&mut self, ev: OrcEvent) {
        match ev {
            OrcEvent::Status(s) => {
                self.progress_label = s.clone();
                self.push_log(format!("[*] {s}"));
            }
            OrcEvent::Progress { done, total, label } => {
                self.progress_done = done;
                self.progress_total = total;
                self.progress_label = label;
            }
            OrcEvent::SubStep { idx, total, label } => {
                self.substep_idx = idx;
                self.substep_total = total;
                self.substep_label = label;
            }
            OrcEvent::Info(s) => {
                if !s.is_empty() {
                    self.push_log(format!("    {s}"));
                }
            }
            OrcEvent::Warn(s) => {
                self.push_log(format!("[!] {s}"));
            }
            OrcEvent::Done { launched } => {
                self.push_log("[v] 완료".into());
                self.screen = Screen::Done { launched };
            }
            OrcEvent::Error(e) => {
                self.push_log(format!("[x] {e}"));
                self.screen = Screen::Error(e);
            }
        }
    }

    fn push_log(&mut self, s: String) {
        self.log_lines.push(s);
        if self.log_lines.len() > 200 {
            let drain = self.log_lines.len() - 200;
            self.log_lines.drain(0..drain);
        }
    }

    fn view_welcome(&self) -> Element<'_, Msg> {
        let ram_gb = self.hw.total_ram_mb as f32 / 1024.0;
        let gpu = self
            .hw
            .gpu_name
            .clone()
            .unwrap_or_else(|| "(감지 실패)".to_string());
        let vram = self
            .hw
            .gpu_vram_mb
            .map(|m| format!("{} MB", m))
            .unwrap_or_else(|| "(미상)".into());

        let preset_btn = |label: &str, p: Preset| {
            let selected = self.chosen_preset == p;
            let t = if selected {
                format!("● {label}")
            } else {
                format!("○ {label}")
            };
            button(text(t)).on_press(Msg::PickPreset(p))
        };

        let content = column![
            text("CherishPack").size(32),
            text("마인크래프트 NeoForge 1.21.1 모드팩").size(13),
            Space::with_height(10),
            text(format!("RAM: {:.1} GB  |  GPU: {}  |  VRAM: {}", ram_gb, gpu, vram))
                .size(12),
            text(format!(
                "추천 프리셋: {}",
                preset::recommend(&self.hw).key().to_uppercase()
            ))
            .size(12),
            Space::with_height(10),
            row![
                preset_btn("LOW", Preset::Low),
                preset_btn("MEDIUM", Preset::Medium),
                preset_btn("HIGH", Preset::High),
            ]
            .spacing(10),
            Space::with_height(14),
            row![
                button(text("설치 / 업데이트")).on_press(Msg::StartInstall),
                button(text("닫기")).on_press(Msg::Close),
            ]
            .spacing(12),
        ]
        .spacing(8)
        .align_items(Alignment::Center);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x()
            .center_y()
            .padding(24)
            .into()
    }

    fn view_installing(&self) -> Element<'_, Msg> {
        let pct = match self.progress_total {
            Some(t) if t > 0 => (self.progress_done as f32 / t as f32).clamp(0.0, 1.0),
            _ => 0.0,
        };

        let sub = if self.substep_total > 0 {
            format!(
                "[{}/{}] {}",
                self.substep_idx, self.substep_total, self.substep_label
            )
        } else {
            String::new()
        };

        let log_text = self.log_lines.join("\n");

        let content = column![
            text("설치 / 업데이트 중").size(22),
            text(&self.progress_label).size(13),
            progress_bar(0.0..=1.0, pct),
            text(sub).size(12),
            Space::with_height(6),
            scrollable(
                container(text(log_text).size(11))
                    .padding(8)
                    .width(Length::Fill)
            )
            .height(Length::Fixed(260.0)),
        ]
        .spacing(10);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(20)
            .into()
    }

    fn view_done(&self, _launched: bool) -> Element<'_, Msg> {
        let content = column![
            text("✓ 설치 완료").size(28),
            text("이제 Prism Launcher 에서 CherishPack 인스턴스를 실행하세요.").size(13),
            Space::with_height(16),
            button(text("종료")).on_press(Msg::Close),
        ]
        .spacing(10)
        .align_items(Alignment::Center);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x()
            .center_y()
            .padding(24)
            .into()
    }

    fn view_error(&self, err: &str) -> Element<'_, Msg> {
        let content = column![
            text("설치 실패").size(26),
            Space::with_height(8),
            scrollable(
                container(text(err).size(12))
                    .padding(10)
                    .width(Length::Fill)
            )
            .height(Length::Fixed(300.0)),
            Space::with_height(10),
            button(text("닫기")).on_press(Msg::Close),
        ]
        .spacing(8);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(20)
            .into()
    }
}
