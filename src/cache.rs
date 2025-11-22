use crate::trace::{AccessKind, TraceAccess};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionStrategy {
    None,
    Mru,
    MultiColumn,
}

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub cache_size: usize,    // Bytes
    pub block_size: usize,    // Bytes
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
        let blocks = (self.cache_size / self.block_size).max(1);
        let ways = self.associativity.max(1);
        (blocks / ways).max(1)
    }
}

// ===== Cache Stat Utility =====

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
    sets: Vec<Vec<Option<CacheLine>>>,
    victim: Option<VictimBuffer>,
    prediction_mode: PredictionStrategy,
    multi_predictor: Option<MultiColumnPredictor>,
    next_stamp: u64,
    num_sets: usize,
}

impl Cache {
    pub fn new(config: CacheConfig) -> Self {
        let num_sets = config.num_sets();
        let ways = config.associativity.max(1);
        let sets = (0..num_sets)
            .map(|_| vec![None; ways])
            .collect::<Vec<_>>();
        let victim = if config.victim_cache_entries > 0 {
            Some(VictimBuffer::new(config.victim_cache_entries))
        } else {
            None
        };
        let prediction_mode = config.prediction;
        let multi_predictor = match prediction_mode {
            PredictionStrategy::MultiColumn => {
                Some(MultiColumnPredictor::new(num_sets, ways))
            }
            _ => None,
        };
        Self {
            config,
            sets,
            victim,
            prediction_mode,
            multi_predictor,
            next_stamp: 1,
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

        // Capture what the predictor believes before mutate the state.
        let observation = self.observe_prediction(set_index, block_address);

        if let Some((way, is_first_hit)) = self.touch_if_hit(set_index, tag) {
            stats.hits += 1;
            self.update_multi_column_on_hit(set_index, block_address, way);
            self.record_prediction(&observation, Some((way, is_first_hit)), stats);
            self.next_stamp += 1;
            return;
        }

        let mut victim_line = self
            .victim
            .as_mut()
            .and_then(|victim| victim.take(block_address));
        if let Some(line) = victim_line.as_mut() {
            line.stamp = self.next_stamp;
        }

        if let Some(line) = victim_line {
            let (way, evicted) = self.install_line(set_index, line);
            if let Some((evicted_line, evicted_way)) = evicted {
                self.multi_column_on_evict(set_index, &evicted_line, evicted_way);
                if let Some(victim) = self.victim.as_mut() {
                    victim.insert(evicted_line, self.next_stamp);
                }
            }
            if let Some(line) = self.sets[set_index]
                .get_mut(way)
                .and_then(|slot| slot.as_mut())
            {
                line.mark_hit();
            }
            self.update_multi_column_on_hit(set_index, block_address, way);
            stats.hits += 1;
            stats.victim_hits += 1;
        } else {
            let line = CacheLine::new(tag, block_address, self.next_stamp);
            let (way, evicted) = self.install_line(set_index, line);
            if let Some((evicted_line, evicted_way)) = evicted {
                self.multi_column_on_evict(set_index, &evicted_line, evicted_way);
                if let Some(victim) = self.victim.as_mut() {
                    victim.insert(evicted_line, self.next_stamp);
                }
            }
            stats.misses += 1;
        }

        self.next_stamp += 1;
    }

    fn observe_prediction(
        &self,
        set_index: usize,
        block_address: u64,
    ) -> PredictionObservation {
        match self.prediction_mode {
            PredictionStrategy::None => PredictionObservation::None,
            PredictionStrategy::Mru => PredictionObservation::Mru {
                predicted: self.mru_way(set_index),
            },
            PredictionStrategy::MultiColumn => {
                let bits = self
                    .multi_predictor
                    .as_ref()
                    .map(|mc| mc.observe(set_index, block_address))
                    .unwrap_or(0);
                PredictionObservation::MultiColumn { bits }
            }
        }
    }

    fn record_prediction(
        &self,
        observation: &PredictionObservation,
        actual: Option<(usize, bool)>,
        stats: &mut CacheStats,
    ) {
        let pred_stats = match stats.prediction.as_mut() {
            Some(stats) => stats,
            None => return,
        };
        let Some((actual_way, is_first_hit)) = actual else {
            return;
        };
        pred_stats.total_hits_observed += 1;
        if is_first_hit {
            pred_stats.first_hits += 1;
        } else {
            pred_stats.non_first_hits += 1;
        }
        match observation {
            PredictionObservation::None => {}
            PredictionObservation::Mru { predicted } => {
                let _ = predicted;
            }
            PredictionObservation::MultiColumn { bits } => {
                if *bits == 0 {
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
                    pred_stats.bit_vector_search_total += rank as u64;
                } else {
                    pred_stats.bit_vector_search_total += bits.count_ones() as u64;
                }
                pred_stats.bit_vector_observations += 1;
            }
        }
    }

    fn touch_if_hit(&mut self, set_index: usize, tag: u64) -> Option<(usize, bool)> {
        let set = &mut self.sets[set_index];
        for (way, slot) in set.iter_mut().enumerate() {
            if let Some(line) = slot {
                if line.tag == tag {
                    // Refresh the LRU stamp when see a hit.
                    let is_first_hit = line.mark_hit();
                    line.stamp = self.next_stamp;
                    return Some((way, is_first_hit));
                }
            }
        }
        None
    }

    fn install_line(
        &mut self,
        set_index: usize,
        mut line: CacheLine,
    ) -> (usize, Option<(CacheLine, usize)>) {
        // Check for empty slots first
        if let Some((idx, slot)) = self.sets[set_index]
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
            line.stamp = self.next_stamp;
            *slot = Some(line);
            return (idx, None);
        }

        let idx = self.find_victim_index(set_index);
        
        // For MRU strategy, we implement LIP (LRU Insertion Policy).
        // We reuse the victim's stamp so the new line stays at the LRU position.
        if self.prediction_mode == PredictionStrategy::Mru {
            if let Some(victim) = &self.sets[set_index][idx] {
                line.stamp = victim.stamp;
            }
        }

        let set = &mut self.sets[set_index];
        let evicted = set[idx].replace(line).unwrap();
        (idx, Some((evicted, idx)))
    }

