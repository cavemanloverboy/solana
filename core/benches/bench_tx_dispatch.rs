/// Benchmark: batch of N transactions vs N individual dispatches.
/// Measures per-batch dispatch overhead for both simple transfers and vote transactions.
///
/// Run with:
///   cargo bench -p solana-core --bench bench_tx_dispatch --features dev-context-only-utils
use std::{
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use {
    solana_clock::MAX_PROCESSING_AGE,
    solana_keypair::Keypair,
    solana_message::Message,
    solana_pubkey::Pubkey,
    solana_runtime::{
        bank::Bank,
        bank_forks::BankForks,
        genesis_utils::{
            create_genesis_config, create_genesis_config_with_vote_accounts, GenesisConfigInfo,
            ValidatorVoteKeypairs,
        },
    },
    solana_signer::Signer,
    solana_svm::transaction_processor::ExecutionRecordingConfig,
    solana_svm_timings::{ExecuteTimingType, ExecuteTimings},
    solana_transaction::{versioned::VersionedTransaction, Transaction},
    solana_vote::vote_transaction,
    solana_vote_program::vote_state::TowerSync,
};

const NUM_TXS: usize = 16;
const ITERATIONS: usize = 200;

fn make_transfer_tx(payer: &Keypair, recent_blockhash: solana_hash::Hash) -> Transaction {
    let message = Message::new(&[], Some(&payer.pubkey()));
    Transaction::new(&[payer], message, recent_blockhash)
}

fn setup_transfer() -> (solana_genesis_config::GenesisConfig, Keypair, Vec<Keypair>) {
    let GenesisConfigInfo {
        mut genesis_config,
        mint_keypair,
        ..
    } = create_genesis_config(u64::MAX / 2);
    genesis_config.fee_rate_governor = solana_fee_calculator::FeeRateGovernor::new(0, 0);
    let payers: Vec<Keypair> = (0..NUM_TXS).map(|_| Keypair::new()).collect();
    (genesis_config, mint_keypair, payers)
}

fn fresh_bank_transfer(
    gc: &solana_genesis_config::GenesisConfig,
    mk: &Keypair,
    payers: &[Keypair],
) -> (Arc<Bank>, Arc<RwLock<BankForks>>) {
    let (bank, bf) = Bank::new_with_bank_forks_for_tests(gc);
    let bh = bank.last_blockhash();
    for p in payers {
        bank.process_transaction(&solana_system_transaction::transfer(
            mk,
            &p.pubkey(),
            1_000_000_000,
            bh,
        ))
        .unwrap();
    }
    (bank, bf)
}

fn setup_vote() -> (
    solana_genesis_config::GenesisConfig,
    Vec<ValidatorVoteKeypairs>,
) {
    let validators: Vec<ValidatorVoteKeypairs> = (0..NUM_TXS)
        .map(|_| ValidatorVoteKeypairs::new_rand())
        .collect();
    let GenesisConfigInfo { genesis_config, .. } = create_genesis_config_with_vote_accounts(
        u64::MAX / 2,
        &validators,
        vec![1_000_000_000; NUM_TXS],
    );
    (genesis_config, validators)
}

/// Create bank0, freeze it, then create bank1 as a child.
/// Vote transactions vote on slot 0 and execute on bank1.
fn fresh_bank_vote(
    gc: &solana_genesis_config::GenesisConfig,
) -> (Arc<Bank>, Arc<Bank>, Arc<RwLock<BankForks>>) {
    let (bank0, bf) = Bank::new_with_bank_forks_for_tests(gc);
    bank0.freeze();
    let bank1 = Bank::new_from_parent(bank0.clone(), &Pubkey::default(), 1);
    let bank1 = bf.write().unwrap().insert(bank1).clone_without_scheduler();
    (bank0, bank1, bf)
}

fn make_vote_tx(
    validator: &ValidatorVoteKeypairs,
    parent_bank: &Bank,
    blockhash: solana_hash::Hash,
) -> Transaction {
    let tower_sync = TowerSync::new_from_slots(vec![parent_bank.slot()], parent_bank.hash(), None);
    vote_transaction::new_tower_sync_transaction(
        tower_sync,
        blockhash,
        &validator.node_keypair,
        &validator.vote_keypair,
        &validator.vote_keypair,
        None,
    )
}

#[derive(Default, Clone)]
struct Phases {
    prepare: Duration,
    load_exec_commit: Duration,
    unlock: Duration,
}
impl Phases {
    fn total(&self) -> Duration {
        self.prepare
            .saturating_add(self.load_exec_commit)
            .saturating_add(self.unlock)
    }
    fn add(&mut self, o: &Phases) {
        self.prepare = self.prepare.saturating_add(o.prepare);
        self.load_exec_commit = self.load_exec_commit.saturating_add(o.load_exec_commit);
        self.unlock = self.unlock.saturating_add(o.unlock);
    }
}

fn dispatch_timed(
    bank: &Bank,
    txs: Vec<VersionedTransaction>,
    timings: &mut ExecuteTimings,
) -> Phases {
    let t0 = Instant::now();
    let batch = bank.prepare_entry_batch(txs).unwrap();
    let prepare = t0.elapsed();

    let t1 = Instant::now();
    let (results, _) = bank.load_execute_and_commit_transactions(
        &batch,
        MAX_PROCESSING_AGE,
        ExecutionRecordingConfig::new_single_setting(false),
        timings,
        None,
    );
    let load_exec_commit = t1.elapsed();

    for (i, r) in results.iter().enumerate() {
        assert!(
            r.as_ref().map(|c| c.status.is_ok()).unwrap_or(false),
            "tx {i} failed: {r:?}"
        );
    }

    let t2 = Instant::now();
    drop(batch);
    let unlock = t2.elapsed();

    Phases {
        prepare,
        load_exec_commit,
        unlock,
    }
}

const FIELDS: &[(&str, ExecuteTimingType)] = &[
    ("check", ExecuteTimingType::CheckUs),
    ("validate_fees", ExecuteTimingType::ValidateFeesUs),
    ("load", ExecuteTimingType::LoadUs),
    ("execute", ExecuteTimingType::ExecuteUs),
    ("store", ExecuteTimingType::StoreUs),
    ("update_stakes", ExecuteTimingType::UpdateStakesCacheUs),
    ("update_executors", ExecuteTimingType::UpdateExecutorsUs),
    ("collect_logs", ExecuteTimingType::CollectLogsUs),
    (
        "update_tx_stat",
        ExecuteTimingType::UpdateTransactionStatuses,
    ),
    ("program_cache", ExecuteTimingType::ProgramCacheUs),
    ("filter_exec", ExecuteTimingType::FilterExecutableUs),
    ("collect_bal", ExecuteTimingType::CollectBalancesUs),
];

fn print_summary(label: &str, batch_times: &[Duration], individual_times: &[Duration]) {
    let mean = |v: &[Duration]| {
        v.iter()
            .sum::<Duration>()
            .checked_div((v.len().max(1)) as u32)
            .unwrap_or_default()
    };
    println!("Batch of {NUM_TXS}:     mean {:>10.2?}", mean(batch_times));
    println!(
        "{NUM_TXS} x individual: mean {:>10.2?}",
        mean(individual_times)
    );
    println!(
        "Overhead ratio:  {:.2}x  ({label})",
        mean(individual_times).as_nanos() as f64 / mean(batch_times).as_nanos() as f64
    );
}

fn print_phase_breakdown(bp: &Phases, ip: &Phases, n: u32) {
    println!(
        "{:<18} {:>10} {:>14} {:>10} {:>10}",
        "", "prepare", "load+exec+cmit", "unlock", "TOTAL"
    );
    let div_n = |d: Duration| d.checked_div(n).unwrap_or_default();
    println!(
        "{:<18} {:>10.2?} {:>14.2?} {:>10.2?} {:>10.2?}",
        "Batch of 16:",
        div_n(bp.prepare),
        div_n(bp.load_exec_commit),
        div_n(bp.unlock),
        div_n(bp.total()),
    );
    println!(
        "{:<18} {:>10.2?} {:>14.2?} {:>10.2?} {:>10.2?}",
        "16 x individual:",
        div_n(ip.prepare),
        div_n(ip.load_exec_commit),
        div_n(ip.unlock),
        div_n(ip.total()),
    );
    println!(
        "{:<18} {:>10.2}x {:>14.2}x {:>10.2}x {:>10.2}x",
        "Ratio:",
        ip.prepare.as_nanos() as f64 / bp.prepare.as_nanos() as f64,
        ip.load_exec_commit.as_nanos() as f64 / bp.load_exec_commit.as_nanos() as f64,
        ip.unlock.as_nanos() as f64 / bp.unlock.as_nanos() as f64,
        ip.total().as_nanos() as f64 / bp.total().as_nanos() as f64,
    );
}

fn print_timing_breakdown(
    bt: &ExecuteTimings,
    it: &ExecuteTimings,
    bp: &Phases,
    ip: &Phases,
    batch_dispatches: usize,
    indiv_dispatches: usize,
) {
    println!(
        "{:<18} {:>12} {:>12} {:>12}",
        "field", "1x16", "16x1", "delta"
    );
    for &(name, field) in FIELDS {
        let bv = bt.metrics[field].0 as f64 / batch_dispatches as f64;
        let iv = it.metrics[field].0 as f64 / indiv_dispatches as f64 * NUM_TXS as f64;
        println!("{:<18} {:>12.2} {:>12.2} {:>+12.2}", name, bv, iv, iv - bv);
    }

    let bw = bp.total().as_micros() as f64 / batch_dispatches as f64;
    let iw = ip.total().as_micros() as f64 / indiv_dispatches as f64 * NUM_TXS as f64;
    println!(
        "{:<18} {:>12.2} {:>12.2} {:>+12.2}",
        "WALL CLOCK",
        bw,
        iw,
        iw - bw
    );
}

fn bench_transfer() {
    let (gc, mk, payers) = setup_transfer();
    let n = ITERATIONS as u32;

    // ── Top-level comparison ──────────────────────────────────────────
    println!("\n=== Transfer: top-level ({NUM_TXS} txs, {ITERATIONS} iters) ===\n");
    let mut batch_times = Vec::with_capacity(ITERATIONS);
    let mut individual_times = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let (bank, _bf) = fresh_bank_transfer(&gc, &mk, &payers);
        let bh = bank.last_blockhash();
        let txs: Vec<Transaction> = payers.iter().map(|p| make_transfer_tx(p, bh)).collect();
        let s = Instant::now();
        bank.process_transactions(txs.iter())
            .iter()
            .for_each(|r| assert!(r.is_ok()));
        batch_times.push(s.elapsed());
    }
    for _ in 0..ITERATIONS {
        let (bank, _bf) = fresh_bank_transfer(&gc, &mk, &payers);
        let bh = bank.last_blockhash();
        let txs: Vec<Transaction> = payers.iter().map(|p| make_transfer_tx(p, bh)).collect();
        let s = Instant::now();
        txs.iter()
            .for_each(|tx| assert!(bank.process_transaction(tx).is_ok()));
        individual_times.push(s.elapsed());
    }
    batch_times.sort();
    individual_times.sort();
    print_summary("transfer", &batch_times, &individual_times);

    // ── Phase breakdown ───────────────────────────────────────────────
    let mut bp = Phases::default();
    let mut bt = ExecuteTimings::default();
    for _ in 0..ITERATIONS {
        let (bank, _bf) = fresh_bank_transfer(&gc, &mk, &payers);
        let bh = bank.last_blockhash();
        let vtxs: Vec<VersionedTransaction> = payers
            .iter()
            .map(|p| VersionedTransaction::from(make_transfer_tx(p, bh)))
            .collect();
        bp.add(&dispatch_timed(&bank, vtxs, &mut bt));
    }

    let mut ip = Phases::default();
    let mut it = ExecuteTimings::default();
    for _ in 0..ITERATIONS {
        let (bank, _bf) = fresh_bank_transfer(&gc, &mk, &payers);
        let bh = bank.last_blockhash();
        for p in &payers {
            let vtx = VersionedTransaction::from(make_transfer_tx(p, bh));
            ip.add(&dispatch_timed(&bank, vec![vtx], &mut it));
        }
    }

    println!("\n=== Transfer: phase breakdown (mean per iteration) ===\n");
    print_phase_breakdown(&bp, &ip, n);

    println!("\n=== Transfer: absolute cost per iteration (us) ===\n");
    print_timing_breakdown(&bt, &it, &bp, &ip, ITERATIONS, ITERATIONS * NUM_TXS);
}

