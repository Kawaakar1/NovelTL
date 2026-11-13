//! Linux: LunaHook только для Windows — заглушка с тем же интерфейсом.

use std::sync::mpsc::Sender;

use crate::source::{HookEvent, HookSource, Waker};

pub const HOSTINFO_WARNING: i32 = 1;
pub const HOSTINFO_ENGINE: i32 = 4;

pub struct LunaHost;

impl LunaHost {
    pub fn start(_events: Sender<HookEvent>, _waker: Waker) -> LunaHost {
        LunaHost
    }
}

impl HookSource for LunaHost {
    fn attach(&mut self, _pid: u32, _codepage: u32) {}
    fn detach(&mut self, _pid: u32) {}
    fn shutdown(&mut self) {}
    fn error(&self) -> String {
        String::new()
    }
}
