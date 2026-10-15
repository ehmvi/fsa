//! lv2 hosting is reported as unavailable where livi doesn't build.

use std::collections::BTreeMap;
use std::convert::Infallible;

use super::{Lv2Info, Lv2Param};
use crate::engine::Unit;

pub struct Lv2Host;

impl Lv2Host {
    pub fn new() -> Self {
        Self
    }

    pub fn plugins(&self) -> Vec<Lv2Info> {
        Vec::new()
    }

    pub fn load(
        &self,
        _uri: &str,
        _controls: &BTreeMap<String, f32>,
    ) -> anyhow::Result<(Lv2Plugin, Lv2Unit)> {
        anyhow::bail!("lv2 hosting is only available on x86_64 linux")
    }
}

pub struct Lv2Plugin {
    pub info: Lv2Info,
    pub params: Vec<Lv2Param>,
    never: Infallible,
}

impl Lv2Plugin {
    pub fn set(&mut self, _param: usize, _value: f32) {
        match self.never {}
    }

    pub fn controls(&self) -> BTreeMap<String, f32> {
        match self.never {}
    }
}

pub enum Lv2Unit {}

impl Unit for Lv2Unit {
    fn process(&mut self, _buf: &mut [f32]) {
        match *self {}
    }

    fn reset(&mut self) {
        match *self {}
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send> {
        match *self {}
    }
}
