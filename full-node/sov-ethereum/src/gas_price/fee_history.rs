//! Consist of types adjacent to the fee history cache and its configs
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use ethers::types::H256;
use reth_primitives::U256;
use reth_rpc_types::{
    Block, BlockTransactions, Rich, Transaction, TransactionReceipt, TxGasAndReward,
};
use schnellru::{ByLength, LruMap};
use serde::{Deserialize, Serialize};
use sov_evm::EthApiError;
use sov_modules_api::WorkingSet;

use super::cache::BlockCache;
use super::gas_oracle::{
    convert_u256_to_u128, convert_u256_to_u64, effective_gas_tip, MAX_HEADER_HISTORY,
};

/// Settings for the [FeeHistoryCache].
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeHistoryCacheConfig {
    /// Max number of blocks in cache.
    ///
    /// Default is [MAX_HEADER_HISTORY] plus some change to also serve slightly older blocks from
    /// cache, since fee_history supports the entire range
    pub max_blocks: u64,
    /// Percentile approximation resolution
    ///
    /// Default is 4 which means 0.25
    pub resolution: u64,
}

impl Default for FeeHistoryCacheConfig {
    fn default() -> Self {
        FeeHistoryCacheConfig {
            max_blocks: MAX_HEADER_HISTORY + 100,
            resolution: 4,
        }
    }
}

/// Wrapper struct for BTreeMap
pub struct FeeHistoryCache<C: sov_modules_api::Context> {
    /// Config for FeeHistoryCache, consists of resolution for percentile approximation
    /// and max number of blocks
    config: FeeHistoryCacheConfig,
    /// Stores the entries of the cache
    entries: Mutex<LruMap<u64, FeeHistoryEntry, ByLength>>,
    /// Block cache
    block_cache: Arc<BlockCache<C>>,
}

impl<C: sov_modules_api::Context> FeeHistoryCache<C> {
    /// Creates new FeeHistoryCache instance, initialize it with the mose recent data, set bounds
    pub fn new(config: FeeHistoryCacheConfig, block_cache: Arc<BlockCache<C>>) -> Self {
        let max_blocks = config.max_blocks;
        Self {
            config,
            entries: Mutex::new(LruMap::new(ByLength::new(max_blocks as u32))),
            block_cache,
        }
    }

    /// How the cache is configured.
    #[inline]
    pub fn config(&self) -> &FeeHistoryCacheConfig {
        &self.config
    }

    /// Returns the configured resolution for percentile approximation.
    #[inline]
    pub fn resolution(&self) -> u64 {
        self.config().resolution
    }

    /// Processing of the arriving blocks
    pub fn insert_blocks<I>(&self, entries: &mut LruMap<u64, FeeHistoryEntry, ByLength>, blocks: I)
    where
        I: Iterator<Item = (Rich<Block>, Vec<TransactionReceipt>)>,
    {
        let percentiles = self.predefined_percentiles();
        // Insert all new blocks and calculate approximated rewards
        for (block, receipts) in blocks {
            let mut fee_history_entry = FeeHistoryEntry::new(&block);
            let transactions = match &block.transactions {
                BlockTransactions::Full(transactions) => transactions,
                _ => unreachable!(),
            };
            fee_history_entry.rewards = calculate_reward_percentiles_for_block(
                &percentiles,
                fee_history_entry.gas_used,
                fee_history_entry.base_fee_per_gas,
                transactions,
                &receipts,
            )
            .unwrap_or_default();
            let block_number =
                convert_u256_to_u64(block.header.number.unwrap_or_default()).unwrap_or_default();
            entries.insert(block_number, fee_history_entry);
        }
    }

    /// Collect fee history for given range.
    ///
    /// This function retrieves fee history entries from the cache for the specified range.
    /// If the requested range (start_block to end_block) is within the cache bounds,
    /// it returns the corresponding entries.
    /// Otherwise it returns None.
    pub fn get_history(
        &self,
        start_block: u64,
        end_block: u64,
        working_set: &mut WorkingSet<C>,
    ) -> Vec<FeeHistoryEntry> {
        let mut entries = self.entries.lock().unwrap();

        let mut result = Vec::new();
        let mut empty_blocks = Vec::new();
        for block_number in start_block..=end_block {
            let entry = entries.get(&block_number);

            // if entry, push to result
            if let Some(entry) = entry {
                result.push(entry.clone());
                continue;
            } else {
                result.push(FeeHistoryEntry::default());
                empty_blocks.push(block_number);
            }
        }

        // Get blocks from cache (fallback rpc) and receipts from rpc
        let blocks_with_receipts = empty_blocks.clone().into_iter().filter_map(|block_number| {
            self.block_cache
                .get_block_with_receipts(block_number, working_set)
                .unwrap_or(None)
        });

        // Insert blocks with receipts into cache
        self.insert_blocks(&mut entries, blocks_with_receipts);

        // Get entries from cache for empty blocks
        for block_number in empty_blocks {
            let entry = entries.get(&block_number);
            if let Some(entry) = entry {
                result[block_number as usize - start_block as usize] = entry.clone();
            }
        }

        result
    }

