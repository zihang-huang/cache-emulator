mod cache;
mod experiments;
mod trace;
use cache::{CacheConfig, PredictionStrategy};
use experiments::{
    ScenarioResult, block_sizes, direct_mapped, predictor_configs, run_scenarios, set_associative,
    victim_cache_configs,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use trace::TraceFile;

fn main() {
    run_experiments();
}

fn run_experiments() {
    let trace_paths = default_trace_paths();
    let traces = load_traces(&trace_paths);

    let base_cfg = CacheConfig::default();

    println!("Loaded {} trace files.", traces.len());

    // Experiment 1: Direct-Mapped
    let dm = run_scenarios(&traces, &[direct_mapped(&base_cfg)]);
    print_section("Direct-Mapped", &dm);

    // Experiment 2: Set-Associative for multiple ways
    let sa_configs = set_associative(&base_cfg, &[2, 4, 8, 16]);
    let sa_results = run_scenarios(&traces, &sa_configs);
    print_section("Set-Associative Sweep", &sa_results);

    // Experiment 3: Block size sweep (4-way)
    let block_cfg = {
        let mut cfg = base_cfg.clone();
        cfg.associativity = 4;
        cfg
    };
    let block_scenarios = block_sizes(&block_cfg, &[8, 16, 32, 64, 128, 256]);
    let block_results = run_scenarios(&traces, &block_scenarios);
    print_section("Block Size Sweep (4-way)", &block_results);

    // Experiment 4: Victim cache sizes on DM cache
    let victim_base = {
        let mut cfg = base_cfg.clone();
        cfg.associativity = 1;
        cfg
    };
    let victim_scenarios = victim_cache_configs(&victim_base, &[4, 8, 16, 32]);
    let victim_results = run_scenarios(&traces, &victim_scenarios);
    print_section("Victim Cache on DM", &victim_results);

    // Experiment 5: MRU prediction
    let mru_scenarios = predictor_configs(&base_cfg, &[2, 4, 8, 16], PredictionStrategy::Mru);
    let mru_results = run_scenarios(&traces, &mru_scenarios);
    print_section("MRU Prediction", &mru_results);

    // Experiment 6: Multi-column prediction
    let mc_scenarios =
        predictor_configs(&base_cfg, &[2, 4, 8, 16], PredictionStrategy::MultiColumn);
    let mc_results = run_scenarios(&traces, &mc_scenarios);
    print_section("Multi-column Prediction", &mc_results);
}

fn print_section(title: &str, results: &[ScenarioResult]) {
    // Format the expriments result
    println!("\n== {title} ==");
    for scenario in results {
        println!("  {}", scenario.label);
        for trace in &scenario.trace_results {
            let stats = &trace.stats;
            let mut line = format!(
                "    {:<14} hit {:>6.2}% miss {:>6.2}%",
                trace.trace_name,
                stats.hit_rate() * 100.0,
                (1.0 - stats.hit_rate()) * 100.0
            );
            if stats.victim_hits > 0 {
                line.push_str(&format!(
                    " victim {:>5.1}%",
                    stats.victim_hit_ratio() * 100.0
                ));
            }
            if let Some(pred) = &stats.prediction {
                line.push_str(&format!(
                    " first {:>6.2}% non-first {:>6.2}%",
                    pred.first_hit_rate() * 100.0,
                    pred.non_first_hit_rate() * 100.0
                ));
                if matches!(pred.mode, PredictionStrategy::MultiColumn) {
                    line.push_str(&format!(" avg-search {:.2}", pred.avg_bit_vector_search()));
                }
            }
            println!("{line}");
        }
    }
}

fn load_traces(paths: &[PathBuf]) -> Vec<TraceFile> {
    let mut traces = Vec::new();
    for path in paths {
        traces.push(TraceFile::load(path));
    }
    traces
}

fn default_trace_paths() -> Vec<PathBuf> {
    let dir = Path::new("trace");
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|_| panic!("Unable to read trace dir {}", dir.display()))
        .filter_map(|entry| {
            entry.ok().and_then(|e| {
                let path = e.path();
                (path.extension().and_then(|ext| ext.to_str()) == Some("trace")).then_some(path)
            })
        })
        .collect();
    entries.sort();
    entries
}