fn bench_vote() {
    let (gc, validators) = setup_vote();
    let n = ITERATIONS as u32;

    // ── Top-level comparison ──────────────────────────────────────────
    println!("\n=== Vote: top-level ({NUM_TXS} txs, {ITERATIONS} iters) ===\n");
    let mut batch_times = Vec::with_capacity(ITERATIONS);
    let mut individual_times = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let (bank0, bank1, _bf) = fresh_bank_vote(&gc);
        let bh = bank1.last_blockhash();
        let txs: Vec<Transaction> = validators
            .iter()
            .map(|v| make_vote_tx(v, &bank0, bh))
            .collect();
        let s = Instant::now();
        bank1
            .process_transactions(txs.iter())
            .iter()
            .for_each(|r| assert!(r.is_ok(), "vote tx failed: {r:?}"));
        batch_times.push(s.elapsed());
    }
    for _ in 0..ITERATIONS {
        let (bank0, bank1, _bf) = fresh_bank_vote(&gc);
        let bh = bank1.last_blockhash();
        let txs: Vec<Transaction> = validators
            .iter()
            .map(|v| make_vote_tx(v, &bank0, bh))
            .collect();
        let s = Instant::now();
        txs.iter()
            .for_each(|tx| assert!(bank1.process_transaction(tx).is_ok(), "vote tx failed"));
        individual_times.push(s.elapsed());
    }
    batch_times.sort();
    individual_times.sort();
    print_summary("vote", &batch_times, &individual_times);

    // ── Phase breakdown ───────────────────────────────────────────────
    let mut bp = Phases::default();
    let mut bt = ExecuteTimings::default();
    for _ in 0..ITERATIONS {
        let (bank0, bank1, _bf) = fresh_bank_vote(&gc);
        let bh = bank1.last_blockhash();
        let vtxs: Vec<VersionedTransaction> = validators
            .iter()
            .map(|v| VersionedTransaction::from(make_vote_tx(v, &bank0, bh)))
            .collect();
        bp.add(&dispatch_timed(&bank1, vtxs, &mut bt));
    }

    let mut ip = Phases::default();
    let mut it = ExecuteTimings::default();
    for _ in 0..ITERATIONS {
        let (bank0, bank1, _bf) = fresh_bank_vote(&gc);
        let bh = bank1.last_blockhash();
        for v in &validators {
            let vtx = VersionedTransaction::from(make_vote_tx(v, &bank0, bh));
            ip.add(&dispatch_timed(&bank1, vec![vtx], &mut it));
        }
    }

    println!("\n=== Vote: phase breakdown (mean per iteration) ===\n");
    print_phase_breakdown(&bp, &ip, n);

    println!("\n=== Vote: absolute cost per iteration (us) ===\n");
    print_timing_breakdown(&bt, &it, &bp, &ip, ITERATIONS, ITERATIONS * NUM_TXS);
}

fn main() {
    bench_transfer();
    bench_vote();
}
