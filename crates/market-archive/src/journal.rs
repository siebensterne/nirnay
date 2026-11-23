use std::fs::{
    File, //
    OpenOptions,
};
use std::io::{
    BufRead, //
    BufReader,
    Write,
};
use std::path::Path;

use fs2::FileExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::frame::MAX_RAW_RECORD_BYTES;
use crate::{ArchiveError, RawFrame};

/// Current health of the journal writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriterState {
    Healthy,
    Poisoned,
}

/// Failure point for the private test I/O seam. It is not part of the public
/// journal API.
#[cfg(all(test, feature = "market-shaped-tests"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailPoint {
    PartialRecordWrite,
    RecordWrite,
    NewlineWrite,
    Sync,
}

/// Private I/O boundary. Production uses a real `File`; tests use `Failing`,
/// which performs the real OS call before returning the injected `io::Error`.
/// `RecordWrite` is the exception: it models a failure before any bytes are
/// written, so no OS call occurs and the file is unchanged. The writer
/// observes the same error path as an actual write or `sync_data` failure
/// without a public generic storage abstraction.
enum JournalIo {
    File(File),
    #[cfg(all(test, feature = "market-shaped-tests"))]
    Failing {
        file: File,
        point: Option<FailPoint>,
    },
}

impl JournalIo {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            Self::File(file) => file.write_all(bytes),
            #[cfg(all(test, feature = "market-shaped-tests"))]
            Self::Failing { file, point } => match point {
                Some(FailPoint::PartialRecordWrite) => {
                    let split = bytes.len().saturating_sub(1);
                    file.write_all(&bytes[..split])?;
                    *point = None;
                    Err(std::io::Error::other(
                        "injected partial record write failure",
                    ))
                }
                Some(FailPoint::RecordWrite) => {
                    *point = None;
                    Err(std::io::Error::other("injected record write failure"))
                }
                Some(FailPoint::NewlineWrite) if bytes == b"\n" => {
                    file.write_all(bytes)?;
                    *point = None;
                    Err(std::io::Error::other("injected newline write failure"))
                }
                _ => file.write_all(bytes),
            },
        }
    }

    fn sync_data(&mut self) -> std::io::Result<()> {
        match self {
            Self::File(file) => file.sync_data(),
            #[cfg(all(test, feature = "market-shaped-tests"))]
            Self::Failing { file, point } => {
                file.sync_data()?;
                if matches!(point, Some(FailPoint::Sync)) {
                    *point = None;
                    return Err(std::io::Error::other("injected sync_data failure"));
                }
                Ok(())
            }
        }
    }
}

/// Single-writer, newline-delimited raw-frame journal.
///
/// Each append calls `sync_data`; this favours durability over throughput.
/// The OS file lock prevents two processes from concurrently appending. A
/// future recorder may add bounded group commit, but must preserve the recovery
/// contract and measure its durability/latency trade-off.
pub struct JournalWriter {
    io: JournalIo,
    state: WriterState,
}

fn uncertain_outcome(context: &'static str, source: std::io::Error) -> ArchiveError {
    ArchiveError::CommitOutcomeUnknown { context, source }
}

fn create_journal_file(path: &Path) -> Result<File, ArchiveError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(path)?;
    file.try_lock_exclusive()?;
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(file)
}

impl JournalWriter {
    pub fn create_new(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        Ok(Self {
            io: JournalIo::File(create_journal_file(path.as_ref())?),
            state: WriterState::Healthy,
        })
    }

    /// Test-only constructor using the private I/O seam. It performs real OS
    /// writes where applicable and then returns an `io::Error` at the selected
    /// point; `RecordWrite` fails before any OS write.
    #[cfg(all(test, feature = "market-shaped-tests"))]
    fn create_new_with_fail_point(
        path: impl AsRef<Path>,
        point: FailPoint,
    ) -> Result<Self, ArchiveError> {
        Ok(Self {
            io: JournalIo::Failing {
                file: create_journal_file(path.as_ref())?,
                point: Some(point),
            },
            state: WriterState::Healthy,
        })
    }