    fn find_victim_index(&self, set_index: usize) -> usize {
        let set = &self.sets[set_index];
        match self.prediction_mode {
            PredictionStrategy::Mru => set
                .iter()
                .enumerate()
                .min_by_key(|(_, slot)| slot.as_ref().map(|line| line.stamp).unwrap_or(u64::MIN))
                .map(|(idx, _)| idx)
                .unwrap(),
            PredictionStrategy::MultiColumn => {
                let predictor = self.multi_predictor.as_ref().unwrap();
                set.iter()
                    .enumerate()
                    .map(|(way, slot)| {
                        let line = slot.as_ref().unwrap();
                        let bits = predictor.observe(set_index, line.block_address);
                        let is_hot = (bits >> way) & 1;
                        (is_hot, line.stamp, way)
                    })
                    .min()
                    .map(|(_, _, way)| way)
                    .unwrap()
            }
            PredictionStrategy::None => set
                .iter()
                .enumerate()
                .min_by_key(|(_, slot)| slot.as_ref().map(|line| line.stamp).unwrap_or(u64::MIN))
                .map(|(idx, _)| idx)
                .unwrap(),
        }
    }

    fn update_multi_column_on_hit(&mut self, set_index: usize, block_address: u64, way: usize) {
        if let Some(predictor) = self.multi_predictor.as_mut() {
            predictor.mark(set_index, block_address, way);
        }
    }

    fn multi_column_on_evict(
        &mut self,
        set_index: usize,
        line: &CacheLine,
        way: usize,
    ) {
        if let Some(predictor) = self.multi_predictor.as_mut() {
            predictor.clear(set_index, line.block_address, way);
        }
    }

    fn mru_way(&self, set_index: usize) -> Option<usize> {
        self.sets[set_index]
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| slot.as_ref().map(|line| (idx, line.stamp)))
            .max_by_key(|(_, stamp)| *stamp)
            .map(|(idx, _)| idx)
    }
}

// ===== Cache line====

#[derive(Clone)]
struct CacheLine {
    tag: u64,
    block_address: u64,
    stamp: u64,
    has_received_hit: bool,
}

impl CacheLine {
    fn new(tag: u64, block_address: u64, stamp: u64) -> Self {
        Self {
            tag,
            block_address,
            stamp,
            has_received_hit: false,
        }
    }

    /// Marks the cache line as hit once it is accessed.
    /// Returns true if this is the first hit observed since the line was inserted.
    fn mark_hit(&mut self) -> bool {
        let is_first_hit = !self.has_received_hit;
        self.has_received_hit = true;
        is_first_hit
    }
}

// ===== Victim cache buffer =====

struct VictimBuffer {
    entries: Vec<CacheLine>,
    capacity: usize,
}

impl VictimBuffer {
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
            return Some(self.entries.remove(idx));
        }
        None
    }

    fn insert(&mut self, mut line: CacheLine, stamp: u64) {
        if self.capacity == 0 {
            return;
        }
        line.stamp = stamp;
        if self.entries.len() == self.capacity {
            if let Some(idx) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, line)| line.stamp)
                .map(|(idx, _)| idx)
            {
                self.entries.remove(idx);
            }
        }
        self.entries.push(line);
    }
}

// ===== Prediction Utility ====

#[derive(Clone, Copy)]
enum PredictionObservation {
    None,
    Mru { predicted: Option<usize> },
    MultiColumn { bits: u32 },
}

struct MultiColumnPredictor {
    bits: Vec<u32>,
    sets: usize,
    columns: usize,
}

impl MultiColumnPredictor {
    fn new(num_sets: usize, ways: usize) -> Self {
        let columns = match ways {
            0..=1 => 1,
            2..=4 => 2,
            5..=8 => 4,
            _ => 8,
        }
        .clamp(1, ways.max(1));
        Self {
            bits: vec![0; num_sets * columns],
            sets: num_sets,
            columns,
        }
    }

    fn observe(&self, set_index: usize, block_address: u64) -> u32 {
        self.bits[self.index(set_index, self.column(block_address))]
    }

    fn mark(&mut self, set_index: usize, block_address: u64, way: usize) {
        if way >= 32 {
            return;
        }
        let idx = self.index(set_index, self.column(block_address));
        self.bits[idx] |= 1u32 << way;
    }

    fn clear(&mut self, set_index: usize, block_address: u64, way: usize) {
        if way >= 32 {
            return;
        }
        let idx = self.index(set_index, self.column(block_address));
        // Clear bits to avoid predicting stale ways after eviction
        self.bits[idx] &= !(1u32 << way);
    }

    fn column(&self, block_address: u64) -> usize {
        if self.columns == 1 {
            0
        } else {
            let tag = block_address / self.sets as u64;
            (tag as usize) % self.columns
        }
    }

    fn index(&self, set_index: usize, column: usize) -> usize {
        set_index * self.columns + column
    }
}
