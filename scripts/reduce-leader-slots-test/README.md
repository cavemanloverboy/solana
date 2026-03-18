# Multinode Demo + Leader Schedule Verification

Instructions for running a multi-validator cluster and verifying the `reduce_consecutive_leader_slots` feature via the leader schedule.

**Slots per leader:** Feature **inactive** → 4 slots. Feature **active** → 2 slots.

## Quick Summary

1. **Setup** – Genesis with feature deactivated (starts at 4 slots/leader)
2. **Start validators** – Faucet, bootstrap, second validator
3. **Delegate stake** – Give second validator stake so both appear in leader schedule
4. **Feature state** – Activate to test 4→2 transition

Feature ID: `9N4TN7bBtviskXWRo8pFcvp4caa6pPUQTx9BRWyeYLo`

## READ THIS

The pubkey `9N4TN7bBtviskXWRo8pFcvp4caa6pPUQTx9BRWyeYLo` above is the real one. For this demo to work you will have to generate your own feature keypair and replace the pubkey everywhere in the codebase and in this script.

---

## 1. Setup Genesis

Create genesis with short epochs (32 slots) and the feature **deactivated** so you start at 4 slots/leader:

```bash
./multinode-demo/setup.sh --slots-per-epoch 128 \
  --deactivate-feature 9N4TN7bBtviskXWRo8pFcvp4caa6pPUQTx9BRWyeYLo \
  --faucet-lamports 10000000000000000 \
  --bootstrap-validator-stake-lamports 10000000000
```

Faucet: 10M SOL. Bootstrap stake: 10 SOL (for equal stake with second validator).

---

## 2. Start Validators

**Terminal 1 – Faucet:**

```bash
./multinode-demo/faucet.sh
```

**Terminal 2 – Bootstrap:**

```bash
./multinode-demo/bootstrap-validator.sh --no-restart --dynamic-port-range 8000-8200
```

Wait ~30 seconds for the bootstrap to produce blocks and create its first snapshot (every 200 slots), then:

**Terminal 3 – Second validator:** (appears delinquent until it catches up; monitor with `solana catchup --our-localhost 18899 -u http://localhost:8899`)

```bash
./multinode-demo/validator.sh \
  --no-restart \
  --dynamic-port-range 8200-8400 \
  --init-complete-file init-complete-node1.log \
  --rpc-port 18899
```

---

## 3. Delegate Stake

The leader schedule only includes validators with stake. Bootstrap has **10 SOL** from genesis; the second validator has none until you delegate. Run this **after both validators are running** (the second validator creates its vote account on startup).

**Minimum stake:** ~1.1 SOL (rent + minimum delegation). Use **10 SOL** to match bootstrap stake.

```bash
./multinode-demo/delegate-stake.sh \
  --url http://localhost:8899 \
  --vote-account config/validator/vote-account.json \
  --stake-account config/validator/stake-account.json \
  --keypair config/validator/identity.json \
  --no-airdrop \
  10
```

**Airdrop:** Omit `--no-airdrop` to fund the stake from the faucet (faucet must be running). With `--no-airdrop`, the keypair must already have 10+ SOL.

**"stake action is not permitted while the epoch rewards period is active"**: Wait for the next epoch (~30s with 32 slots), then retry the same command.

Recall there are warmup epochs... will need to wait for stake to fully activate (lol)

---

## 4. Feature State: 4 → 2 Slots

There is **one feature** (`reduce_consecutive_leader_slots`). Ensure it is deactivated in genesis so you start at **4 slots/leader**; then activate to get **2 slots/leader**.

Activate the feature once validators are running (wait several warmup epochs):

```bash
./scripts/reduce-leader-slots-test/1-activate-feature.sh
```

---

## Verify via Leader Schedule

1. **Check activation epoch** – Use the feature status script to see when the feature activates:

```bash
./scripts/reduce-leader-slots-test/2-check-feature-status.sh
```

2. **Note the activation epoch** from the script output.

3. **Check that epoch** – Leader schedules are computed one epoch in advance, so the activation epoch will still show the 4-slot pattern (A A A A B B B B …):

```bash
solana leader-schedule -u http://localhost:8899 --epoch <activation_epoch>
```

4. **Check the next epoch** – The 2-slot pattern (A A B B A A B B …) appears in the following epoch:

```bash
solana leader-schedule -u http://localhost:8899 --epoch <activation_epoch+1>
```

---

## Reset Demo

```bash
rm -rf config/bootstrap-validator config/validator
./multinode-demo/setup.sh --slots-per-epoch 128 \
  --deactivate-feature 9N4TN7bBtviskXWRo8pFcvp4caa6pPUQTx9BRWyeYLo \
  --faucet-lamports 10000000000000000 \
  --bootstrap-validator-stake-lamports 10000000000
```

