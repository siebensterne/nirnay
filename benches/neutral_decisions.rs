// SPDX-License-Identifier: Apache-2.0
//! Neutral, std-only benchmark harness for in-process decision stages.
//!
//! The workload contains only opaque byte payloads and monotonically increasing
//! sequence values derived from an explicit seed. There is no market-shaped
//! data, no provider, broker, live-execution path, and no dependency outside
//! the standard library. The measured routines are neutral analogues of the
//! named stages (decision evaluation, risk reservation, OMS append/lifecycle,
//! checkpoint creation/recovery, replay, account reconciliation and their
//! end-to-end pipeline). Numbers are local observations, not gates,
//! certificates, or throughput/latency claims about production components.
//!
//! Usage:
//!   cargo bench -p trading-core --bench neutral_decisions --no-run
//!   cargo bench -p trading-core --bench neutral_decisions -- --self-test
//!   cargo bench -p trading-core --bench neutral_decisions -- --run
//!   cargo bench -p trading-core --bench neutral_decisions -- --run --out /tmp/result.json

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::{
    collections::BTreeMap,
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::sync_channel,
    },
    time::Instant,
};

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0_u8; 64],
            block_len: 0,
            total: 0,
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.block[self.block_len] = *byte;
            self.block_len += 1;
            self.total = self.total.wrapping_add(1);
            if self.block_len == self.block.len() {
                let block = self.block;
                self.compress(&block);
                self.block_len = 0;
            }
        }
    }

    fn finish(mut self) -> [u8; 32] {
        let bit_len = self.total.wrapping_mul(8);
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            let block = self.block;
            self.compress(&block);
            self.block_len = 0;
        }
        for shift in [56, 48, 40, 32, 24, 16, 8, 0] {
            self.block[self.block_len] = (bit_len >> shift) as u8;
            self.block_len += 1;
        }
        let block = self.block;
        self.compress(&block);
        let mut out = [0_u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            out[index * 4] = (word >> 24) as u8;
            out[index * 4 + 1] = (word >> 16) as u8;
            out[index * 4 + 2] = (word >> 8) as u8;
            out[index * 4 + 3] = *word as u8;
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut words: Vec<u32> = Vec::with_capacity(64);
        for chunk in block.as_chunks::<4>().0 {
            match be_u32(chunk) {
                Some(word) => words.push(word),
                None => words.push(0),
            }
        }
        let mut index = 16;
        while index < 64 {
            let left = words[index - 15];
            let right = words[index - 2];
            let small_zero = left.rotate_right(7) ^ left.rotate_right(18) ^ (left >> 3);
            let small_one = right.rotate_right(17) ^ right.rotate_right(19) ^ (right >> 10);
            let word = words[index - 16]
                .wrapping_add(small_zero)
                .wrapping_add(words[index - 7])
                .wrapping_add(small_one);
            words.push(word);
            index += 1;
        }
        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];
        for (index, word) in words.iter().enumerate() {
            let big_one = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temp_one = h
                .wrapping_add(big_one)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(*word);
            let big_zero = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp_two = big_zero.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp_one);
            d = c;
            c = b;
            b = a;
            a = temp_one.wrapping_add(temp_two);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

fn be_u32(bytes: &[u8]) -> Option<u32> {
    let slice = bytes.get(0..4)?;
    let array: [u8; 4] = slice.try_into().ok()?;
    Some(u32::from_be_bytes(array))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let slice = bytes.get(*cursor..*cursor + 8)?;
    *cursor += 8;
    let array: [u8; 8] = slice.try_into().ok()?;
    Some(u64::from_be_bytes(array))
}

fn push_u64(buffer: &mut Vec<u8>, value: u64) {
    buffer.extend_from_slice(&value.to_be_bytes());
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn digest_hex(text: &str) -> String {
    let mut sha = Sha256::new();
    sha.update(text.as_bytes());
    hex(&sha.finish())
}

fn escape_json(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if other.is_control() => {
                out.push_str("\\u");
                out.push_str(&format!("{:04x}", other as u32));
            }
            other => out.push(other),
        }
    }
    out
}

struct Json {
    fields: Vec<String>,
}

impl Json {
    fn new() -> Self {
        Self { fields: Vec::new() }
    }

    fn string(&mut self, key: &str, value: &str) {
        self.fields.push(format!(
            "\"{}\":\"{}\"",
            escape_json(key),
            escape_json(value)
        ));
    }

    fn uint(&mut self, key: &str, value: u64) {
        self.raw(key, &value.to_string());
    }

    fn uint128(&mut self, key: &str, value: u128) {
        self.raw(key, &value.to_string());
    }

    fn float(&mut self, key: &str, value: f64) {
        if value.is_finite() {
            self.raw(key, &format!("{value:.6}"));
        } else {
            self.raw(key, "null");
        }
    }

    fn boolean(&mut self, key: &str, value: bool) {
        self.raw(key, if value { "true" } else { "false" });
    }

