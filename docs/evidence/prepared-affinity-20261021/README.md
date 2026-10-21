# Auxiliary prepared-affinity syscall evidence

At clean source `7c64bcf872dd4274e2f896842923d605f6710930`, the explicit owned-child
test passed under strace 7.2 with seccomp filtering on Linux 6.18.55. The test
checked Applying(token), complete original identity/mask data and an unchanged
kernel mask inside the preparation callback. It then observed actual apply and
strict startup restore with independent original-mask readback and a clean journal.

[The receipt](result.json) binds source, binaries, lockfile and private raw trace
by SHA-256. Trace lines 4–10 prove snapshot file sync/rename/parent sync, prepared
journal file sync/rename/parent sync, then the first successful affinity syscall.
The trace has exactly two successful affinity syscalls: apply and restore.

The first ordinary ptrace run failed safely with no effective plan before mutation;
it remains retained locally. The successful run used seccomp syscall filtering.
This receipt is auxiliary functional ordering evidence. It is not a canonical
portable verified bundle, real SIGKILL acceptance, or provider graduation.
