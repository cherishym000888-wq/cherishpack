//! iced GUI — CherishPack 설치 프로그램
//!
//! 화면: Welcome → Installing → Done / Error
//! 디자인: 체리쉬 웹 다크 네이비 테마
//!   배경 #0e1020 / 패널 #10172a / 보더 #243047
//!   포인트 #f43f5e (체리 핑크) / 버튼 #1a2946 + #5e7ab1

use anyhow::Result;
use iced::{
    executor,
    widget::{
        button, column, container, progress_bar, row, scrollable, text, text_input, Space,
    },
    Alignment, Application, Background, Border, Color, Command, Element, Length, Settings,
    Shadow, Subscription, Theme, Vector,
};
use std::sync::{Arc, Mutex};

use crate::{
    channel::{self, Channel},
    config::VersionIndex,
    hwdetect::{self, HwSnapshot},
    net,
    orchestrator::{self, Event as OrcEvent, RunOptions},
    paths::AppDirs,
    preset::{self, Preset},
    state,
};

// ─────────────────────── 색상 팔레트 ────────────────────────

const BG:          Color = Color { r: 0.055, g: 0.063, b: 0.125, a: 1.0 }; // #0e1020
const PANEL:       Color = Color { r: 0.063, g: 0.090, b: 0.165, a: 1.0 }; // #10172a
const PANEL2:      Color = Color { r: 0.102, g: 0.161, b: 0.275, a: 1.0 }; // #1a2946
const BORDER:      Color = Color { r: 0.141, g: 0.188, b: 0.278, a: 1.0 }; // #243047
const BORDER2:     Color = Color { r: 0.369, g: 0.478, b: 0.694, a: 1.0 }; // #5e7ab1
const TEXT:        Color = Color { r: 0.953, g: 0.965, b: 1.000, a: 1.0 }; // #f3f6ff
const TEXT_MUTED:  Color = Color { r: 0.624, g: 0.702, b: 0.851, a: 1.0 }; // #9fb3d9
const CHERRY:      Color = Color { r: 0.957, g: 0.247, b: 0.369, a: 1.0 }; // #f43f5e
const CHERRY_DARK: Color = Color { r: 0.882, g: 0.114, b: 0.282, a: 1.0 }; // #e11d48
const SUCCESS:     Color = Color { r: 0.392, g: 0.863, b: 0.545, a: 1.0 }; // #64dc8b
const WARN:        Color = Color { r: 1.000, g: 0.878, b: 0.541, a: 1.0 }; // #ffe08a
const LOG_BG:      Color = Color { r: 0.031, g: 0.047, b: 0.094, a: 1.0 }; // #080c18

// ─────────────────────── 스타일시트 구조체 ─────────────────────

/// 전체 배경 컨테이너
struct BgStyle;
impl container::StyleSheet for BgStyle {
    type Style = Theme;
    fn appearance(&self, _: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(BG)),
            text_color: Some(TEXT),
            ..Default::default()
        }
    }
}

/// 카드 패널 (보더 + 그림자)
struct CardStyle;
impl container::StyleSheet for CardStyle {
    type Style = Theme;
    fn appearance(&self, _: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(PANEL)),
            text_color: Some(TEXT),
            border: Border { color: BORDER, width: 1.0, radius: 12.0.into() },
            shadow: Shadow {
                color: Color { r: 0.882, g: 0.114, b: 0.282, a: 0.10 },
                offset: Vector::new(0.0, 4.0),
                blur_radius: 20.0,
            },
        }
    }
}

/// 로그/코드 배경
struct LogStyle;
impl container::StyleSheet for LogStyle {
    type Style = Theme;
    fn appearance(&self, _: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(LOG_BG)),
            text_color: Some(TEXT_MUTED),
            border: Border { color: BORDER, width: 1.0, radius: 8.0.into() },
            ..Default::default()
        }
    }
}

/// 경고 배지
struct WarnBadge;
impl container::StyleSheet for WarnBadge {
    type Style = Theme;
    fn appearance(&self, _: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(Color { r: 0.4, g: 0.3, b: 0.0, a: 0.20 })),
            text_color: Some(WARN),
            border: Border { color: WARN, width: 1.0, radius: 6.0.into() },
            ..Default::default()
        }
    }
}

