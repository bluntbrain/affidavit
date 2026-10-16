//! Verification Security #1 and #2, exercised explicitly on real mainnet material.

mod common;

use affidavit_core::bls::{Blst, DST};
use affidavit_core::config::{ChainSpec, Preset};
use affidavit_core::error::Error;
use affidavit_core::execution::AccountProof;
use affidavit_core::lc::{self, compute_domain, verify_sync_aggregate, InstanceConfig, DOMAIN_SYNC_COMMITTEE};
use affidavit_core::ssz::{self, hash_pair};
use affidavit_core::statement::{attest, attest_with_membership, Evidence, Predicate};
use affidavit_core::types::{HeaderRecord, LightClientUpdate, SyncAggregate, SyncCommittee};
use affidavit_io::rpc;
use alloy_primitives::{address, keccak256, Bytes, B256, U256};
use alloy_trie::{proof::ProofRetainer, HashBuilder, Nibbles, TrieAccount, EMPTY_ROOT_HASH, KECCAK_EMPTY};
use blst::min_pk::{AggregatePublicKey, AggregateSignature, SecretKey};
use common::*;

/// Recomputes the root a branch proves for a new leaf: what an attacker does to keep a
/// forged header internally consistent.
fn root_from_branch(leaf: &B256, branch: &[B256], index: u64) -> B256 {
    let mut value = *leaf;
    for (i, node) in branch.iter().enumerate() {
        value = if (index >> i) & 1 == 1 { hash_pair(node, &value) } else { hash_pair(&value, node) };
    }
    value
}

/// A state trie with one account holding a million ETH, and its proof: an inclusion proof
/// that is perfectly valid against a root nobody finalised.
fn fabricated_state() -> (B256, AccountProof) {
    let victim = address!("00000000219ab540356cbb839cbe05303d7705fa");
    let account = TrieAccount {
        nonce: 1,
        balance: U256::from(1_000_000u64) * U256::from(10u64).pow(U256::from(18u64)),
        storage_root: EMPTY_ROOT_HASH,
        code_hash: KECCAK_EMPTY,
    };
    let key = Nibbles::unpack(keccak256(victim));
    let mut hb = HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![key]));
    hb.add_leaf(key, &alloy_rlp::encode(account));
    let root = hb.root();
    let proof = hb.take_proof_nodes().matching_nodes_sorted(&key).into_iter().map(|(_, n)| n).collect();
    (
        root,
        AccountProof {
            address: victim,
            nonce: account.nonce,
            balance: account.balance,
            storage_root: account.storage_root,
            code_hash: account.code_hash,
            proof,
        },
    )
}

/// The central adversarial test (Verification Security #1): a self-consistent header and
/// inclusion proof over state that was never finalised must not verify.
#[test]
fn fabricated_chain_is_rejected() {
    let follower = {
        let mut f = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
        for update in updates() {
            f.process(&update, NOW_SLOT).unwrap();
        }
        f
    };
    let (fake_root, fake_account) = fabricated_state();

    // Rewrite the finalised header around the fabricated state root and repair every
    // Merkle path above it, so all hashes are consistent with each other.
    let mut forged: LightClientUpdate = finality_update();
    let spec = ChainSpec::mainnet();
    let fin = forged.finalized_header.as_mut().unwrap();
    fin.execution.state_root = fake_root;
    let fork = spec.fork_at_slot(fin.beacon.slot);
    fin.beacon.body_root = root_from_branch(&fin.execution.hash_tree_root(fork), &fin.execution_branch, 9);
    let finalized_root = fin.beacon.hash_tree_root();
    forged.attested_header.beacon.state_root = root_from_branch(&finalized_root, &forged.finality_branch, 41);

    // The inclusion proof alone is happy with it: this is why inclusion is not enough.
    let fake_record = HeaderRecord::from_header(forged.finalized_header.as_ref().unwrap());
    attest(&spec, B256::ZERO, &fake_record, &Evidence::Account(fake_account.clone()), &Predicate::default())
        .expect("the fabricated proof is internally consistent");

    // The light client is not: the committee never signed the rewritten attested header.
    let committee = follower.signing_committee(&forged);
    assert_eq!(
        lc::verify_update(&follower.instance, &forged, &committee, NOW_SLOT, &Blst).unwrap_err(),
        Error::SyncCommitteeSignatureInvalid
    );
    // So the fabricated header never enters the instance, and attesting against it fails.
    let membership = follower.headers.prove(follower.records.len() as u64 - 1).unwrap();
    assert_eq!(
        attest_with_membership(
            &spec,
            B256::ZERO,
            &follower.instance.state.headers.root(),
            &fake_record,
            &membership,
            &Evidence::Account(fake_account),
            &Predicate::default()
        )
        .unwrap_err(),
        Error::HeaderUnknown
    );
}

