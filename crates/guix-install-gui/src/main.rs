mod app;
mod bridge;
mod styles;

use std::sync::mpsc;
use std::thread;

use iced::Font;
use iced::futures::channel::mpsc as async_mpsc;

use app::State;
use bridge::{IcedUi, UiEvent};

const DEJAVU: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
const DEJAVU_BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf");

fn main() -> iced::Result {
    // Worker -> GUI: async channel the iced subscription drains.
    let (to_gui, from_worker) = async_mpsc::unbounded::<UiEvent>();
    // GUI -> worker: sync channel the worker blocks on for each answer.
    let (to_worker, from_gui) = mpsc::channel::<bridge::PromptResponse>();

    // GUI -> worker: retry signal. After a failure the worker parks here; the
    // failure screen's Retry button wakes it to re-run via the resume path.
    let (retry_tx, retry_rx) = mpsc::channel::<()>();

    // Default to a real install; `--dry-run` keeps it interview-only.
    let dry_run = std::env::args().any(|a| a == "--dry-run");

    let worker_tx = to_gui.clone();
    thread::spawn(move || {
        let mut ui = IcedUi::new(worker_tx.clone(), from_gui);
        loop {
            // First pass runs the full interview; retries re-enter the resume
            // path, which skips completed phases.
            match guix_install_core::run::run_interactive(&mut ui, dry_run) {
                Ok(()) => {
                    let _ = worker_tx.unbounded_send(UiEvent::Finished);
                    break;
                }
                Err(e) if guix_install_core::ui::is_cancelled(&e) => {
                    let _ = worker_tx.unbounded_send(UiEvent::Finished);
                    break;
                }
                Err(e) => {
                    let summary = e.to_string();
                    let detail = format!("{e:?}");
                    let _ = worker_tx.unbounded_send(UiEvent::Failed { summary, detail });
                    // Dry-run failures aren't retryable installs; just stop.
                    if dry_run {
                        break;
                    }
                    // Park until the GUI asks to retry (or the channel closes).
                    if retry_rx.recv().is_err() {
                        break;
                    }
                }
            }
        }
    });

    let from_worker = std::cell::Cell::new(Some(from_worker));
    let to_worker = std::cell::Cell::new(Some(to_worker));
    let retry_tx = std::cell::Cell::new(Some(retry_tx));

    iced::application(
        move || {
            let rx = from_worker.take().expect("boot runs once");
            let tx = to_worker.take().expect("boot runs once");
            let retry = retry_tx.take().expect("boot runs once");
            State::new(tx, rx, retry, dry_run)
        },
        State::update,
        State::view,
    )
    .title(State::title)
    .theme(State::theme)
    .subscription(State::subscription)
    .default_font(Font::with_name("DejaVu Sans"))
    .font(DEJAVU)
    .font(DEJAVU_BOLD)
    .run()
}
