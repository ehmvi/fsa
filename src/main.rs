mod app;
mod audio;
mod dsp;
mod engine;
mod shared;
mod ui;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail};

use crate::app::{App, ChainItem, ItemKind};
use crate::dsp::eq::{DEFAULT_BANDS, Eq, EqParams};
use crate::dsp::gate::{Gate, GateParams, GateSettings};
use crate::engine::{Engine, Slot, SlotState};
use crate::shared::{InputChannel, Shared};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut input = None;
    let mut list = false;
    let shared = Arc::new(Shared::default());
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" | "-i" => input = args.next(),
            "--channel" | "-c" => {
                let Some(ch) = args.next().as_deref().and_then(InputChannel::parse) else {
                    bail!("--channel expects left, right or mix");
                };
                shared.set_input_channel(ch);
            }
            "--list" | "-l" => list = true,
            "--help" | "-h" => {
                println!("usage: fsa [--input <node.name>] [--channel left|right|mix] [--list]");
                return Ok(());
            }
            other => bail!("unknown argument {other:?}, see --help"),
        }
    }

    let fs = audio::SAMPLE_RATE as f32;
    let eq = Arc::new(EqParams::new(&DEFAULT_BANDS));
    let gate = Arc::new(GateParams::new(&GateSettings::default()));
    let chain = vec![
        ChainItem {
            id: 0,
            kind: ItemKind::Eq,
            state: SlotState::new(true),
        },
        ChainItem {
            id: 1,
            kind: ItemKind::Gate,
            state: SlotState::new(true),
        },
    ];
    let slots = chain
        .iter()
        .map(|item| {
            let unit: Box<dyn engine::Unit> = match item.kind {
                ItemKind::Eq => Box::new(Eq::new(eq.clone(), fs)),
                ItemKind::Gate => Box::new(Gate::new(gate.clone(), fs)),
            };
            Slot::new(item.id, unit, item.state.clone(), fs)
        })
        .collect();
    let (engine, engine_handle) = Engine::new(shared.clone(), slots, fs);
    let backend = audio::Backend::start(shared.clone(), engine, input.clone())?;

    if list {
        std::thread::sleep(Duration::from_millis(500));
        for d in shared.devices.lock().unwrap().iter() {
            println!("{}\t{}", d.name, d.description);
        }
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("fsa")
            .with_title("fsa – mic processor")
            .with_inner_size([900.0, 620.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "fsa",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(App::new(
                shared,
                backend,
                engine_handle,
                eq,
                gate,
                chain,
                input,
            )))
        }),
    )
    .map_err(|e| anyhow!("gui: {e}"))
}