fn keys(n: usize, seed: u8) -> Vec<SecretKey> {
    (0..n).map(|i| SecretKey::key_gen(&[seed, (i >> 8) as u8, i as u8, 7].repeat(8), &[]).unwrap()).collect()
}

/// A well-formed committee of the given keys, aggregate key included, as an attacker would
/// build one.
fn committee_of(sks: &[SecretKey]) -> SyncCommittee {
    let keys: Vec<_> = sks.iter().map(|sk| sk.sk_to_pk()).collect();
    let aggregate = AggregatePublicKey::aggregate(&keys.iter().collect::<Vec<_>>(), true).unwrap();
    SyncCommittee {
        pubkeys: keys.iter().map(|pk| pk.compress()).collect(),
        aggregate_pubkey: aggregate.to_public_key().compress(),
    }
}

fn sign(sks: &[SecretKey], signers: usize, message: &B256) -> SyncAggregate {
    let sigs: Vec<_> = sks[..signers].iter().map(|sk| sk.sign(message.as_slice(), DST, &[])).collect();
    let refs: Vec<_> = sigs.iter().collect();
    let aggregate = AggregateSignature::aggregate(&refs, true).unwrap().to_signature().compress();
    let mut bits = vec![0u8; sks.len() / 8];
    for i in 0..signers {
        bits[i / 8] |= 1 << (i % 8);
    }
    SyncAggregate { sync_committee_bits: bits, sync_committee_signature: aggregate }
}

/// Verification Security #2: a header signed by a committee that was never handed off.
#[test]
fn committee_never_handed_off_is_rejected() {
    let follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let spec = ChainSpec::mainnet();
    // An attacker's own 512 keys sign the real update's attested header, fully.
    let attacker = keys(512, 1);
    let mut update = updates()[0].clone();
    let fork_version = spec.fork_version_at_epoch(spec.epoch_at_slot(update.signature_slot - 1));
    let domain = compute_domain(DOMAIN_SYNC_COMMITTEE, fork_version, &spec.genesis_validators_root);
    let message = hash_pair(&update.attested_header.beacon.hash_tree_root(), &domain);
    update.sync_aggregate = sign(&attacker, 512, &message);

    // Presenting the attacker's committee: it is not the committee the instance tracks.
    assert_eq!(
        lc::verify_update(&follower.instance, &update, &committee_of(&attacker), NOW_SLOT, &Blst).unwrap_err(),
        Error::CommitteeMismatch
    );
    // Presenting the real committee: the attacker's signature does not verify against it.
    let real = follower.signing_committee(&update);
    assert_eq!(
        lc::verify_update(&follower.instance, &update, &real, NOW_SLOT, &Blst).unwrap_err(),
        Error::SyncCommitteeSignatureInvalid
    );
    // A forged handoff to the attacker's committee does not verify against the attested state.
    let mut handoff = updates()[0].clone();
    handoff.next_sync_committee = Some(committee_of(&attacker));
    assert_eq!(
        lc::verify_update(&follower.instance, &handoff, &real, NOW_SLOT, &Blst).unwrap_err(),
        Error::CommitteeHandoffNotVerified
    );
}

