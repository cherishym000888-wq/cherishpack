//! iced GUI — CherishPack 설치 프로그램
//!
//! 화면: Welcome → Installing → Done / Error
//! 디자인: 분홍 파스텔 라이트 테마
//!   배경 #fff4f7 / 패널 #ffffff / 보더 #f8bbd0
//!   포인트 #f43f5e (체리 핑크) — 텍스트 #4a1d2e

use anyhow::Result;
use iced::{
    executor,
    font::{Family, Stretch, Style, Weight},
    widget::{
        button, column, container, progress_bar, row, scrollable, text, text_input, Row, Space,
    },
    Alignment, Application, Background, Border, Color, Command, Element, Font, Length, Settings,
    Shadow, Subscription, Theme, Vector,
};

/// 임베드한 Cafe24 Ssurround 폰트는 Bold weight (usWeightClass=700) 로만 들어있다.
/// `Font::with_name()` 은 default weight (Normal=400) 라 매칭 실패해서 한글이 깨진다 —
/// weight=Bold 명시한 const 를 default 로 사용해야 한다.
const APP_FONT: Font = Font {
    family: Family::Name("Cafe24 Ssurround"),
    weight: Weight::Bold,
    stretch: Stretch::Normal,
    style: Style::Normal,
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

const BG:          Color = Color { r: 1.000, g: 0.957, b: 0.969, a: 1.0 }; // #fff4f7  분홍 파스텔 배경
const PANEL:       Color = Color { r: 1.000, g: 1.000, b: 1.000, a: 1.0 }; // #ffffff  흰 카드
const PANEL2:      Color = Color { r: 0.988, g: 0.894, b: 0.925, a: 1.0 }; // #fce4ec
const BORDER:      Color = Color { r: 0.973, g: 0.733, b: 0.816, a: 1.0 }; // #f8bbd0
const BORDER2:     Color = Color { r: 0.957, g: 0.561, b: 0.694, a: 1.0 }; // #f48fb1
const TEXT:        Color = Color { r: 0.290, g: 0.114, b: 0.180, a: 1.0 }; // #4a1d2e  진한 분홍 갈
const TEXT_MUTED:  Color = Color { r: 0.545, g: 0.380, b: 0.471, a: 1.0 }; // #8b6178
const CHERRY:      Color = Color { r: 0.957, g: 0.247, b: 0.369, a: 1.0 }; // #f43f5e (포인트 유지)
const CHERRY_DARK: Color = Color { r: 0.882, g: 0.114, b: 0.282, a: 1.0 }; // #e11d48
const SUCCESS:     Color = Color { r: 0.957, g: 0.247, b: 0.369, a: 1.0 }; // #f43f5e  CHERRY 와 동일 (브랜드 통일, 라이트 테마에서 녹색이 촌스러워서 변경)
const WARN:        Color = Color { r: 0.612, g: 0.376, b: 0.000, a: 1.0 }; // #9c6000  진한 호박색
const LOG_BG:      Color = Color { r: 0.988, g: 0.937, b: 0.957, a: 1.0 }; // #fcf0f4

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

/// 카드 패널 (보더 + 그림자) — 라이트 테마용 옅은 그림자
struct CardStyle;
impl container::StyleSheet for CardStyle {
    type Style = Theme;
    fn appearance(&self, _: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(PANEL)),
            text_color: Some(TEXT),
            border: Border { color: BORDER, width: 1.0, radius: 12.0.into() },
            shadow: Shadow {
                color: Color { r: 0.957, g: 0.247, b: 0.369, a: 0.12 }, // 옅은 분홍 그림자
                offset: Vector::new(0.0, 3.0),
                blur_radius: 14.0,
            },
        }
    }
}

/// 로그/코드 배경 — 라이트 톤에 맞춰 옅은 분홍 박스
struct LogStyle;
impl container::StyleSheet for LogStyle {
    type Style = Theme;
    fn appearance(&self, _: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(LOG_BG)),
            text_color: Some(TEXT),
            border: Border { color: BORDER, width: 1.0, radius: 8.0.into() },
            ..Default::default()
        }
    }
}

/// 경고 배지 — 라이트 배경 위에 어두운 호박색
struct WarnBadge;
impl container::StyleSheet for WarnBadge {
    type Style = Theme;
    fn appearance(&self, _: &Theme) -> container::Appearance {
        container::Appearance {
            background: Some(Background::Color(Color { r: 1.0, g: 0.945, b: 0.812, a: 1.0 })), // #fff1cf
            text_color: Some(WARN),
            border: Border { color: WARN, width: 1.0, radius: 6.0.into() },
            ..Default::default()
        }
    }
}

