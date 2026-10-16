//! Operator contention simulation (Reliability #9, #10; Supportability #7).
//!
//! Several independent operators follow the same instance. For every update each one
//! notices it after its own latency, waits its deferral (`operator::deferral_ms`), reads
//! the instance, and only if the update is still needed does the work: proving it (on the
//! private-verifier path), reading the instance again, and submitting. The first
//! submission to land wins; a submission already in flight when another lands is refused
//! and its gas is wasted. Proofs started for an update somebody else then lands are
//! wasted compute.
//!
//! The optional hint: an operator that starts proving announces it over Logos Messaging
//! with its own ETA. Others that see the hint wait until that ETA (capped) before reading
//! the instance again. The hint is never trusted: a silent or lying announcer costs at most
//! the cap in delay, never safety, and nobody needs permission to ignore it.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use affidavit_core::operator::deferral_ms;
use alloy_primitives::{B256, U256};

#[derive(Clone, Copy)]
struct Operator {
    name: &'static str,
    /// Time from the update appearing on a beacon node to the operator noticing it (ms).
    latency_ms: u64,
    /// Time to prove the update on this operator's hardware (ms); 0 on the public path.
    prove_ms: u64,
}

#[derive(Clone, Copy)]
struct Strategy {
    window_ms: u64,
    announce: bool,
}

const GOSSIP_MS: u64 = 1_000;
const HINT_CAP_MS: u64 = 1_800_000;

#[derive(Default)]
struct Totals {
    paid: Vec<u64>,
    wasted_gas: u64,
    wasted_proofs: u64,
    latencies: Vec<u64>,
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum Event {
    /// The operator reads the instance and decides whether to start.
    Check(usize),
    /// The operator finished proving; it reads the instance again and submits if needed.
    Proved(usize),
    /// A submission is included.
    Lands(usize),
}

fn simulate(operators: &[Operator], strategy: Strategy, inclusion_ms: u64, updates: u64) -> Totals {
    let mut totals = Totals { paid: vec![0; operators.len()], ..Totals::default() };
    let mut rng = 7u64;
    for k in 0..updates {
        let digest = B256::from(U256::from(k));
        let mut queue = BinaryHeap::new();
        for (i, op) in operators.iter().enumerate() {
            let jitter = splitmix(&mut rng) % (op.latency_ms / 2 + 1);
            let key = B256::repeat_byte(i as u8 + 1);
            queue.push(Reverse((
                op.latency_ms + jitter + deferral_ms(&key, &digest, strategy.window_ms),
                Event::Check(i),
            )));
        }
        let mut landed: Option<(u64, usize)> = None;
        let mut hint: Option<u64> = None; // ETA of the first announced proof
        let mut honoured = vec![false; operators.len()];
        let mut started = 0u64;
        let mut submitted = 0u64;
        while let Some(Reverse((now, event))) = queue.pop() {
            match event {
                Event::Check(i) => {
                    if landed.is_some() {
                        continue;
                    }
                    if strategy.announce {
                        if let Some(eta) = hint {
                            if eta > now && !honoured[i] {
                                honoured[i] = true;
                                queue.push(Reverse((eta.min(now + HINT_CAP_MS) + GOSSIP_MS, Event::Check(i))));
                                continue;
                            }
                        }
                    }
                    started += 1;
                    let done = now + operators[i].prove_ms;
                    if strategy.announce && hint.is_none() {
                        hint = Some(done + inclusion_ms);
                    }
                    queue.push(Reverse((done, Event::Proved(i))));
                }
                Event::Proved(i) => {
                    if landed.is_some() {
                        continue;
                    }
                    submitted += 1;
                    queue.push(Reverse((now + inclusion_ms, Event::Lands(i))));
                }
                Event::Lands(i) => {
                    if landed.is_none() {
                        landed = Some((now, i));
                    }
                }
            }
        }
        let (at, winner) = landed.expect("someone submits");
        totals.paid[winner] += 1;
        totals.wasted_gas += submitted - 1;
        totals.wasted_proofs += started - 1;
        totals.latencies.push(at);
    }
    totals
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

fn report(
    title: &str,
    operators: &[Operator],
    strategies: &[Strategy],
    inclusion_ms: u64,
    updates: u64,
    proving: bool,
) {
    println!("### {title}\n");
    let names: Vec<&str> = operators.iter().map(|o| o.name).collect();
    let extra = if proving { " wasted proofs /100 |" } else { "" };
    println!(
        "| strategy | share of updates paid ({}) | wasted submissions /100 |{extra} median / p95 latency |",
        names.join(", ")
    );
    println!("|---|---|---:|{}---|", if proving { "---:|" } else { "" });
    for &s in strategies {
        let mut t = simulate(operators, s, inclusion_ms, updates);
        t.latencies.sort_unstable();
        let shares: Vec<String> =
            t.paid.iter().map(|p| format!("{:.0}%", 100.0 * *p as f64 / updates as f64)).collect();
        let label = match (s.window_ms, s.announce) {
            (0, false) => "submit on sight".to_string(),
            (w, false) => format!("deferral window {} s", w / 1000),
            (w, true) => format!("deferral {} s + proving hint", w / 1000),
        };
        let proofs =
            if proving { format!(" {:.1} |", 100.0 * t.wasted_proofs as f64 / updates as f64) } else { String::new() };
        println!(
            "| {label} | {} | {:.1} |{proofs} {:.0} s / {:.0} s |",
            shares.join(" / "),
            100.0 * t.wasted_gas as f64 / updates as f64,
            percentile(&t.latencies, 0.5) as f64 / 1000.0,
            percentile(&t.latencies, 0.95) as f64 / 1000.0,
        );
    }
    println!();
}

fn main() {
    let updates = 20_000;
    let inclusion_ms = 6_000;
    // Five operators, fastest first. The fastest is co-located with a beacon node.
    let latencies = [300, 1_200, 2_500, 4_000, 8_000];
    let names = ["A", "B", "C", "D", "E"];

    let public: Vec<Operator> =
        names.iter().zip(latencies).map(|(name, latency_ms)| Operator { name, latency_ms, prove_ms: 0 }).collect();
    let windows = [0, 30_000, 120_000, 300_000];
    let strategies: Vec<Strategy> = windows.iter().map(|&w| Strategy { window_ms: w, announce: false }).collect();
    report(
        "Public path: the sequencer executes the verification, nothing to prove",
        &public,
        &strategies,
        inclusion_ms,
        updates,
        false,
    );

    // Private-verifier path: the operator first proves the update (about 15M cycles): about
    // two minutes on a GPU, about twenty on a laptop CPU.
    let proving = [120_000, 150_000, 1_200_000, 1_400_000, 100_000];
    let private: Vec<Operator> = names
        .iter()
        .zip(latencies)
        .zip(proving)
        .map(|((name, latency_ms), prove_ms)| Operator { name, latency_ms, prove_ms })
        .collect();
    let strategies = [
        Strategy { window_ms: 0, announce: false },
        Strategy { window_ms: 600_000, announce: false },
        Strategy { window_ms: 1_800_000, announce: false },
        Strategy { window_ms: 600_000, announce: true },
        Strategy { window_ms: 1_800_000, announce: true },
    ];
    report(
        "Private-verifier path: prove, then submit (A, B, E on GPUs; C, D on laptop CPUs)",
        &private,
        &strategies,
        inclusion_ms,
        updates,
        true,
    );
}
