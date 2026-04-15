//! iced GUI. Phase 1 은 최소 플레이스홀더 창만.
//!
//! Phase 2에서 화면 전환(환영 → 감지 → 진행 → 완료 → 에러) 구현.

use anyhow::Result;
use iced::{
    widget::{button, column, container, text},
    Alignment, Application, Command, Element, Length, Settings, Theme,
};

use crate::paths::AppDirs;

pub fn run(dirs: AppDirs) -> Result<()> {
    App::run(Settings {
        window: iced::window::Settings {
            size: iced::Size::new(640.0, 420.0),
            resizable: false,
            ..Default::default()
        },
        flags: dirs,
        ..Default::default()
    })
    .map_err(|e| anyhow::anyhow!("iced 실행 실패: {e}"))
}

struct App {
    dirs: AppDirs,
}

#[derive(Debug, Clone)]
enum Msg {
    Close,
}

impl Application for App {
    type Executor = iced::executor::Default;
    type Message = Msg;
    type Theme = Theme;
    type Flags = AppDirs;

    fn new(flags: AppDirs) -> (Self, Command<Msg>) {
        (Self { dirs: flags }, Command::none())
    }

    fn title(&self) -> String {
        "CherishPack 설치 프로그램".into()
    }

    fn update(&mut self, msg: Msg) -> Command<Msg> {
        match msg {
            Msg::Close => iced::window::close(iced::window::Id::MAIN),
        }
    }

    fn view(&self) -> Element<Msg> {
        let content = column![
            text("CherishPack").size(36),
            text("Phase 1 — 기반 구조 검증용 플레이스홀더 창").size(14),
            text(format!("설치 경로: {}", self.dirs.root.display())).size(12),
            button(text("닫기")).on_press(Msg::Close),
        ]
        .spacing(14)
        .align_items(Alignment::Center);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x()
            .center_y()
            .padding(24)
            .into()
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}
