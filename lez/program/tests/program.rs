//! The program's message handlers, driven natively with LEZ's own `ReceiveInput` and
//! `Response` types over the mainnet fixtures. What the runtime would do between handlers
//! (deliver a sync message, store a post-state) is done by hand here.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use affidavit_core::config::{weak_subjectivity_period_epochs, ChainSpec};
use affidavit_core::execution::TransactionProof;
use affidavit_core::lc::InstanceConfig;
use affidavit_core::mmr::Mmr;
use affidavit_core::ssz;
use affidavit_core::statement::{Clause, Evidence, Field, Op, Predicate};
use affidavit_core::types::{LightClientBootstrap, LightClientUpdate};
use affidavit_io::beacon;
use affidavit_io::follower::Follower;
use affidavit_lez::{handle_message, header_actor, instance_actor, HeaderAccount, InstanceAccount, Message, Reply};
use alloy_primitives::{b256, Bytes, B256, U256};
use lee_core::account::{AccountId, Actor, ActorState};
use lee_core::program::{ReceiveInput, Response};

const CHECKPOINT: B256 = b256!("5afc212a7924789b2bc86acad3ab3a6ffb1f6e97253ea50bee7f4f51422c9275");
const NOW_SLOT: u64 = 7_109_500;

fn program() -> AccountId {
    AccountId::new([0xaf; 32])
}

fn read(path: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata").join(path)).unwrap()
}

fn config() -> InstanceConfig {
    InstanceConfig {
        chain: ChainSpec::mainnet(),
        checkpoint_root: CHECKPOINT,
        ws_period_epochs: weak_subjectivity_period_epochs(24_000_000 * 1_000_000_000),
    }
}

fn bootstrap() -> LightClientBootstrap {
    beacon::parse_bootstrap(&read("mainnet/light-client/bootstrap.json")).unwrap()
}

fn all_updates() -> Vec<LightClientUpdate> {
    let mut u = beacon::parse_updates(&read("mainnet/light-client/updates.json")).unwrap();
    u.push(beacon::parse_finality_update(&read("mainnet/light-client/finality.json")).unwrap());
    u
}

fn deliver(receiver: Actor, from: Option<Actor>, pre_state: &[u8], message: &Message) -> Response {
    let input = ReceiveInput {
        receiver,
        from,
        is_authorized: false,
        pre_state: ActorState::from(pre_state.to_vec()),
        message: borsh::to_vec(message).unwrap(),
    };
    handle_message(&input)
}

/// Runs a handler and returns the `AFF-xxx` code it refused with.
fn refusal(f: impl FnOnce() -> Response) -> String {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(prev);
    let payload = result.err().expect("handler should refuse");
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    message.split(' ').next().unwrap_or_default().to_string()
}

fn state_of(response: &Response) -> Vec<u8> {
    response.post_state.as_ref().expect("state written").to_vec()
}

fn sent_message(response: &Response, i: usize) -> (Actor, Message) {
    let m = &response.sync_messages[i];
    (m.to, borsh::from_slice(&m.message).unwrap())
}

/// A chain of events as the runtime would execute it, for the whole fixture set.
struct Chain {
    instance_state: Vec<u8>,
    records: Vec<(Actor, Vec<u8>)>,
}

fn initialise() -> Chain {
    let cfg = config();
    let at = instance_actor(program(), &cfg.id());
    let response =
        deliver(at, None, &[], &Message::Initialize { config: cfg, bootstrap: bootstrap(), now_slot: NOW_SLOT });
    let (to, record) = sent_message(&response, 0);
    let written = deliver(to, Some(at), &[], &record);
    Chain { instance_state: state_of(&response), records: vec![(to, state_of(&written))] }
}

#[test]
fn whole_mainnet_flow_through_the_handlers() {
    let mut chain = initialise();
    let mut follower = Follower::new(config(), &bootstrap(), NOW_SLOT).unwrap();
    // Any actor of this program can verify; here an account the instance has never seen.
    let verifier = Actor::new(AccountId::new([0x51; 32]), program());
    for update in all_updates() {
        let snapshot: InstanceAccount = borsh::from_slice(&chain.instance_state).unwrap();
        let committee = follower.signing_committee(&update);
        let verified = deliver(
            verifier,
            None,
            &[],
            &Message::Verify {
                snapshot: snapshot.instance.clone(),
                update: update.clone(),
                witness: Some(Follower::committee_witness(&committee, &update.sync_aggregate)),
                committee,
                now_slot: NOW_SLOT,
            },
        );
        assert!(verified.post_state.is_none(), "verification writes nothing");
        let (instance, apply) = sent_message(&verified, 0);
        assert_eq!(instance, instance_actor(program(), &snapshot.instance.config.id()));
        let applied = deliver(instance, Some(verifier), &chain.instance_state, &apply);
        chain.instance_state = state_of(&applied);
        if let Some(m) = applied.sync_messages.first() {
            let (to, record) = (m.to, borsh::from_slice::<Message>(&m.message).unwrap());
            let written = deliver(to, Some(instance), &[], &record);
            chain.records.push((to, state_of(&written)));
        }
        follower.process(&update, NOW_SLOT).unwrap();
    }
    let account: InstanceAccount = borsh::from_slice(&chain.instance_state).unwrap();
    assert_eq!(account.instance, follower.instance, "the program and the native follower agree");
    assert_eq!(account.instance.state.finalized.block_number, 17_923_026);
    assert_eq!(chain.records.len(), 7, "one write-once record per finalised header");
    let head: HeaderAccount = borsh::from_slice(&chain.records.last().unwrap().1).unwrap();
    assert_eq!(head.record, account.instance.state.finalized);
}

