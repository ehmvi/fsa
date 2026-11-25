mod app;
mod audio;
mod config;
mod dsp;
mod engine;
mod plugins;
mod shared;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, bail};

use crate::app::{App, Setup};
use crate::config::Config;
use crate::engine::Engine;
use crate::shared::{InputChannel, Shared};

static HEADLESS_STOP: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
fn install_stop_signals() {
    extern "C" fn handle(_: libc::c_int) {
        HEADLESS_STOP.store(true, Ordering::Relaxed);
    }
    // safety: `handle` only performs an async-signal-safe atomic store
    unsafe {
        libc::signal(libc::SIGINT, handle as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle as *const () as libc::sighandler_t);
    }
}

#[cfg(not(unix))]
fn install_stop_signals() {}

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Run,
    Headless,
    List,
    Help,
    Version,
}

#[derive(Debug, PartialEq, Eq)]
struct Cli {
    mode: Mode,
    config_path: PathBuf,
    input: Option<String>,
    channel: Option<InputChannel>,
}

fn parse_args<I>(args: I) -> anyhow::Result<Cli>
where
    I: IntoIterator<Item = String>,
{
    let mut cli = Cli {
        mode: Mode::Run,
        config_path: config::default_path(),
        input: None,
        channel: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" | "-i" => {
                cli.input =
                    Some(args.next().ok_or_else(|| {
                        anyhow!("--input expects a device name, auto, or default")
                    })?);
            }
            "--channel" | "-c" => {
                let Some(ch) = args.next().as_deref().and_then(InputChannel::parse) else {
                    bail!("--channel expects left, right or mix");
                };
                cli.channel = Some(ch);
            }
            "--config" => {
                cli.config_path = args
                    .next()
                    .ok_or_else(|| anyhow!("--config expects a path"))?
                    .into()
            }
            "--headless" => cli.mode = Mode::Headless,
            "--list" | "-l" => cli.mode = Mode::List,
            "--version" | "-V" => {
                cli.mode = Mode::Version;
                return Ok(cli);
            }
            "--help" | "-h" => {
                cli.mode = Mode::Help;
                return Ok(cli);
            }
            other => bail!("unknown argument {other:?}, see --help"),
        }
    }
    Ok(cli)
}

fn main() -> anyhow::Result<()> {
    // safety: nothing else is running yet
    unsafe { plugins::lv2::export_search_path() };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = parse_args(std::env::args().skip(1))?;
    match cli.mode {
        Mode::Help => {
            println!(
                "usage: fsa [--input <node.name>|auto|default] [--channel left|right|mix] [--config <path>] [--headless] [--list] [--version]\n\
                 settings are stored in {}",
                config::default_path().display()
            );
            return Ok(());
        }
        Mode::Version => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Mode::List => {
            for d in audio::list_devices()? {
                println!("{}\t{}", d.name, d.description);
            }
            return Ok(());
        }
        Mode::Run | Mode::Headless => {}
    }

    let mut config = Config::load(&cli.config_path);
    if let Some(name) = cli.input {
        config.set_input_choice(&match name.as_str() {
            "auto" => audio::InputChoice::Auto,
            "default" => audio::InputChoice::Default,
            _ => audio::InputChoice::Device(name),
        });
    }
    if let Some(ch) = cli.channel {
        config.input_channel = ch;
    }

    let shared = Arc::new(Shared::default());
    let mut setup = Setup::from_config(&config, &shared);
    let (engine, engine_handle) = Engine::new(
        shared.clone(),
        std::mem::take(&mut setup.slots),
        audio::SAMPLE_RATE as f32,
    );
    let backend = audio::Backend::start(
        shared.clone(),
        engine,
        config.picker(),
        config.output.clone(),
    )?;

    if cli.mode == Mode::Headless {
        HEADLESS_STOP.store(false, Ordering::Relaxed);
        install_stop_signals();
        App::new(
            shared,
            backend,
            engine_handle,
            setup,
            config,
            cli.config_path,
        )
        .run_headless(&HEADLESS_STOP);
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("fsa")
            .with_title("fsa – mic processor")
            .with_inner_size([1040.0, 700.0])
            .with_min_inner_size([680.0, 440.0]),
        // repaint is already timed (30/60 hz); vsync blocks when hidden on wayland
        glow_options: eframe::egui_glow::GlowConfiguration {
            vsync: false,
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "fsa",
        options,
        Box::new(move |cc| {
            ui::theme::install(&cc.egui_ctx);
            Ok(Box::new(App::new(
                shared,
                backend,
                engine_handle,
                setup,
                config,
                cli.config_path,
            )))
        }),
    )
    .map_err(|e| anyhow!("gui: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> anyhow::Result<Cli> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn missing_input_value_errors() {
        assert!(parse(&["--input"]).is_err());
        assert!(parse(&["-i"]).is_err());
    }

    #[test]
    fn parses_modes_and_flags() {
        assert_eq!(parse(&["--version"]).unwrap().mode, Mode::Version);
        assert_eq!(parse(&["-V"]).unwrap().mode, Mode::Version);
        assert_eq!(parse(&["--list"]).unwrap().mode, Mode::List);
        assert_eq!(parse(&["--headless"]).unwrap().mode, Mode::Headless);
        assert_eq!(parse(&["--help"]).unwrap().mode, Mode::Help);

        let cli = parse(&["-i", "usb.mic", "-c", "left", "--config", "/tmp/a.toml"]).unwrap();
        assert_eq!(cli.mode, Mode::Run);
        assert_eq!(cli.input.as_deref(), Some("usb.mic"));
        assert_eq!(cli.channel, Some(InputChannel::Left));
        assert_eq!(cli.config_path, PathBuf::from("/tmp/a.toml"));
    }
}