/// Verification Security #2: participation below two thirds, at the exact boundary.
#[test]
fn participation_threshold_boundary() {
    let mut spec = ChainSpec::mainnet();
    spec.preset = Preset::MAINNET;
    let sks = keys(512, 2);
    let committee = committee_of(&sks);
    let attested_root = B256::repeat_byte(0x11);
    let signature_slot = 8_000_000;
    let fork_version = spec.fork_version_at_epoch(spec.epoch_at_slot(signature_slot - 1));
    let domain = compute_domain(DOMAIN_SYNC_COMMITTEE, fork_version, &spec.genesis_validators_root);
    let message = hash_pair(&attested_root, &domain);

    assert_eq!(lc::required_participants(512), 342);
    let at = sign(&sks, 342, &message);
    assert_eq!(verify_sync_aggregate(&spec, &committee, &at, &attested_root, signature_slot, &Blst).unwrap(), 342);
    let below = sign(&sks, 341, &message);
    assert_eq!(
        verify_sync_aggregate(&spec, &committee, &below, &attested_root, signature_slot, &Blst).unwrap_err(),
        Error::ParticipationBelowThreshold { participants: 341, required: 342 }
    );
    // Claiming more signers than actually signed breaks the aggregate.
    let mut inflated = at.clone();
    inflated.sync_committee_bits[342 / 8] |= 1 << (342 % 8);
    assert_eq!(
        verify_sync_aggregate(&spec, &committee, &inflated, &attested_root, signature_slot, &Blst).unwrap_err(),
        Error::SyncCommitteeSignatureInvalid
    );

    // The program's path agrees at the boundary, where it subtracts the most non-signers.
    use affidavit_core::bls::Zkcrypto;
    use affidavit_core::lc::verify_sync_aggregate_with_witness as with_witness;
    let zk = Zkcrypto { check_subgroups: false };
    let witness = Follower::committee_witness(&committee, &at);
    assert_eq!(witness.non_signers.len(), 170);
    assert_eq!(with_witness(&spec, &committee, Some(&witness), &at, &attested_root, signature_slot, &Blst), Ok(342));
    assert_eq!(with_witness(&spec, &committee, Some(&witness), &at, &attested_root, signature_slot, &zk), Ok(342));
    let witness = Follower::committee_witness(&committee, &inflated);
    for result in [
        with_witness(&spec, &committee, Some(&witness), &inflated, &attested_root, signature_slot, &Blst),
        with_witness(&spec, &committee, Some(&witness), &inflated, &attested_root, signature_slot, &zk),
    ] {
        assert_eq!(result.unwrap_err(), Error::SyncCommitteeSignatureInvalid);
    }
}

#[test]
fn real_update_with_bits_cleared_is_rejected() {
    let follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let mut update = updates()[0].clone();
    // Keep only the first 341 set bits.
    let mut kept = 0;
    for byte in update.sync_aggregate.sync_committee_bits.iter_mut() {
        for bit in 0..8 {
            if *byte >> bit & 1 == 1 {
                if kept >= 341 {
                    *byte &= !(1 << bit);
                } else {
                    kept += 1;
                }
            }
        }
    }
    let committee = follower.signing_committee(&update);
    assert_eq!(
        lc::verify_update(&follower.instance, &update, &committee, NOW_SLOT, &Blst).unwrap_err(),
        Error::ParticipationBelowThreshold { participants: 341, required: 342 }
    );
}

/// Verification Security #2: a valid inclusion proof against a header that is not finalised.
#[test]
fn non_finalised_header_is_not_attestable() {
    let follower = synced();
    let update = finality_update();
    // The attested header is signed by the committee, but not finalised.
    let attested = HeaderRecord::from_header(&update.attested_header);
    let membership = follower.headers.prove(follower.records.len() as u64 - 1).unwrap();
    let evidence = Evidence::Account(rpc::parse_get_proof(&read("mainnet/block-21925176/proof.json")).unwrap().0);
    assert_eq!(
        attest_with_membership(
            &ChainSpec::mainnet(),
            B256::ZERO,
            &follower.instance.state.headers.root(),
            &attested,
            &membership,
            &evidence,
            &Predicate::default()
        )
        .unwrap_err(),
        Error::HeaderUnknown
    );
    // An update with no finality branch never advances the instance.
    let mut optimistic = update;
    optimistic.finalized_header = None;
    let fresh = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let committee = fresh.signing_committee(&optimistic);
    assert_eq!(
        lc::verify_update(&fresh.instance, &optimistic, &committee, NOW_SLOT, &Blst).unwrap_err(),
        Error::HeaderNotFinalized
    );
}

/// Verification Security #2: a valid finalised header with a tampered state or receipts root.
#[test]
fn tampered_execution_roots_are_rejected() {
    let mut follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    for update in updates() {
        follower.process(&update, NOW_SLOT).unwrap();
    }
    for tamper in [0u8, 1, 2] {
        let mut update = finality_update();
        let execution = &mut update.finalized_header.as_mut().unwrap().execution;
        match tamper {
            0 => execution.state_root = B256::repeat_byte(0xaa),
            1 => execution.receipts_root = B256::repeat_byte(0xbb),
            _ => execution.transactions_root = B256::repeat_byte(0xcc),
        }
        let committee = follower.signing_committee(&update);
        assert_eq!(
            lc::verify_update(&follower.instance, &update, &committee, NOW_SLOT, &Blst).unwrap_err(),
            Error::ExecutionNotBound
        );
    }
}

