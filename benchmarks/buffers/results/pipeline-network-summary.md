# Receive/parse/slice/send pipeline measurements

360 optimized samples. Ratios are medians of paired per-repeat throughput; CPU ratios compare total process user + system time for the same byte count.

| Transport | Bytes | Connections | Ownership | Placement | Registered copy | ZC | Fixed ZC | Fixed ZC CPU | Fixed / ZC |
|---|---:|---:|---|---|---:|---:|---:|---:|---:|
| tcp | 1200 | 1 | retained | handoff | 0.981× | 1.016× | 0.979× | 1.097× | 0.974× |
| tcp | 1200 | 1 | retained | local | 0.977× | 0.971× | 0.968× | 1.119× | 1.033× |
| tcp | 1200 | 8 | retained | handoff | 1.003× | 1.006× | 1.122× | 1.029× | 0.998× |
| tcp | 1200 | 8 | retained | local | 1.054× | 1.022× | 1.004× | 0.885× | 1.000× |
| tcp | 16384 | 1 | retained | handoff | 0.997× | 0.995× | 1.008× | 1.037× | 1.002× |
| tcp | 16384 | 1 | retained | local | 1.000× | 1.005× | 1.011× | 0.628× | 1.009× |
| tcp | 16384 | 8 | retained | handoff | 1.109× | 0.945× | 0.910× | 0.911× | 0.974× |
| tcp | 16384 | 8 | retained | local | 0.841× | 1.025× | 0.883× | 1.060× | 0.865× |
| tcp | 65536 | 1 | retained | handoff | 1.001× | 1.010× | 1.001× | 1.016× | 1.002× |
| tcp | 65536 | 1 | retained | local | 0.990× | 0.972× | 0.980× | 0.694× | 0.990× |
| tcp | 65536 | 8 | retained | handoff | 0.899× | 0.923× | 1.025× | 0.798× | 1.231× |
| tcp | 65536 | 8 | retained | local | 1.089× | 1.026× | 0.928× | 0.896× | 1.012× |
| tcp | 131072 | 1 | retained | handoff | 0.991× | 0.997× | 0.998× | 0.689× | 1.000× |
| tcp | 131072 | 1 | retained | local | 1.009× | 1.008× | 1.004× | 0.814× | 0.996× |
| tcp | 131072 | 8 | retained | handoff | 1.092× | 1.028× | 1.042× | 1.026× | 0.994× |
| tcp | 131072 | 8 | retained | local | 1.015× | 0.979× | 0.918× | 0.953× | 0.966× |
| udp | 1200 | 1 | retained | handoff | 1.037× | 1.046× | 1.073× | 1.061× | 1.024× |
| udp | 1200 | 1 | retained | local | 0.977× | 0.934× | 0.925× | 1.860× | 0.932× |

The JSON summary retains ranges, absolute throughput, usage notifications, and pool stalls. “Registered copy” registers the same storage but uses ordinary send operations.
