# Perception Benchmark Guide

This guide explains how to compare pooled (`shared`) vs. ephemeral perception runs using the
`scripts/perception_bench.sh` helper. It records the duration for each run, so you can track the
impact of `SOULBROWSER_DISABLE_PERCEPTION_POOL` and capture before/after data when tuning the
service.

## 1. Prerequisites

- Chrome/Chromium installed locally (`SOULBROWSER_USE_REAL_CHROME=1`).
- `cargo` available (the script shells out to `cargo run -- perceive`).
- Bash environment (WSL, macOS, Linux, or Git Bash on Windows).

## 2. Run the benchmark

```bash
# From repo root; defaults to https://example.com
scripts/perception_bench.sh

# Specify a different URL
scripts/perception_bench.sh https://www.wikipedia.org/
```

What the script does:

1. Creates `soulbrowser-output/perf/perception.csv`.
2. Runs 20 pooled (`shared`) perceives, then 20 ephemeral runs (`SOULBROWSER_DISABLE_PERCEPTION_POOL=1`).
3. Parses the `PERCEPTION_DURATION_MS=...` log line emitted by the CLI and appends it to the CSV.

Sample output:

```
mode,iteration,duration_ms
shared,1,2431.55
...
ephemeral,20,4560.32
Benchmark complete → soulbrowser-output/perf/perception.csv
```

## 3. Compare results

Open the CSV in your favorite tool or use Python:

```python
import pandas as pd
from pathlib import Path

df = pd.read_csv(Path('soulbrowser-output/perf/perception.csv'))
summary = df.groupby('mode')['duration_ms'].agg(['mean', 'median', 'min', 'max'])
print(summary)
```

This lets you quantify the pooled speed-up (e.g., `shared` average 2.4s vs. `ephemeral` 4.6s). Add the
plot/table to release notes when reporting optimizations.

## 4. Tips

- Warm up Chrome once before running the script to avoid first-run penalties.
- Use the same URL and timeout for apples-to-apples comparison.
- If a run fails, the script aborts—rerun after addressing the failure (e.g., flaky URL).
- You can edit `RUNS` inside the script for longer tests. Increase to 50 when collecting final numbers.
