//! no-op backend for non-linux hosts.

use std::sync::Arc;

use super::Ctrl;
use crate::engine::Engine;
use crate::shared::Shared;

pub struct Backend {}

impl Backend {
    pub fn start(
        shared: Arc<Shared>,
        _engine: Engine,
        _input: Option<String>,
    ) -> anyhow::Result<Self> {
        shared.set_status("no audio backend on this platform");
        Ok(Self {})
    }

    pub fn send(&self, _ctrl: Ctrl) {}
}