fn synced() -> (Chain, Follower) {
    let mut chain = initialise();
    let mut follower = Follower::new(config(), &bootstrap(), NOW_SLOT).unwrap();
    let verifier = Actor::new(AccountId::new([0x51; 32]), program());
    for update in all_updates() {
        let snapshot: InstanceAccount = borsh::from_slice(&chain.instance_state).unwrap();
        let committee = follower.signing_committee(&update);
        let verified = deliver(
            verifier,
            None,
            &[],
            &Message::Verify {
                snapshot: snapshot.instance,
                update: update.clone(),
                witness: Some(Follower::committee_witness(&committee, &update.sync_aggregate)),
                committee,
                now_slot: NOW_SLOT,
            },
        );
        let (instance, apply) = sent_message(&verified, 0);
        let applied = deliver(instance, Some(verifier), &chain.instance_state, &apply);
        chain.instance_state = state_of(&applied);
        if let Some(m) = applied.sync_messages.first() {
            let record = borsh::from_slice::<Message>(&m.message).unwrap();
            chain.records.push((m.to, state_of(&deliver(m.to, Some(instance), &[], &record))));
        }
        follower.process(&update, NOW_SLOT).unwrap();
    }
    (chain, follower)
}

#[test]
fn instance_lives_at_its_config_and_initialises_once() {
    let cfg = config();
    let at = instance_actor(program(), &cfg.id());
    let msg = Message::Initialize { config: cfg.clone(), bootstrap: bootstrap(), now_slot: NOW_SLOT };
    // Anywhere but the PDA of its own config is refused.
    let elsewhere = Actor::new(AccountId::new([1; 32]), program());
    assert_eq!(refusal(|| deliver(elsewhere, None, &[], &msg)), "AFF-100");
    // Initialising over an existing instance is refused: there is no reconfiguration path.
    let chain = initialise();
    assert_eq!(refusal(|| deliver(at, None, &chain.instance_state, &msg)), "AFF-100");
    // A different checkpoint is a different instance at a different address.
    let other = InstanceConfig { checkpoint_root: B256::repeat_byte(9), ..cfg };
    assert_ne!(instance_actor(program(), &other.id()), at);
}

#[test]
fn only_this_programs_actors_can_apply_or_record() {
    let (chain, follower) = synced();
    let account: InstanceAccount = borsh::from_slice(&chain.instance_state).unwrap();
    let instance = instance_actor(program(), &account.instance.config.id());
    // A forged Apply sent directly, or from another program, is refused before anything else.
    let fake = Message::Apply(affidavit_core::lc::Advance {
        instance_id: account.instance.config.id(),
        prior_period: 867,
        prior_current_committee_root: account.instance.state.current_committee_root,
        prior_next_committee_root: account.instance.state.next_committee_root,
        new_finalized: Some(account.instance.state.finalized),
        new_current_committee_root: B256::repeat_byte(6),
        new_next_committee_root: None,
        participants: 512,
    });
    assert_eq!(refusal(|| deliver(instance, None, &chain.instance_state, &fake)), "AFF-104");
    let stranger = Actor::new(AccountId::new([7; 32]), AccountId::new([8; 32]));
    assert_eq!(refusal(|| deliver(instance, Some(stranger), &chain.instance_state, &fake)), "AFF-104");
    // Replaying a genuine update through the verifier is refused as not newer.
    let verifier = Actor::new(AccountId::new([0x52; 32]), program());
    let last = all_updates().pop().unwrap();
    let committee = follower.signing_committee(&last);
    assert_eq!(
        refusal(|| deliver(
            verifier,
            None,
            &[],
            &Message::Verify {
                snapshot: account.instance.clone(),
                update: last,
                witness: None,
                committee,
                now_slot: NOW_SLOT
            }
        )),
        "AFF-207"
    );
    // A record is written once; a different record at the same address is refused.
    let (record_at, record_state) = chain.records.last().unwrap().clone();
    let mut altered: HeaderAccount = borsh::from_slice(&record_state).unwrap();
    altered.record.state_root = B256::repeat_byte(1);
    let refused =
        catch_unwind(AssertUnwindSafe(|| deliver(record_at, Some(instance), &record_state, &Message::Record(altered))));
    assert!(refused.is_err());
}

