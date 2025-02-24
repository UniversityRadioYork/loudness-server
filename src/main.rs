use std::collections::HashMap;

use ebur128::{EbuR128, Mode as EbuMode};
use tracing_subscriber::prelude::*;

mod config;
mod web_server;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct LoudnessState {
    pub inputs: HashMap<String, InputLoudness>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct InputsLoudness {
    pub inputs: HashMap<String, InputLoudness>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct InputLoudness {
    pub momentary: f64,
    pub short_term: f64,
    pub global: f64,
    pub range: f64,
}

struct InputState {
    ebur128: EbuR128,
    ports: Vec<jack::Port<jack::AudioIn>>,
    last_values: InputLoudness,
}

enum Command {
    Reset { input: String },
}

const UPDATE_INTERVAL: f64 = 0.1;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::filter::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "debug".into()),
        ))
        .init();

    let config = config::load();

    let (client, _status) = jack::Client::new(
        &config.jack.client_name,
        jack::ClientOptions::NO_START_SERVER,
    )?;
    let sample_rate = client.sample_rate();
    let sample_t = 1.0 / sample_rate as f64;

    let mut input_states = HashMap::<String, InputState>::new();
    for (input, config) in &config.inputs {
        let mut ports = vec![];
        for i in 0..(config.channels) {
            ports.push(client.register_port(&format!("{input}_{i}"), jack::AudioIn::default())?);
        }
        let ebur128 = EbuR128::new(
            config.channels as u32,
            sample_rate as u32,
            EbuMode::M | EbuMode::S | EbuMode::I | EbuMode::LRA,
        )?;
        input_states.insert(
            input.clone(),
            InputState {
                ebur128,
                ports,
                last_values: InputLoudness {
                    momentary: f64::NEG_INFINITY,
                    short_term: f64::NEG_INFINITY,
                    global: f64::NEG_INFINITY,
                    range: 0.0,
                },
            },
        );
    }
    let mut next_update = UPDATE_INTERVAL;

    let (tx, state) = tokio::sync::watch::channel(InputsLoudness::default());
    let (command_tx, command_rx) = std::sync::mpsc::channel();

    let process = jack::contrib::ClosureProcessHandler::new(move |client, ps| {
        while let Ok(command) = command_rx.try_recv() {
            match command {
                Command::Reset { input } => {
                    let Some(input) = input_states.get_mut(&input) else {
                        continue;
                    };
                    input.ebur128.reset();
                }
            }
        }
        next_update -= sample_t * (client.buffer_size() as f64);
        for (_, state) in &mut input_states {
            let buffers = state
                .ports
                .iter()
                .map(|p| p.as_slice(ps))
                .collect::<Vec<_>>();
            state
                .ebur128
                .add_frames_planar_f32(&buffers)
                .expect("invalid state");
        }
        if next_update <= 0.0 {
            let mut loudness = HashMap::new();
            for (input, state) in &mut input_states {
                let momentary = state
                    .ebur128
                    .loudness_momentary()
                    .expect("failed to calculate momentary loudness");
                let short_term = state
                    .ebur128
                    .loudness_shortterm()
                    .expect("failed to calculate short-term loudness");
                let global = state
                    .ebur128
                    .loudness_global()
                    .expect("failed to calculate global loudness");
                let range = state
                    .ebur128
                    .loudness_range()
                    .expect("failed to calculate loudness range");
                let current_loudness = InputLoudness {
                    momentary,
                    short_term,
                    global,
                    range,
                };
                if current_loudness != state.last_values {
                    state.last_values = current_loudness;
                    loudness.insert(input.clone(), current_loudness);
                }
            }
            if !loudness.is_empty() {
                let _ = tx.send(InputsLoudness { inputs: loudness });
            }
        }
        while next_update <= 0.0 {
            next_update += UPDATE_INTERVAL;
        }
        jack::Control::Continue
    });

    let _active_client = client.activate_async((), process).unwrap();

    web_server::run(config, state, command_tx)
}
