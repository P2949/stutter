# Real-world stack artifact notes

The real-world stack comparison uses six valid `stutter` analysis artifacts:

- clean-01
- clean-02
- clean-03
- personal-stack-01
- personal-stack-02
- personal-stack-03

All six runs contain ingested MangoHud frame timing data in their committed
`*-analysis.json` outputs and passed the basic validity gates: full-duration
recording, `max_duration_reached`, non-zero frame count, and
`monotonic_observed` frame timestamp alignment.

The raw MangoHud CSV used by `clean-01` and `clean-02` was no longer available
when the artifact archive was finalized:

```text
/home/p2949/.local/state/stutter/mangohud-kcd-clean/KingdomCome_2026-06-04_13-41-07.csv
````

This does not invalidate the analysis results, because the frame timing data was
already ingested into the committed `stutter` analysis JSON files. However, it
means the raw MangoHud import source for those two runs is not preserved as a
standalone CSV artifact.

The remaining available raw MangoHud CSVs are archived under:

```text
reports/kcd1-case-study/realworld-stack/mangohud/
```

This real-world stack section should therefore be treated as an exploratory
comparison backed primarily by the committed `stutter` run directories and
analysis JSONs, not as a fully raw-source-replayable benchmark package.
