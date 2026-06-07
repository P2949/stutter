# Autotune Implementation Checklist

This checklist applies to every new autotune action. It exists to keep autotune work tied to `TuningAction`, rollback, auditability, cooldown, and tests instead of random knob-twiddling.

For FYP supervisor review, this is a future implementation checklist for
experimental autotune work, not the current assessed delivery plan. The proposed
FYP scope remains CPU-affinity/process-placement validation unless supervision
explicitly changes it.

For every new autotune action:

```text
- [ ] Has TuningAction implementation
- [ ] Has preflight
- [ ] Has dry-run
- [ ] Has apply
- [ ] Has verify
- [ ] Has rollback
- [ ] Has safety class
- [ ] Has cooldown
- [ ] Has audit event
- [ ] Has fake/sysfs test
- [ ] Has failure-injection test
- [ ] Is disabled by default unless low risk
```
