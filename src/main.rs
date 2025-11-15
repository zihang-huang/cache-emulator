mod cache;
mod experiments;
mod trace;

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};

use cache::{Cache, CacheConfig, CacheStats, PredictionStrategy};
use experiments::{
    ScenarioResult, block_sizes, direct_mapped, predictor_configs, run_scenarios, set_associative,
    victim_cache_configs,
};
use trace::TraceFile;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Simulate(args) => run_simulation(args),
        Command::Experiments(args) => run_experiments(args),
    }
}

#[derive(Parser)]
#[command(
    name = "cache-lab",
    author = "Cache Lab Simulator",
    version,
    about = "LRU Cache simulator for the course lab"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a single simulation with custom parameters
    Simulate(SimulateArgs),
    /// Run the full experimental suite defined in the lab handout
    Experiments(ExperimentArgs),
}

#[derive(Args)]
struct SimulateArgs {
    /// Trace file to replay
    #[arg(long)]
    trace: PathBuf,
    /// Total cache capacity in bytes (default 256 KiB)
    #[arg(long, default_value_t = 256 * 1024)]
    cache_size: usize,
    /// Block size in bytes
    #[arg(long, default_value_t = 32)]
    block_size: usize,
    /// Associativity (1 = direct-mapped)
    #[arg(long, default_value_t = 4)]
    associativity: usize,
    /// Victim cache entries (fully associative)
    #[arg(long, default_value_t = 0)]
    victim: usize,
    /// Way prediction strategy
    #[arg(long, default_value_t = PredictionCli::None, value_enum)]
    prediction: PredictionCli,
    /// Optional log file that records every miss (and victim hits)
    #[arg(long)]
    miss_log: Option<PathBuf>,
}

#[derive(Args)]
struct ExperimentArgs {
    /// Explicit trace files to run. Defaults to every *.trace under ./trace
    #[arg(long = "trace", value_name = "PATH")]
    traces: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum PredictionCli {
    None,
    Mru,
    #[value(name = "multi-column")]
    MultiColumn,
}

impl From<PredictionCli> for PredictionStrategy {
    fn from(value: PredictionCli) -> Self {
        match value {
            PredictionCli::None => PredictionStrategy::None,
            PredictionCli::Mru => PredictionStrategy::Mru,
            PredictionCli::MultiColumn => PredictionStrategy::MultiColumn,
        }
    }
}

fn run_simulation(args: SimulateArgs) -> Result<()> {
    let trace = TraceFile::load(&args.trace)
        .with_context(|| format!("Failed to load trace {}", args.trace.display()))?;
    let mut config = CacheConfig::default();
    config.cache_size = args.cache_size;
    config.block_size = args.block_size;
    config.associativity = args.associativity;
    config.victim_cache_entries = args.victim;
    config.prediction = args.prediction.into();

    let mut cache = Cache::new(config)?;

    let mut miss_file = if let Some(path) = &args.miss_log {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| fs::create_dir_all(p));
        if let Some(res) = parent {
            res.with_context(|| {
                format!("Failed to create miss log directory for {}", path.display())
            })?;
        }
        Some(
            File::create(path)
                .with_context(|| format!("Failed to open miss log {}", path.display()))?,
        )
    } else {
        None
    };

    let stats = cache.run_trace(
        &trace.entries,
        miss_file.as_mut().map(|file| file as &mut dyn Write),
    );

    println!("Trace             : {}", trace.name);
    println!("Cache size        : {} bytes", args.cache_size);
    println!("Block size        : {} bytes", args.block_size);
    println!("Associativity     : {}", args.associativity);
    println!("Victim cache size : {} entries", args.victim);
    println!("Prediction        : {:?}", args.prediction);
    println!();
    print_stats(&stats);
    if let Some(path) = &args.miss_log {
        println!("\nMiss log captured at {}", path.display());
    }
    Ok(())
}

