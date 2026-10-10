//! Smithay reports through `tracing`; this hands its events to the `log` logger so the session log keeps one format and filter (`RUST_LOG`). Spans are dropped: Smithay opens one per key press and client bind, which would bury the log.

use std::fmt::Write as _;

use tracing::{
    Event, Id, Level, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Record},
    subscriber::Interest,
};

pub fn install() {
    if tracing::subscriber::set_global_default(Bridge).is_err() {
        log::warn!("a tracing subscriber was already set");
    }
}

struct Bridge;

fn level(l: &Level) -> log::Level {
    match *l {
        Level::ERROR => log::Level::Error,
        Level::WARN => log::Level::Warn,
        Level::INFO => log::Level::Info,
        Level::DEBUG => log::Level::Debug,
        Level::TRACE => log::Level::Trace,
    }
}

fn log_enabled(meta: &Metadata<'_>) -> bool {
    log::logger().enabled(
        &log::Metadata::builder()
            .level(level(meta.level()))
            .target(meta.target())
            .build(),
    )
}

#[derive(Default)]
struct Text(String);

impl Visit for Text {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let sep = if self.0.is_empty() { "" } else { " " };
        if field.name() == "message" {
            let _ = write!(self.0, "{sep}{value:?}");
        } else {
            let _ = write!(self.0, "{sep}{}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            let sep = if self.0.is_empty() { "" } else { " " };
            let _ = write!(self.0, "{sep}{value}");
        } else {
            self.record_debug(field, &value);
        }
    }
}

impl Subscriber for Bridge {
    fn register_callsite(&self, meta: &'static Metadata<'static>) -> Interest {
        if meta.is_event() && log_enabled(meta) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        meta.is_event() && log_enabled(meta)
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let meta = event.metadata();
        let mut text = Text::default();
        event.record(&mut text);
        log::logger().log(
            &log::Record::builder()
                .level(level(meta.level()))
                .target(meta.target())
                .module_path(meta.module_path())
                .file(meta.file())
                .line(meta.line())
                .args(format_args!("{}", text.0))
                .build(),
        );
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_map_one_to_one() {
        assert_eq!(level(&Level::WARN), log::Level::Warn);
        assert_eq!(level(&Level::TRACE), log::Level::Trace);
    }
}