    fn object(&mut self, key: &str, value: &Json) {
        self.raw(key, &value.render());
    }

    fn array(&mut self, key: &str, values: &[Json]) {
        let rendered: Vec<String> = values.iter().map(Json::render).collect();
        self.raw(key, &format!("[{}]", rendered.join(",")));
    }

    fn raw(&mut self, key: &str, value: &str) {
        self.fields
            .push(format!("\"{}\":{}", escape_json(key), value));
    }

    fn render(&self) -> String {
        format!("{{{}}}", self.fields.join(","))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Decision,
    RiskReservation,
    OmsLifecycle,
    CheckpointRecovery,
    Replay,
    Reconciliation,
    Pipeline,
}

impl Stage {
    fn all() -> [Stage; 7] {
        [
            Stage::Decision,
            Stage::RiskReservation,
            Stage::OmsLifecycle,
            Stage::CheckpointRecovery,
            Stage::Replay,
            Stage::Reconciliation,
            Stage::Pipeline,
        ]
    }

    fn as_str(self) -> &'static str {
        match self {
            Stage::Decision => "decision_evaluation",
            Stage::RiskReservation => "risk_reservation",
            Stage::OmsLifecycle => "oms_append_lifecycle",
            Stage::CheckpointRecovery => "checkpoint_creation_recovery",
            Stage::Replay => "replay",
            Stage::Reconciliation => "account_reconciliation",
            Stage::Pipeline => "end_to_end_pipeline",
        }
    }

