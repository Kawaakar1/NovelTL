//! Общий интерфейс источника текста (LunaHook или демо).
#![allow(dead_code)]

use std::sync::Arc;

/// Будильник интерфейса (позже — `egui::Context::request_repaint`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ThreadInfo {
    pub code: String,
    pub name: String,
    pub pid: u32,
    pub addr: u64,
    pub ctx: u64,
    pub ctx2: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookEvent {
    Connected(u32),
    Disconnected(u32),
    ThreadCreated(ThreadInfo),
    ThreadRemoved(ThreadInfo),
    Text(ThreadInfo, String),
    /// 1 — предупреждение, 4 — тип движка
    HostInfo(i32, String),
    AttachFailed(u32, String),
}

pub trait HookSource: Send {
    fn attach(&mut self, pid: u32, codepage: u32);
    fn detach(&mut self, pid: u32);
    fn shutdown(&mut self);
    /// "" если всё хорошо
    fn error(&self) -> String;
}
