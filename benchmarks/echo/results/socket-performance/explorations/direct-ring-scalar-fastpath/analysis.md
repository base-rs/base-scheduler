# Read+write fastpath rejected

Idle zero ticks. Conformance 180/180 passed. Three randomized serial repetitions: original 236,525; write-only 229,215; read+write 205,630; read+write vs original -13.06%. Read-first typically adds an unsuccessful syscall before ring submission. Reverted to write-only candidate for broad matrix.
