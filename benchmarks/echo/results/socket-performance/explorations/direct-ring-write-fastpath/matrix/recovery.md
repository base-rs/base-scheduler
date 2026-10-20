# Recovered matrix data

The original 48-sample aggregate JSON was overwritten by a serial runner that reused its output path. `../matrix-recovered.json` reconstructs every sample throughput (rounded to the nearest request/s) and exact voluntary context-switch count from `../run-matrix.log`. Full-precision throughput medians/deltas are preserved from that same completed log. Detailed per-sample RTT, CPU, and lifecycle data are unavailable and have not been invented. Binary/source/runner provenance remains intact in `../provenance.json`.
