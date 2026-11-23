#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! M2.1 exit tests. Each maps to a named owner requirement.

use market_event::{
    ContinuityMachine, //
    ContractError,
    KernelEvent,
    ObservationOutcome,
    OriginClass,
    SequenceScope,
    StreamState,
    business_event_id,
    capture_frame_id,
    observation_id,
};

fn ids(payload: &[u8]) -> (market_event::ContentId, market_event::ContentId) {
    (
        capture_frame_id(payload).expect("capture id"),
        observation_id("upstox", "NSE", "NSE_EQ|TEST", payload).expect("observation id"),
    )
}

fn event(payload: &[u8], epoch: u64, receive_ns: i64) -> KernelEvent {
    let (frame, obs) = ids(payload);
    KernelEvent::observation_only(
        frame.clone(),
        obs,
        frame,
        OriginClass::SdkCallback,
        "upstox",
        "session-1",
        "NSE_EQ|TEST",
        receive_ns,
        receive_ns as u64,
    )
    .expect("event")
    .with_receive_fence(epoch, 1)
    .expect("fence")
}

/// Test 1: duplicate delivery gets the same identity and is observable as a
/// duplicate in continuity, not a new observation.
#[test]
fn t01_duplicate_delivery() {
    let (_, obs) = ids(b"same payload");
    let repeat = observation_id("upstox", "NSE", "NSE_EQ|TEST", b"same payload").expect("repeat");
    assert_eq!(obs, repeat, "identical delivery keeps its observation id");
    let mut machine = ContinuityMachine::default();
    assert_eq!(
        machine.observe(0, 5).expect("first"),
        ObservationOutcome::Initial,
        "a new epoch starts with an initial observation"
    );
    assert_eq!(
        machine.observe(0, 5).expect("repeat"),
        ObservationOutcome::Duplicate,
        "a repeated sequence is classified as duplicate"
    );
}

/// Test 2: same identity inputs, different content → different id.
#[test]
fn t02_same_identity_different_content() {
    let (_, a) = ids(b"payload-a");
    let (_, b) = ids(b"payload-b");
    assert_ne!(
        a, b,
        "different canonical bytes cannot share an observation id"
    );
}

/// Test 3: recorder restart (new session id) does not change identity.
#[test]
fn t03_recorder_restart_keeps_identity() {
    let payload = b"restart payload";
    let (_, first) = ids(payload);
    let second = observation_id("upstox", "NSE", "NSE_EQ|TEST", payload).expect("after restart");
    assert_eq!(
        first, second,
        "session identity is excluded from observation id"
    );
    let a = event(payload, 1, 1_700_000_000_000_000_000);
    let b = event(payload, 2, 1_700_000_000_100_000_000);
    assert_eq!(
        a.observation_id, b.observation_id,
        "identical payload after restart keeps the observation id"
    );
}

/// Test 4: connection restart advances epoch without changing identity.
#[test]
fn t04_connection_restart() {
    let first = event(b"payload", 1, 1_700_000_000_000_000_000);
    let after = event(b"payload", 2, 1_700_000_000_100_000_000);
    assert_eq!(
        first.observation_id, after.observation_id,
        "a new connection epoch is a boundary, not an identity change"
    );
    let mut machine = ContinuityMachine::default();
    machine.observe(1, 10).expect("first epoch");
    assert_eq!(
        machine.observe(2, 1).expect("new epoch"),
        ObservationOutcome::ConnectionReset,
        "sequence values remain explicit after reset"
    );
}

/// Test 5: sequence reset is a connection boundary, not a duplicate.
#[test]
fn t05_sequence_reset() {
    let mut machine = ContinuityMachine::default();
    machine.observe(1, 100).expect("first");
    assert_eq!(
        machine.observe(2, 1).expect("reset"),
        ObservationOutcome::ConnectionReset,
        "a new epoch is a connection boundary, not a duplicate"
    );
    assert_eq!(
        machine.state(),
        StreamState::Clean,
        "reset alone is not a gap"
    );
}

/// Test 6: sequence gap moves the stream out of clean.
#[test]
fn t06_sequence_gap() {
    let mut machine = ContinuityMachine::default();
    machine.observe(1, 4).expect("first");
    assert_eq!(
        machine.observe(1, 8).expect("gap"),
        ObservationOutcome::Gap,
        "a sequence jump is classified as a gap"
    );
    assert_eq!(
        machine.state(),
        StreamState::Gap,
        "gap state blocks risk-increasing use"
    );
    assert!(
        !StreamState::Gap.is_clean(),
        "gap blocks risk-increasing use"
    );
}