/// 일반 버튼 (어두운 네이비)
struct BtnNormal;
impl button::StyleSheet for BtnNormal {
    type Style = Theme;
    fn active(&self, _: &Theme) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(Color { r: 0.075, g: 0.118, b: 0.208, a: 1.0 })),
            text_color: TEXT,
            border: Border { color: BORDER, width: 1.0, radius: 8.0.into() },
            ..Default::default()
        }
    }
    fn hovered(&self, style: &Theme) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(PANEL2)),
            border: Border { color: BORDER2, width: 1.0, radius: 8.0.into() },
            ..self.active(style)
        }
    }
}

/// 선택된 버튼 (체리 핑크 강조)
struct BtnSelected;
impl button::StyleSheet for BtnSelected {
    type Style = Theme;
    fn active(&self, _: &Theme) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(CHERRY)),
            text_color: Color::WHITE,
            border: Border { color: CHERRY_DARK, width: 1.0, radius: 8.0.into() },
            shadow: Shadow {
                color: Color { r: 0.957, g: 0.247, b: 0.369, a: 0.35 },
                offset: Vector::new(0.0, 3.0),
                blur_radius: 10.0,
            },
            ..Default::default()
        }
    }
    fn hovered(&self, style: &Theme) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(CHERRY_DARK)),
            ..self.active(style)
        }
    }
}

/// 주요 액션 버튼 (설치/실행)
struct BtnPrimary;
impl button::StyleSheet for BtnPrimary {
    type Style = Theme;
    fn active(&self, _: &Theme) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(CHERRY)),
            text_color: Color::WHITE,
            border: Border { color: CHERRY_DARK, width: 0.0, radius: 8.0.into() },
            shadow: Shadow {
                color: Color { r: 0.957, g: 0.247, b: 0.369, a: 0.40 },
                offset: Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..Default::default()
        }
    }
    fn hovered(&self, style: &Theme) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(CHERRY_DARK)),
            ..self.active(style)
        }
    }
}

/// 진행바 — 체리 핑크
struct ProgressStyle;
impl progress_bar::StyleSheet for ProgressStyle {
    type Style = Theme;
    fn appearance(&self, _: &Theme) -> progress_bar::Appearance {
        progress_bar::Appearance {
            background: Background::Color(BORDER),
            bar: Background::Color(CHERRY),
            border_radius: 4.0.into(),
        }
    }
}

// ─────────────────────── 업데이트 체크 ──────────────────────

async fn check_for_update() -> Option<String> {
    let idx: VersionIndex = net::fetch_json(channel::VERSION_INDEX_URL).await.ok()?;
    let latest = idx.installer_version.as_deref()?;
    let current = env!("CARGO_PKG_VERSION");
    if state::compare(current, latest) == std::cmp::Ordering::Less {
        Some(format!("업데이트 가능: v{current} → v{latest}"))
    } else {
        None
    }
}

// ─────────────────────── 앱 구조체 ──────────────────────────

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
        default_font: iced::Font::with_name("Malgun Gothic"),
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
    update_notice: Option<String>,
    progress_done: u64,
    progress_total: Option<u64>,
    progress_label: String,
    substep_label: String,
    substep_idx: usize,
    substep_total: usize,
    log_lines: Vec<String>,
    rx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<OrcEvent>>>>,
    #[cfg(feature = "offline")]
    nickname: String,
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
    UpdateCheck(Option<String>),
    #[cfg(feature = "offline")]
    SetNickname(String),
    #[cfg(feature = "offline")]
    StartOfflineInstall,
}

impl Application for App {
    type Executor = executor::Default;
    type Message  = Msg;
    type Theme    = Theme;
    type Flags    = AppDirs;

    fn new(flags: AppDirs) -> (Self, Command<Msg>) {
        let hw = hwdetect::detect();
        let chosen_preset = preset::recommend(&hw);
        (
            Self {
                dirs: flags,
                screen: Screen::Welcome,
                hw,
                chosen_preset,
                update_notice: None,
                progress_done: 0,
                progress_total: None,
                progress_label: String::new(),
                substep_label: String::new(),
                substep_idx: 0,
                substep_total: 0,
                log_lines: Vec::new(),
                rx: Arc::new(Mutex::new(None)),
                #[cfg(feature = "offline")]
                nickname: String::new(),
            },
            Command::perform(check_for_update(), Msg::UpdateCheck),
        )
    }

    fn title(&self) -> String { "CherishWorld".into() }

