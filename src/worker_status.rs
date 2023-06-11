// Copyright 2022 the Tectonic Project
// Licensed under the MIT License.

//! Termcolor-based status reporting with context for the TeX workers. This is a
//! duplicate of Tectonic's WorkerStatusBackend with a hack to indicate which
//! worker is reporting.

use std::{fmt::Arguments, io::Write};
use tectonic_errors::Error;
use tectonic_status_base::{MessageKind, StatusBackend};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

pub struct WorkerStatusBackend {
    context: String,
    stderr: StandardStream,
    warning_spec: ColorSpec,
    error_spec: ColorSpec,
}

impl WorkerStatusBackend {
    pub fn new<C: ToString>(context: C) -> WorkerStatusBackend {
        let context = context.to_string();

        let mut warning_spec = ColorSpec::new();
        warning_spec.set_fg(Some(Color::Yellow)).set_bold(true);

        let mut error_spec = ColorSpec::new();
        error_spec.set_fg(Some(Color::Red)).set_bold(true);

        WorkerStatusBackend {
            context,
            stderr: StandardStream::stderr(ColorChoice::Auto),
            warning_spec,
            error_spec,
        }
    }

    fn styled<F>(&mut self, kind: MessageKind, f: F)
    where
        F: FnOnce(&mut StandardStream),
    {
        let spec = match kind {
            MessageKind::Note => {
                return;
            }
            MessageKind::Warning => &self.warning_spec,
            MessageKind::Error => &self.error_spec,
        };

        self.stderr.set_color(spec).expect("failed to set color");
        f(&mut self.stderr);
        self.stderr.reset().expect("failed to clear color");
    }

    fn generic_message(&mut self, kind: MessageKind, prefix: Option<&str>, args: Arguments) {
        let text = match prefix {
            Some(s) => {
                let s = s.strip_suffix(':').unwrap_or(s);
                format!("{}({}):", s, self.context)
            }

            None => match kind {
                MessageKind::Note => {
                    return;
                }
                MessageKind::Warning => format!("warning({}):", self.context),
                MessageKind::Error => format!("error({}):", self.context),
            },
        };

        self.styled(kind, |s| {
            write!(s, "{}", text).expect("failed to write to standard stream");
        });
        writeln!(self.stderr, " {}", args).expect("failed to write to standard stream");
    }

    // Helpers for the CLI program that aren't needed by the internal bits,
    // so we put them here to minimize the cross-section of the StatusBackend
    // trait.

    fn error_styled(&mut self, args: Arguments) {
        self.styled(MessageKind::Error, |s| {
            writeln!(s, "{}", args).expect("write to stderr failed");
        });
    }
}

/// Show formatted text to the user, styled as an error message.
///
/// On the console, this will normally cause the printed text to show up in
/// bright red.
#[macro_export]
macro_rules! tt_error_styled {
    ($dest:expr, $( $fmt_args:expr ),*) => {
        $dest.error_styled(format_args!($( $fmt_args ),*))
    };
}

impl StatusBackend for WorkerStatusBackend {
    fn report(&mut self, kind: MessageKind, args: Arguments, err: Option<&Error>) {
        self.generic_message(kind, None, args);

        if let Some(e) = err {
            for item in e.chain() {
                self.generic_message(kind, Some("caused by:"), format_args!("{}", item));
            }
        }
    }

    fn report_error(&mut self, err: &Error) {
        let mut first = true;
        let kind = MessageKind::Error;

        for item in err.chain() {
            if first {
                self.generic_message(kind, None, format_args!("{}", item));
                first = false;
            } else {
                self.generic_message(kind, Some("caused by:"), format_args!("{}", item));
            }
        }
    }

    fn note_highlighted(&mut self, _before: &str, _highlighted: &str, _after: &str) {}

    fn dump_error_logs(&mut self, output: &[u8]) {
        tt_error_styled!(
            self,
            "==============================================================================="
        );

        self.stderr
            .write_all(output)
            .expect("write to stderr failed");

        tt_error_styled!(
            self,
            "==============================================================================="
        );
    }
}
