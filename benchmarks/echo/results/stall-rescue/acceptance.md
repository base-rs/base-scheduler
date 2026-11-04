| Case | Metric | `main` median (min–max) | Branch median | Change | Verdict |
| --- | --- | --- | --- | --- | --- |
| timers iouring reuse expire 128/4 | ns/op | 8289 (7933–8367) | 8282 | -0.1% | within 3% |
| timers iouring reuse expire 128/4 | cpu ns/op | 1258 (1116–3487) | 1293 | +2.8% | within 3% |
| timers iouring reuse expire 128/4 | late p50 us | 15.01 (5.843–20.43) | 14.86 | -1.0% | within 3% |
| timers iouring reuse expire 128/4 | late p99 us | 203.8 (171.8–206.5) | 202.9 | -0.4% | within 3% |
| echo native-poll 1/1/64 | req/s | 7.829e+04 (7.643e+04–8.172e+04) | 7.829e+04 | -0.0% | within noise |
| echo native-poll 1/1/64 | p99 us | 23.28 (16.13–28.19) | 25.05 | +7.6% | within noise |
| echo native-poll 1/1/64 | server cpu us/req | 7.345 (7.09–7.524) | 7.407 | +0.8% | within noise |
| echo native 4/128/64 @220000 | p50 us | 37.53 (36.93–38.24) | 37.82 | +0.8% | within noise |
| echo native 4/128/64 @220000 | p99 us | 391.2 (178.8–1534) | 439 | +12.2% | within noise |
| echo native 4/128/64 @220000 | server cpu s | 9.292 (9.02–9.542) | 9.344 | +0.6% | within noise |
| timers iouring reuse refresh 1/1 | ns/op | 126.8 (124.9–129.8) | 128.3 | +1.1% | within 3% |
| timers iouring reuse refresh 1/1 | cpu ns/op | 126.6 (124.7–129.5) | 128.1 | +1.1% | within 3% |
| echo iouring-poll 4/128/4096 @190000 | p50 us | 41.79 (41.38–43.07) | 42.27 | +1.1% | within noise |
| echo iouring-poll 4/128/4096 @190000 | p99 us | 389.6 (208.9–503.6) | 388.4 | -0.3% | within noise |
| echo iouring-poll 4/128/4096 @190000 | server cpu s | 9.964 (9.714–10.26) | 10.1 | +1.3% | within noise |
| timers iouring sleep expire 128/4 | ns/op | 8251 (8037–8362) | 8241 | -0.1% | within 3% |
| timers iouring sleep expire 128/4 | cpu ns/op | 1409 (1186–2850) | 1278 | -9.2% | within 3% |
| timers iouring sleep expire 128/4 | late p50 us | 16.92 (6.04–27.51) | 15.35 | -9.3% | within 3% |
| timers iouring sleep expire 128/4 | late p99 us | 202 (182.3–206) | 203.6 | +0.8% | within 3% |
| timers native reuse cycle 64/4 | ns/op | 501.1 (495.9–525.7) | 510.3 | +1.8% | within 3% |
| timers native reuse cycle 64/4 | cpu ns/op | 1398 (744.7–1753) | 1232 | -11.9% | within 3% |
| timers native reuse cycle 64/4 | late p50 us | 2.409 (2.039–3.77) | 2.757 | +14.4% | over 3% |
| timers native reuse cycle 64/4 | late p99 us | 37.14 (9.411–128.9) | 84.81 | +128.3% | over 3% |
| timers iouring reuse cycle 64/4 | ns/op | 516 (496.4–522.7) | 514.7 | -0.2% | within 3% |
| timers iouring reuse cycle 64/4 | cpu ns/op | 557.2 (511.7–1225) | 593.8 | +6.6% | over 3% |
| timers iouring reuse cycle 64/4 | late p50 us | 3.359 (2.023–4.165) | 3.159 | -6.0% | within 3% |
| timers iouring reuse cycle 64/4 | late p99 us | 139.4 (114.2–230.2) | 137.4 | -1.5% | within 3% |
| echo iouring-poll 4/128/64 @220000 | p50 us | 39.49 (38.98–40.19) | 39.33 | -0.4% | within noise |
| echo iouring-poll 4/128/64 @220000 | p99 us | 472.3 (292.4–545.3) | 488.7 | +3.5% | within noise |
| echo iouring-poll 4/128/64 @220000 | server cpu s | 10.94 (10.48–11.16) | 10.92 | -0.2% | within noise |
| diag native 4/1024/1000us aligned | cpu s | 9.788 (8.648–10.63) | 9.332 | -4.7% | within 3% |
| diag native 4/1024/1000us aligned | late p50 us | 414.5 (359.2–450) | 427.8 | +3.2% | over 3% |
| diag native 4/1024/1000us aligned | late p99 us | 773.6 (741.4–792.6) | 734.2 | -5.1% | within 3% |
| timers iouring sleep cycle 64/4 | ns/op | 530.9 (504–545.6) | 531.4 | +0.1% | within 3% |
| timers iouring sleep cycle 64/4 | cpu ns/op | 642.7 (610.9–1095) | 575.5 | -10.5% | within 3% |
| timers iouring sleep cycle 64/4 | late p50 us | 4.96 (2.369–7.353) | 5.737 | +15.7% | over 3% |
| timers iouring sleep cycle 64/4 | late p99 us | 176.7 (147.3–188.3) | 164.4 | -7.0% | within 3% |
| rb idle | cpu_ms | 0.037 (0.021–0.055) | 0.032 | -13.5% | within noise |
| echo native 4/128/4096 @190000 | p50 us | 40.06 (39.55–41.63) | 40.67 | +1.5% | within noise |
| echo native 4/128/4096 @190000 | p99 us | 395.5 (194.8–931.8) | 357.1 | -9.7% | within noise |
| echo native 4/128/4096 @190000 | server cpu s | 8.659 (8.566–8.811) | 8.718 | +0.7% | within noise |
| diag iouring 4/1024/1000us | cpu s | 3.009 (2.846–3.956) | 2.954 | -1.8% | within 3% |
| diag iouring 4/1024/1000us | late p50 us | 20.83 (6.823–34.4) | 21.55 | +3.5% | over 3% |
| diag iouring 4/1024/1000us | late p99 us | 249.7 (234–256) | 239.6 | -4.0% | within 3% |
| rb spawn | ns/op | 8614 (7104–1.456e+04) | 9128 | +6.0% | within noise |
| rb spawn | cpu ns/op | 9221 (7590–1.528e+04) | 9759 | +5.8% | within noise |
| echo iouring-poll 1/1/64 | req/s | 7.813e+04 (7.452e+04–7.916e+04) | 7.694e+04 | -1.5% | within noise |
| echo iouring-poll 1/1/64 | p99 us | 25.36 (17.3–28.14) | 25.41 | +0.2% | within noise |
| echo iouring-poll 1/1/64 | server cpu us/req | 7.782 (7.546–7.979) | 7.902 | +1.6% | within noise |
| echo native 4/128/4096 | req/s | 3.797e+05 (3.725e+05–3.86e+05) | 3.821e+05 | +0.6% | within noise |
| echo native 4/128/4096 | p99 us | 664.1 (604.7–706) | 653.3 | -1.6% | within noise |
| echo native 4/128/4096 | server cpu us/req | 10.5 (10.42–10.68) | 10.46 | -0.4% | within noise |
| echo native-poll 4/128/4096 @190000 | p50 us | 39.07 (38.46–40.06) | 39.17 | +0.2% | within noise |
| echo native-poll 4/128/4096 @190000 | p99 us | 160.1 (97.09–636.9) | 204 | +27.4% | within noise |
| echo native-poll 4/128/4096 @190000 | server cpu s | 7.764 (7.692–7.935) | 7.842 | +1.0% | within noise |
| rb task-sleep-pending | ns/op | 1.078e+06 (1.064e+06–1.1e+06) | 1.079e+06 | +0.1% | within noise |
| rb task-sleep-pending | cpu ns/op | 2.425e+05 (2.081e+05–2.801e+05) | 2.652e+05 | +9.4% | within noise |
| rb task-sleep-pending | late p50 us | 66.07 (52.97–108.1) | 71.82 | +8.7% | within noise |
| rb task-sleep-pending | late p99 us | 160.3 (151.9–172) | 153.9 | -4.0% | within noise |
| diag iouring 1/1/10us | cpu s | 0.6455 (0.6338–0.6929) | 0.6614 | +2.5% | within 3% |
| diag iouring 1/1/10us | late p50 us | 2.963 (2.925–3.123) | 2.973 | +0.3% | within 3% |
| diag iouring 1/1/10us | late p99 us | 14.86 (13.7–15.68) | 15.67 | +5.4% | over 3% |
| echo native 1/1/64 @35000 | p50 us | 15.32 (14.34–17.77) | 16.09 | +5.1% | within noise |
| echo native 1/1/64 @35000 | p99 us | 67.71 (58.27–88.77) | 72.58 | +7.2% | within noise |
| echo native 1/1/64 @35000 | server cpu s | 0.9357 (0.9089–0.9467) | 0.9483 | +1.4% | outside range |
| echo native 1/32/64 | req/s | 2.022e+05 (1.979e+05–2.048e+05) | 2.036e+05 | +0.7% | within noise |
| echo native 1/32/64 | p99 us | 234.9 (219.9–248.3) | 234.2 | -0.3% | within noise |
| echo native 1/32/64 | server cpu us/req | 5.021 (4.973–5.151) | 4.994 | -0.5% | within noise |
| echo native-poll 1/32/64 | req/s | 2.462e+05 (2.388e+05–2.482e+05) | 2.459e+05 | -0.2% | within noise |
| echo native-poll 1/32/64 | p99 us | 193.2 (190.3–210.8) | 197.2 | +2.1% | within noise |
| echo native-poll 1/32/64 | server cpu us/req | 3.954 (3.912–4.081) | 3.953 | -0.0% | within noise |
| echo iouring 1/32/64 | req/s | 2.599e+05 (2.547e+05–2.652e+05) | 2.593e+05 | -0.2% | within noise |
| echo iouring 1/32/64 | p99 us | 229.9 (225.5–235.5) | 229.2 | -0.3% | within noise |
| echo iouring 1/32/64 | server cpu us/req | 3.665 (3.571–3.741) | 3.676 | +0.3% | within noise |
| echo iouring 1/1/64 @1 idle | server cpu ms | 3.08 (2.885–4.435) | 4.035 | +31.0% | within noise |
| echo native-poll 4/128/64 | req/s | 4.545e+05 (4.442e+05–4.595e+05) | 4.51e+05 | -0.8% | within noise |
| echo native-poll 4/128/64 | p99 us | 539.6 (511.2–636.4) | 572.4 | +6.1% | within noise |
| echo native-poll 4/128/64 | server cpu us/req | 8.482 (8.421–8.576) | 8.559 | +0.9% | within noise |
| echo iouring-poll 1/1/64 @35000 | p50 us | 17.49 (14.81–18.21) | 17.66 | +1.0% | within noise |
| echo iouring-poll 1/1/64 @35000 | p99 us | 69.95 (56.48–714.8) | 72.96 | +4.3% | within noise |
| echo iouring-poll 1/1/64 @35000 | server cpu s | 0.8732 (0.8445–0.8977) | 0.8868 | +1.6% | within noise |
| echo iouring 4/128/4096 | req/s | 3.783e+05 (3.726e+05–3.888e+05) | 3.794e+05 | +0.3% | within noise |
| echo iouring 4/128/4096 | p99 us | 673.8 (607.7–714.2) | 662.5 | -1.7% | within noise |
| echo iouring 4/128/4096 | server cpu us/req | 10.01 (9.787–10.18) | 10.02 | +0.1% | within noise |
| rb pulse | cpu_ms | 1.052 (0.961–1.136) | 1.352 | +28.5% | outside range |
| timers native sleep yield 64/4 | ns/op | 131.7 (130.6–140.4) | 118.8 | -9.8% | within 3% |
| timers native sleep yield 64/4 | cpu ns/op | 525.7 (521.2–560.5) | 474.5 | -9.8% | within 3% |
| rb root-sleep | ns/op | 1.145e+06 (1.13e+06–1.157e+06) | 1.12e+06 | -2.3% | better |
| rb root-sleep | cpu ns/op | 1.559e+05 (1.323e+05–1.801e+05) | 1.137e+05 | -27.1% | better |
| rb root-sleep | late p50 us | 146.9 (128.4–155.8) | 122.2 | -16.9% | better |
| rb root-sleep | late p99 us | 215.4 (208.6–228.9) | 195 | -9.5% | better |
| rb spawn-pending | ns/op | 9717 (7535–1.206e+04) | 1.129e+04 | +16.2% | within noise |
| rb spawn-pending | cpu ns/op | 1.028e+04 (8297–1.303e+04) | 1.219e+04 | +18.6% | within noise |
| echo iouring 4/128/64 | req/s | 4.504e+05 (4.412e+05–4.557e+05) | 4.493e+05 | -0.2% | within noise |
| echo iouring 4/128/64 | p99 us | 565.2 (532.5–623.6) | 566.3 | +0.2% | within noise |
| echo iouring 4/128/64 | server cpu us/req | 8.591 (8.471–8.635) | 8.609 | +0.2% | within noise |
| diag native 1/1/10us | cpu s | 1.257 (1.19–1.337) | 1.288 | +2.4% | within 3% |
| diag native 1/1/10us | late p50 us | 5.179 (4.055–5.627) | 5.495 | +6.1% | over 3% |
| diag native 1/1/10us | late p99 us | 8.431 (6.919–10.38) | 8.551 | +1.4% | within 3% |
| timers native sleep cycle 64/4 | ns/op | 523.3 (499–572.1) | 525.5 | +0.4% | within 3% |
| timers native sleep cycle 64/4 | cpu ns/op | 1356 (970.7–1885) | 1493 | +10.1% | over 3% |
| timers native sleep cycle 64/4 | late p50 us | 3.663 (2.246–7.823) | 3.997 | +9.1% | over 3% |
| timers native sleep cycle 64/4 | late p99 us | 144.9 (12.19–174) | 102 | -29.6% | within 3% |
| echo native 1/1/64 @1 idle | server cpu ms | 3.189 (3.063–7.704) | 6.479 | +103.2% | within noise |
| timers iouring reuse yield 64/4 | ns/op | 97.09 (96.14–101) | 99.5 | +2.5% | within 3% |
| timers iouring reuse yield 64/4 | cpu ns/op | 387.7 (384–403) | 397.2 | +2.5% | within 3% |
| echo native 1/1/64 | req/s | 7.88e+04 (7.461e+04–8.039e+04) | 7.886e+04 | +0.1% | within noise |
| echo native 1/1/64 | p99 us | 24.67 (16.69–29.01) | 25.02 | +1.4% | within noise |
| echo native 1/1/64 | server cpu us/req | 8.335 (8.163–8.672) | 8.413 | +0.9% | within noise |
| echo iouring-poll 1/32/64 | req/s | 2.448e+05 (2.42e+05–2.512e+05) | 2.453e+05 | +0.2% | within noise |
| echo iouring-poll 1/32/64 | p99 us | 194.7 (186.1–217.6) | 197.8 | +1.6% | within noise |
| echo iouring-poll 1/32/64 | server cpu us/req | 3.94 (3.859–3.973) | 3.959 | +0.5% | within noise |
| echo iouring 1/32/64 @100000 | p50 us | 15.7 (15.54–15.83) | 15.84 | +0.9% | outside range |
| echo iouring 1/32/64 @100000 | p99 us | 44.45 (38.08–47.49) | 41.6 | -6.4% | within noise |
| echo iouring 1/32/64 @100000 | server cpu s | 2.282 (2.251–2.316) | 2.337 | +2.4% | outside range |
| echo native-poll 1/1/64 @1 idle | server cpu ms | 6.318 (2.967–7.488) | 3.344 | -47.1% | within noise |
| echo iouring 1/1/64 @35000 | p50 us | 17.07 (14.7–17.63) | 17.38 | +1.8% | within noise |
| echo iouring 1/1/64 @35000 | p99 us | 74.17 (65.02–103.3) | 74.69 | +0.7% | within noise |
| echo iouring 1/1/64 @35000 | server cpu s | 0.9174 (0.8947–0.9665) | 0.9343 | +1.8% | within noise |
| echo native-poll 1/32/64 @100000 | p50 us | 15.73 (15.62–15.97) | 15.81 | +0.6% | within noise |
| echo native-poll 1/32/64 @100000 | p99 us | 42.17 (38.08–47.49) | 40.99 | -2.8% | within noise |
| echo native-poll 1/32/64 @100000 | server cpu s | 2.109 (2.092–2.183) | 2.155 | +2.2% | within noise |
| timers iouring sleep yield 64/4 | ns/op | 124.3 (123.4–125.8) | 112.3 | -9.6% | within 3% |
| timers iouring sleep yield 64/4 | cpu ns/op | 496.5 (492.7–501.8) | 448.6 | -9.6% | within 3% |
| echo iouring-poll 4/128/64 | req/s | 4.488e+05 (4.362e+05–4.596e+05) | 4.497e+05 | +0.2% | within noise |
| echo iouring-poll 4/128/64 | p99 us | 579.1 (518.4–668.7) | 559.1 | -3.4% | within noise |
| echo iouring-poll 4/128/64 | server cpu us/req | 8.856 (8.601–8.986) | 8.811 | -0.5% | within noise |
| diag iouring 4/1024/1000us aligned | cpu s | 3.008 (2.864–3.116) | 2.796 | -7.0% | within 3% |
| diag iouring 4/1024/1000us aligned | late p50 us | 215.7 (113.4–253.6) | 219 | +1.5% | within 3% |
| diag iouring 4/1024/1000us aligned | late p99 us | 345.9 (212.7–356.1) | 336.1 | -2.8% | within 3% |
| echo iouring-poll 1/1/64 @1 idle | server cpu ms | 3.86 (3.759–4.372) | 3.119 | -19.2% | better |
| timers native sleep refresh 1/1 | ns/op | 214.3 (206.7–225) | 191.8 | -10.5% | within 3% |
| timers native sleep refresh 1/1 | cpu ns/op | 213.9 (206.3–224.6) | 191.5 | -10.5% | within 3% |
| echo native 1/32/64 @100000 | p50 us | 15.84 (15.77–16.31) | 16.03 | +1.2% | within noise |
| echo native 1/32/64 @100000 | p99 us | 46.11 (38.4–54.17) | 42.66 | -7.5% | within noise |
| echo native 1/32/64 @100000 | server cpu s | 2.394 (2.349–2.477) | 2.415 | +0.9% | within noise |
| echo native-poll 4/128/4096 | req/s | 3.828e+05 (3.731e+05–3.876e+05) | 3.851e+05 | +0.6% | within noise |
| echo native-poll 4/128/4096 | p99 us | 671.7 (643.1–790.5) | 656.9 | -2.2% | within noise |
| echo native-poll 4/128/4096 | server cpu us/req | 9.924 (9.798–9.989) | 9.906 | -0.2% | within noise |
| echo native-poll 4/128/64 @220000 | p50 us | 36.35 (35.87–37.41) | 36.19 | -0.4% | within noise |
| echo native-poll 4/128/64 @220000 | p99 us | 258 (181–908.8) | 270.6 | +4.9% | within noise |
| echo native-poll 4/128/64 @220000 | server cpu s | 8.291 (8.175–8.326) | 8.372 | +1.0% | outside range |
| echo iouring 1/1/64 | req/s | 7.906e+04 (7.677e+04–8.064e+04) | 7.867e+04 | -0.5% | within noise |
| echo iouring 1/1/64 | p99 us | 23.04 (18.54–27.86) | 24.82 | +7.7% | within noise |
| echo iouring 1/1/64 | server cpu us/req | 8.154 (7.934–8.507) | 8.372 | +2.7% | within noise |
| echo iouring 4/128/4096 @190000 | p50 us | 40.06 (39.55–40.99) | 40.35 | +0.7% | within noise |
| echo iouring 4/128/4096 @190000 | p99 us | 184.2 (118.1–331.5) | 151.4 | -17.8% | within noise |
| echo iouring 4/128/4096 @190000 | server cpu s | 7.993 (7.925–8.122) | 8.128 | +1.7% | outside range |
| diag native 4/1024/1000us | cpu s | 10.03 (7.814–10.3) | 8.8 | -12.3% | within 3% |
| diag native 4/1024/1000us | late p50 us | 63.97 (53.82–73.92) | 80.45 | +25.8% | over 3% |
| diag native 4/1024/1000us | late p99 us | 234.8 (213–242.2) | 242.7 | +3.4% | over 3% |
| echo native 4/128/64 | req/s | 4.421e+05 (4.318e+05–4.481e+05) | 4.436e+05 | +0.3% | within noise |
| echo native 4/128/64 | p99 us | 581.1 (536.1–660) | 577.5 | -0.6% | within noise |
| echo native 4/128/64 | server cpu us/req | 9.086 (8.966–9.223) | 9.098 | +0.1% | within noise |
| rb task-sleep | ns/op | 1.079e+06 (1.062e+06–1.089e+06) | 1.075e+06 | -0.4% | within noise |
| rb task-sleep | cpu ns/op | 2.4e+05 (1.629e+05–3.064e+05) | 2.33e+05 | -2.9% | within noise |
| rb task-sleep | late p50 us | 66.87 (52.82–88.48) | 65.91 | -1.4% | within noise |
| rb task-sleep | late p99 us | 154.9 (137.7–158.8) | 150.1 | -3.1% | within noise |
| timers native reuse refresh 1/1 | ns/op | 128.6 (124.5–130.3) | 126.3 | -1.8% | within 3% |
| timers native reuse refresh 1/1 | cpu ns/op | 128.4 (124.3–129.8) | 126 | -1.9% | within 3% |
| echo iouring 4/128/64 @220000 | p50 us | 36.99 (36.67–37.85) | 37.28 | +0.8% | within noise |
| echo iouring 4/128/64 @220000 | p99 us | 174.5 (115.3–759.3) | 237.8 | +36.3% | within noise |
| echo iouring 4/128/64 @220000 | server cpu s | 8.516 (8.337–8.678) | 8.617 | +1.2% | within noise |
| echo iouring-poll 1/32/64 @100000 | p50 us | 15.78 (15.65–15.95) | 15.86 | +0.5% | within noise |
| echo iouring-poll 1/32/64 @100000 | p99 us | 40.77 (38.11–51.39) | 40.41 | -0.9% | within noise |
| echo iouring-poll 1/32/64 @100000 | server cpu s | 2.194 (2.153–2.224) | 2.209 | +0.7% | within noise |
| echo native-poll 1/1/64 @35000 | p50 us | 16.99 (14.46–17.47) | 17.5 | +3.0% | outside range |
| echo native-poll 1/1/64 @35000 | p99 us | 75.78 (62.14–1061) | 69.95 | -7.7% | within noise |
| echo native-poll 1/1/64 @35000 | server cpu s | 0.8178 (0.8032–0.8628) | 0.8165 | -0.2% | within noise |
| timers native reuse expire 128/4 | ns/op | 8514 (8162–8609) | 8507 | -0.1% | within 3% |
| timers native reuse expire 128/4 | cpu ns/op | 3734 (1322–4808) | 3744 | +0.3% | within 3% |
| timers native reuse expire 128/4 | late p50 us | 78.35 (17.63–93.75) | 71.25 | -9.1% | within 3% |
| timers native reuse expire 128/4 | late p99 us | 185.2 (169.3–211) | 176 | -5.0% | within 3% |
| echo iouring-poll 4/128/4096 | req/s | 3.819e+05 (3.761e+05–3.866e+05) | 3.792e+05 | -0.7% | within noise |
| echo iouring-poll 4/128/4096 | p99 us | 669.7 (606.7–688.6) | 671.7 | +0.3% | within noise |
| echo iouring-poll 4/128/4096 | server cpu us/req | 10.27 (10.18–10.41) | 10.4 | +1.2% | within noise |
| timers native reuse yield 64/4 | ns/op | 105.2 (104.3–107.9) | 104.5 | -0.6% | within 3% |
| timers native reuse yield 64/4 | cpu ns/op | 419.5 (416.8–430.6) | 417.1 | -0.6% | within 3% |
| timers iouring sleep refresh 1/1 | ns/op | 213.2 (205.7–221.8) | 201.9 | -5.3% | within 3% |
| timers iouring sleep refresh 1/1 | cpu ns/op | 212.8 (205.3–221.1) | 201.2 | -5.4% | within 3% |
| timers native sleep expire 128/4 | ns/op | 8551 (8336–8670) | 8502 | -0.6% | within 3% |
| timers native sleep expire 128/4 | cpu ns/op | 3897 (1196–6254) | 2915 | -25.2% | within 3% |
| timers native sleep expire 128/4 | late p50 us | 81.25 (34.23–96.9) | 72.48 | -10.8% | within 3% |
| timers native sleep expire 128/4 | late p99 us | 183.8 (147.3–205.8) | 186.3 | +1.4% | within 3% |

1170 samples; 19 metrics outside budget:
- timers native reuse cycle 64/4: late p50 us
- timers native reuse cycle 64/4: late p99 us
- timers iouring reuse cycle 64/4: cpu ns/op
- diag native 4/1024/1000us aligned: late p50 us
- timers iouring sleep cycle 64/4: late p50 us
- diag iouring 4/1024/1000us: late p50 us
- diag iouring 1/1/10us: late p99 us
- echo native 1/1/64 @35000: server cpu s
- rb pulse: cpu_ms
- diag native 1/1/10us: late p50 us
- timers native sleep cycle 64/4: cpu ns/op
- timers native sleep cycle 64/4: late p50 us
- echo iouring 1/32/64 @100000: p50 us
- echo iouring 1/32/64 @100000: server cpu s
- echo native-poll 4/128/64 @220000: server cpu s
- echo iouring 4/128/4096 @190000: server cpu s
- diag native 4/1024/1000us: late p50 us
- diag native 4/1024/1000us: late p99 us
- echo native-poll 1/1/64 @35000: p50 us
