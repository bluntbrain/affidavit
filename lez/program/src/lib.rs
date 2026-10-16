//! affidavit as a LEZ program, written against the actor model on LEZ `main` (the road to
//! testnet v0.4): one `handle_message` per actor, program-derived actors, sync messages.
//!
//! Actors of this program:
//!
//! - **Instance** (public, at the PDA of its own config id): checkpoint, chain binding and
//!   the tracked consensus state. Because its address is derived from its configuration, a
//!   configuration cannot change without the instance moving: there is nothing to rewrite
//!   (Functionality #9, #13, #15).
//! - **Header record** (public, write-once PDA per finalised header): what a public
//!   consumer reads (Reliability #3).
//! - **Verifier** (any actor of this program, normally run *privately*): checks an update
//!   and sends the result to the instance. Placed privately, its 15M cycles are proven by
//!   the submitter and the sequencer verifies one receipt instead of re-executing them.
//!
//! Every refusal is a panic carrying the same `AFF-xxx` code the CLI dry run prints.

use affidavit_core::bls::Zkcrypto;
use affidavit_core::lc::{self, Advance, Instance, InstanceConfig};
use affidavit_core::mmr::MmrProof;
use affidavit_core::ssz::sha256;
use affidavit_core::statement::{attest, attest_with_membership, Evidence, Predicate, Statement};
use affidavit_core::types::{HeaderRecord, LightClientBootstrap, LightClientUpdate, SyncCommittee};
use affidavit_core::Error;
use alloy_primitives::B256;
use borsh::{BorshDeserialize, BorshSerialize};
use lee_core::account::{AccountId, Actor};
use lee_core::program::{write_once, PdaSeed, ReceiveInput, Response, SyncMessage};

/// Accumulator roots an instance keeps answering for, so a private consumer whose proof
/// was built against a slightly older root still lands (Reliability #11).
pub const RECENT_ROOTS: usize = 64;

