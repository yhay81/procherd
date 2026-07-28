use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

use crate::{
    error::AppError,
    hex::encode_lower,
    model::{
        LOG_RECORD_SCHEMA_VERSION, LogEncoding, LogRecord, LogStream, LogSummary, LogsResult,
        RunState,
    },
    store::{create_new_private_file, ensure_regular_file, now_ms},
};

const READ_CHUNK_BYTES: usize = 8 * 1024;
const MAX_RECORD_LINE_BYTES: usize = 32 * 1024;

#[derive(Debug)]
pub struct LogChunk {
    pub stream: LogStream,
    pub bytes: Vec<u8>,
}

pub fn capture_stream<R>(stream: LogStream, mut reader: R, sender: Sender<LogChunk>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        loop {
            let mut bytes = vec![0; READ_CHUNK_BYTES];
            match reader.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => {
                    bytes.truncate(count);
                    if sender.send(LogChunk { stream, bytes }).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
}

pub fn channel() -> (Sender<LogChunk>, Receiver<LogChunk>) {
    mpsc::channel()
}

pub struct LogWriter {
    writer: BufWriter<File>,
    summary: LogSummary,
    stdout_digest: Sha256,
    stderr_digest: Sha256,
}

impl LogWriter {
    pub fn create(run_dir: &Path, max_bytes: u64) -> Result<Self, AppError> {
        let file = create_new_private_file(&run_dir.join("logs.ndjson"))?;
        Ok(Self {
            writer: BufWriter::new(file),
            summary: LogSummary::new(max_bytes),
            stdout_digest: Sha256::new(),
            stderr_digest: Sha256::new(),
        })
    }

    pub fn record(&mut self, chunk: LogChunk) -> Result<(), AppError> {
        match chunk.stream {
            LogStream::Stdout => self.stdout_digest.update(&chunk.bytes),
            LogStream::Stderr => self.stderr_digest.update(&chunk.bytes),
        }

        let available = self
            .summary
            .max_bytes
            .saturating_sub(self.summary.captured_bytes);
        let available_usize = usize::try_from(available).unwrap_or(usize::MAX);
        let captured = chunk.bytes.len().min(available_usize);
        let dropped = chunk.bytes.len().saturating_sub(captured);

        if captured > 0 {
            let bytes = &chunk.bytes[..captured];
            let record = LogRecord {
                schema_version: LOG_RECORD_SCHEMA_VERSION.to_owned(),
                cursor: self.summary.next_cursor,
                timestamp_ms: now_ms(),
                stream: chunk.stream,
                encoding: LogEncoding::Base64,
                data: STANDARD.encode(bytes),
                byte_count: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            };
            serde_json::to_writer(&mut self.writer, &record)?;
            self.writer.write_all(b"\n")?;
            self.writer.flush()?;
            self.summary.next_cursor = self.summary.next_cursor.saturating_add(1);
            self.summary.captured_bytes = self
                .summary
                .captured_bytes
                .saturating_add(record.byte_count);
        }

        self.summary.dropped_bytes = self
            .summary
            .dropped_bytes
            .saturating_add(u64::try_from(dropped).unwrap_or(u64::MAX));
        Ok(())
    }

    pub fn capturable_bytes<'a>(&self, chunk: &'a LogChunk) -> &'a [u8] {
        let available = self
            .summary
            .max_bytes
            .saturating_sub(self.summary.captured_bytes);
        let available_usize = usize::try_from(available).unwrap_or(usize::MAX);
        &chunk.bytes[..chunk.bytes.len().min(available_usize)]
    }

    pub fn snapshot(&self) -> LogSummary {
        self.summary.clone()
    }

    pub fn finish(mut self) -> Result<LogSummary, AppError> {
        self.writer.flush()?;
        self.writer.get_ref().sync_all()?;
        self.summary.stdout_sha256 = Some(encode_lower(self.stdout_digest.finalize()));
        self.summary.stderr_sha256 = Some(encode_lower(self.stderr_digest.finalize()));
        Ok(self.summary)
    }
}

pub fn read_logs(
    run_dir: &Path,
    state: &RunState,
    after_cursor: u64,
    limit: usize,
    stream: Option<LogStream>,
) -> Result<LogsResult, AppError> {
    if limit == 0 || limit > 1_000 {
        return Err(AppError::usage("log limit must be between 1 and 1000"));
    }
    let path = run_dir.join("logs.ndjson");
    let mut records = Vec::new();
    let mut has_more = false;
    let mut previous_cursor = 0;

    match OpenOptions::new().read(true).open(&path) {
        Ok(file) => {
            ensure_regular_file(&path)?;
            let mut reader = BufReader::new(file);
            loop {
                let mut line = Vec::new();
                let count = reader.read_until(b'\n', &mut line)?;
                if count == 0 {
                    break;
                }
                if line.len() > MAX_RECORD_LINE_BYTES {
                    return Err(AppError::integrity(format!(
                        "log record exceeds {MAX_RECORD_LINE_BYTES} bytes"
                    )));
                }
                if !line.ends_with(b"\n") {
                    if state.status.is_terminal() {
                        return Err(AppError::integrity(
                            "terminal run contains an incomplete log record",
                        ));
                    }
                    break;
                }
                let record: LogRecord = serde_json::from_slice(&line)
                    .map_err(|error| AppError::integrity(format!("invalid log record: {error}")))?;
                if record.schema_version != LOG_RECORD_SCHEMA_VERSION {
                    return Err(AppError::integrity(format!(
                        "unsupported log schema {}",
                        record.schema_version
                    )));
                }
                if record.cursor == 0 || record.cursor <= previous_cursor {
                    return Err(AppError::integrity(format!(
                        "log cursor {} is not strictly monotonic",
                        record.cursor
                    )));
                }
                if record.cursor >= state.logs.next_cursor {
                    return Err(AppError::integrity(format!(
                        "log cursor {} is outside the durable summary",
                        record.cursor
                    )));
                }
                previous_cursor = record.cursor;
                let decoded = decode_record(&record)?;
                if record.byte_count == 0
                    || u64::try_from(decoded.len()).unwrap_or(u64::MAX) != record.byte_count
                {
                    return Err(AppError::integrity(format!(
                        "log cursor {} byte count does not match its decoded data",
                        record.cursor
                    )));
                }
                if record.cursor <= after_cursor
                    || stream.is_some_and(|selected| selected != record.stream)
                {
                    continue;
                }
                if records.len() == limit {
                    has_more = true;
                    break;
                }
                records.push(record);
            }
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && matches!(
                    state.status,
                    crate::model::RunStatus::Created | crate::model::RunStatus::Starting
                ) => {}
        Err(error) => return Err(error.into()),
    }

    let next_after_cursor = records.last().map_or(after_cursor, |record| record.cursor);
    Ok(LogsResult {
        schema_version: "procherd.logs.v1".to_owned(),
        run_id: state.run_id.clone(),
        after_cursor,
        records,
        next_after_cursor,
        has_more,
        captured_bytes: state.logs.captured_bytes,
        dropped_bytes: state.logs.dropped_bytes,
        terminal: state.status.is_terminal(),
    })
}

pub fn decode_record(record: &LogRecord) -> Result<Vec<u8>, AppError> {
    match record.encoding {
        LogEncoding::Base64 => STANDARD.decode(&record.data).map_err(|error| {
            AppError::integrity(format!(
                "log cursor {} contains invalid base64: {error}",
                record.cursor
            ))
        }),
    }
}
