# Autotune Implementation Checklist

This checklist applies to every new autotune action. It exists to keep autotune work tied to `TuningAction`, rollback, auditability, cooldown, and tests instead of random knob-twiddling.

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