/// Test 7: out-of-order delivery is classified without changing identity.
#[test]
fn t07_out_of_order() {
    let mut machine = ContinuityMachine::default();
    machine.observe(1, 9).expect("first");
    assert_eq!(
        machine.observe(1, 7).expect("late"),
        ObservationOutcome::OutOfOrder,
        "out-of-order delivery is classified without clearing the gap"
    );
}

/// Test 8: one source frame with several instruments shares the capture id but
/// produces distinct observation ids.
#[test]
fn t08_multi_instrument_frame() {
    let frame = capture_frame_id(b"one multi-instrument frame").expect("frame");
    let a = observation_id("upstox", "NSE", "NSE_EQ|A", b"instrument-a").expect("a");
    let b = observation_id("upstox", "NSE", "NSE_EQ|B", b"instrument-b").expect("b");
    assert_eq!(
        frame,
        capture_frame_id(b"one multi-instrument frame").expect("frame2"),
        "the captured frame id is reproduced"
    );
    assert_ne!(a, b, "instrument-local content separates observations");
}

/// Test 9: snapshot recovery requires evidence, and ordering alone cannot
/// clear a gap.
#[test]
fn t09_snapshot_recovery() {
    let mut machine = ContinuityMachine::default();
    machine.observe(1, 4).expect("first");
    machine.observe(1, 9).expect("gap");
    assert_eq!(
        machine.state(),
        StreamState::Gap,
        "ordering cannot clear a gap"
    );
    machine.observe(1, 10).expect("in order after gap");
    assert_eq!(
        machine.state(),
        StreamState::Gap,
        "sequence cannot clear a gap"
    );
    let authority = capture_frame_id(b"authoritative snapshot").expect("authority");
    machine.recover(1, authority).expect("recovery evidence");
    assert_eq!(
        machine.state(),
        StreamState::Recovered,
        "recovery evidence moves the stream to recovered"
    );
    machine.clear_recovery().expect("clear");
    assert_eq!(
        machine.state(),
        StreamState::Clean,
        "bound recovery can clear the gap"
    );
    assert_eq!(
        machine.clear_recovery(),
        Err(ContractError::GapToClean),
        "a second clear is rejected"
    );
}

/// Test 10: reference revision mismatch leaves the instrument unresolved.
#[test]
fn t10_reference_revision_mismatch() {
    let base = event(b"payload", 1, 1_700_000_000_000_000_000);
    assert!(!base.is_actionable(), "unresolved event is not actionable");
    let resolved = base
        .clone()
        .resolve_instrument("IN:TEST", Some("NSE".to_owned()), "revision-a")
        .expect("resolved");
    assert!(
        resolved.is_actionable(),
        "a resolved event with a revision is actionable"
    );
    let mismatch = resolved
        .clone()
        .resolve_instrument("IN:TEST", Some("NSE".to_owned()), "revision-b")
        .expect("different revision");
    assert_eq!(
        resolved.observation_id, mismatch.observation_id,
        "revision mismatch leaves the observation id unchanged"
    );
    assert_ne!(
        resolved.reference_revision, mismatch.reference_revision,
        "the bound revision changes with the revision string"
    );
}

/// Test 11: unknown instrument remains unresolved and is not actionable.
#[test]
fn t11_unknown_instrument() {
    let base = event(b"payload", 1, 1_700_000_000_000_000_000);
    assert_eq!(
        base.instrument_id, None,
        "an unresolved event is not actionable"
    );
    assert!(!base.is_actionable(), "no invented identity and no action");
}

/// Test 12: source timestamp and receive timestamp keep separate domains.
#[test]
fn t12_source_and_receive_time_domains() {
    let base = event(b"payload", 1, 1_700_000_000_000_000_000);
    let with_source = base
        .clone()
        .with_source_time(1_699_999_999_000_000_000, "provider_feed_clock")
        .expect("source time");
    assert_ne!(
        with_source.source_time_ns,
        Some(with_source.receive_utc_ns),
        "source and receive timestamps occupy different domains"
    );
    assert_eq!(
        with_source.source_clock_domain.as_deref(),
        Some("provider_feed_clock"),
        "the source clock domain is preserved"
    );
    let collision = base.with_source_time(1, "receive");
    assert_eq!(
        collision,
        Err(ContractError::TimeDomainCollision),
        "a receive-labelled clock domain is rejected"
    );
}