/// Verification Security #2: material replayed against another network.
#[test]
fn replay_against_another_chain_is_rejected() {
    // A Hoodi instance configured with the mainnet checkpoint: Hoodi's fork schedule puts
    // that slot in Fulu, so the Capella-shaped header does not even bind.
    let hoodi = InstanceConfig { chain: ChainSpec::hoodi(), ..mainnet_config() };
    assert!(lc::initialize(hoodi, &bootstrap(), NOW_SLOT).is_err());

    // A Sepolia-shaped instance with mainnet's fork schedule but Sepolia's genesis root:
    // the bootstrap binds, but no mainnet signature verifies under Sepolia's domain.
    let mut chain = ChainSpec::mainnet();
    chain.chain_id = ChainSpec::sepolia().chain_id;
    chain.genesis_validators_root = ChainSpec::sepolia().genesis_validators_root;
    let mut foreign = Follower::new(InstanceConfig { chain, ..mainnet_config() }, &bootstrap(), NOW_SLOT).unwrap();
    assert_eq!(foreign.process(&updates()[0], NOW_SLOT).unwrap_err(), Error::SyncCommitteeSignatureInvalid);

    // A mainnet statement is refused by a consumer bound to another chain or instance.
    let follower = synced();
    let header = follower.instance.state.finalized;
    let (account, _) = rpc::parse_get_proof(&read("mainnet/block-21925176/proof.json")).unwrap();
    let record = HeaderRecord { state_root: keccak256(&account.proof[0]), ..header };
    let statement = attest(
        &ChainSpec::mainnet(),
        follower.instance.config.id(),
        &record,
        &Evidence::Account(account),
        &Predicate::default(),
    )
    .unwrap();
    statement.check_binding(&ChainSpec::mainnet(), &follower.instance.config.id()).unwrap();
    assert_eq!(
        statement.check_binding(&ChainSpec::sepolia(), &follower.instance.config.id()).unwrap_err(),
        Error::ChainMismatch
    );
    assert_eq!(
        statement.check_binding(&ChainSpec::mainnet(), &B256::repeat_byte(9)).unwrap_err(),
        Error::InstanceMismatch
    );
}

#[test]
fn skipping_a_period_is_rejected() {
    let follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let skip = updates()[3].clone(); // period 865 straight after the period-862 checkpoint
    let committee = follower.signing_committee(&skip);
    assert_eq!(
        lc::verify_update(&follower.instance, &skip, &committee, NOW_SLOT, &Blst).unwrap_err(),
        Error::CommitteeHandoffNotVerified
    );
}

#[test]
fn checkpoint_must_match() {
    let wrong = InstanceConfig { checkpoint_root: B256::repeat_byte(7), ..mainnet_config() };
    assert_eq!(lc::initialize(wrong, &bootstrap(), NOW_SLOT).unwrap_err(), Error::CheckpointMismatch);
    let mut forged = bootstrap();
    forged.current_sync_committee = committee_of(&keys(512, 3));
    assert_eq!(lc::initialize(mainnet_config(), &forged, NOW_SLOT).unwrap_err(), Error::CheckpointMismatch);
}

#[test]
fn duplicate_and_racing_updates_apply_once() {
    // Reliability #8: two operators verify the same update against the same state.
    let mut follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let update = updates()[0].clone();
    let committee = follower.signing_committee(&update);
    let first = lc::verify_update(&follower.instance, &update, &committee, NOW_SLOT, &Blst).unwrap();
    let second = lc::verify_update(&follower.instance, &update, &committee, NOW_SLOT, &Blst).unwrap();
    lc::apply_advance(&mut follower.instance, &first).unwrap();
    let after_first = follower.instance.clone();
    assert_eq!(lc::apply_advance(&mut follower.instance, &second).unwrap_err(), Error::UpdateNotNewer);
    assert_eq!(follower.instance, after_first, "a rejected duplicate changes nothing");
    // Resubmitting the raw update is refused before any signature work.
    assert_eq!(
        lc::verify_update(&follower.instance, &update, &committee, NOW_SLOT, &Blst).unwrap_err(),
        Error::UpdateNotNewer
    );
}

#[test]
fn stale_instance_stops_advancing_but_keeps_its_attestations() {
    // Functionality #10, Reliability #3: beyond the weak-subjectivity period an instance
    // refuses updates, yet every header it already verified stays attestable.
    let follower = synced();
    let ws = follower.instance.config.ws_period_epochs;
    let much_later = follower.instance.state.finalized.slot + (ws + 1) * 32;
    let late = follower.instance.clone();
    let update = updates()[5].clone();
    let committee = follower.signing_committee(&update);
    assert!(matches!(
        lc::verify_update(&late, &update, &committee, much_later, &Blst).unwrap_err(),
        Error::CheckpointStale { .. }
    ));
    let (_, txs) =
        affidavit_io::beacon::parse_block_transactions(&read("mainnet/light-client/beacon-block-7109344.json"))
            .unwrap();
    let proof = affidavit_core::execution::TransactionProof {
        tx_index: 0,
        tx: Bytes::from(txs[0].clone()),
        branch: ssz::transaction_branch(&txs, 0),
    };
    let membership = follower.headers.prove(follower.records.len() as u64 - 1).unwrap();
    attest_with_membership(
        &ChainSpec::mainnet(),
        follower.instance.config.id(),
        &follower.instance.state.headers.root(),
        &follower.instance.state.finalized,
        &membership,
        &Evidence::Transfer { tx: proof, receipt: None },
        &Predicate::default(),
    )
    .unwrap();
}

