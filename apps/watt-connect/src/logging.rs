//! Logging. stdout belongs to the MCP protocol, so logs go to stderr and/or a
//! log file, and every line passes through the process-wide redactor.

use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
struct RedactingMakeWriter {
    file: Option<Arc<Mutex<std::fs::File>>>,
    stderr: bool,
}

struct RedactingWriter {
    file: Option<Arc<Mutex<std::fs::File>>>,
    stderr: bool,
}

impl Write for RedactingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = connect_vault::redact::redact(&String::from_utf8_lossy(buf));
        if self.stderr {
            io::stderr().write_all(text.as_bytes())?;
        }
        if let Some(file) = &self.file {
            file.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .write_all(text.as_bytes())?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stderr().flush()
    }
}

impl<'a> MakeWriter<'a> for RedactingMakeWriter {
    type Writer = RedactingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter {
            file: self.file.clone(),
            stderr: self.stderr,
        }
    }
}

/// `default_level` applies unless `WATT_CONNECT_LOG` is set.
pub fn init(default_level: &str, log_dir: Option<&Path>, stderr: bool) {
    let filter = EnvFilter::try_from_env("WATT_CONNECT_LOG").unwrap_or_else(|_| {
        EnvFilter::new(format!("{default_level},rmcp=warn,reqwest=warn,hyper=warn"))
    });
    let file = log_dir.and_then(|dir| {
        std::fs::create_dir_all(dir).ok()?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("watt-connect.log"))
            .ok()
            .map(|file| Arc::new(Mutex::new(file)))
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(RedactingMakeWriter { file, stderr })
        .with_ansi(false)
        .with_target(false)
        .try_init();
}