    fn update(&mut self, msg: Msg) -> Command<Msg> {
        match msg {
            Msg::PickPreset(p) => { self.chosen_preset = p; Command::none() }
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
                let exe = self.dirs.prism_root.join("prismlauncher.exe");
                if exe.exists() {
                    let _ = crate::prism::spawn_detached_ex(&exe, &self.dirs.prism_root, true);
                }
                iced::window::close(iced::window::Id::MAIN)
            }
            Msg::Close          => iced::window::close(iced::window::Id::MAIN),
            Msg::UpdateCheck(n) => { self.update_notice = n; Command::none() }
            Msg::Orc(ev)        => { self.apply_event(ev);   Command::none() }
            #[cfg(feature = "offline")]
            Msg::SetNickname(s) => { self.nickname = s; Command::none() }
            #[cfg(feature = "offline")]
            Msg::StartOfflineInstall => {
                use crate::launcher::orchestrator::{run_launcher, Event as LEv, RunOptions as LOpt};
                let (tx_orc, rx_orc) = tokio::sync::mpsc::unbounded_channel::<OrcEvent>();
                *self.rx.lock().unwrap() = Some(rx_orc);

                let (tx_l, mut rx_l) = tokio::sync::mpsc::unbounded_channel::<LEv>();
                let opts = LOpt {
                    channel: Channel::Stable,
                    auto_launch: true,
                    preset: Some(self.chosen_preset.key().to_string()),
                    offline_nickname: Some(self.nickname.clone()),
                };
                tokio::spawn(run_launcher(opts, tx_l));
                tokio::spawn(async move {
                    while let Some(ev) = rx_l.recv().await {
                        let mapped = match ev {
                            LEv::Status(s)   => OrcEvent::Status(s),
                            LEv::Info(s)     => OrcEvent::Info(s),
                            LEv::Warn(s)     => OrcEvent::Warn(s),
                            LEv::Progress { done, total, label } =>
                                OrcEvent::Progress { done, total, label },
                            LEv::AuthChallenge { user_code, verification_uri, .. } =>
                                OrcEvent::Info(format!("MSA 코드: {} → {}", user_code, verification_uri)),
                            LEv::Done { launched } => OrcEvent::Done { launched },
                            LEv::Error(e)    => OrcEvent::Error(e),
                        };
                        if tx_orc.send(mapped).is_err() { break; }
                    }
                });
                self.screen = Screen::Installing;
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
                    None    => None,
                }
            };
            match ev_opt {
                Some(ev) => (Msg::Orc(ev), rx),
                None => {
                    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                    (Msg::Orc(OrcEvent::Info(String::new())), rx)
                }
            }
        })
    }

    fn view(&self) -> Element<'_, Msg> {
        match &self.screen {
            Screen::Welcome            => self.view_welcome(),
            Screen::Installing         => self.view_installing(),
            Screen::Done { launched } => self.view_done(*launched),
            Screen::Error(e)          => self.view_error(e),
        }
    }

    fn theme(&self) -> Theme { Theme::Dark }
}

// ─────────────────────── 이벤트 처리 ────────────────────────

impl App {
    fn apply_event(&mut self, ev: OrcEvent) {
        match ev {
            OrcEvent::Status(s)                      => { self.progress_label = s.clone(); self.push_log(format!("[*] {s}")); }
            OrcEvent::Progress { done, total, label } => { self.progress_done = done; self.progress_total = total; self.progress_label = label; }
            OrcEvent::SubStep { idx, total, label }  => { self.substep_idx = idx; self.substep_total = total; self.substep_label = label; }
            OrcEvent::Info(s)                        => { if !s.is_empty() { self.push_log(format!("    {s}")); } }
            OrcEvent::Warn(s)                        => { self.push_log(format!("[!] {s}")); }
            OrcEvent::Done { launched }              => { self.push_log("[v] 완료".into()); self.screen = Screen::Done { launched }; }
            OrcEvent::Error(e)                       => { self.push_log(format!("[x] {e}")); self.screen = Screen::Error(e); }
        }
    }

    fn push_log(&mut self, s: String) {
        self.log_lines.push(s);
        if self.log_lines.len() > 200 {
            self.log_lines.drain(0..self.log_lines.len() - 200);
        }
    }

// ─────────────────────── 화면: 웰컴 ─────────────────────────

    fn view_welcome(&self) -> Element<'_, Msg> {
        let ram_gb = self.hw.total_ram_mb as f32 / 1024.0;
        let gpu    = self.hw.gpu_name.clone().unwrap_or_else(|| "감지 실패".into());
        let vram   = self.hw.gpu_vram_mb.map(|m| format!("{} MB", m)).unwrap_or_else(|| "미상".into());

