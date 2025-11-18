# Cache Lab Simulator

Rust implementation of the cache simulator required by “实验1：Cache算法的设计、实现与比较”. The binary is intentionally minimal: it looks for every `*.trace` file inside the `trace/` directory, runs the experiments requested in the lab handout, and prints the aggregate results. There are no CLI switches to tweak inputs or cache parameters beyond what the experiments already cover.

## Building

```
cargo build
```

The code targets Rust 1.78+ (edition 2024).

## Running the experiments

```
cargo run --release
```

This loads every `*.trace` file in the `trace/` directory and prints summary tables for:

1. Direct-mapped cache hit rates.
2. Set-associative caches (2/4/8/16 ways).
3. Block-size sweep (4-way cache, 8–256B blocks).
4. Direct-mapped cache with victim caches (4/8/16/32 entries) including the victim-hit contribution.
5. MRU way prediction (2/4/8/16 ways) with first/non-first hit rates.
6. Multi-column way prediction (2/4/8/16 ways) with first/non-first hit rates and the average bit-vector search length.

## Implementation notes

- Core cache: physically-indexed, physically-tagged, write-allocate, write-back model with per-set LRU timestamps. Block size, associativity, cache size, and address width are all parameterised.
- Victim cache: fully-associative buffer that also uses LRU replacement. Victim hits are counted inside the overall hit rate and the share of hits coming from the victim cache is reported.
- MRU predictor: each set tracks its most recently touched way. Statistics distinguish between first-hit (the MRU guess hits immediately) and non-first hits.
- Multi-column predictor: each set owns a small number of columns derived from the associativity. A column is selected by hashing the tag, and it stores a bit-vector of candidate ways. Multiple tags may map to the same column, therefore several bits can be set and the simulator derives both the prediction accuracy (first/non-first hits) and the average number of bits that must be scanned before the match is found (or the predictor gives up). When the predictor provides no candidates, the access is considered a non-first hit.

## Suggested workflow

1. `cargo run --release` – collect all required hit-rate metrics for every trace in the `trace/` folder.
2. Document your observations (associativity trends, block-size trade-offs, predictor behaviour) in the lab report with the numeric outputs from the tool.
