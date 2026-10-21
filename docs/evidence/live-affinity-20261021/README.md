# Controlled real affinity lifecycle

PASS at clean committed source `9130ea1`, with matching binary revision, on
Linux 6.18.55. Ran as the normal user without eBPF or elevated privilege.
The helper contained a parent and two child processes, totaling five tasks.
The recorded binary digest and toolchain/source metadata identify this run.

For all five task identities, the original kernel affinity was `0-11`.
The audited dry-run left masks unchanged and created no restore file. Apply
restricted them to CPU `0`, selected dynamically from the allowed online set.
The suite verified the original masks and task/process identities in durable
rollback records, checked applied kernel masks through procfs, rolled back, and
independently verified every original mask again before disposing of the tree.
Ten audit events cover preflight, dry-run, apply, verify, rollback and final
readback. The pre-rollback journal snapshot is retained alongside the receipt.

Original full local bundle: `/tmp/stutter-affinity-process-tree-20261021`.
Published metadata redacts builder user/host identities; audit restore paths
are replaced by `<acceptance-output>/restore.json`. No process command lines
or environment dump are included. PID/start-time values remain to bind evidence.

This proves the success path on this host, not controller SIGKILL recovery,
verification/rollback fault injection, stale-PID recovery, suspend/resume,
multiple kernels, overhead, calibration or provider graduation. Those gates
remain outstanding. See `docs/LIVE_ACCEPTANCE.md` to reproduce at the recorded
source with a new output directory; host masks and PIDs may differ.