        // ── 프리셋 버튼 helper ──
        let mk_preset_btn = |label: &str, sub: &str, p: Preset| -> Element<'_, Msg> {
            let selected = self.chosen_preset == p;
            let inner: Element<'_, Msg> = column![
                text(label).size(13).style(if selected { Color::WHITE } else { TEXT }),
                text(sub).size(10).style(if selected { Color { a: 0.80, ..Color::WHITE } } else { TEXT_MUTED }),
            ]
            .spacing(2)
            .align_items(Alignment::Center)
            .into();

            let b = button(inner)
                .width(Length::Fill)
                .padding([10, 8])
                .on_press(Msg::PickPreset(p));

            if selected {
                b.style(iced::theme::Button::Custom(Box::new(BtnSelected))).into()
            } else {
                b.style(iced::theme::Button::Custom(Box::new(BtnNormal))).into()
            }
        };

        // ── 업데이트 알림 ──
        let update_row: Element<'_, Msg> = if let Some(notice) = &self.update_notice {
            container(text(format!("⚠  {notice}")).size(11).style(WARN))
                .padding([5, 12])
                .style(iced::theme::Container::Custom(Box::new(WarnBadge)))
                .into()
        } else {
            Space::with_height(0).into()
        };

        // ── HW 정보 카드 ──
        let hw_card = container(
            column![
                row![
                    text("RAM").size(11).style(TEXT_MUTED),
                    text(format!("{:.1} GB", ram_gb)).size(11).style(TEXT),
                    Space::with_width(16),
                    text("GPU").size(11).style(TEXT_MUTED),
                    text(&gpu).size(11).style(TEXT),
                    Space::with_width(16),
                    text("VRAM").size(11).style(TEXT_MUTED),
                    text(&vram).size(11).style(TEXT),
                ].spacing(6).align_items(Alignment::Center),
                text(format!("추천 프리셋: {}", preset::recommend(&self.hw).key().to_uppercase()))
                    .size(11).style(TEXT_MUTED),
            ].spacing(4)
        )
        .padding([10, 16])
        .style(iced::theme::Container::Custom(Box::new(CardStyle)));

        // ── 전체 레이아웃 ──
        let content = column![
            column![
                text("CHERISH").size(36).style(CHERRY),
                text("Minecraft NeoForge 1.21.1 모드팩").size(12).style(TEXT_MUTED),
            ].spacing(2).align_items(Alignment::Center),

            update_row,
            hw_card,

            column![
                text("그래픽 품질").size(12).style(TEXT_MUTED),
                row![
                    mk_preset_btn("LOW",    "쉐이더 OFF",    Preset::Low),
                    mk_preset_btn("MEDIUM", "C. Reimagined", Preset::Medium),
                    mk_preset_btn("HIGH",   "C. Unbound",    Preset::High),
                    mk_preset_btn("HIGH++", "Reth. Voxels",  Preset::HighPlus),
                ].spacing(8).width(Length::Fill),
            ].spacing(8).align_items(Alignment::Center).width(Length::Fill),

            Space::with_height(4),

            row![
                button(text("  설치 / 업데이트  ").size(14))
                    .on_press(Msg::StartInstall)
                    .style(iced::theme::Button::Custom(Box::new(BtnPrimary)))
                    .padding([10, 22]),
                button(text("닫기").size(14))
                    .on_press(Msg::Close)
                    .style(iced::theme::Button::Custom(Box::new(BtnNormal)))
                    .padding([10, 16]),
            ].spacing(12),

            self.view_offline_panel(),
        ]
        .spacing(14)
        .align_items(Alignment::Center)
        .max_width(640);

        container(
            container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x()
                .center_y()
                .padding(28)
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(iced::theme::Container::Custom(Box::new(BgStyle)))
        .into()
    }

// ─────────────────────── 오프라인 패널 (테스트 빌드 한정) ─────

    #[cfg(feature = "offline")]
    fn view_offline_panel(&self) -> Element<'_, Msg> {
        let nick_trim = self.nickname.trim().to_string();
        let mut start_btn = button(text("  오프라인 테스트 시작  ").size(13))
            .style(iced::theme::Button::Custom(Box::new(BtnPrimary)))
            .padding([8, 18]);
        if !nick_trim.is_empty() {
            start_btn = start_btn.on_press(Msg::StartOfflineInstall);
        }
        container(
            column![
                text("⚠ 내부 테스트 빌드 — MSA 없이 닉네임으로 자체 런처 실행")
                    .size(11).style(WARN),
                row![
                    text_input("닉네임", &self.nickname)
                        .on_input(Msg::SetNickname)
                        .padding(6)
                        .width(Length::Fixed(160.0)),
                    start_btn,
                ].spacing(10).align_items(Alignment::Center),
            ].spacing(6).align_items(Alignment::Center)
        )
        .padding([10, 14])
        .style(iced::theme::Container::Custom(Box::new(WarnBadge)))
        .into()
    }

    #[cfg(not(feature = "offline"))]
    fn view_offline_panel(&self) -> Element<'_, Msg> {
        Space::with_height(0).into()
    }

