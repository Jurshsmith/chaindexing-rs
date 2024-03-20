#[derive(Clone)]
pub struct PruningConfig {
    /// Retains events inserted within the max age specified
    /// below. Unit in seconds.
    pub prune_n_blocks_away: u64,
    /// Advnace option for how often stale data gets pruned.
    /// Unit in seconds.
    pub prune_interval: u64,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            prune_n_blocks_away: 1_000,
            prune_interval: 12 * 60 * 60,
        }
    }
}

impl PruningConfig {
    pub fn get_min_block_number(&self, current_block_number: u64) -> u64 {
        if current_block_number < self.prune_n_blocks_away {
            current_block_number
        } else {
            current_block_number - self.prune_n_blocks_away
        }
    }
}