    fn parse(value: &str) -> Option<Stage> {
        match value {
            "decision_evaluation" => Some(Stage::Decision),
            "risk_reservation" => Some(Stage::RiskReservation),
            "oms_append_lifecycle" => Some(Stage::OmsLifecycle),
            "checkpoint_creation_recovery" => Some(Stage::CheckpointRecovery),
            "replay" => Some(Stage::Replay),
            "account_reconciliation" => Some(Stage::Reconciliation),
            "end_to_end_pipeline" => Some(Stage::Pipeline),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoopMode {
    Open,
    Closed,
}

impl LoopMode {
    fn as_str(self) -> &'static str {
        match self {
            LoopMode::Open => "open",
            LoopMode::Closed => "closed",
        }
    }

    fn parse(value: &str) -> Option<LoopMode> {
        match value {
            "open" => Some(LoopMode::Open),
            "closed" => Some(LoopMode::Closed),
            _ => None,
        }
    }
}

struct Config {
    self_test: bool,
    run: bool,
    help: bool,
    custom: bool,
    stage: Stage,
    payload_bytes: usize,
    batch: usize,
    producers: usize,
    loop_mode: LoopMode,
    queue_capacity: usize,
    events: u64,
    warmup_events: u64,
    repetitions: usize,
    seed: u64,
    out: Option<PathBuf>,
}

impl Config {
    fn defaults() -> Self {
        Self {
            self_test: false,
            run: false,
            help: false,
            custom: false,
            stage: Stage::Pipeline,
            payload_bytes: 256,
            batch: 8,
            producers: 1,
            loop_mode: LoopMode::Closed,
            queue_capacity: 64,
            events: 2048,
            warmup_events: 256,
            repetitions: 3,
            seed: 0x4e69_726e_6179_0000,
            out: None,
        }
    }
}

#[derive(Clone)]
struct Event {
    sequence: u64,
    payload: Vec<u8>,
}

#[derive(Clone, Default)]
struct NeutralState {
    decisions: u64,
    decision_xor: u8,
    reservations: u64,
    reserved_peak: u64,
    appended: u64,
    terminal: u64,
    checkpoints: u64,
    checkpoint_bytes: u64,
    replayed: u64,
    reconciled: u64,
    mismatches: u64,
    errors: u64,
    records: Vec<(u64, u8)>,
    last_checkpoint: Vec<u8>,
    retained_bytes: u64,
}

impl NeutralState {
    fn accumulate(&mut self, other: &NeutralState) {
        self.decisions = self.decisions.wrapping_add(other.decisions);
        self.decision_xor ^= other.decision_xor;
        self.reservations = self.reservations.wrapping_add(other.reservations);
        self.reserved_peak = self.reserved_peak.max(other.reserved_peak);
        self.appended = self.appended.wrapping_add(other.appended);
        self.terminal = self.terminal.wrapping_add(other.terminal);
        self.checkpoints = self.checkpoints.wrapping_add(other.checkpoints);
        self.checkpoint_bytes = self.checkpoint_bytes.wrapping_add(other.checkpoint_bytes);
        self.replayed = self.replayed.wrapping_add(other.replayed);
        self.reconciled = self.reconciled.wrapping_add(other.reconciled);
        self.mismatches = self.mismatches.wrapping_add(other.mismatches);
        self.errors = self.errors.wrapping_add(other.errors);
        if !other.last_checkpoint.is_empty() {
            self.last_checkpoint = other.last_checkpoint.clone();
        }
        self.retained_bytes = self.retained_bytes.max(other.retained_bytes);
    }
}

#[derive(Clone)]
struct CaseConfig {
    name: String,
    stage: Stage,
    payload_bytes: usize,
    batch: usize,
    producers: usize,
    loop_mode: LoopMode,
    queue_capacity: usize,
}

struct CaseResult {
    case: CaseConfig,
    samples: Vec<u128>,
    state: NeutralState,
    events_processed: u64,
    bytes_processed: u64,
    elapsed_ns: u128,
}

fn score_event(event: &Event) -> u64 {
    let mut score = event.sequence ^ 0x9e37_79b9_7f4a_7c15;
    for byte in &event.payload {
        score ^= score.wrapping_shl(5).wrapping_add(score.wrapping_shr(59));
        score ^= u64::from(*byte).wrapping_mul(0xd1b5_4a32_d192_ed03);
    }
    score.rotate_right(17)
}

fn generate_events(seed: u64, count: u64, size: usize) -> Vec<Event> {
    let mut events = Vec::with_capacity(count as usize);
    for sequence in 0..count {
        let mut state = seed ^ sequence.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let mut payload = Vec::with_capacity(size);
        for index in 0..size {
            state ^= state.wrapping_shl(13);
            state ^= state.wrapping_shr(7);
            state ^= state.wrapping_shl(17);
            let mixed = state ^ (index as u64).wrapping_mul(0xd1b5_4a32_d192_ed03);
            payload.push(mixed as u8);
        }
        events.push(Event { sequence, payload });
    }
    events
}

fn prepare(stage: Stage, events: &[Event]) -> NeutralState {
    let mut state = NeutralState::default();
    if matches!(
        stage,
        Stage::CheckpointRecovery | Stage::Replay | Stage::Reconciliation
    ) {
        for event in events {
            let decision = (score_event(event) & 0xff) as u8;
            state.records.push((event.sequence, decision));
            state.decisions = state.decisions.wrapping_add(1);
            state.decision_xor ^= decision;
            state.appended = state.appended.wrapping_add(1);
        }
        state.retained_bytes = (state.records.capacity() * 16) as u64;
    }
    state
}

fn stage_decision(state: &mut NeutralState, events: &[Event]) {
    for event in events {
        let decision = (score_event(event) & 0xff) as u8;
        state.decisions = state.decisions.wrapping_add(1);
        state.decision_xor ^= decision;
    }
}

fn stage_risk(state: &mut NeutralState, events: &[Event]) {
    let limit = 1_000_000_000_u64;
    let mut reserved = 0_u64;
    for event in events {
        let cost = (score_event(event) % 1_000) + 1;
        if reserved.wrapping_add(cost) > limit {
            state.errors = state.errors.wrapping_add(1);
            continue;
        }
        reserved = reserved.wrapping_add(cost);
        state.reserved_peak = state.reserved_peak.max(reserved);
        state.reservations = state.reservations.wrapping_add(1);
        reserved = reserved.wrapping_sub(cost);
    }
}

fn stage_oms(state: &mut NeutralState, events: &[Event]) {
    for event in events {
        let decision = (score_event(event) & 0xff) as u8;
        state.records.push((event.sequence, decision));
        state.appended = state.appended.wrapping_add(1);
        state.terminal = state.terminal.wrapping_add(1);
    }
    state.retained_bytes = (state.records.capacity() * 16) as u64;
}

fn stage_checkpoint(state: &mut NeutralState, events: &[Event]) {
    let mut buffer = Vec::with_capacity(64);
    push_u64(&mut buffer, events.len() as u64);
    push_u64(&mut buffer, state.decisions);
    buffer.push(state.decision_xor);
    push_u64(&mut buffer, state.reservations);
    push_u64(&mut buffer, state.appended);
    push_u64(&mut buffer, state.records.len() as u64);
    let mut hasher = Sha256::new();
    hasher.update(&buffer);
    let checksum = hasher.finish();
    buffer.extend_from_slice(&checksum[..8]);

    let split = buffer.len().saturating_sub(8);
    let (body, tail) = buffer.split_at(split);
    let mut verifier = Sha256::new();
    verifier.update(body);
    let recovered_checksum = verifier.finish();
    if tail != &recovered_checksum[..8] {
        state.errors = state.errors.wrapping_add(1);
    }
    let mut cursor = 0_usize;
    let recovered_events = read_u64(body, &mut cursor);
    let recovered_decisions = read_u64(body, &mut cursor);
    let recovered_xor = body.get(cursor).copied();
    cursor += 1;
    let recovered_reservations = read_u64(body, &mut cursor);
    let recovered_appended = read_u64(body, &mut cursor);
    let recovered_records = read_u64(body, &mut cursor);
    if recovered_events != Some(events.len() as u64)
        || recovered_decisions != Some(state.decisions)
        || recovered_xor != Some(state.decision_xor)
        || recovered_reservations != Some(state.reservations)
        || recovered_appended != Some(state.appended)
        || recovered_records != Some(state.records.len() as u64)
    {
        state.errors = state.errors.wrapping_add(1);
    }
    state.checkpoints = state.checkpoints.wrapping_add(1);
    state.checkpoint_bytes = state.checkpoint_bytes.wrapping_add(buffer.len() as u64);
    state.last_checkpoint = buffer;
    state.retained_bytes = state
        .retained_bytes
        .wrapping_add(state.last_checkpoint.capacity() as u64);
}

fn stage_replay(state: &mut NeutralState, events: &[Event]) {
    let mut by_sequence = BTreeMap::new();
    for (sequence, decision) in &state.records {
        by_sequence.insert(*sequence, *decision);
    }
    for event in events {
        let expected = (score_event(event) & 0xff) as u8;
        match by_sequence.get(&event.sequence) {
            Some(decision) if *decision == expected => {
                state.replayed = state.replayed.wrapping_add(1);
            }
            _ => state.errors = state.errors.wrapping_add(1),
        }
    }
}

fn stage_reconciliation(state: &mut NeutralState, events: &[Event]) {
    let mut ledger = BTreeMap::new();
    let mut ledger_xor = 0_u8;
    for (sequence, decision) in &state.records {
        ledger.insert(*sequence, *decision);
        ledger_xor ^= *decision;
    }
    let mut reconciled = 0_u64;
    for event in events {
        let expected = (score_event(event) & 0xff) as u8;
        match ledger.get(&event.sequence) {
            Some(decision) if *decision == expected => {
                reconciled = reconciled.wrapping_add(1);
            }
            _ => state.errors = state.errors.wrapping_add(1),
        }
    }
    state.reconciled = state.reconciled.wrapping_add(reconciled);
    if ledger_xor != state.decision_xor {
        state.mismatches = state.mismatches.wrapping_add(1);
    }
}

fn apply_stage(stage: Stage, state: &mut NeutralState, events: &[Event]) {
    match stage {
        Stage::Decision => stage_decision(state, events),
        Stage::RiskReservation => stage_risk(state, events),
        Stage::OmsLifecycle => stage_oms(state, events),
        Stage::CheckpointRecovery => stage_checkpoint(state, events),
        Stage::Replay => stage_replay(state, events),
        Stage::Reconciliation => stage_reconciliation(state, events),
        Stage::Pipeline => {
            stage_decision(state, events);
            stage_risk(state, events);
            stage_oms(state, events);
            stage_checkpoint(state, events);
            stage_replay(state, events);
            stage_reconciliation(state, events);
        }
    }
}

struct RunOutput {
    state: NeutralState,
    samples: Vec<u128>,
    events_processed: u64,
    bytes_processed: u64,
    elapsed_ns: u128,
}

fn run_single(case: &CaseConfig, events: &[Event], collect: bool) -> RunOutput {
    let mut state = prepare(case.stage, events);
    let mut samples = Vec::new();
    let mut processed = 0_u64;
    let mut bytes = 0_u64;
    let started = Instant::now();
    for chunk in events.chunks(case.batch) {
        let item_started = Instant::now();
        apply_stage(case.stage, &mut state, chunk);
        let elapsed = item_started.elapsed().as_nanos();
        if collect {
            samples.push(elapsed);
        }
        processed = processed.wrapping_add(chunk.len() as u64);
        for event in chunk {
            bytes = bytes.wrapping_add(event.payload.len() as u64);
        }
    }
    RunOutput {
        state,
        samples,
        events_processed: processed,
        bytes_processed: bytes,
        elapsed_ns: started.elapsed().as_nanos(),
    }
}

fn run_parallel(case: &CaseConfig, events: &[Event], collect: bool) -> RunOutput {
    let batch_count = events.len().div_ceil(case.batch);
    let shared = Arc::new(events.to_vec());
    let next_batch = Arc::new(AtomicU64::new(0));
    let (sender, receiver) = sync_channel::<Vec<Event>>(case.queue_capacity);
    let (ack_sender, ack_receiver) = sync_channel::<()>(case.producers.max(1));
    let ack = Arc::new(Mutex::new(ack_receiver));
    let closed = case.loop_mode == LoopMode::Closed;
    let mut handles = Vec::with_capacity(case.producers);
    for _ in 0..case.producers {
        let shared = Arc::clone(&shared);
        let next_batch = Arc::clone(&next_batch);
        let ack = Arc::clone(&ack);
        let sender = sender.clone();
        let batch = case.batch;
        handles.push(std::thread::spawn(move || {
            let mut sent = 0_usize;
            loop {
                let index = next_batch.fetch_add(1, Ordering::Relaxed) as usize;
                if index >= batch_count {
                    break;
                }
                let start = index * batch;
                let end = (start + batch).min(shared.len());
                let chunk = shared[start..end].to_vec();
                if sender.send(chunk).is_err() {
                    break;
                }
                sent += 1;
                if closed {
                    let guard = match ack.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    if guard.recv().is_err() {
                        break;
                    }
                }
            }
            sent
        }));
    }
    drop(sender);
    let mut state = prepare(case.stage, events);
    let mut samples = Vec::new();
    let mut processed = 0_u64;
    let mut bytes = 0_u64;
    let started = Instant::now();
    for chunk in receiver {
        let item_started = Instant::now();
        apply_stage(case.stage, &mut state, &chunk);
        let elapsed = item_started.elapsed().as_nanos();
        if collect {
            samples.push(elapsed);
        }
        processed = processed.wrapping_add(chunk.len() as u64);
        for event in &chunk {
            bytes = bytes.wrapping_add(event.payload.len() as u64);
        }
        if closed {
            let _delivered = ack_sender.send(());
        }
    }
    for handle in handles {
        let _joined = handle.join();
    }
    RunOutput {
        state,
        samples,
        events_processed: processed,
        bytes_processed: bytes,
        elapsed_ns: started.elapsed().as_nanos(),
    }
}

fn run_case(case: &CaseConfig, events: &[Event], collect: bool) -> RunOutput {
    if case.producers == 1 {
        run_single(case, events, collect)
    } else {
        run_parallel(case, events, collect)
    }
}

fn build_cases(config: &Config) -> Vec<CaseConfig> {
    if config.self_test {
        return vec![CaseConfig {
            name: "self-test-pipeline".to_owned(),
            stage: Stage::Pipeline,
            payload_bytes: 64,
            batch: 4,
            producers: 1,
            loop_mode: LoopMode::Closed,
            queue_capacity: 16,
        }];
    }
    if config.custom {
        return vec![CaseConfig {
            name: "custom".to_owned(),
            stage: config.stage,
            payload_bytes: config.payload_bytes,
            batch: config.batch,
            producers: config.producers,
            loop_mode: config.loop_mode,
            queue_capacity: config.queue_capacity,
        }];
    }
    let mut cases = Vec::new();
    for stage in Stage::all() {
        cases.push(CaseConfig {
            name: format!("baseline-{}", stage.as_str()),
            stage,
            payload_bytes: 256,
            batch: 8,
            producers: 1,
            loop_mode: LoopMode::Closed,
            queue_capacity: 64,
        });
    }
    for payload_bytes in [64, 256, 1024, 4096] {
        cases.push(CaseConfig {
            name: format!("payload-{payload_bytes}"),
            stage: Stage::Pipeline,
            payload_bytes,
            batch: 8,
            producers: 1,
            loop_mode: LoopMode::Closed,
            queue_capacity: 64,
        });
    }
    for batch in [1, 8, 32, 128] {
        cases.push(CaseConfig {
            name: format!("batch-{batch}"),
            stage: Stage::Pipeline,
            payload_bytes: 256,
            batch,
            producers: 1,
            loop_mode: LoopMode::Closed,
            queue_capacity: 64,
        });
    }
    for producers in [1, 2, 4, 8] {
        cases.push(CaseConfig {
            name: format!("producers-{producers}"),
            stage: Stage::Pipeline,
            payload_bytes: 256,
            batch: 8,
            producers,
            loop_mode: LoopMode::Closed,
            queue_capacity: 64,
        });
    }
    for mode in [LoopMode::Open, LoopMode::Closed] {
        cases.push(CaseConfig {
            name: format!("loop-{}", mode.as_str()),
            stage: Stage::Pipeline,
            payload_bytes: 256,
            batch: 8,
            producers: 4,
            loop_mode: mode,
            queue_capacity: 64,
        });
    }
    for queue_capacity in [16, 64, 256] {
        cases.push(CaseConfig {
            name: format!("queue-{queue_capacity}"),
            stage: Stage::Pipeline,
            payload_bytes: 256,
            batch: 8,
            producers: 4,
            loop_mode: LoopMode::Open,
            queue_capacity,
        });
    }
    cases
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn workload_canonical(config: &Config, cases: &[CaseConfig]) -> String {
    let mut text = format!(
        "seed={:016x}|events={}|warmup={}|repetitions={}|mode={}\n",
        config.seed,
        config.events,
        config.warmup_events,
        config.repetitions,
        if config.self_test {
            "self-test"
        } else {
            "full"
        }
    );
    for case in cases {
        text.push_str(&format!(
            "{}|{}|{}|{}|{}|{}|{}\n",
            case.name,
            case.stage.as_str(),
            case.payload_bytes,
            case.batch,
            case.producers,
            case.loop_mode.as_str(),
            case.queue_capacity
        ));
    }
    text
}

fn state_canonical(case: &CaseConfig, state: &NeutralState, events: u64, bytes: u64) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}\n",
        case.name,
        case.stage.as_str(),
        state.decisions,
        state.decision_xor,
        state.reservations,
        state.reserved_peak,
        state.appended,
        state.terminal,
        state.checkpoints,
        state.checkpoint_bytes,
        state.replayed,
        state.reconciled,
        state.mismatches,
        state.errors,
        events,
        bytes
    )
}

