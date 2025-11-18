use crate::trace::{AccessKind, TraceAccess};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionStrategy {
    None,
    Mru,
    MultiColumn,
}

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub cache_size: usize,    // in Bytes
    pub block_size: usize,    // in Bytes
    pub associativity: usize, // set to 1 for Direct-Mapped
    pub victim_cache_entries: usize,
    pub prediction: PredictionStrategy,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            cache_size: 256 * 1024,
            block_size: 32,
            associativity: 4,
            victim_cache_entries: 0,
            prediction: PredictionStrategy::None,
        }
    }
}

impl CacheConfig {
    pub fn num_sets(&self) -> usize {
        (self.cache_size / self.block_size).max(1) / self.associativity.max(1)
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub accesses: u64,
    pub reads: u64,
    pub writes: u64,
    pub hits: u64,
    pub misses: u64,
    pub victim_hits: u64,
    pub prediction: Option<PredictionStats>,
}

impl CacheStats {
    pub fn new(prediction: PredictionStrategy) -> Self {
        Self {
            accesses: 0,
            reads: 0,
            writes: 0,
            hits: 0,
            misses: 0,
            victim_hits: 0,
            prediction: match prediction {
                PredictionStrategy::None => None,
                mode => Some(PredictionStats::new(mode)),
            },
        }
    }

    pub fn hit_rate(&self) -> f64 {
        if self.accesses == 0 {
            0.0
        } else {
            self.hits as f64 / self.accesses as f64
        }
    }

    pub fn victim_hit_ratio(&self) -> f64 {
        if self.hits == 0 {
            0.0
        } else {
            self.victim_hits as f64 / self.hits as f64
        }
    }
}

#[derive(Debug, Clone)]
pub struct PredictionStats {
    pub mode: PredictionStrategy,
    pub first_hits: u64,
    pub non_first_hits: u64,
    pub total_hits_observed: u64,
    pub bit_vector_search_total: u64,
    pub bit_vector_observations: u64,
}

impl PredictionStats {
    fn new(mode: PredictionStrategy) -> Self {
        Self {
            mode,
            first_hits: 0,
            non_first_hits: 0,
            total_hits_observed: 0,
            bit_vector_search_total: 0,
            bit_vector_observations: 0,
        }
    }

    pub fn first_hit_rate(&self) -> f64 {
        if self.total_hits_observed == 0 {
            0.0
        } else {
            self.first_hits as f64 / self.total_hits_observed as f64
        }
    }

    pub fn non_first_hit_rate(&self) -> f64 {
        if self.total_hits_observed == 0 {
            0.0
        } else {
            self.non_first_hits as f64 / self.total_hits_observed as f64
        }
    }

    pub fn avg_bit_vector_search(&self) -> f64 {
        if self.bit_vector_observations == 0 {
            0.0
        } else {
            self.bit_vector_search_total as f64 / self.bit_vector_observations as f64
        }
    }
}

pub struct Cache {
    config: CacheConfig,
    sets: Vec<CacheSet>,
    victim: Option<VictimCache>,
    prediction_mode: PredictionStrategy,
    multi_column: Option<MultiColumnState>,
    global_tick: u64,
    num_sets: usize,
}

impl Cache {
    pub fn new(config: CacheConfig) -> Self {
        let num_sets = config.num_sets();
        let sets = (0..num_sets)
            .map(|_| CacheSet::new(config.associativity))
            .collect();
        let victim = if config.victim_cache_entries > 0 {
            Some(VictimCache::new(config.victim_cache_entries))
        } else {
            None
        };
        let prediction_mode = config.prediction;
        let multi_column = match prediction_mode {
            PredictionStrategy::MultiColumn => {
                Some(MultiColumnState::new(num_sets, config.associativity))
            }
            _ => None,
        };
        Self {
            config,
            sets,
            victim,
            prediction_mode,
            multi_column,
            global_tick: 1,
            num_sets,
        }
    }

    pub fn run_trace(&mut self, trace: &[TraceAccess]) -> CacheStats {
        let mut stats = CacheStats::new(self.prediction_mode);
        for access in trace {
            self.process_access(access, &mut stats);
        }
        stats
    }