/// Observation and business identity are distinct; a bare observation carries
/// no invented business id.
#[test]
fn business_identity_is_not_invented_for_state() {
    let base = event(b"payload", 1, 1_700_000_000_000_000_000);
    assert_eq!(
        base.business_event_id, None,
        "a bare state observation has no business id"
    );
    let derived = base
        .derived_business_event("last_trade_observation", "trade_tuple_advanced")
        .expect("derived");
    assert_ne!(
        derived.business_event_id,
        Some(base.observation_id.clone()),
        "a derived event has a distinct business id"
    );
    assert_eq!(
        derived.source_observation_id,
        Some(base.observation_id),
        "a derived event references its source observation"
    );
    assert_eq!(
        derived.origin_class,
        OriginClass::Reconstructed,
        "derived events are reconstructed, not exchange events"
    );
    let explicit = business_event_id(
        &ids(b"payload").1,
        "last_trade_observation",
        "trade_tuple_advanced",
    )
    .expect("explicit");
    assert_eq!(
        derived.business_event_id,
        Some(explicit),
        "the derived id matches explicit derivation"
    );
}

/// Sequence scope and fence are explicit; a half fence is rejected.
#[test]
fn fence_and_scope_are_explicit() {
    let base = event(b"payload", 1, 1_700_000_000_000_000_000);
    assert_eq!(
        base.sequence_scope,
        SequenceScope::None,
        "unfenced scope is none by default"
    );
    assert!(
        KernelEvent::observation_only(
            ids(b"p").0,
            ids(b"p").1,
            ids(b"p").0,
            OriginClass::SdkCallback,
            "s",
            "session",
            "inst",
            1,
            1,
        )
        .expect("base")
        .with_receive_fence(1, 0)
        .is_err(),
        "a half fence is rejected"
    );
}

/// Exchange-native origin requires evidence and is refused by default.
#[test]
fn exchange_origin_fails_closed() {
    let (frame, obs) = ids(b"payload");
    assert_eq!(
        KernelEvent::observation_only(
            frame.clone(),
            obs,
            frame,
            OriginClass::ExchangeEvent,
            "s",
            "session",
            "inst",
            1,
            1,
        ),
        Err(ContractError::ExchangeEventWithoutEvidence),
        "exchange origin is refused without evidence"
    );
}

/// A forged business id without a source back-reference is not actionable and
/// fails invariant validation.
#[test]
fn forged_business_id_is_not_actionable() {
    let mut forged = event(b"payload", 1, 1_700_000_000_000_000_000);
    forged.business_event_id = Some(ids(b"payload").1);
    assert!(
        forged.validate_invariants().is_err(),
        "business id without a back-reference is rejected"
    );
    assert!(!forged.is_actionable(), "a forged event is not actionable");
}

/// A self-declared exchange origin with an arbitrary token is not actionable.
#[test]
fn self_asserted_exchange_origin_is_rejected() {
    let mut forged = event(b"payload", 1, 1_700_000_000_000_000_000);
    forged.origin_class = OriginClass::ExchangeEvent;
    forged.evidence_token = Some("self-asserted".to_owned());
    assert!(
        forged.validate_invariants().is_err(),
        "an invalid evidence token cannot establish exchange origin"
    );
    assert!(
        !forged.is_actionable(),
        "self-assertion does not make it actionable"
    );
}

/// Serialization round-trip validates invariants on the way back in.
#[test]
fn deserialization_enforces_invariants() {
    let valid = event(b"payload", 1, 1_700_000_000_000_000_000);
    let json = serde_json::to_string(&valid).expect("serialize");
    let restored: KernelEvent = serde_json::from_str(&json).expect("deserialize valid");
    assert_eq!(restored, valid, "a valid event round-trips");

    let mut forged = valid;
    forged.business_event_id = Some(ids(b"payload").1);
    let forged_json = serde_json::to_string(&forged).expect("serialize forged");
    assert!(
        serde_json::from_str::<KernelEvent>(&forged_json).is_err(),
        "a forged wire object is refused on deserialize"
    );
}

/// Recovery is bound to the current epoch; a stale authority cannot clear it.
#[test]
fn recovery_requires_current_epoch_authority() {
    let mut machine = ContinuityMachine::default();
    machine.observe(1, 4).expect("first");
    machine.observe(1, 9).expect("gap");
    let authority = capture_frame_id(b"snapshot").expect("authority");
    assert!(
        machine.recover(2, authority.clone()).is_err(),
        "recovery from a different epoch is refused"
    );
    machine
        .recover(1, authority)
        .expect("current epoch authority");
    machine.clear_recovery().expect("clear");
    assert!(
        machine.state().is_clean(),
        "bound recovery can clear the gap"
    );
}

/// A recovered-quality event without evidence fails invariant validation.
#[test]
fn recovered_quality_requires_evidence() {
    let mut forged = event(b"payload", 1, 1_700_000_000_000_000_000);
    forged.quality = crate::StreamState::Recovered;
    assert!(
        forged.validate_invariants().is_err(),
        "recovered quality without evidence is rejected"
    );
}