fn command_text(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
        _ => "unavailable".to_owned(),
    }
}

fn cpu_text() -> String {
    let brand = command_text("sysctl", &["-n", "machdep.cpu.brand_string"]);
    if brand != "unavailable" {
        return brand;
    }
    match std::fs::read_to_string("/proc/cpuinfo") {
        Ok(text) => text
            .lines()
            .find(|line| line.starts_with("model name"))
            .unwrap_or("unavailable")
            .to_owned(),
        Err(_) => "unavailable".to_owned(),
    }
}

fn compiler_flags() -> String {
    let rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let encoded = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    let debug = cfg!(debug_assertions);
    format!(
        "profile={} debug_assertions={debug} RUSTFLAGS={rustflags} CARGO_ENCODED_RUSTFLAGS={encoded}",
        if debug { "debug" } else { "optimized" }
    )
}

fn percentile_json(samples: &[u128]) -> Json {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let mut json = Json::new();
    for (label, fraction) in [
        ("p50", 0.50),
        ("p90", 0.90),
        ("p95", 0.95),
        ("p99", 0.99),
        ("p999", 0.999),
    ] {
        let value = percentile(&sorted, fraction);
        json.uint128(label, value);
        let beyond = sorted.iter().filter(|sample| **sample > value).count();
        json.uint(&format!("count_beyond_{label}"), beyond as u64);
    }
    json.uint128("max", sorted.last().copied().unwrap_or(0));
    json.uint("sample_count", sorted.len() as u64);
    json
}