    fn process_access(&mut self, access: &TraceAccess, stats: &mut CacheStats) {
        stats.accesses += 1;
        match access.kind {
            AccessKind::Read => stats.reads += 1,
            AccessKind::Write => stats.writes += 1,
        }
        let block_address = access.address / self.config.block_size as u64;
        let set_index = (block_address % self.num_sets as u64) as usize;
        let tag = block_address / self.num_sets as u64;

        let prediction_snapshot = self
            .prediction_observation(set_index, block_address)
            .unwrap_or(PredictionObservation::None);

        if let Some(line_idx) = self.sets[set_index].find_line(tag) {
            // Primary hit
            self.sets[set_index].touch(line_idx, self.global_tick, access.kind);
            self.update_multi_column_on_hit(set_index, block_address, line_idx);
            stats.hits += 1;
            self.record_prediction(&prediction_snapshot, Some(line_idx), stats);
            self.global_tick += 1;
            return;
        }

        // Cache miss - check victim if enabled
        let mut incoming_line = None;
        if let Some(victim) = self.victim.as_mut() {
            // If have victim cache extention
            if let Some(mut line) = victim.take(block_address) {
                line.last_used = self.global_tick;
                incoming_line = Some(line);
            }
        }

        if let Some(line_from_victim) = incoming_line {
            let (inserted_idx, evicted) = self.sets[set_index].install_existing(line_from_victim);
            if let Some(evicted_line) = evicted {
                self.multi_column_on_evict(set_index, &evicted_line);
                if let Some(victim) = self.victim.as_mut() {
                    victim.insert(evicted_line, self.global_tick);
                }
            }
            self.update_multi_column_on_hit(set_index, block_address, inserted_idx);
            stats.hits += 1;
            stats.victim_hits += 1;
        } else {
            // true miss, fetch from memory
            let is_write = matches!(access.kind, AccessKind::Write);
            let (inserted_idx, evicted) =
                self.sets[set_index].insert_new(tag, block_address, self.global_tick, is_write);
            if let Some(evicted_line) = evicted {
                self.multi_column_on_evict(set_index, &evicted_line);
                if let Some(victim) = self.victim.as_mut() {
                    victim.insert(evicted_line, self.global_tick);
                }
            }
            self.update_multi_column_on_hit(set_index, block_address, inserted_idx);
            stats.misses += 1;
        }
        self.global_tick += 1;
    }

    fn prediction_observation(
        &self,
        set_index: usize,
        block_address: u64,
    ) -> Option<PredictionObservation> {
        match self.prediction_mode {
            PredictionStrategy::None => None,
            PredictionStrategy::Mru => {
                let predicted = self.sets[set_index].mru_way();
                Some(PredictionObservation::Mru { predicted })
            }
            PredictionStrategy::MultiColumn => {
                if let Some(mc) = &self.multi_column {
                    let bits = mc.bits_for(set_index, block_address);
                    Some(PredictionObservation::MultiColumn { bits })
                } else {
                    None
                }
            }
        }
    }

    fn record_prediction(
        &mut self,
        observation: &PredictionObservation,
        actual_way: Option<usize>,
        stats: &mut CacheStats,
    ) {
        let pred_stats = match stats.prediction.as_mut() {
            Some(stats) => stats,
            None => return,
        };
        let Some(actual_way) = actual_way else {
            return;
        };
        pred_stats.total_hits_observed += 1;
        match observation {
            PredictionObservation::None => {
                pred_stats.non_first_hits += 1;
            }
            PredictionObservation::Mru { predicted } => {
                if predicted == &Some(actual_way) {
                    pred_stats.first_hits += 1;
                } else {
                    pred_stats.non_first_hits += 1;
                }
            }
            PredictionObservation::MultiColumn { bits } => {
                if *bits == 0 {
                    pred_stats.non_first_hits += 1;
                    pred_stats.bit_vector_observations += 1;
                    return;
                }
                let mask = 1u32 << actual_way;
                let mut rank = None;
                if bits & mask != 0 {
                    let before = bits & (mask - 1);
                    rank = Some(before.count_ones() + 1);
                }
                if let Some(rank) = rank {
                    if rank == 1 {
                        pred_stats.first_hits += 1;
                    } else {
                        pred_stats.non_first_hits += 1;
                    }
                    pred_stats.bit_vector_search_total += rank as u64;
                } else {
                    pred_stats.non_first_hits += 1;
                    pred_stats.bit_vector_search_total += bits.count_ones() as u64;
                }
                pred_stats.bit_vector_observations += 1;
            }
        }
    }

    fn update_multi_column_on_hit(&mut self, set_index: usize, block_address: u64, way: usize) {
        if let Some(mc) = self.multi_column.as_mut() {
            mc.set_bit(set_index, block_address, way);
        }
    }

