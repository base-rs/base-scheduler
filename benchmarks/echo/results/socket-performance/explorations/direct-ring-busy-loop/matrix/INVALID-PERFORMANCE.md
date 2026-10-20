# Invalid performance candidate

The original benchmark binary (19e11b1b8e9309fcf6f04117fd62294e8f309d7fd0abe8d620595bccde8c18b5) has an idle busy loop: a notification CQE rearms PollAdd while eventfd remains readable. The resulting throughput measurements do not establish an acceptable performance improvement. Preserve the original source.patch, benchmark, control, matched/, matrix/, syscalls/, and idle.json as rejected experimental evidence. Corrected production source and results will be stored separately at /home/samuel/.cache/core-scheduler-direct-notification-production.
