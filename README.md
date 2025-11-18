# Cache Lab Simulator
Run the command `cargo run --release` at the root folder, to execute all the simulations.

This loads every `*.trace` file in the `trace/` directory and prints result for:
1. Direct-mapped cache hit rates
2. Set-associative caches (2/4/8/16 ways)
3. Block-size sweep (4-way cache, 8–256B blocks)
4. Direct-mapped cache with victim caches (4/8/16/32 entries) including the victim-hit
5. MRU way prediction (2/4/8/16 ways) with first/non-first hit rates
6. Multi-column way prediction (2/4/8/16 ways) with first/non-first hit rates and the average bit-vector search length

Combining with `>>` command to save the simulations result to a file.

e.g. `cargo run --release >> result.txt`