    fn multi_column_on_evict(&mut self, set_index: usize, line: &CacheLine) {
        if let Some(mc) = self.multi_column.as_mut() {
            if line.valid {
                mc.clear_bit(set_index, line.block_address, line.way_hint);
            }
        }
    }
}

#[derive(Clone)]
struct CacheLine {
    tag: u64,
    block_address: u64,
    last_used: u64,
    valid: bool,
    dirty: bool,
    way_hint: usize,
}

impl CacheLine {
    fn invalid(way_hint: usize) -> Self {
        Self {
            tag: 0,
            block_address: 0,
            last_used: 0,
            valid: false,
            dirty: false,
            way_hint,
        }
    }
}

struct CacheSet {
    lines: Vec<CacheLine>,
}

impl CacheSet {
    fn new(ways: usize) -> Self {
        let mut lines = Vec::with_capacity(ways);
        for way in 0..ways {
            lines.push(CacheLine::invalid(way));
        }
        Self { lines }
    }

    fn find_line(&self, tag: u64) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| line.valid && line.tag == tag)
    }

    fn touch(&mut self, idx: usize, tick: u64, access: AccessKind) {
        if let Some(line) = self.lines.get_mut(idx) {
            line.last_used = tick;
            if matches!(access, AccessKind::Write) {
                line.dirty = true;
            }
        }
    }

    fn insert_new(
        &mut self,
        tag: u64,
        block_address: u64,
        tick: u64,
        dirty: bool,
    ) -> (usize, Option<CacheLine>) {
        let new_line = CacheLine {
            tag,
            block_address,
            last_used: tick,
            valid: true,
            dirty,
            way_hint: 0,
        };
        self.install_line(new_line)
    }

    fn install_existing(&mut self, line: CacheLine) -> (usize, Option<CacheLine>) {
        self.install_line(line)
    }

    fn install_line(&mut self, mut line: CacheLine) -> (usize, Option<CacheLine>) {
        line.valid = true;
        if let Some(idx) = self.lines.iter().position(|line| !line.valid) {
            line.way_hint = idx;
            self.lines[idx] = line;
            return (idx, None);
        }
        let idx = self
            .lines
            .iter()
            .enumerate()
            .min_by_key(|(_, line)| line.last_used)
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        let mut evicted = self.lines[idx].clone();
        evicted.way_hint = idx;
        line.way_hint = idx;
        self.lines[idx] = line;
        (idx, Some(evicted))
    }

    fn mru_way(&self) -> Option<usize> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.valid)
            .max_by_key(|(_, line)| line.last_used)
            .map(|(idx, _)| idx)
    }
}

struct VictimCache {
    entries: Vec<CacheLine>,
    capacity: usize,
}

impl VictimCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            capacity,
        }
    }

    fn take(&mut self, block_address: u64) -> Option<CacheLine> {
        if self.capacity == 0 {
            return None;
        }
        if let Some(idx) = self
            .entries
            .iter()
            .position(|line| line.block_address == block_address)
        {
            let line = self.entries.remove(idx);
            return Some(line);
        }
        None
    }

    fn insert(&mut self, mut line: CacheLine, tick: u64) {
        if self.capacity == 0 || !line.valid {
            return;
        }
        if self.entries.len() == self.capacity {
            let idx = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, line)| line.last_used)
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            self.entries.remove(idx);
        }
        line.last_used = tick;
        self.entries.push(line);
    }
}

#[derive(Clone, Copy)]
enum PredictionObservation {
    None,
    Mru { predicted: Option<usize> },
    MultiColumn { bits: u32 },
}

struct MultiColumnState {
    bits: Vec<u32>,
    num_sets: usize,
    num_columns: usize,
}

impl MultiColumnState {
    fn new(num_sets: usize, ways: usize) -> Self {
        let default_columns = match ways {
            0..=1 => 1,
            2..=4 => 2,
            5..=8 => 4,
            _ => 8,
        };
        let num_columns = default_columns.clamp(1, ways.max(1));
        Self {
            bits: vec![0u32; num_sets * num_columns],
            num_sets,
            num_columns,
        }
    }

    fn bits_for(&self, set_index: usize, block_address: u64) -> u32 {
        let column = self.column_index(block_address);
        self.bits[self.index(set_index, column)]
    }

    fn set_bit(&mut self, set_index: usize, block_address: u64, way: usize) {
        if way >= 32 {
            return;
        }
        let column = self.column_index(block_address);
        let idx = self.index(set_index, column);
        self.bits[idx] |= 1u32 << way;
    }

    fn clear_bit(&mut self, set_index: usize, block_address: u64, way: usize) {
        if way >= 32 {
            return;
        }
        let column = self.column_index(block_address);
        let idx = self.index(set_index, column);
        self.bits[idx] &= !(1u32 << way);
    }

    fn column_index(&self, block_address: u64) -> usize {
        if self.num_columns == 1 {
            0
        } else {
            let tag = block_address / self.num_sets as u64;
            (tag as usize) % self.num_columns
        }
    }

    fn index(&self, set_index: usize, column: usize) -> usize {
        set_index * self.num_columns + column
    }
}