fn case_json(result: &CaseResult) -> Json {
    let case = &result.case;
    let elapsed_seconds = result.elapsed_ns as f64 / 1_000_000_000.0;
    let throughput = if elapsed_seconds > 0.0 {
        result.events_processed as f64 / elapsed_seconds
    } else {
        0.0
    };
    let bytes_per_second = if elapsed_seconds > 0.0 {
        result.bytes_processed as f64 / elapsed_seconds
    } else {
        0.0
    };
    let mut json = Json::new();
    json.string("name", &case.name);
    json.string("stage", case.stage.as_str());
    json.uint("payload_bytes", case.payload_bytes as u64);
    json.uint("batch", case.batch as u64);
    json.uint("producers", case.producers as u64);
    json.string("loop_mode", case.loop_mode.as_str());
    json.uint("queue_capacity", case.queue_capacity as u64);
    json.uint("samples", result.samples.len() as u64);
    json.uint128("events_processed", result.events_processed as u128);
    json.uint128("bytes_processed", result.bytes_processed as u128);
    json.uint128("elapsed_ns", result.elapsed_ns);
    json.float("throughput_events_per_sec", throughput);
    json.float("throughput_bytes_per_sec", bytes_per_second);
    json.object("latency_ns", &percentile_json(&result.samples));
    json.uint("error_count", result.state.errors);
    json.uint("mismatch_count", result.state.mismatches);

    let sample_bytes = (result.samples.capacity() * 16) as u64;
    let observed = result.state.retained_bytes
        + result.state.checkpoint_bytes
        + result.bytes_processed
        + sample_bytes;
    let mut memory = Json::new();
    memory.uint("checkpoint_bytes", result.state.checkpoint_bytes);
    memory.uint("retained_record_bytes", result.state.retained_bytes);
    memory.uint("sample_buffer_bytes", sample_bytes);
    memory.uint("payload_bytes_total", result.bytes_processed);
    memory.uint("observed_allocation_bytes", observed);
    memory.boolean("allocator_stats_available", false);
    json.object("memory_allocation", &memory);

    let mut state = Json::new();
    state.uint("decisions", result.state.decisions);
    state.uint("decision_xor", u64::from(result.state.decision_xor));
    state.uint("reservations", result.state.reservations);
    state.uint("reserved_peak", result.state.reserved_peak);
    state.uint("appended", result.state.appended);
    state.uint("terminal", result.state.terminal);
    state.uint("checkpoints", result.state.checkpoints);
    state.uint("replayed", result.state.replayed);
    state.uint("reconciled", result.state.reconciled);
    json.object("stage_counters", &state);
    json
}

