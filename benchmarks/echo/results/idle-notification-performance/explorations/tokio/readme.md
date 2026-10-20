# Tokio follow-up evidence

All variants start from `a98f172f8b06f23b34a354a4b6f6c294aa5859ab`. `analysis.md` documents rejected default-release source experiments. `thin-lto-analysis.md` documents the completed, separately requested matched profile diagnostic (30 matched samples across all five backends).

`provenance.json` preserves original frozen binary paths, hashes, compiler/build settings, and source patch paths. Binaries, target directories, disassembly, and Python caches are deliberately excluded from this compact export. The exact source patches and successful build logs are under `variants/`; apply each patch independently to the base commit. The thin-LTO variant has no source patch.

`run.py` and `sample-runner.py` are the exact frozen timing scripts. Their hashes and the fixed external client's hash occur in each aggregate `results/*/aiko.json`. Each aggregate contains the complete randomized matrix and all sample metrics; individual sample JSON files are also preserved. Nonempty sample stderr is retained if present. Every timing matrix used the global exclusive performance lock; builds used its shared mode.
