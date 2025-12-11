# Scripts

The repo keeps a small set of supported helper scripts. Prefer these canonical
entries and treat any missing/legacy copies as archived (see
`docs/examples/legacy_examples.md`).

| Script | Description |
| --- | --- |
| `clean_output.sh` / `.ps1` | Remove generated artifacts under `soulbrowser-output/`, `tmp/`, and stale `plan*.json`. |
| `cleanup_profiles.sh` / `.ps1` | Delete leftover `.soulbrowser-profile-*` directories used by Chrome profiles. |
| `perception_bench.sh` | Run a structured/shared perception benchmark (20 iterations per mode) and write `soulbrowser-output/perf/perception.csv`. |

> `perception_benchmark.sh` has been removed in favor of `perception_bench.sh`. If
> you scripted against the old name, update your tooling to call
> `perception_bench.sh`.
