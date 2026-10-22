# Receive/parse/slice/send pipeline measurements

56 optimized samples. Ratios are medians of paired per-repeat throughput; CPU ratios compare total process user + system time for the same byte count.

| Transport | Bytes | Connections | Ownership | Placement | Registered copy | ZC | Fixed ZC | Fixed ZC CPU | Fixed / ZC |
|---|---:|---:|---|---|---:|---:|---:|---:|---:|
| tcp | 65536 | 1 | retained | local | 1.000× | 1.005× | 1.004× | 1.639× | 1.002× |
| tcp | 131072 | 1 | retained | local | 0.986× | 0.996× | 0.989× | 1.275× | 0.998× |

The JSON summary retains ranges, absolute throughput, usage notifications, and pool stalls. “Registered copy” registers the same storage but uses ordinary send operations.