#[test]
fn updates_from_the_future_or_out_of_order_are_rejected() {
    let follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let update = updates()[0].clone();
    let committee = follower.signing_committee(&update);
    assert_eq!(
        lc::verify_update(&follower.instance, &update, &committee, update.signature_slot - 1, &Blst).unwrap_err(),
        Error::SlotOrdering
    );
    let mut swapped = update.clone();
    swapped.signature_slot = swapped.attested_header.beacon.slot;
    assert_eq!(
        lc::verify_update(&follower.instance, &swapped, &committee, NOW_SLOT, &Blst).unwrap_err(),
        Error::SlotOrdering
    );
}

#[test]
fn signatures_do_not_transfer_between_headers() {
    let follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let mut update = updates()[0].clone();
    // A real signature from the same committee, over a different header.
    update.sync_aggregate.sync_committee_signature = updates()[1].sync_aggregate.sync_committee_signature;
    update.sync_aggregate.sync_committee_bits = updates()[0].sync_aggregate.sync_committee_bits.clone();
    let committee = follower.signing_committee(&update);
    assert_eq!(
        lc::verify_update(&follower.instance, &update, &committee, NOW_SLOT, &Blst).unwrap_err(),
        Error::SyncCommitteeSignatureInvalid
    );
}

/// The committee witness replaces decompression only: any witness that is not exactly the
/// committed key is refused, by both libraries, and the aggregate cannot be swapped because
/// the committee root commits to it.
#[test]
fn forged_key_witnesses_are_refused() {
    use affidavit_core::bls::{uncompress, Zkcrypto};
    let follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    // 511 of 512 signed: one non-signer to witness.
    let update = updates()[0].clone();
    let committee = follower.signing_committee(&update);
    let honest = Follower::committee_witness(&committee, &update.sync_aggregate);
    assert_eq!(honest.non_signers.len(), 1);
    let verify = |committee: &SyncCommittee, witness: &lc::CommitteeWitness| {
        let with_blst =
            lc::verify_update_with_witness(&follower.instance, &update, committee, Some(witness), NOW_SLOT, &Blst);
        let with_zk = lc::verify_update_with_witness(
            &follower.instance,
            &update,
            committee,
            Some(witness),
            NOW_SLOT,
            &Zkcrypto { check_subgroups: false },
        );
        assert_eq!(with_blst, with_zk);
        with_blst
    };
    assert!(verify(&committee, &honest).is_ok());

    let identity = {
        let mut point = [0u8; 96];
        point[0] = 0x40;
        point
    };
    let signer_point = uncompress(&committee.pubkeys[0]).unwrap();
    let forged = [
        (
            "a signer's key as the non-signer",
            lc::CommitteeWitness { non_signers: vec![signer_point], ..honest.clone() },
        ),
        ("the identity as the non-signer", lc::CommitteeWitness { non_signers: vec![identity], ..honest.clone() }),
        ("a signer's key as the aggregate", lc::CommitteeWitness { aggregate: signer_point, ..honest.clone() }),
        ("the identity as the aggregate", lc::CommitteeWitness { aggregate: identity, ..honest.clone() }),
        ("an aggregate off the curve", {
            let mut witness = honest.clone();
            witness.aggregate[95] ^= 1;
            witness
        }),
    ];
    for (label, witness) in forged {
        assert_eq!(verify(&committee, &witness).unwrap_err(), Error::SyncCommitteeSignatureInvalid, "{label}");
    }
    let missing = lc::CommitteeWitness { non_signers: vec![], ..honest.clone() };
    assert_eq!(verify(&committee, &missing).unwrap_err(), Error::MalformedInput("uncompressed key witnesses"));

    // A committee claiming another aggregate key, with a matching witness, is not the
    // committee the instance tracks.
    let mut relabelled = committee.clone();
    relabelled.aggregate_pubkey = committee.pubkeys[0];
    let witness = lc::CommitteeWitness { aggregate: signer_point, ..honest.clone() };
    assert_eq!(verify(&relabelled, &witness).unwrap_err(), Error::CommitteeMismatch);
}
