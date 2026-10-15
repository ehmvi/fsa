mod app;
mod audio;
mod config;
mod dsp;
mod engine;
mod plugins;
mod shared;
mod ui;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail};

use crate::app::{App, Setup};
use crate::config::Config;
use crate::engine::Engine;
use crate::shared::{InputChannel, Shared};

fn main() -> anyhow::Result<()> {
    // safety: nothing else is running yet
    unsafe { plugins::lv2::export_search_path() };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut config_path = config::default_path();
    let mut input = None;
    let mut channel = None;
    let mut list = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" | "-i" => input = args.next(),
            "--channel" | "-c" => {
                let Some(ch) = args.next().as_deref().and_then(InputChannel::parse) else {
                    bail!("--channel expects left, right or mix");
                };
                channel = Some(ch);
            }
            "--config" => {
                config_path = args
                    .next()
                    .ok_or_else(|| anyhow!("--config expects a path"))?
                    .into()
            }
            "--list" | "-l" => list = true,
            "--help" | "-h" => {
                println!(
                    "usage: fsa [--input <node.name>] [--channel left|right|mix] [--config <path>] [--list]\n\
                     settings are stored in {}",
                    config::default_path().display()
                );
                return Ok(());
            }
            other => bail!("unknown argument {other:?}, see --help"),
        }
    }

    let mut config = Config::load(&config_path);
    if input.is_some() {
        config.input = input;
    }
    if let Some(ch) = channel {
        config.input_channel = ch;
    }

    let shared = Arc::new(Shared::default());
    let mut setup = Setup::from_config(&config, &shared);
    let (engine, engine_handle) = Engine::new(
        shared.clone(),
        std::mem::take(&mut setup.slots),
        audio::SAMPLE_RATE as f32,
    );
    let backend = audio::Backend::start(shared.clone(), engine, config.input.clone())?;

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
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([680.0, 440.0]),
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
                setup,
                config,
                config_path,
            )))
        }),
    )
    .map_err(|e| anyhow!("gui: {e}"))
}