// ─────────────────────── 화면: 설치 중 ──────────────────────

    fn view_installing(&self) -> Element<'_, Msg> {
        let pct = match self.progress_total {
            Some(t) if t > 0 => (self.progress_done as f32 / t as f32).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let sub = if self.substep_total > 0 {
            format!("[{}/{}]  {}", self.substep_idx, self.substep_total, self.substep_label)
        } else { String::new() };

        let log_text = self.log_lines.join("\n");

        let content = column![
            column![
                text("설치 / 업데이트 중").size(24).style(CHERRY),
                text(&self.progress_label).size(13).style(TEXT),
            ].spacing(4),

            column![
                progress_bar(0.0..=1.0, pct)
                    .style(iced::theme::ProgressBar::Custom(Box::new(ProgressStyle)))
                    .height(8),
                row![
                    text(&sub).size(11).style(TEXT_MUTED),
                    Space::with_width(Length::Fill),
                    text(format!("{:.0}%", pct * 100.0)).size(11).style(CHERRY),
                ],
            ].spacing(4),

            container(
                scrollable(
                    container(
                        text(log_text).size(11)
                            .font(iced::Font::with_name("Malgun Gothic"))
                            .style(TEXT_MUTED)
                    )
                    .padding(12)
                    .width(Length::Fill)
                )
                .height(Length::Fixed(300.0))
            )
            .style(iced::theme::Container::Custom(Box::new(LogStyle)))
            .width(Length::Fill),
        ]
        .spacing(16)
        .width(Length::Fill);

        container(
            container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(28)
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(iced::theme::Container::Custom(Box::new(BgStyle)))
        .into()
    }

// ─────────────────────── 화면: 완료 ─────────────────────────

    fn view_done(&self, _launched: bool) -> Element<'_, Msg> {
        let content = column![
            text("설치 완료").size(30).style(SUCCESS),
            Space::with_height(4),
            container(
                column![
                    text("· 바탕화면과 시작메뉴에 '체리쉬월드' 바로가기가 생성되었습니다.").size(12),
                ].spacing(6)
            )
            .padding([14, 18])
            .style(iced::theme::Container::Custom(Box::new(CardStyle))),

            Space::with_height(8),
            row![
                button(text("  실행  ").size(14))
                    .on_press(Msg::Launch)
                    .style(iced::theme::Button::Custom(Box::new(BtnPrimary)))
                    .padding([10, 22]),
                button(text("종료").size(14))
                    .on_press(Msg::Close)
                    .style(iced::theme::Button::Custom(Box::new(BtnNormal)))
                    .padding([10, 16]),
            ].spacing(12),
        ]
        .spacing(14)
        .align_items(Alignment::Center)
        .max_width(520);

        container(
            container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x()
                .center_y()
                .padding(28)
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(iced::theme::Container::Custom(Box::new(BgStyle)))
        .into()
    }

// ─────────────────────── 화면: 오류 ─────────────────────────

    fn view_error(&self, err: &str) -> Element<'_, Msg> {
        let content = column![
            text("설치 실패").size(26).style(Color { r: 1.0, g: 0.4, b: 0.4, a: 1.0 }),
            Space::with_height(4),
            container(
                scrollable(
                    container(
                        text(err).size(11)
                            .font(iced::Font::with_name("Malgun Gothic"))
                            .style(TEXT_MUTED)
                    )
                    .padding(12)
                    .width(Length::Fill)
                )
                .height(Length::Fixed(300.0))
            )
            .style(iced::theme::Container::Custom(Box::new(LogStyle)))
            .width(Length::Fill),

            Space::with_height(8),
            button(text("닫기").size(14))
                .on_press(Msg::Close)
                .style(iced::theme::Button::Custom(Box::new(BtnNormal)))
                .padding([10, 20]),
        ]
        .spacing(12)
        .align_items(Alignment::Center)
        .max_width(580);

        container(
            container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(28)
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(iced::theme::Container::Custom(Box::new(BgStyle)))
        .into()
    }
}