    pub fn open_append(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let path = path.as_ref();
        let file = OpenOptions::new().append(true).open(path)?;
        file.try_lock_exclusive()?;
        // Never append behind a torn or corrupted tail. Validation is streamed
        // while the exclusive lock is held, so reopening a large journal does
        // not require loading every historical frame into memory.
        let _ = verify_journal(path)?;
        Ok(Self {
            io: JournalIo::File(file),
            state: WriterState::Healthy,
        })
    }

    /// Append one validated frame.
    ///
    /// Validation and serialization happen before any file I/O, so an invalid
    /// frame leaves the writer healthy. Once any write or sync fails the commit
    /// outcome is unknown: the writer is poisoned and every later append is
    /// refused before touching the file. Recovery requires dropping and
    /// reopening the journal, which verifies the committed tail.
    pub fn append(&mut self, frame: &RawFrame) -> Result<(), ArchiveError> {
        if self.state == WriterState::Poisoned {
            return Err(ArchiveError::Poisoned);
        }
        let line = frame.to_json_line()?;
        if let Err(error) = self.io.write_all(line.as_bytes()) {
            self.state = WriterState::Poisoned;
            return Err(uncertain_outcome("record write", error));
        }
        if let Err(error) = self.io.write_all(b"\n") {
            self.state = WriterState::Poisoned;
            return Err(uncertain_outcome("commit marker write", error));
        }
        if let Err(error) = self.io.sync_data() {
            self.state = WriterState::Poisoned;
            return Err(uncertain_outcome("sync_data", error));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalScan {
    pub frames: Vec<RawFrame>,
    pub bytes_scanned: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalSummary {
    pub frame_count: u64,
    pub bytes_scanned: u64,
}

/// A bounded, streaming reader for newline-delimited raw-frame journals.
///
/// Each call to [`JournalReader::next_frame`] returns at most one decoded
/// frame. The record buffer is reused and never grows beyond
/// [`MAX_RAW_RECORD_BYTES`]; callers that need every frame resident should use
/// [`scan_journal`] instead. After any error the reader is terminal and must be
/// discarded.
pub struct JournalReader<R> {
    inner: R,
    record: Vec<u8>,
    bytes_scanned: u64,
    frame_count: u64,
    line_number: u64,
    eof: bool,
}

impl<R: BufRead> JournalReader<R> {
    /// Wrap a buffered reader. No bytes are read until [`Self::next_frame`].
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            record: Vec::new(),
            bytes_scanned: 0,
            frame_count: 0,
            line_number: 0,
            eof: false,
        }
    }

    /// Return the next complete record, `Ok(None)` at a clean EOF, or an error.
    ///
    /// An unterminated final record is [`ArchiveError::TornTail`]; a record
    /// larger than [`MAX_RAW_RECORD_BYTES`] (newline excluded) is
    /// [`ArchiveError::RecordTooLarge`]. Both fail closed: nothing is repaired
    /// or skipped. I/O failures are propagated unchanged.
    pub fn next_frame(&mut self) -> Result<Option<RawFrame>, ArchiveError> {
        if self.eof {
            return Ok(None);
        }
        loop {
            let available = self.inner.fill_buf()?;
            if available.is_empty() {
                if !self.record.is_empty() {
                    return Err(ArchiveError::TornTail);
                }
                self.eof = true;
                return Ok(None);
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.map_or(available.len(), |index| index + 1);
            // The limit applies to record bytes, excluding the newline.
            let content_len = newline.map_or(self.record.len().saturating_add(take), |index| {
                self.record.len().saturating_add(index)
            });
            if content_len > MAX_RAW_RECORD_BYTES {
                return Err(ArchiveError::RecordTooLarge {
                    size: content_len,
                    limit: MAX_RAW_RECORD_BYTES,
                });
            }
            self.record.extend_from_slice(&available[..take]);
            self.inner.consume(take);
            self.bytes_scanned = self.bytes_scanned.saturating_add(take as u64);
            if newline.is_none() {
                continue;
            }

            self.line_number = self.line_number.saturating_add(1);
            let line = &self.record[..self.record.len() - 1];
            if line.is_empty() {
                return Err(ArchiveError::MalformedRecord(format!(
                    "line {} is empty",
                    self.line_number
                )));
            }
            let text = std::str::from_utf8(line).map_err(|error| {
                ArchiveError::MalformedRecord(format!(
                    "line {} is not UTF-8: {error}",
                    self.line_number
                ))
            })?;
            let frame = RawFrame::from_json_line(text)?;
            self.frame_count = self.frame_count.saturating_add(1);
            self.record.clear();
            return Ok(Some(frame));
        }
    }

    /// Consume the reader and return the underlying buffered stream.
    ///
    /// Callers that need observed byte counts should read them first.
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Number of complete frames returned so far.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Number of bytes consumed from the underlying reader so far.
    pub fn bytes_scanned(&self) -> u64 {
        self.bytes_scanned
    }
}

/// Visit and validate each journal record using record-by-record streaming.
/// The callback determines whether callers retain frames or only count them.
fn visit_reader<R: BufRead>(
    reader: &mut R,
    mut visit: impl FnMut(RawFrame) -> Result<(), ArchiveError>,
) -> Result<JournalSummary, ArchiveError> {
    let mut reader = JournalReader::new(reader);
    while let Some(frame) = reader.next_frame()? {
        visit(frame)?;
    }
    Ok(JournalSummary {
        frame_count: reader.frame_count(),
        bytes_scanned: reader.bytes_scanned(),
    })
}

fn visit_journal(
    path: impl AsRef<Path>,
    visit: impl FnMut(RawFrame) -> Result<(), ArchiveError>,
) -> Result<JournalSummary, ArchiveError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    visit_reader(&mut reader, visit)
}

/// Validate the entire journal without retaining decoded frames in memory.
/// This is the recovery/open-append path for long-running recordings.
pub fn verify_journal(path: impl AsRef<Path>) -> Result<JournalSummary, ArchiveError> {
    visit_journal(path, |_| Ok(()))
}

/// Scan and return all decoded frames. Use [`verify_journal`] for a large
/// archive when only integrity and counts are required.
pub fn scan_journal(path: impl AsRef<Path>) -> Result<JournalScan, ArchiveError> {
    let mut frames = Vec::new();
    let summary = visit_journal(path, |frame| {
        frames.push(frame);
        Ok(())
    })?;
    Ok(JournalScan {
        frames,
        bytes_scanned: summary.bytes_scanned,
    })
}

#[cfg(all(test, feature = "market-shaped-tests"))]
mod tests {
    use super::*;
    use crate::frame::{
        MAX_FIELD_BYTES, //
        MAX_PAYLOAD_BYTES,
    };

    fn frame(sequence: u64) -> RawFrame {
        let payload = format!("payload-{sequence}");
        RawFrame::new(
            "test-feed",
            "TEST:ABC",
            sequence as i64,
            sequence,
            None,
            Some(sequence),
            "synthetic-fixture",
            payload.as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn append_and_scan_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer = JournalWriter::create_new(&path).unwrap();
        writer.append(&frame(1)).unwrap();
        writer.append(&frame(2)).unwrap();
        drop(writer);

        let scan = scan_journal(&path).unwrap();
        assert_eq!(scan.frames, vec![frame(1), frame(2)]);
        assert!(scan.bytes_scanned > 0);
        assert_eq!(
            verify_journal(&path).unwrap(),
            JournalSummary {
                frame_count: 2,
                bytes_scanned: scan.bytes_scanned,
            }
        );
    }

    #[test]
    fn torn_tail_is_reported_and_not_repaired() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        std::fs::write(&path, b"{\"partial\":").unwrap();

        assert!(matches!(scan_journal(&path), Err(ArchiveError::TornTail)));
        assert!(
            matches!(verify_journal(&path), Err(ArchiveError::TornTail)),
            "unterminated record remains a torn tail"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"partial\":",
            "torn tail is not repaired"
        );
    }

    #[test]
    fn tampered_record_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let line = frame(1).to_json_line().unwrap();
        let tampered = line.replace("cGF5bG9hZC0x", "cGF5bG9hZC0y");
        std::fs::write(&path, format!("{tampered}\n")).unwrap();

        assert!(matches!(
            scan_journal(&path),
            Err(ArchiveError::PayloadHashMismatch)
        ));
    }

    #[test]
    fn append_refuses_to_extend_a_torn_journal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        std::fs::write(&path, b"{\"partial\":").unwrap();
        assert!(matches!(
            JournalWriter::open_append(&path),
            Err(ArchiveError::TornTail)
        ));
    }

