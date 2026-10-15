mod audio;
mod engine;
mod shared;

use std::sync::Arc;
use std::time::Duration;

use anyhow::bail;

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

    let _backend = audio::Backend::start(shared.clone(), engine::Engine::new(), input)?;

    if list {
        std::thread::sleep(Duration::from_millis(500));
        for d in shared.devices.lock().unwrap().iter() {
            println!("{}\t{}", d.name, d.description);
        }
        return Ok(());
    }

    log::info!(
        "virtual source \"{}\" is up, ctrl+c to quit",
        audio::SOURCE_DESCRIPTION
    );
    loop {
        std::thread::park();
    }
}