    /// Generates predefined set of percentiles
    ///
    /// This returns 100 * resolution points
    pub fn predefined_percentiles(&self) -> Vec<f64> {
        let res = self.resolution() as f64;
        (0..=100 * self.resolution())
            .map(|p| p as f64 / res)
            .collect()
    }
}

/// Calculates reward percentiles for transactions in a block header.
/// Given a list of percentiles and a sealed block header, this function computes
/// the corresponding rewards for the transactions at each percentile.
///
/// The results are returned as a vector of U256 values.
pub(crate) fn calculate_reward_percentiles_for_block(
    percentiles: &[f64],
    gas_used: u64,
    base_fee_per_gas: u64,
    transactions: &[Transaction],
    receipts: &[TransactionReceipt],
) -> Result<Vec<U256>, EthApiError> {
    let mut transactions = transactions
        .iter()
        .zip(receipts)
        .scan(0, |previous_gas, (tx, receipt)| {
            // Convert the cumulative gas used in the receipts
            // to the gas usage by the transaction
            //
            // While we will sum up the gas again later, it is worth
            // noting that the order of the transactions will be different,
            // so the sum will also be different for each receipt.
            let cumulative_gas_used =
                convert_u256_to_u64(receipt.cumulative_gas_used).unwrap_or_default();
            let gas_used = cumulative_gas_used - *previous_gas;
            *previous_gas = cumulative_gas_used;

            Some(TxGasAndReward {
                gas_used,
                reward: convert_u256_to_u128(
                    effective_gas_tip(tx, Some(U256::from(base_fee_per_gas))).unwrap_or_default(),
                )
                .unwrap(),
            })
        })
        .collect::<Vec<_>>();

    // Sort the transactions by their rewards in ascending order
    transactions.sort_by_key(|tx| tx.reward);

    // Find the transaction that corresponds to the given percentile
    //
    // We use a `tx_index` here that is shared across all percentiles, since we know
    // the percentiles are monotonically increasing.
    let mut tx_index = 0;
    let mut cumulative_gas_used = transactions
        .first()
        .map(|tx| tx.gas_used)
        .unwrap_or_default();
    let mut rewards_in_block = Vec::new();
    for percentile in percentiles {
        // Empty blocks should return in a zero row
        if transactions.is_empty() {
            rewards_in_block.push(U256::ZERO);
            continue;
        }

        let threshold = (gas_used as f64 * percentile / 100.) as u64;
        while cumulative_gas_used < threshold && tx_index < transactions.len() - 1 {
            tx_index += 1;
            cumulative_gas_used += transactions[tx_index].gas_used;
        }
        rewards_in_block.push(U256::from(transactions[tx_index].reward));
    }

    Ok(rewards_in_block)
}

/// A cached entry for a block's fee history.
#[derive(Debug, Clone, Default)]
pub struct FeeHistoryEntry {
    /// The base fee per gas for this block.
    pub base_fee_per_gas: u64,
    /// Gas used ratio this block.
    pub gas_used_ratio: f64,
    /// Gas used by this block.
    pub gas_used: u64,
    /// Gas limit by this block.
    pub gas_limit: u64,
    /// Hash of the block.
    pub header_hash: H256,
    /// Approximated rewards for the configured percentiles.
    pub rewards: Vec<U256>,
}

impl FeeHistoryEntry {
    /// Creates a new entry from a sealed block.
    ///
    /// Note: This does not calculate the rewards for the block.
    pub fn new(block: &Rich<Block>) -> Self {
        let base_fee_per_gas =
            convert_u256_to_u64(block.header.base_fee_per_gas.unwrap_or_default()).unwrap();

        let gas_used = convert_u256_to_u64(block.header.gas_used).unwrap_or_default();
        let gas_limit = convert_u256_to_u64(block.header.gas_limit).unwrap_or_default();
        let gas_used_ratio = gas_used as f64 / gas_limit as f64;

        FeeHistoryEntry {
            base_fee_per_gas,
            gas_used_ratio,
            gas_used,
            header_hash: block.header.hash.unwrap_or_default().into(),
            gas_limit,
            rewards: Vec::new(),
        }
    }
}