    #[test]
    fn a_second_writer_cannot_acquire_the_same_journal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let writer = JournalWriter::create_new(&path).unwrap();
        assert!(JournalWriter::open_append(&path).is_err());
        drop(writer);
        assert!(JournalWriter::open_append(&path).is_ok());
    }

    #[test]
    fn an_empty_file_is_a_valid_empty_journal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty.jsonl");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(
            verify_journal(&path).unwrap(),
            JournalSummary {
                frame_count: 0,
                bytes_scanned: 0,
            }
        );
    }

    fn unknown_context(error: &ArchiveError) -> &'static str {
        match error {
            ArchiveError::CommitOutcomeUnknown { context, .. } => context,
            other => panic!("expected uncertain commit outcome, got {other:?}"),
        }
    }

    #[test]
    fn partial_record_write_is_uncertain_and_poisons_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer =
            JournalWriter::create_new_with_fail_point(&path, FailPoint::PartialRecordWrite)
                .unwrap();

        let error = writer.append(&frame(1)).unwrap_err();
        assert_eq!(
            unknown_context(&error),
            "record write",
            "partial write context"
        );
        assert!(
            matches!(
                &error,
                ArchiveError::CommitOutcomeUnknown { source, .. }
                    if source.to_string().contains("partial record write")
            ),
            "partial write reports uncertain outcome with original error"
        );
        assert_eq!(
            writer.state,
            WriterState::Poisoned,
            "partial write poisons writer"
        );
        let bytes_after_failure = std::fs::read(&path).unwrap();
        assert!(
            !bytes_after_failure.is_empty(),
            "partial record bytes must remain on disk"
        );

        assert!(
            matches!(writer.append(&frame(2)), Err(ArchiveError::Poisoned)),
            "poisoned writer refuses append"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes_after_failure,
            "poisoned writer must not mutate file"
        );
    }

    #[test]
    fn newline_write_failure_is_uncertain_and_poisons_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer =
            JournalWriter::create_new_with_fail_point(&path, FailPoint::NewlineWrite).unwrap();

        let error = writer.append(&frame(1)).unwrap_err();
        assert_eq!(
            unknown_context(&error),
            "commit marker write",
            "newline failure context"
        );
        assert_eq!(
            writer.state,
            WriterState::Poisoned,
            "newline failure poisons writer"
        );
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.ends_with(b"\n"),
            "a failed newline write may still leave bytes on disk"
        );
        assert!(
            matches!(writer.append(&frame(2)), Err(ArchiveError::Poisoned)),
            "poisoned writer refuses append"
        );
    }

    #[test]
    fn sync_data_failure_is_uncertain_and_poisons_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer = JournalWriter::create_new_with_fail_point(&path, FailPoint::Sync).unwrap();

        let error = writer.append(&frame(1)).unwrap_err();
        assert_eq!(unknown_context(&error), "sync_data", "sync failure context");
        assert_eq!(
            writer.state,
            WriterState::Poisoned,
            "sync failure poisons writer"
        );
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.ends_with(b"\n"),
            "bytes written before failed sync remain"
        );
        assert!(
            matches!(writer.append(&frame(2)), Err(ArchiveError::Poisoned)),
            "poisoned writer refuses append"
        );
    }

    #[test]
    fn append_after_poisoning_refuses_before_mutating_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer =
            JournalWriter::create_new_with_fail_point(&path, FailPoint::RecordWrite).unwrap();
        assert!(
            matches!(
                writer.append(&frame(1)),
                Err(ArchiveError::CommitOutcomeUnknown { .. })
            ),
            "failed dispatch reports uncertain outcome"
        );
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            matches!(writer.append(&frame(2)), Err(ArchiveError::Poisoned)),
            "poisoned writer refuses append"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "poisoned append leaves file unchanged"
        );
    }

    #[test]
    fn invalid_frame_before_writing_leaves_writer_healthy() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer = JournalWriter::create_new(&path).unwrap();
        let mut invalid = frame(1);
        invalid.payload_sha256 =
            "0000000000000000000000000000000000000000000000000000000000000000".to_owned();

        assert!(
            matches!(
                writer.append(&invalid),
                Err(ArchiveError::PayloadHashMismatch)
            ),
            "invalid frame is rejected before I/O"
        );
        assert_eq!(
            writer.state,
            WriterState::Healthy,
            "invalid frame must not poison writer"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"",
            "invalid frame performs no I/O"
        );
        writer.append(&frame(1)).unwrap();
        assert_eq!(
            verify_journal(&path).unwrap().frame_count,
            1,
            "valid frame after invalid one appends"
        );
    }

    #[test]
    fn successful_append_writes_a_complete_synced_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer = JournalWriter::create_new(&path).unwrap();
        writer.append(&frame(7)).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.ends_with(b"\n"),
            "successful append writes commit marker"
        );
        assert_eq!(
            writer.state,
            WriterState::Healthy,
            "successful append keeps writer healthy"
        );
        let summary = verify_journal(&path).unwrap();
        assert_eq!(summary.frame_count, 1, "one complete committed record");
        assert_eq!(
            summary.bytes_scanned as usize,
            bytes.len(),
            "scan covers every written byte"
        );
    }

    fn frame_with_serialized_size(target: usize) -> (RawFrame, String) {
        let payload = vec![0_u8; MAX_PAYLOAD_BYTES];
        let mut frame = RawFrame::new(
            "source",
            "instrument",
            1,
            1,
            None,
            Some(1),
            "entitlement",
            &payload,
        )
        .unwrap();
        let base = frame.to_json_line().unwrap().len();
        assert!(target >= base, "target {target} below base {base}");
        let mut remaining = target - base;
        for field in [
            &mut frame.entitlement_ref,
            &mut frame.source,
            &mut frame.provider_instrument_id,
        ] {
            let pad = remaining.min(MAX_FIELD_BYTES - field.len());
            field.push_str(&"x".repeat(pad));
            remaining -= pad;
        }
        assert_eq!(remaining, 0, "padding capacity exhausted");
        let line = frame.to_json_line().unwrap();
        assert_eq!(line.len(), target, "padding must hit the exact target");
        (frame, line)
    }

    #[test]
    fn record_exactly_at_limit_is_accepted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exact.jsonl");
        let (frame, line) = frame_with_serialized_size(MAX_RAW_RECORD_BYTES);
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let scan = scan_journal(&path).unwrap();
        assert_eq!(scan.frames, vec![frame], "exact-size record must decode");
        assert_eq!(
            verify_journal(&path).unwrap().frame_count,
            1,
            "one record verifies"
        );
    }

    #[test]
    fn record_one_byte_over_limit_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("over.jsonl");
        let (_, mut line) = frame_with_serialized_size(MAX_RAW_RECORD_BYTES);
        line.insert(line.len() - 1, ' '); // valid JSON whitespace, one byte longer
        assert_eq!(
            line.len(),
            MAX_RAW_RECORD_BYTES + 1,
            "test line is one byte over"
        );
        std::fs::write(&path, format!("{line}\n")).unwrap();
        assert!(
            matches!(
                scan_journal(&path),
                Err(ArchiveError::RecordTooLarge { .. })
            ),
            "oversized record must fail closed"
        );
    }

    #[test]
    fn oversized_record_without_newline_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unterminated-over.jsonl");
        std::fs::write(&path, vec![b'x'; MAX_RAW_RECORD_BYTES + 1]).unwrap();
        assert!(
            matches!(
                verify_journal(&path),
                Err(ArchiveError::RecordTooLarge { .. })
            ),
            "oversized unterminated record must be rejected as oversized"
        );
    }

    #[test]
    fn multiple_records_at_limit_are_accepted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("many.jsonl");
        let (frame, line) = frame_with_serialized_size(MAX_RAW_RECORD_BYTES);
        std::fs::write(&path, format!("{line}\n{line}\n")).unwrap();
        let scan = scan_journal(&path).unwrap();
        assert_eq!(
            scan.frames,
            vec![frame.clone(), frame],
            "two exact-size records decode"
        );
        assert_eq!(
            verify_journal(&path).unwrap().frame_count,
            2,
            "two records verify"
        );
    }

    #[test]
    fn empty_journal_is_valid() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty.jsonl");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(
            verify_journal(&path).unwrap().frame_count,
            0,
            "empty journal has no frames"
        );
    }

    #[test]
    fn torn_tail_behavior_is_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("torn.jsonl");
        std::fs::write(&path, b"{\"partial\":").unwrap();
        assert!(
            matches!(verify_journal(&path), Err(ArchiveError::TornTail)),
            "unterminated record remains a torn tail"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"partial\":",
            "torn tail is not repaired"
        );
    }

    struct FailingReader {
        bytes: Vec<u8>,
        offset: usize,
        fail_at: usize,
    }

    impl std::io::Read for FailingReader {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if self.offset >= self.fail_at {
                return Err(std::io::Error::other("injected scan read failure"));
            }
            let remaining = (self.fail_at - self.offset).min(output.len());
            output[..remaining].copy_from_slice(&self.bytes[self.offset..self.offset + remaining]);
            self.offset += remaining;
            Ok(remaining)
        }
    }

    #[test]
    fn io_error_during_scan_is_propagated() {
        let line = frame(1).to_json_line().unwrap();
        let mut bytes = line.into_bytes();
        bytes.push(b'\n');
        bytes.extend_from_slice(b"partial");
        let reader = FailingReader {
            bytes,
            offset: 0,
            fail_at: 24,
        };
        let mut buffered = BufReader::new(reader);
        let result = visit_reader(&mut buffered, |_| Ok(()));
        assert!(
            matches!(result, Err(ArchiveError::Io(_))),
            "I/O failure must propagate"
        );
    }

    #[test]
    fn large_journal_verifies_without_collecting_frames() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.jsonl");
        let line = frame(1).to_json_line().unwrap();
        let mut contents = String::new();
        for _ in 0..2_000 {
            contents.push_str(&line);
            contents.push('\n');
        }
        std::fs::write(&path, &contents).unwrap();
        let summary = verify_journal(&path).unwrap();
        assert_eq!(
            summary.frame_count, 2_000,
            "all large-journal frames verified"
        );
        assert_eq!(
            summary.bytes_scanned as usize,
            contents.len(),
            "every byte scanned"
        );
        assert!(line.len() <= MAX_RAW_RECORD_BYTES, "record within bound");
    }

    #[test]
    fn oversized_text_field_is_rejected_at_construction() {
        let mut frame = RawFrame::new(
            "source",
            "instrument",
            1,
            1,
            None,
            Some(1),
            "entitlement",
            b"payload",
        )
        .unwrap();
        frame.source = "x".repeat(MAX_FIELD_BYTES + 1);
        assert!(
            matches!(frame.validate(), Err(ArchiveError::FieldTooLarge { .. })),
            "oversized metadata field is rejected"
        );
    }

    #[test]
    fn pre_existing_oversized_journal_fails_closed_on_verify_and_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy-oversized.jsonl");
        let (_, mut line) = frame_with_serialized_size(MAX_RAW_RECORD_BYTES);
        line.insert(line.len() - 1, ' ');
        std::fs::write(&path, format!("{line}\n")).unwrap();
        assert!(
            matches!(
                verify_journal(&path),
                Err(ArchiveError::RecordTooLarge { .. })
            ),
            "legacy oversized record must fail verification"
        );
        assert!(
            matches!(
                JournalWriter::open_append(&path),
                Err(ArchiveError::RecordTooLarge { .. })
            ),
            "legacy oversized journal must not be reopened for append"
        );
    }

    fn reader_from_lines(lines: &[String]) -> JournalReader<std::io::Cursor<Vec<u8>>> {
        let mut bytes = Vec::new();
        for line in lines {
            bytes.extend_from_slice(line.as_bytes());
            bytes.push(b'\n');
        }
        JournalReader::new(std::io::Cursor::new(bytes))
    }

    #[test]
    fn reader_streams_a_valid_journal_record_by_record() {
        let lines = vec![
            frame(1).to_json_line().unwrap(),
            frame(2).to_json_line().unwrap(),
            frame(3).to_json_line().unwrap(),
        ];
        let expected_bytes: usize = lines.iter().map(|line| line.len() + 1).sum();
        let mut reader = reader_from_lines(&lines);
        let mut seen = Vec::new();
        while let Some(frame) = reader.next_frame().unwrap() {
            seen.push(frame);
        }
        assert_eq!(
            seen,
            vec![frame(1), frame(2), frame(3)],
            "all frames stream in order"
        );
        assert_eq!(reader.frame_count(), 3, "frame count matches");
        assert_eq!(
            reader.bytes_scanned() as usize,
            expected_bytes,
            "every byte consumed"
        );
        assert_eq!(reader.next_frame().unwrap(), None, "EOF is sticky");
    }

    #[test]
    fn reader_returns_explicit_empty_result_for_empty_journal() {
        let mut reader = JournalReader::new(std::io::Cursor::new(Vec::new()));
        assert_eq!(
            reader.next_frame().unwrap(),
            None,
            "empty journal has no first frame"
        );
        assert_eq!(reader.frame_count(), 0, "empty journal has zero frames");
        assert_eq!(reader.bytes_scanned(), 0, "empty journal consumes no bytes");
    }

    #[test]
    fn reader_reports_torn_tail_for_json_without_final_newline() {
        let line = frame(7).to_json_line().unwrap();
        let mut reader = JournalReader::new(std::io::Cursor::new(line.into_bytes()));
        assert!(
            matches!(reader.next_frame(), Err(ArchiveError::TornTail)),
            "valid JSON without a newline is still a torn tail"
        );
    }

    #[test]
    fn reader_rejects_oversized_record() {
        let (_, mut line) = frame_with_serialized_size(MAX_RAW_RECORD_BYTES);
        line.insert(line.len() - 1, ' ');
        let mut reader = JournalReader::new(std::io::Cursor::new(format!("{line}\n").into_bytes()));
        assert!(
            matches!(
                reader.next_frame(),
                Err(ArchiveError::RecordTooLarge { .. })
            ),
            "oversized record fails closed"
        );
    }

    #[test]
    fn reader_rejects_empty_record() {
        let mut reader = JournalReader::new(std::io::Cursor::new(b"\n".to_vec()));
        assert!(
            matches!(reader.next_frame(), Err(ArchiveError::MalformedRecord(_))),
            "an empty record is rejected"
        );
    }

    #[test]
    fn reader_rejects_corrupt_payload_hash() {
        let line = frame(1).to_json_line().unwrap();
        let tampered = line.replace("cGF5bG9hZC0x", "cGF5bG9hZC0y");
        let mut reader =
            JournalReader::new(std::io::Cursor::new(format!("{tampered}\n").into_bytes()));
        assert!(
            matches!(reader.next_frame(), Err(ArchiveError::PayloadHashMismatch)),
            "corrupt payload hash is rejected"
        );
    }

    #[test]
    fn reader_propagates_read_error() {
        let line = frame(1).to_json_line().unwrap();
        let mut bytes = line.into_bytes();
        bytes.push(b'\n');
        let failing = FailingReader {
            bytes,
            offset: 0,
            fail_at: 4,
        };
        let mut reader = JournalReader::new(std::io::BufReader::new(failing));
        assert!(
            matches!(reader.next_frame(), Err(ArchiveError::Io(_))),
            "a read failure is propagated"
        );
    }

    #[test]
    fn reader_streams_large_journal_with_bounded_record_buffer() {
        let line = frame(1).to_json_line().unwrap();
        let mut bytes = Vec::new();
        for _ in 0..2_000 {
            bytes.extend_from_slice(line.as_bytes());
            bytes.push(b'\n');
        }
        let total = bytes.len();
        let mut reader = JournalReader::new(std::io::Cursor::new(bytes));
        let mut count = 0_u64;
        while reader.next_frame().unwrap().is_some() {
            count = count.saturating_add(1);
        }
        assert_eq!(count, 2_000, "all large-journal frames streamed");
        assert_eq!(
            reader.bytes_scanned() as usize,
            total,
            "every byte was scanned"
        );
        assert!(
            reader.record.capacity() <= MAX_RAW_RECORD_BYTES,
            "the reused record buffer stays bounded"
        );
    }
}