#[test]
fn public_and_private_consumers_get_the_same_statement() {
    let (chain, follower) = synced();
    let account: InstanceAccount = borsh::from_slice(&chain.instance_state).unwrap();
    let instance_id = account.instance.config.id();
    let head = account.instance.state.finalized;
    let (_, txs) = beacon::parse_block_transactions(&read("mainnet/light-client/beacon-block-7109344.json")).unwrap();
    let index = 0usize;
    let evidence = Evidence::Transfer {
        tx: TransactionProof {
            tx_index: index as u64,
            tx: Bytes::from(txs[index].clone()),
            branch: ssz::transaction_branch(&txs, index),
        },
        receipt: None,
    };
    let predicate = Predicate(vec![Clause { field: Field::TxValue, op: Op::Gt, value: U256::ZERO }]);
    let consumer = Actor::new(AccountId::new([0xc0; 32]), AccountId::new([0xc1; 32]));

    // Public path: the consumer calls the header record in its own transaction.
    let record_at = header_actor(program(), &instance_id, head.slot);
    let (_, record_state) = chain.records.iter().find(|(a, _)| *a == record_at).cloned().unwrap();
    let public = deliver(
        record_at,
        Some(consumer),
        &record_state,
        &Message::AttestRecord {
            chain: ChainSpec::mainnet(),
            evidence: evidence.clone(),
            predicate: predicate.clone(),
        },
    );
    let (to, reply) =
        (public.sync_messages[0].to, borsh::from_slice::<Reply>(&public.sync_messages[0].message).unwrap());
    assert_eq!(to, consumer);
    let Reply::Attested(public_statement) = reply;

    // Private path: an actor of this program, run privately, proves accumulator membership
    // and discloses only the accumulator root to the instance.
    let mut mmr = Mmr::default();
    for r in &follower.records {
        mmr.append(r.leaf());
    }
    let membership = mmr.prove(follower.records.len() as u64 - 1).unwrap();
    let root = account.instance.state.headers.root();
    let private_actor = Actor::new(AccountId::new([0x99; 32]), program());
    let private = deliver(
        private_actor,
        Some(consumer),
        &[],
        &Message::AttestPrivate {
            chain: ChainSpec::mainnet(),
            instance_id,
            headers_root: root,
            header: head,
            membership,
            evidence,
            predicate,
        },
    );
    let Reply::Attested(private_statement) = borsh::from_slice::<Reply>(&private.sync_messages[0].message).unwrap();
    assert_eq!(private_statement.id, public_statement.id, "same fact, same identifier on both paths");
    let (check_to, check) = sent_message(&private, 1);
    assert_eq!(check_to, instance_actor(program(), &instance_id));
    match &check {
        Message::CheckRoot { root: disclosed, .. } => assert_eq!(*disclosed, root),
        _ => panic!("expected CheckRoot"),
    }
    // The instance answers for a recent root and refuses an unknown one.
    assert!(deliver(check_to, Some(private_actor), &chain.instance_state, &check).post_state.is_none());
    let unknown = Message::CheckRoot { root: B256::repeat_byte(3), chain_digest: ChainSpec::mainnet().digest() };
    assert_eq!(refusal(|| deliver(check_to, Some(private_actor), &chain.instance_state, &unknown)), "AFF-210");
    // And refuses a Sepolia consumer at a mainnet record.
    assert_eq!(
        refusal(|| deliver(
            record_at,
            Some(consumer),
            &record_state,
            &Message::AttestRecord {
                chain: ChainSpec::sepolia(),
                evidence: Evidence::Transfer {
                    tx: TransactionProof {
                        tx_index: 0,
                        tx: Bytes::from(txs[0].clone()),
                        branch: ssz::transaction_branch(&txs, 0)
                    },
                    receipt: None
                },
                predicate: Predicate::default(),
            }
        )),
        "AFF-101"
    );
}

#[test]
fn storage_sizes() {
    // Performance #7: what accumulates, measured on the synced fixture chain.
    let (chain, _) = synced();
    let account: InstanceAccount = borsh::from_slice(&chain.instance_state).unwrap();
    let record_bytes = chain.records.last().unwrap().1.len();
    println!(
        "instance account: {} bytes ({} roots in the ring, {} accumulator peaks)",
        chain.instance_state.len(),
        account.recent_roots.len(),
        account.instance.state.headers.peaks.len()
    );
    println!("header record: {record_bytes} bytes");
    // Worst case for the instance: a full 64-root ring and 64 accumulator peaks.
    let mut full = account.clone();
    full.recent_roots = vec![B256::ZERO; affidavit_lez::RECENT_ROOTS];
    full.instance.state.headers.peaks = vec![B256::ZERO; 64];
    println!("instance account, worst case: {} bytes", borsh::to_vec(&full).unwrap().len());
    assert!(borsh::to_vec(&full).unwrap().len() < 5_000);
    assert!(record_bytes < 300);
}