fn parse_usize(value: &str, flag: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid value for {flag}: {value}"))
}

fn parse_u64(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("invalid value for {flag}: {value}"))
}

fn parse_seed(value: &str) -> Result<u64, String> {
    let (radix, digits) = match value.strip_prefix("0x") {
        Some(rest) => (16_u32, rest),
        None => (10_u32, value),
    };
    u64::from_str_radix(digits, radix).map_err(|_| format!("invalid seed: {value}"))
}

fn parse_args() -> Result<Config, String> {
    let mut config = Config::defaults();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut position = 0_usize;
    while position < args.len() {
        let arg = args[position].clone();
        position += 1;
        if arg == "--self-test" {
            config.self_test = true;
            continue;
        }
        if arg == "--run" {
            config.run = true;
            continue;
        }
        if arg == "--help" {
            config.help = true;
            continue;
        }
        if matches!(
            arg.as_str(),
            "--test" | "--nocapture" | "--exact" | "--list"
        ) {
            continue;
        }
        if matches!(arg.as_str(), "--bench") {
            if position < args.len() && !args[position].starts_with('-') {
                position += 1;
            }
            continue;
        }
        let (key, inline) = match arg.split_once('=') {
            Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
            None => (arg, None),
        };
        let value = match inline {
            Some(value) => value,
            None => {
                let next = args
                    .get(position)
                    .ok_or_else(|| format!("missing value for {key}"))?;
                position += 1;
                next.clone()
            }
        };
        match key.as_str() {
            "--stage" => {
                config.stage =
                    Stage::parse(&value).ok_or_else(|| format!("unknown stage: {value}"))?;
                config.custom = true;
            }
            "--payload-bytes" => {
                config.payload_bytes = parse_usize(&value, &key)?;
                config.custom = true;
            }
            "--batch" => {
                config.batch = parse_usize(&value, &key)?;
                config.custom = true;
            }
            "--producers" => {
                config.producers = parse_usize(&value, &key)?;
                config.custom = true;
            }
            "--loop" => {
                config.loop_mode =
                    LoopMode::parse(&value).ok_or_else(|| format!("unknown loop mode: {value}"))?;
                config.custom = true;
            }
            "--queue-capacity" => {
                config.queue_capacity = parse_usize(&value, &key)?;
                config.custom = true;
            }
            "--events" => config.events = parse_u64(&value, &key)?,
            "--warmup-events" => config.warmup_events = parse_u64(&value, &key)?,
            "--repetitions" => config.repetitions = parse_usize(&value, &key)?,
            "--seed" => config.seed = parse_seed(&value)?,
            "--out" => config.out = Some(PathBuf::from(value)),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if config.self_test && config.run {
        return Err("--self-test and --run are mutually exclusive".to_owned());
    }
    // Cargo invokes harness=false bench binaries with --bench under
    // `cargo test --all-targets`. Any invocation without an explicit --run stays
    // in the tiny deterministic self-test mode; the full matrix is opt-in.
    if !config.run {
        config.self_test = true;
    }
    if config.self_test {
        config.events = 64;
        config.warmup_events = 8;
        config.repetitions = 1;
        config.seed = 0x4e69_726e_6179_0000;
    }
    if config.batch == 0 {
        return Err("--batch must be at least 1".to_owned());
    }
    if config.producers == 0 {
        return Err("--producers must be at least 1".to_owned());
    }
    if config.queue_capacity == 0 {
        return Err("--queue-capacity must be at least 1".to_owned());
    }
    if config.repetitions == 0 {
        return Err("--repetitions must be at least 1".to_owned());
    }
    if config.payload_bytes == 0 {
        return Err("--payload-bytes must be at least 1".to_owned());
    }
    Ok(config)
}

fn print_help() {
    eprintln!("neutral decision benchmark harness (std only, opaque payloads)");
    eprintln!("  --self-test            tiny deterministic mode (default)");
    eprintln!("  --run                  full neutral workload matrix");
    eprintln!("  --stage NAME           one stage or end_to_end_pipeline");
    eprintln!("  --payload-bytes N      64 | 256 | 1024 | 4096");
    eprintln!("  --batch N              1 | 8 | 32 | 128");
    eprintln!("  --producers N          1 | 2 | 4 | 8");
    eprintln!("  --loop MODE            open | closed");
    eprintln!("  --queue-capacity N     bounded queue capacity");
    eprintln!("  --events N             measured events per case");
    eprintln!("  --warmup-events N      unmeasured warm-up events");
    eprintln!("  --repetitions N        measured repetitions");
    eprintln!("  --seed N               decimal or 0x-prefixed seed");
    eprintln!("  --out PATH             JSON artifact destination");
}

fn filesystem_text(path: &Path) -> String {
    let target = path.parent().unwrap_or_else(|| Path::new("."));
    let rendered = target.to_string_lossy();
    command_text("df", &["-Pk", rendered.as_ref()])
}

fn run_main() -> Result<(), String> {
    let config = parse_args()?;
    if config.help {
        print_help();
        return Ok(());
    }
    let cases = build_cases(&config);
    let workload_digest = digest_hex(&workload_canonical(&config, &cases));
    let mut results = Vec::with_capacity(cases.len());
    let mut raw_canonical = String::new();
    let mut total_duration_ns = 0_u128;

    for case in &cases {
        let mut combined = CaseResult {
            case: case.clone(),
            samples: Vec::new(),
            state: NeutralState::default(),
            events_processed: 0,
            bytes_processed: 0,
            elapsed_ns: 0,
        };
        for _repetition in 0..config.repetitions {
            let warmup = generate_events(config.seed, config.warmup_events, case.payload_bytes);
            let _warmed = run_single(case, &warmup, false);
            let events = generate_events(config.seed, config.events, case.payload_bytes);
            let output = run_case(case, &events, true);
            combined.state.accumulate(&output.state);
            combined.samples.extend(output.samples.iter().copied());
            combined.events_processed = combined
                .events_processed
                .wrapping_add(output.events_processed);
            combined.bytes_processed = combined
                .bytes_processed
                .wrapping_add(output.bytes_processed);
            combined.elapsed_ns = combined.elapsed_ns.wrapping_add(output.elapsed_ns);
            raw_canonical.push_str(&state_canonical(
                case,
                &output.state,
                output.events_processed,
                output.bytes_processed,
            ));
        }
        total_duration_ns = total_duration_ns.wrapping_add(combined.elapsed_ns);
        results.push(combined);
    }

    let raw_digest = digest_hex(&raw_canonical);
    if config.self_test {
        let result = &results[0];
        assert_eq!(
            result.state.decisions, config.events,
            "self-test decision count"
        );
        assert_eq!(result.state.errors, 0, "self-test error count");
        assert_eq!(result.state.mismatches, 0, "self-test mismatch count");
        assert_eq!(
            result.state.appended, config.events,
            "self-test append count"
        );
        assert_eq!(
            result.state.replayed, config.events,
            "self-test replay count"
        );
        assert_eq!(
            result.state.reconciled, config.events,
            "self-test reconcile count"
        );
        assert!(
            !result.samples.is_empty(),
            "self-test latency samples exist"
        );
        assert!(
            result.state.checkpoint_bytes > 0,
            "self-test checkpoint bytes"
        );
        assert_eq!(raw_digest.len(), 64, "self-test raw digest width");
    }

    let out = config.out.clone().unwrap_or_else(|| {
        std::env::temp_dir()
            .join("nirnay-neutral-bench")
            .join(if config.self_test {
                "self-test.json"
            } else {
                "result.json"
            })
    });
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let mut artifact = Json::new();
    artifact.uint("schema_version", 1);
    artifact.string("harness", "neutral_decisions");
    artifact.string(
        "mode",
        if config.self_test {
            "self-test"
        } else {
            "full"
        },
    );
    artifact.string("source_sha", &command_text("git", &["rev-parse", "HEAD"]));
    artifact.string(
        "toolchain",
        &format!(
            "{} | {}",
            command_text("rustc", &["--version"]),
            command_text("cargo", &["--version"])
        ),
    );
    artifact.string("compiler_flags", &compiler_flags());
    artifact.string("os", std::env::consts::OS);
    artifact.string("arch", std::env::consts::ARCH);
    artifact.string("cpu", &cpu_text());
    artifact.string("machine", &command_text("uname", &["-a"]));
    artifact.string("filesystem", &filesystem_text(&out));
    artifact.string("artifact_path", &out.to_string_lossy());
    artifact.string("workload_digest_sha256", &workload_digest);
    artifact.string("raw_result_digest_sha256", &raw_digest);
    artifact.uint128("seed", config.seed as u128);
    artifact.uint("event_count", config.events);
    artifact.uint("warmup_events", config.warmup_events);
    artifact.uint128("duration_ns", total_duration_ns);
    artifact.uint("repetitions", config.repetitions as u64);
    artifact.uint("stage_count", cases.len() as u64);
    artifact.array(
        "payload_bytes_dimensions",
        &[64_u64, 256, 1024, 4096].map(|value| {
            let mut item = Json::new();
            item.uint("payload_bytes", value);
            item
        }),
    );
    artifact.array(
        "batch_dimensions",
        &[1_u64, 8, 32, 128].map(|value| {
            let mut item = Json::new();
            item.uint("batch", value);
            item
        }),
    );
    artifact.array(
        "producer_dimensions",
        &[1_u64, 2, 4, 8].map(|value| {
            let mut item = Json::new();
            item.uint("producers", value);
            item
        }),
    );
    artifact.array(
        "loop_dimensions",
        &["open", "closed"].map(|value| {
            let mut item = Json::new();
            item.string("loop_mode", value);
            item
        }),
    );
    artifact.array(
        "queue_capacity_dimensions",
        &[16_u64, 64, 256].map(|value| {
            let mut item = Json::new();
            item.uint("queue_capacity", value);
            item
        }),
    );
    artifact.array(
        "measurements",
        &results.iter().map(case_json).collect::<Vec<Json>>(),
    );
    artifact.string(
        "scope",
        "Neutral in-process stage analogues over opaque bytes and monotonic sequence values; \
         no market-shaped data, production path, throughput gate or latency certificate.",
    );
    let mut text = artifact.render();
    text.push('\n');
    std::fs::write(&out, text.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    eprintln!(
        "neutral-bench: mode={} cases={} events={} repetitions={} raw_digest={} artifact={}",
        if config.self_test {
            "self-test"
        } else {
            "full"
        },
        cases.len(),
        config.events,
        config.repetitions,
        raw_digest,
        out.display()
    );
    Ok(())
}

fn main() {
    match run_main() {
        Ok(()) => {}
        Err(message) => {
            eprintln!("neutral-bench: {message}");
            std::process::exit(2);
        }
    }
}