/// The BLS backend in the program. Committee keys are bound to the attested beacon state
/// by the committee root, so their subgroup checks are skipped (`docs/DECISIONS.md` D3).
const BLS: Zkcrypto = Zkcrypto { check_subgroups: false };

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct InstanceAccount {
    pub instance: Instance,
    /// The last `RECENT_ROOTS` header-accumulator roots, newest last.
    pub recent_roots: Vec<B256>,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct HeaderAccount {
    pub instance_id: B256,
    pub chain_digest: B256,
    pub record: HeaderRecord,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
#[allow(clippy::large_enum_variant)]
pub enum Message {
    /// Anyone, to the instance PDA of `config`.
    Initialize { config: InstanceConfig, bootstrap: LightClientBootstrap, now_slot: u64 },
    /// Anyone, to any actor of this program. Verifies `update` against `snapshot` and asks
    /// the instance to apply the result. Touches no state of its own.
    Verify { snapshot: Instance, update: LightClientUpdate, committee: SyncCommittee, now_slot: u64 },
    /// From this program only: compare-and-swap of the instance's tracked state.
    Apply(Advance),
    /// From this program only: write a finalised header record, once.
    Record(HeaderAccount),
    /// Public consumer path, to a header record: prove `evidence` under it and reply.
    AttestRecord { chain: affidavit_core::config::ChainSpec, evidence: Evidence, predicate: Predicate },
    /// Private consumer path, to any actor of this program run privately: prove the header
    /// is in the accumulator, prove the evidence, and disclose only the accumulator root.
    AttestPrivate {
        chain: affidavit_core::config::ChainSpec,
        instance_id: B256,
        headers_root: B256,
        header: HeaderRecord,
        membership: MmrProof,
        evidence: Evidence,
        predicate: Predicate,
    },
    /// From this program only, to an instance: is `root` one of its recent accumulator roots?
    CheckRoot { root: B256, chain_digest: B256 },
}

/// What a consumer receives back.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum Reply {
    Attested(Statement),
}

fn fail(error: Error) -> ! {
    panic!("{error}")
}

fn require(condition: bool, error: Error) {
    if !condition {
        fail(error)
    }
}

fn decode<T: BorshDeserialize>(bytes: &[u8], what: &'static str) -> T {
    borsh::from_slice(bytes).unwrap_or_else(|_| fail(Error::MalformedInput(what)))
}

pub fn instance_seed(instance_id: &B256) -> PdaSeed {
    PdaSeed::new(instance_id.0)
}

pub fn instance_actor(program: AccountId, instance_id: &B256) -> Actor {
    Actor::new(AccountId::for_public_pda(&program, &instance_seed(instance_id)), program)
}

pub fn header_seed(instance_id: &B256, slot: u64) -> PdaSeed {
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(b"affidavit/header-record/v1");
    data.extend_from_slice(instance_id.as_slice());
    data.extend_from_slice(&slot.to_le_bytes());
    PdaSeed::new(sha256(&data).0)
}

pub fn header_actor(program: AccountId, instance_id: &B256, slot: u64) -> Actor {
    Actor::new(AccountId::for_public_pda(&program, &header_seed(instance_id, slot)), program)
}

fn record_message(program: AccountId, instance: &Instance, record: HeaderRecord) -> SyncMessage {
    let instance_id = instance.config.id();
    SyncMessage::new(
        header_actor(program, &instance_id, record.slot),
        &Message::Record(HeaderAccount { instance_id, chain_digest: instance.config.chain.digest(), record }),
    )
    .with_pda_seeds([header_seed(&instance_id, record.slot)])
}

pub fn handle_message(input: &ReceiveInput) -> Response {
    let program = input.receiver.program_account_id;
    match decode::<Message>(&input.message, "message") {
        Message::Initialize { config, bootstrap, now_slot } => {
            let instance_id = config.id();
            require(input.receiver == instance_actor(program, &instance_id), Error::MalformedInput("instance address"));
            require(input.pre_state.is_empty(), Error::MalformedInput("instance already initialised"));
            let instance = lc::initialize(config, &bootstrap, now_slot).unwrap_or_else(|e| fail(e));
            let record = instance.state.finalized;
            let account = InstanceAccount { recent_roots: vec![instance.state.headers.root()], instance };
            Response::set_state(borsh::to_vec(&account).expect("encodes")).send(record_message(
                program,
                &account.instance,
                record,
            ))
        }
        Message::Verify { snapshot, update, committee, now_slot } => {
            let advance = lc::verify_update(&snapshot, &update, &committee, now_slot, &BLS).unwrap_or_else(|e| fail(e));
            let instance_id = snapshot.config.id();
            Response::keep_state().send(
                SyncMessage::new(instance_actor(program, &instance_id), &Message::Apply(advance))
                    .with_pda_seeds([instance_seed(&instance_id)]),
            )
        }
        Message::Apply(advance) => {
            require(input.from_own_program(), Error::Unauthorized);
            let mut account: InstanceAccount = decode(&input.pre_state, "instance state");
            require(
                input.receiver == instance_actor(program, &account.instance.config.id()),
                Error::MalformedInput("instance address"),
            );
            lc::apply_advance(&mut account.instance, &advance).unwrap_or_else(|e| fail(e));
            account.recent_roots.push(account.instance.state.headers.root());
            if account.recent_roots.len() > RECENT_ROOTS {
                account.recent_roots.remove(0);
            }
            let mut response = Response::set_state(borsh::to_vec(&account).expect("encodes"));
            if let Some(record) = advance.new_finalized {
                response = response.send(record_message(program, &account.instance, record));
            }
            response
        }
        Message::Record(header) => {
            require(input.from_own_program(), Error::Unauthorized);
            require(
                input.receiver == header_actor(program, &header.instance_id, header.record.slot),
                Error::MalformedInput("header record address"),
            );
            Response::set_state(write_once(&input.pre_state, borsh::to_vec(&header).expect("encodes")))
        }
        Message::AttestRecord { chain, evidence, predicate } => {
            require(!input.pre_state.is_empty(), Error::HeaderUnknown);
            let header: HeaderAccount = decode(&input.pre_state, "header record");
            require(
                input.receiver == header_actor(program, &header.instance_id, header.record.slot),
                Error::HeaderUnknown,
            );
            require(chain.digest() == header.chain_digest, Error::ChainMismatch);
            let statement =
                attest(&chain, header.instance_id, &header.record, &evidence, &predicate).unwrap_or_else(|e| fail(e));
            reply(input, statement)
        }
        Message::AttestPrivate { chain, instance_id, headers_root, header, membership, evidence, predicate } => {
            let statement =
                attest_with_membership(&chain, instance_id, &headers_root, &header, &membership, &evidence, &predicate)
                    .unwrap_or_else(|e| fail(e));
            // The single public crossing: a root every consumer of this instance shares.
            reply(input, statement).send(
                SyncMessage::new(
                    instance_actor(program, &instance_id),
                    &Message::CheckRoot { root: headers_root, chain_digest: chain.digest() },
                )
                .with_pda_seeds([instance_seed(&instance_id)]),
            )
        }
        Message::CheckRoot { root, chain_digest } => {
            require(input.from_own_program(), Error::Unauthorized);
            let account: InstanceAccount = decode(&input.pre_state, "instance state");
            require(account.instance.config.chain.digest() == chain_digest, Error::ChainMismatch);
            require(account.recent_roots.contains(&root), Error::HeaderUnknown);
            Response::keep_state()
        }
    }
}

fn reply(input: &ReceiveInput, statement: Statement) -> Response {
    match input.from {
        Some(caller) => Response::keep_state().send(SyncMessage::new(caller, &Reply::Attested(statement))),
        // Called at the top level there is nobody to answer; succeeding is the answer.
        None => Response::keep_state(),
    }
}