/// 일반 버튼 — 라이트 테마: 흰 배경 + 분홍 보더 + 진한 텍스트
struct BtnNormal;
impl button::StyleSheet for BtnNormal {
    type Style = Theme;
    fn active(&self, _: &Theme) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(PANEL)),
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

/// 진행바 — 트랙은 옅은 분홍, 바는 체리 핑크
struct ProgressStyle;
impl progress_bar::StyleSheet for ProgressStyle {
    type Style = Theme;
    fn appearance(&self, _: &Theme) -> progress_bar::Appearance {
        progress_bar::Appearance {
            background: Background::Color(PANEL2),
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
    // 작업표시줄/창 아이콘 — iced 가 별도로 안 박으면 Windows 가 generic doc 으로 표시함.
    // build.rs 의 winres exe 아이콘과 별개로 런타임 창에도 명시 지정 필요.
    let icon = iced::window::icon::from_file_data(
        include_bytes!("../assets/window_icon.png"),
        None,
    ).ok();

    App::run(Settings {
        id: None,
        window: iced::window::Settings {
            size: iced::Size::new(720.0, 520.0),
            resizable: false,
            icon,
            ..iced::window::Settings::default()
        },
        flags: dirs,
        // 마인크래프트 클라가 쓰는 Cafe24 Ssurround 폰트 임베드 → exe 내장.
        // Bold weight 만 들어있으므로 APP_FONT (Bold 명시) 사용.
        fonts: vec![include_bytes!("../assets/cafe24ssurround.ttf").as_slice().into()],
        default_font: APP_FONT,
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
    /// 현재 큰 단계 이름 (Status 이벤트로 갱신).
    /// 멈췄을 때 어느 단계에서 멈췄는지 알리는 용도.
    current_step_label: String,
    /// 현재 단계 진입 후 발생한 메시지들. 단계가 바뀌면 자동 clear.
    /// 정상 진행 중에는 화면에 표시 안 하고, Error 화면에서만 노출.
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
                current_step_label: String::new(),
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
            Msg::Orc(ev)        => { self.apply_event(ev); Command::none() }
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
            // Status = 큰 단계 경계. 단계 바뀌면 이전 단계 로그/서브 모두 리셋해서
            // Installing 화면이 깨끗하게 갱신되고, Error 화면에서는 마지막 단계의 메시지만 보이게 한다.
            OrcEvent::Status(s) => {
                self.current_step_label = s.clone();
                self.progress_label = s;
                self.substep_idx = 0;
                self.substep_total = 0;
                self.substep_label.clear();
                self.log_lines.clear();
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
            OrcEvent::Info(s) => { if !s.is_empty() { self.push_log(s); } }
            OrcEvent::Warn(s) => { self.push_log(format!("[!] {s}")); }
            OrcEvent::Done { launched } => { self.screen = Screen::Done { launched }; }
            OrcEvent::Error(e) => { self.push_log(format!("[x] {e}")); self.screen = Screen::Error(e); }
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
                {
                    // row! 매크로는 인자 안에 #[cfg(...)] 를 받지 못해
                    // Row builder 로 직접 조립한다.
                    let mut r: Row<'_, Msg> = Row::new();
                    #[cfg(feature = "verylow_preset")]
                    {
                        r = r.push(mk_preset_btn("V.LOW", "VRAM 극절약", Preset::VeryLow));
                    }
                    r = r.push(mk_preset_btn("LOW",    "쉐이더 OFF",    Preset::Low));
                    r = r.push(mk_preset_btn("MEDIUM", "C. Reimagined", Preset::Medium));
                    r = r.push(mk_preset_btn("HIGH",   "C. Unbound",    Preset::High));
                    r = r.push(mk_preset_btn("HIGH++", "Reth. Voxels",  Preset::HighPlus));
                    r.spacing(8).width(Length::Fill)
                },
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
        let mut start_btn = button(text("  시작  ").size(13))
            .style(iced::theme::Button::Custom(Box::new(BtnPrimary)))
            .padding([8, 18]);
        if !nick_trim.is_empty() {
            start_btn = start_btn.on_press(Msg::StartOfflineInstall);
        }
        container(
            row![
                text_input("닉네임", &self.nickname)
                    .on_input(Msg::SetNickname)
                    .padding(6)
                    .width(Length::Fixed(160.0)),
                start_btn,
            ].spacing(10).align_items(Alignment::Center)
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
        let pct_int = (pct * 100.0).round() as u32;

        // 메인 단계명 (Status 로 들어온 가장 최근 큰 단계).
        // 시작 직후엔 비어있을 수 있으니 progress_label 로 폴백.
        let step_main = if !self.current_step_label.is_empty() {
            self.current_step_label.clone()
        } else {
            self.progress_label.clone()
        };

        // 서브 라인 — substep([idx/total] label) 우선, 없으면 progress_label 이 단계명과 다를 때만.
        let sub_line = if self.substep_total > 0 {
            format!("{} / {}  ·  {}", self.substep_idx, self.substep_total, self.substep_label)
        } else if !self.progress_label.is_empty() && self.progress_label != step_main {
            self.progress_label.clone()
        } else {
            String::new()
        };

        let content = column![
            Space::with_height(Length::Fill),

            // 헤더 라벨 — 작게
            text("설치 / 업데이트 중").size(11).style(TEXT_MUTED),
            Space::with_height(4),

            // 큰 percentage
            text(format!("{}%", pct_int)).size(72).style(CHERRY),

            Space::with_height(18),

            // progress bar
            container(
                progress_bar(0.0..=1.0, pct)
                    .style(iced::theme::ProgressBar::Custom(Box::new(ProgressStyle)))
                    .height(10)
            )
            .width(Length::Fixed(440.0)),

            Space::with_height(22),

            // 단계명 (큰 글씨)
            text(&step_main).size(18).style(TEXT),

            // 서브 라인 (있을 때만)
            text(&sub_line).size(12).style(TEXT_MUTED),

            Space::with_height(Length::Fill),
        ]
        .spacing(0)
        .align_items(Alignment::Center)
        .width(Length::Fill);

        container(
            container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(40)
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
        let red = Color { r: 1.0, g: 0.40, b: 0.42, a: 1.0 };
        let red_dim = Color { r: 1.0, g: 0.65, b: 0.66, a: 1.0 };

        let step_name = if self.current_step_label.is_empty() {
            "(단계 미상)".to_string()
        } else {
            self.current_step_label.clone()
        };

        // 단계 진입 후 메시지들 — Error 발생 직전 상태가 그대로 남아있음.
        // 마지막 줄이 [x] err 일 가능성 — 중복 회피 위해 err 와 같은 줄은 제거.
        let step_lines: Vec<&str> = self.log_lines
            .iter()
            .map(|s| s.as_str())
            .filter(|s| !s.contains(err))
            .collect();
        let step_log_joined = step_lines.join("\n");

        // 헤더 카드 — 멈춘 단계 강조
        let stop_card = container(
            column![
                text("멈춘 단계").size(11).style(red_dim),
                text(&step_name).size(18).style(TEXT),
            ].spacing(4)
        )
        .padding([14, 18])
        .width(Length::Fill)
        .style(iced::theme::Container::Custom(Box::new(CardStyle)));

        // 에러 메시지 박스
        let err_box = container(
            scrollable(
                container(
                    text(err).size(12)
                        .font(APP_FONT)
                        .style(TEXT)
                )
                .padding(12)
                .width(Length::Fill)
            )
            .height(Length::Fixed(110.0))
        )
        .style(iced::theme::Container::Custom(Box::new(LogStyle)))
        .width(Length::Fill);

        // 단계 진입 후 발생한 메시지 (있을 때만)
        let step_log_section: Element<'_, Msg> = if step_log_joined.is_empty() {
            Space::with_height(0).into()
        } else {
            column![
                text("이 단계의 상세 로그").size(11).style(TEXT_MUTED),
                container(
                    scrollable(
                        container(
                            text(&step_log_joined).size(11)
                                .font(APP_FONT)
                                .style(TEXT_MUTED)
                        )
                        .padding(12)
                        .width(Length::Fill)
                    )
                    .height(Length::Fixed(160.0))
                )
                .style(iced::theme::Container::Custom(Box::new(LogStyle)))
                .width(Length::Fill),
            ].spacing(6).into()
        };

        let content = column![
            text("설치 실패").size(28).style(red),
            Space::with_height(8),
            stop_card,
            Space::with_height(2),
            err_box,
            step_log_section,
            Space::with_height(8),
            button(text("닫기").size(14))
                .on_press(Msg::Close)
                .style(iced::theme::Button::Custom(Box::new(BtnNormal)))
                .padding([10, 20]),
        ]
        .spacing(8)
        .align_items(Alignment::Center)
        .max_width(640);

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
