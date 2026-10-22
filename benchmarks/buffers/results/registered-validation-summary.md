Samples: 352; matched pairs: 176; repeats: 11
Binary SHA-256: 2b374a277e76923a0b245f1b6e1086c7556abf2df43b8c80c19f78f421b08189

| Kind | Direction | Bytes | Depth | Ownership | Ordinary GiB/s | Fixed GiB/s | Matched fixed/ordinary | Pair range |
| --- | --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |
| cached | read | 65536 | 16 | pooled | 57.979 | 53.320 | 0.918× | 0.899–0.932 |
| cached | read | 65536 | 16 | retained | 53.109 | 49.067 | 0.925× | 0.906–0.932 |
| cached | write | 65536 | 16 | pooled | 43.640 | 40.890 | 0.936× | 0.915–1.252 |
| cached | write | 65536 | 16 | retained | 41.654 | 38.206 | 0.917× | 0.892–0.934 |
| direct | read | 65536 | 1 | pooled | 0.975 | 1.029 | 1.051× | 0.502–2.132 |
| direct | read | 65536 | 1 | retained | 0.992 | 1.030 | 1.042× | 0.480–2.294 |
| direct | read | 65536 | 16 | pooled | 6.862 | 6.749 | 0.983× | 0.980–1.023 |
| direct | read | 65536 | 16 | retained | 6.744 | 6.726 | 0.994× | 0.946–1.020 |
| direct | write | 65536 | 1 | pooled | 2.118 | 2.885 | 1.356× | 1.046–1.555 |
| direct | write | 65536 | 1 | retained | 2.135 | 2.811 | 1.331× | 1.154–1.514 |
| direct | write | 65536 | 16 | pooled | 6.443 | 6.440 | 1.000× | 0.983–1.003 |
| direct | write | 65536 | 16 | retained | 6.447 | 6.436 | 1.000× | 0.986–1.003 |
| tcp | read | 65536 | 16 | pooled | 10.631 | 10.126 | 0.957× | 0.931–0.972 |
| tcp | read | 65536 | 16 | retained | 10.462 | 9.975 | 0.950× | 0.926–0.979 |
| udp | read | 1200 | 1 | pooled | 0.273 | 0.272 | 1.008× | 0.945–1.056 |
| udp | read | 1200 | 1 | retained | 0.273 | 0.262 | 0.979× | 0.923–1.025 |

| Bytes | Depth | Registered slots | Registration µs | Unregistration µs |
| ---: | ---: | ---: | ---: | ---: |
| 1200 | 1 | 3 | 2.09 | 0.72 |
| 65536 | 1 | 3 | 3.18 | 1.44 |
| 65536 | 16 | 33 | 18.80 | 5.55 |
