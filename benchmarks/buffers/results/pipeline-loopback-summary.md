# Receive/parse/slice/send pipeline measurements

960 optimized samples. Ratios are medians of paired per-repeat throughput; CPU ratios compare total process user + system time for the same byte count.

| Transport | Bytes | Connections | Ownership | Placement | Registered copy | ZC | Fixed ZC | Fixed ZC CPU | Fixed / ZC |
|---|---:|---:|---|---|---:|---:|---:|---:|---:|
| tcp | 64 | 1 | pooled | handoff | 0.963× | 0.969× | 0.983× | 1.048× | 1.008× |
| tcp | 64 | 1 | pooled | local | 1.019× | 0.892× | 0.884× | 1.085× | 0.931× |
| tcp | 64 | 1 | retained | handoff | 1.013× | 1.007× | 1.032× | 1.063× | 1.011× |
| tcp | 64 | 1 | retained | local | 0.997× | 0.857× | 0.861× | 1.111× | 0.993× |
| tcp | 64 | 8 | pooled | handoff | 0.983× | 0.877× | 0.886× | 1.025× | 1.047× |
| tcp | 64 | 8 | pooled | local | 0.985× | 0.792× | 0.844× | 1.027× | 1.023× |
| tcp | 64 | 8 | retained | handoff | 1.087× | 0.978× | 0.990× | 1.072× | 0.995× |
| tcp | 64 | 8 | retained | local | 0.991× | 0.844× | 0.867× | 1.010× | 1.018× |
| tcp | 1200 | 1 | pooled | handoff | 1.002× | 1.009× | 1.043× | 1.041× | 1.028× |
| tcp | 1200 | 1 | pooled | local | 0.987× | 0.903× | 0.944× | 1.019× | 1.047× |
| tcp | 1200 | 1 | retained | handoff | 1.003× | 1.020× | 1.035× | 1.032× | 1.021× |
| tcp | 1200 | 1 | retained | local | 0.997× | 0.906× | 0.929× | 1.034× | 1.032× |
| tcp | 1200 | 8 | pooled | handoff | 1.072× | 0.904× | 0.917× | 1.021× | 1.015× |
| tcp | 1200 | 8 | pooled | local | 1.002× | 0.873× | 0.907× | 1.004× | 1.030× |
| tcp | 1200 | 8 | retained | handoff | 1.032× | 0.946× | 0.919× | 1.022× | 0.956× |
| tcp | 1200 | 8 | retained | local | 1.006× | 0.901× | 0.895× | 0.986× | 1.018× |
| tcp | 16384 | 1 | pooled | handoff | 1.007× | 0.981× | 1.012× | 0.909× | 1.046× |
| tcp | 16384 | 1 | pooled | local | 1.000× | 0.954× | 0.960× | 0.893× | 1.004× |
| tcp | 16384 | 1 | retained | handoff | 0.997× | 0.986× | 0.991× | 0.938× | 1.023× |
| tcp | 16384 | 1 | retained | local | 0.992× | 0.981× | 0.972× | 0.893× | 0.882× |
| tcp | 16384 | 8 | pooled | handoff | 1.032× | 0.888× | 0.912× | 0.904× | 1.054× |
| tcp | 16384 | 8 | pooled | local | 1.017× | 0.812× | 0.852× | 0.882× | 1.076× |
| tcp | 16384 | 8 | retained | handoff | 1.079× | 0.961× | 0.976× | 0.906× | 1.050× |
| tcp | 16384 | 8 | retained | local | 1.013× | 0.807× | 0.840× | 0.889× | 1.048× |
| tcp | 65536 | 1 | pooled | handoff | 0.996× | 0.838× | 0.856× | 0.914× | 1.012× |
| tcp | 65536 | 1 | pooled | local | 1.007× | 0.829× | 0.847× | 0.918× | 1.019× |
| tcp | 65536 | 1 | retained | handoff | 0.993× | 0.829× | 0.855× | 0.918× | 1.018× |
| tcp | 65536 | 1 | retained | local | 0.992× | 0.822× | 0.823× | 0.941× | 1.011× |
| tcp | 65536 | 8 | pooled | handoff | 0.998× | 0.696× | 0.742× | 0.874× | 1.066× |
| tcp | 65536 | 8 | pooled | local | 0.995× | 0.747× | 0.764× | 0.853× | 1.015× |
| tcp | 65536 | 8 | retained | handoff | 1.119× | 0.961× | 1.003× | 0.856× | 1.048× |
| tcp | 65536 | 8 | retained | local | 1.005× | 0.751× | 0.770× | 0.846× | 1.027× |
| tcp | 131072 | 1 | pooled | handoff | 1.027× | 0.862× | 0.863× | 0.881× | 1.007× |
| tcp | 131072 | 1 | pooled | local | 0.993× | 0.837× | 0.841× | 0.915× | 0.984× |
| tcp | 131072 | 1 | retained | handoff | 0.991× | 0.819× | 0.834× | 0.862× | 0.969× |
| tcp | 131072 | 1 | retained | local | 0.997× | 0.830× | 0.829× | 0.910× | 1.009× |
| tcp | 131072 | 8 | pooled | handoff | 1.008× | 0.715× | 0.734× | 0.836× | 1.029× |
| tcp | 131072 | 8 | pooled | local | 0.975× | 0.748× | 0.758× | 0.833× | 1.024× |
| tcp | 131072 | 8 | retained | handoff | 1.036× | 0.855× | 0.888× | 0.857× | 1.043× |
| tcp | 131072 | 8 | retained | local | 1.017× | 0.771× | 0.792× | 0.820× | 1.027× |
| udp | 1200 | 1 | pooled | handoff | 1.034× | 0.979× | 0.969× | 1.004× | 0.980× |
| udp | 1200 | 1 | pooled | local | 1.028× | 0.927× | 1.040× | 1.022× | 1.074× |
| udp | 1200 | 1 | retained | handoff | 1.017× | 0.956× | 0.962× | 1.027× | 1.006× |
| udp | 1200 | 1 | retained | local | 1.060× | 0.888× | 0.873× | 1.065× | 1.079× |
| udp | 1200 | 8 | pooled | handoff | 1.009× | 0.923× | 0.950× | 1.021× | 1.063× |
| udp | 1200 | 8 | pooled | local | 1.032× | 0.885× | 0.891× | 1.005× | 0.998× |
| udp | 1200 | 8 | retained | handoff | 0.977× | 0.888× | 0.856× | 1.033× | 0.970× |
| udp | 1200 | 8 | retained | local | 0.985× | 0.894× | 0.863× | 0.993× | 0.994× |

The JSON summary retains ranges, absolute throughput, usage notifications, and pool stalls. “Registered copy” registers the same storage but uses ordinary send operations.
