use {
    crate::bank::Bank,
    agave_feature_set::reduce_consecutive_leader_slots,
    solana_clock::{Epoch, NUM_CONSECUTIVE_LEADER_SLOTS, Slot},
    solana_leader_schedule::LeaderSchedule,
    solana_pubkey::Pubkey,
    std::collections::HashMap,
};

/// Default number of consecutive slots per leader when the feature is inactive.
pub const DEFAULT_NUM_CONSECUTIVE_LEADER_SLOTS: u64 = NUM_CONSECUTIVE_LEADER_SLOTS;

/// Number of consecutive slots per leader when the feature is active.
pub const REDUCED_NUM_CONSECUTIVE_LEADER_SLOTS: u64 = 2;

/// Returns the activation slot for the reduce_consecutive_leader_slots feature,
/// or None if not active and not pending.
fn reduce_consecutive_leader_slots_activation_slot(bank: &Bank) -> Option<Slot> {
    let feature_id = reduce_consecutive_leader_slots::id();
    if let Some(slot) = bank.feature_set.activated_slot(&feature_id) {
        return Some(slot);
    }
    bank.compute_pending_activation_slot(&feature_id)
}

/// Returns the number of consecutive leader slots for the given slot.
pub fn num_consecutive_leader_slots(slot: Slot, bank: &Bank) -> u64 {
    if let Some(activation_slot) = reduce_consecutive_leader_slots_activation_slot(bank) {
        if slot >= activation_slot {
            return REDUCED_NUM_CONSECUTIVE_LEADER_SLOTS;
        }
    }
    DEFAULT_NUM_CONSECUTIVE_LEADER_SLOTS
}

/// Returns the number of consecutive leader slots for the first slot of the given epoch.
/// Used when computing the leader schedule for an epoch.
pub fn num_consecutive_leader_slots_for_epoch(epoch: Epoch, bank: &Bank) -> u64 {
    let first_slot = bank.epoch_schedule().get_first_slot_in_epoch(epoch);
    if let Some(activation_slot) = reduce_consecutive_leader_slots_activation_slot(bank) {
        if first_slot >= activation_slot {
            return REDUCED_NUM_CONSECUTIVE_LEADER_SLOTS;
        }
    }
    DEFAULT_NUM_CONSECUTIVE_LEADER_SLOTS
}

/// Return the leader schedule for the given epoch.
pub fn leader_schedule(epoch: Epoch, bank: &Bank) -> Option<LeaderSchedule> {
    let repeat = num_consecutive_leader_slots_for_epoch(epoch, bank);
    bank.epoch_vote_accounts(epoch).map(|vote_accounts_map| {
        LeaderSchedule::new(
            vote_accounts_map,
            epoch,
            bank.get_slots_in_epoch(epoch),
            repeat,
        )
    })
}

/// Map of leader base58 identity pubkeys to the slot indices relative to the first epoch slot
pub type LeaderScheduleByIdentity = HashMap<String, Vec<usize>>;

pub fn leader_schedule_by_identity<'a>(
    upcoming_leaders: impl Iterator<Item = (usize, &'a Pubkey)>,
) -> LeaderScheduleByIdentity {
    let mut leader_schedule_by_identity = HashMap::new();

    for (slot_index, identity_pubkey) in upcoming_leaders {
        leader_schedule_by_identity
            .entry(identity_pubkey)
            .or_insert_with(Vec::new)
            .push(slot_index);
    }

    leader_schedule_by_identity
        .into_iter()
        .map(|(identity_pubkey, slot_indices)| (identity_pubkey.to_string(), slot_indices))
        .collect()
}

/// Return the leader for the given slot.
pub fn slot_leader_at(slot: Slot, bank: &Bank) -> Option<Pubkey> {
    let (epoch, slot_index) = bank.get_epoch_and_slot_index(slot);

    leader_schedule(epoch, bank).map(|leader_schedule| leader_schedule[slot_index].id)
}

// Returns the number of ticks remaining from the specified tick_height to the end of the
// slot implied by the tick_height
pub fn num_ticks_left_in_slot(bank: &Bank, tick_height: u64) -> u64 {
    bank.ticks_per_slot() - tick_height % bank.ticks_per_slot()
}

pub fn first_of_consecutive_leader_slots(slot: Slot) -> Slot {
    (slot / NUM_CONSECUTIVE_LEADER_SLOTS) * NUM_CONSECUTIVE_LEADER_SLOTS
}