fn run_experiments(args: ExperimentArgs) -> Result<()> {
    let trace_paths = if args.traces.is_empty() {
        default_trace_paths()?
    } else {
        args.traces
    };
    if trace_paths.is_empty() {
        bail!("No trace files were provided or discovered.");
    }
    let traces = load_traces(&trace_paths)?;

    let base_cfg = CacheConfig::default();

    println!("Loaded {} trace files.", traces.len());

    // Experiment 1: Direct-Mapped
    let dm = run_scenarios(&traces, &[direct_mapped(&base_cfg)])?;
    print_section("Direct-Mapped", &dm);

    // Experiment 2: Set-Associative for multiple ways
    let sa_configs = set_associative(&base_cfg, &[2, 4, 8, 16]);
    let sa_results = run_scenarios(&traces, &sa_configs)?;
    print_section("Set-Associative Sweep", &sa_results);

    // Experiment 3: Block size sweep (4-way)
    let block_cfg = {
        let mut cfg = base_cfg.clone();
        cfg.associativity = 4;
        cfg
    };
    let block_scenarios = block_sizes(&block_cfg, &[8, 16, 32, 64, 128, 256]);
    let block_results = run_scenarios(&traces, &block_scenarios)?;
    print_section("Block Size Sweep (4-way)", &block_results);

    // Experiment 4: Victim cache sizes on DM cache
    let victim_base = {
        let mut cfg = base_cfg.clone();
        cfg.associativity = 1;
        cfg
    };
    let victim_scenarios = victim_cache_configs(&victim_base, &[4, 8, 16, 32]);
    let victim_results = run_scenarios(&traces, &victim_scenarios)?;
    print_section("Victim Cache on DM", &victim_results);

    // Experiment 5: MRU prediction
    let mru_scenarios = predictor_configs(&base_cfg, &[2, 4, 8, 16], PredictionStrategy::Mru);
    let mru_results = run_scenarios(&traces, &mru_scenarios)?;
    print_section("MRU Prediction", &mru_results);

    // Experiment 6: Multi-column prediction
    let mc_scenarios =
        predictor_configs(&base_cfg, &[2, 4, 8, 16], PredictionStrategy::MultiColumn);
    let mc_results = run_scenarios(&traces, &mc_scenarios)?;
    print_section("Multi-column Prediction", &mc_results);

    Ok(())
}

fn print_stats(stats: &CacheStats) {
    println!("Accesses          : {}", stats.accesses);
    println!("Reads/Writes      : {}/{}", stats.reads, stats.writes);
    println!(
        "Hits              : {} ({:.2}%)",
        stats.hits,
        stats.hit_rate() * 100.0
    );
    println!(
        "Misses            : {} ({:.2}%)",
        stats.misses,
        (1.0 - stats.hit_rate()) * 100.0
    );
    if stats.victim_hits > 0 {
        println!(
            "Victim hits       : {} ({:.2}% of hits)",
            stats.victim_hits,
            stats.victim_hit_ratio() * 100.0
        );
    }
    if let Some(pred) = &stats.prediction {
        println!("First-hit rate    : {:.2}%", pred.first_hit_rate() * 100.0);
        println!(
            "Non-first hit rate: {:.2}%",
            pred.non_first_hit_rate() * 100.0
        );
        if matches!(pred.mode, PredictionStrategy::MultiColumn) {
            println!("Avg. bit search   : {:.2}", pred.avg_bit_vector_search());
        }
    }
}

fn print_section(title: &str, results: &[ScenarioResult]) {
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

fn load_traces(paths: &[PathBuf]) -> Result<Vec<TraceFile>> {
    let mut traces = Vec::new();
    for path in paths {
        traces.push(
            TraceFile::load(path).with_context(|| format!("Failed to parse {}", path.display()))?,
        );
    }
    Ok(traces)
}

fn default_trace_paths() -> Result<Vec<PathBuf>> {
    let dir = Path::new("trace");
    if !dir.exists() {
        bail!(
            "Default trace directory '{}' does not exist.",
            dir.display()
        );
    }
    let mut entries: Vec<_> = fs::read_dir(dir)?
        .filter_map(|entry| {
            entry.ok().and_then(|e| {
                let path = e.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("trace") {
                    Some(path)
                } else {
                    None
                }
            })
        })
        .collect();
    entries.sort();
    if entries.is_empty() {
        bail!(
            "No *.trace files found under '{}'. Use --trace to select files manually.",
            dir.display()
        );
    }
    Ok(entries)
}
