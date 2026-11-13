//! Linux: снимки экрана для разработки — только Windows.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use eframe::egui;

use crate::ui::demo_game::DemoGame;
use crate::ui::glossary::GlossaryWindow;
use crate::ui::settings::SettingsWindow;
use crate::ui::wizard::Wizard;
use crate::ui::{Shared, Windows};

pub struct Shots {
    started: bool,
}

impl Shots {
    pub fn new(_dir: PathBuf) -> Shots {
        Shots { started: false }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        ctx: &egui::Context,
        shared: &Shared,
        _game: &mut DemoGame,
        _wizard: &mut Option<Wizard>,
        _windows: &mut Windows,
        _settings: &mut Option<SettingsWindow>,
        _glossary: &mut Option<GlossaryWindow>,
    ) {
        if !self.started {
            self.started = true;
            eprintln!("--shots: снимки экрана доступны только в Windows");
            shared.quit.store(true, Ordering::SeqCst);
            ctx.request_repaint();
        }
    }
}