/// Returns the first slot in the leader window that contains `slot`.
/// Uses the feature-aware value when a bank is provided.
#[inline]
pub fn first_of_consecutive_leader_slots_with_bank(slot: Slot, bank: &Bank) -> Slot {
    let n = num_consecutive_leader_slots(slot, bank);
    (slot / n) * n
}

/// Returns the last slot in the leader window that contains `slot`
#[inline]
pub fn last_of_consecutive_leader_slots(slot: Slot) -> Slot {
    first_of_consecutive_leader_slots(slot) + NUM_CONSECUTIVE_LEADER_SLOTS - 1
}

/// Returns the last slot in the leader window that contains `slot`.
/// Uses the feature-aware value when a bank is provided.
#[inline]
pub fn last_of_consecutive_leader_slots_with_bank(slot: Slot, bank: &Bank) -> Slot {
    let n = num_consecutive_leader_slots(slot, bank);
    first_of_consecutive_leader_slots_with_bank(slot, bank) + n - 1
}

/// Returns the index within the leader slot range that contains `slot`
#[inline]
pub fn leader_slot_index(slot: Slot) -> usize {
    (slot % NUM_CONSECUTIVE_LEADER_SLOTS) as usize
}

/// Returns the index within the leader slot range that contains `slot`.
/// Uses the feature-aware value when a bank is provided.
#[inline]
pub fn leader_slot_index_with_bank(slot: Slot, bank: &Bank) -> usize {
    let n = num_consecutive_leader_slots(slot, bank);
    (slot % n) as usize
}

/// Returns the number of slots left after `slot` in the leader window
/// that contains `slot`
#[inline]
pub fn remaining_slots_in_window(slot: Slot) -> u64 {
    NUM_CONSECUTIVE_LEADER_SLOTS
        .checked_sub(leader_slot_index(slot) as u64)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::genesis_utils::{
            bootstrap_validator_stake_lamports, create_genesis_config_with_leader,
        },
    };

    #[test]
    fn test_leader_schedule_via_bank() {
        let pubkey = solana_pubkey::new_rand();
        let genesis_config =
            create_genesis_config_with_leader(0, &pubkey, bootstrap_validator_stake_lamports())
                .genesis_config;

        let bank = Bank::new_for_tests(&genesis_config);
        let leader_schedule = leader_schedule(0, &bank).unwrap();

        assert_eq!(leader_schedule[0].id, pubkey);
        assert_eq!(leader_schedule[1].id, pubkey);
        assert_eq!(leader_schedule[2].id, pubkey);
    }

    #[test]
    fn test_leader_scheduler1_basic() {
        let pubkey = solana_pubkey::new_rand();
        let genesis_config =
            create_genesis_config_with_leader(42, &pubkey, bootstrap_validator_stake_lamports())
                .genesis_config;
        let bank = Bank::new_for_tests(&genesis_config);
        assert_eq!(slot_leader_at(bank.slot(), &bank).unwrap(), pubkey);
    }

    #[test]
    fn test_leader_span_math() {
        // All of the test cases assume a 4 slot leader span and need to be
        // adjusted if it changes.
        assert_eq!(NUM_CONSECUTIVE_LEADER_SLOTS, 4);

        assert_eq!(first_of_consecutive_leader_slots(0), 0);
        assert_eq!(first_of_consecutive_leader_slots(1), 0);
        assert_eq!(first_of_consecutive_leader_slots(2), 0);
        assert_eq!(first_of_consecutive_leader_slots(3), 0);
        assert_eq!(first_of_consecutive_leader_slots(4), 4);

        assert_eq!(last_of_consecutive_leader_slots(0), 3);
        assert_eq!(last_of_consecutive_leader_slots(1), 3);
        assert_eq!(last_of_consecutive_leader_slots(2), 3);
        assert_eq!(last_of_consecutive_leader_slots(3), 3);
        assert_eq!(last_of_consecutive_leader_slots(4), 7);

        assert_eq!(leader_slot_index(0), 0);
        assert_eq!(leader_slot_index(1), 1);
        assert_eq!(leader_slot_index(2), 2);
        assert_eq!(leader_slot_index(3), 3);
        assert_eq!(leader_slot_index(4), 0);
        assert_eq!(leader_slot_index(5), 1);
        assert_eq!(leader_slot_index(6), 2);
        assert_eq!(leader_slot_index(7), 3);

        assert_eq!(remaining_slots_in_window(0), 4);
        assert_eq!(remaining_slots_in_window(1), 3);
        assert_eq!(remaining_slots_in_window(2), 2);
        assert_eq!(remaining_slots_in_window(3), 1);
        assert_eq!(remaining_slots_in_window(4), 4);
    }
}
