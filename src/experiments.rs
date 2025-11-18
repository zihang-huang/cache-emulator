use std::fmt;
use crate::{
    cache::{Cache, CacheConfig, CacheStats, PredictionStrategy},
    trace::TraceFile,
};

#[derive(Clone)]
pub struct ScenarioConfig {
    pub label: String, // Label to be printed for the Result
    pub config: CacheConfig,
}

pub struct ScenarioResult {
    pub label: String, // Label to be printed for the Result
    pub trace_results: Vec<TraceResult>,
}

pub struct TraceResult {
    pub trace_name: String,
    pub stats: CacheStats,
}

impl fmt::Display for ScenarioResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.label)?;
        for result in &self.trace_results {
            writeln!(
                f,
                "  {:<16} hit-rate: {:>6.2}%",
                result.trace_name,
                result.stats.hit_rate() * 100.0
            )?;
        }
        Ok(())
    }
}

pub fn run_scenarios(
    traces: &[TraceFile],
    scenarios: &[ScenarioConfig],
) -> Vec<ScenarioResult> {
    let mut results = Vec::new();
    for scenario in scenarios {
        let mut per_trace = Vec::new();
        for trace in traces {
            let mut cache = Cache::new(scenario.config.clone());
            let stats = cache.run_trace(&trace.entries);
            per_trace.push(TraceResult {
                trace_name: trace.name.clone(),
                stats,
            });
        }
        results.push(ScenarioResult {
            label: scenario.label.clone(),
            trace_results: per_trace,
        });
    }
    results
}

pub fn direct_mapped(base: &CacheConfig) -> ScenarioConfig {
    let mut cfg = base.clone();
    cfg.associativity = 1;
    cfg.prediction = PredictionStrategy::None;
    cfg.victim_cache_entries = 0;
    ScenarioConfig {
        label: "Direct-Mapped".to_string(),
        config: cfg,
    }
}

pub fn set_associative(base: &CacheConfig, ways: &[usize]) -> Vec<ScenarioConfig> {
    ways.iter()
        .map(|&assoc| {
            let mut cfg = base.clone();
            cfg.associativity = assoc;
            cfg.prediction = PredictionStrategy::None;
            cfg.victim_cache_entries = 0;
            ScenarioConfig {
                label: format!("{assoc}-way SA"),
                config: cfg,
            }
        })
        .collect()
}

pub fn block_sizes(base: &CacheConfig, block_sizes: &[usize]) -> Vec<ScenarioConfig> {
    block_sizes
        .iter()
        .map(|&block| {
            let mut cfg = base.clone();
            cfg.block_size = block;
            ScenarioConfig {
                label: format!("Block {block}B"),
                config: cfg,
            }
        })
        .collect()
}

pub fn victim_cache_configs(base: &CacheConfig, entries: &[usize]) -> Vec<ScenarioConfig> {
    entries
        .iter()
        .map(|&size| {
            let mut cfg = base.clone();
            cfg.associativity = 1;
            cfg.victim_cache_entries = size;
            ScenarioConfig {
                label: format!("DM + Victim({size})"),
                config: cfg,
            }
        })
        .collect()
}

pub fn predictor_configs(
    base: &CacheConfig,
    ways: &[usize],
    strategy: PredictionStrategy,
) -> Vec<ScenarioConfig> {
    let label_prefix = match strategy {
        PredictionStrategy::None => "No-Predict",
        PredictionStrategy::Mru => "MRU",
        PredictionStrategy::MultiColumn => "Multi-Column",
    };
    ways.iter()
        .map(|&assoc| {
            let mut cfg = base.clone();
            cfg.associativity = assoc;
            cfg.prediction = strategy;
            cfg.victim_cache_entries = 0;
            ScenarioConfig {
                label: format!("{label_prefix} {assoc}-way"),
                config: cfg,
            }
        })
        .collect()
}
