//! The light client on real Ethereum mainnet data: a checkpoint at slot 7,069,376, the
//! sync-committee handoffs of periods 862 to 867, and a finality update for slot 7,109,344.

mod common;

use affidavit_core::bls::Blst;
use affidavit_core::lc;
use affidavit_core::ssz;
use affidavit_io::beacon;
use alloy_primitives::b256;
use common::*;

#[test]
fn bootstrap_hashes_to_the_published_checkpoint() {
    assert_eq!(bootstrap().header.beacon.hash_tree_root(), CHECKPOINT);
}

#[test]
fn follows_five_handoffs_then_finality() {
    let mut follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let spec = follower.instance.config.chain.clone();
    assert_eq!(spec.period_at_slot(follower.instance.state.finalized.slot), 862);

    let mut periods = Vec::new();
    for update in updates() {
        let advance = follower.process(&update, NOW_SLOT).unwrap();
        assert!(advance.participants >= 342);
        periods.push(spec.period_at_slot(follower.instance.state.finalized.slot));
    }
    // The first update teaches the next committee without moving finality; each later one
    // is signed by the committee the previous one handed off.
    assert_eq!(periods, vec![862, 863, 864, 865, 866, 867]);

    let advance = follower.process(&finality_update(), NOW_SLOT).unwrap();
    let head = advance.new_finalized.unwrap();
    assert_eq!(head.slot, 7_109_344);
    assert_eq!(head.block_number, 17_923_026);
    assert_eq!(head.state_root, b256!("226f5ff47ab3725b5a4a3afc74b1e79e4aa3a29704561eccce590e58900baec3"));
    assert_eq!(follower.instance.state.updates_applied, 7);
    // Seven finalised headers recorded: the checkpoint and one per finality advance.
    assert_eq!(follower.instance.state.headers.leaf_count, 7);
    assert_eq!(follower.instance.state.headers, *follower.headers.accumulator());
}

#[test]
fn the_full_block_matches_the_verified_header() {
    // The beacon block for the finalised slot, fetched separately: its transactions must
    // hash to the transactions root the light client verified.
    let follower = synced();
    let (slot, txs) =
        beacon::parse_block_transactions(&read("mainnet/light-client/beacon-block-7109344.json")).unwrap();
    assert_eq!(slot, follower.instance.state.finalized.slot);
    assert_eq!(ssz::transactions_root(&txs), follower.instance.state.finalized.transactions_root);
    assert_eq!(txs.len(), 167);
}

#[test]
fn verification_is_read_only_and_deterministic() {
    // Reliability #1, #2: the same inputs give the same verdict and verifying changes nothing.
    let follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let update = &updates()[0];
    let committee = follower.signing_committee(update);
    let before = follower.instance.clone();
    let a = lc::verify_update(&follower.instance, update, &committee, NOW_SLOT, &Blst).unwrap();
    let b = lc::verify_update(&follower.instance, update, &committee, NOW_SLOT, &Blst).unwrap();
    assert_eq!(a, b);
    assert_eq!(follower.instance, before);
}

#[test]
fn a_stalled_instance_resumes_without_a_gap() {
    // Reliability #4, #7: nobody advanced the instance for days; anyone can submit the
    // outstanding updates in order, and the result equals an instance that never stalled.
    let mut stalled = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let late = NOW_SLOT + 32 * 225 * 5; // five days later, still inside the weak-subjectivity period
    for update in updates() {
        stalled.process(&update, late).unwrap();
    }
    stalled.process(&finality_update(), late).unwrap();
    assert_eq!(stalled.instance, synced().instance);
}

#[test]
fn both_bls_libraries_agree_on_every_real_update() {
    use affidavit_core::bls::{BlsVerifier, Zkcrypto};
    let mut follower = Follower::new(mainnet_config(), &bootstrap(), NOW_SLOT).unwrap();
    let mut all = updates();
    all.push(finality_update());
    for update in &all {
        let committee = follower.signing_committee(update);
        let a = lc::verify_update(&follower.instance, update, &committee, NOW_SLOT, &Blst).unwrap();
        for zk in [Zkcrypto { check_subgroups: true }, Zkcrypto { check_subgroups: false }] {
            assert_eq!(lc::verify_update(&follower.instance, update, &committee, NOW_SLOT, &zk).unwrap(), a);
        }
        // And both refuse a corrupted signature.
        let mut bad = update.clone();
        bad.sync_aggregate.sync_committee_signature = all[0].sync_aggregate.sync_committee_signature;
        if bad.sync_aggregate.sync_committee_signature != update.sync_aggregate.sync_committee_signature {
            let signers: Vec<&[u8; 48]> = committee.pubkeys.iter().collect();
            let msg = alloy_primitives::B256::repeat_byte(1);
            assert!(!Blst.fast_aggregate_verify(&signers, &msg, &bad.sync_aggregate.sync_committee_signature));
            assert!(!Zkcrypto::default().fast_aggregate_verify(
                &signers,
                &msg,
                &bad.sync_aggregate.sync_committee_signature
            ));
        }
        follower.process(update, NOW_SLOT).unwrap();
    }
}
