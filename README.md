# Cache Lab Simulator

Rust implementation of the cache simulator required by “实验1：Cache算法的设计、实现与比较”. The binary can replay the provided trace files, emulate a parameterised cache hierarchy (direct-mapped and set-associative) with LRU replacement, optional victim cache, and two way-prediction schemes (MRU and multi-column). Every experiment requested in the handout is scripted via the CLI.

## Building

```
cargo build
```

The code targets Rust 1.78+ (edition 2024).

## Running a single simulation

Use the `simulate` sub-command when you want to inspect one configuration or emit a miss log:

```
cargo run --release -- \
  simulate \
  --trace trace/game.trace \
  --cache-size 262144 \
  --block-size 32 \
  --associativity 4 \
  --victim 0 \
  --prediction none \
  --miss-log outputs/game-miss.log
```

Flags:

| Flag | Meaning |
| --- | --- |
| `--trace <path>` | Trace file to replay (required) |
| `--cache-size <bytes>` | Capacity in bytes (default 256&nbsp;KiB) |
| `--block-size <bytes>` | Cache line size (default 32B) |
| `--associativity <n>` | Number of ways (1 = direct-mapped) |
| `--victim <entries>` | Fully-associative victim cache entries |
| `--prediction <none|mru|multi-column>` | Way prediction policy |
| `--miss-log <path>` | Optional log; each line is `<seq> <op> <addr> <victim-hit|memory-miss>` |

## Running the full experiment suite

```
cargo run --release -- experiments
```

By default this loads every `*.trace` file in the `trace/` directory and prints summary tables for:

1. Direct-mapped cache hit rates.
2. Set-associative caches (2/4/8/16 ways).
3. Block-size sweep (4-way cache, 8–256B blocks).
4. Direct-mapped cache with victim caches (4/8/16/32 entries) including the victim-hit contribution.
5. MRU way prediction (2/4/8/16 ways) with first/non-first hit rates.
6. Multi-column way prediction (2/4/8/16 ways) with first/non-first hit rates and the average bit-vector search length.

Supply explicit traces with repeated `--trace <path>` flags if you want a subset, e.g.:

```
cargo run --release -- experiments --trace trace/game.trace --trace trace/photo.trace
```

## Implementation notes

- Core cache: physically-indexed, physically-tagged, write-allocate, write-back model with per-set LRU timestamps. Block size, associativity, cache size, and address width are all parameterised.
- Victim cache: fully-associative buffer that also uses LRU replacement. Victim hits are counted inside the overall hit rate and the share of hits coming from the victim cache is reported.
- MRU predictor: each set tracks its most recently touched way. Statistics distinguish between first-hit (the MRU guess hits immediately) and non-first hits.
- Multi-column predictor: each set owns a configurable number of columns (default derived from associativity). A column is selected by hashing the tag, and it stores a bit-vector of candidate ways. Multiple tags may map to the same column, therefore several bits can be set and the simulator derives both the prediction accuracy (first/non-first hits) and the average number of bits that must be scanned before the match is found (or the predictor gives up). When the predictor provides no candidates, the access is considered a non-first hit.
- Miss logging: the log records every event where the primary cache misses. Victim hits are tagged as `victim-hit`, and only pure memory refills are tagged as `memory-miss`. This matches the requirement of logging “缺失访问” while still allowing the victim cache contribution to be measured.

## Suggested workflow

1. `cargo run --release -- experiments` – collect all required hit-rate metrics.
2. `cargo run --release -- simulate --trace trace/<file>.trace --miss-log out.log` – capture detailed miss logs for the traces you analyse in the report.
3. Document your observations (associativity trends, block-size trade-offs, predictor behaviour) in the lab report with the numeric outputs from the tool.
